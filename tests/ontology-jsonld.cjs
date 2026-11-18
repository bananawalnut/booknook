// SPDX-License-Identifier: MIT OR Apache-2.0
'use strict';

const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const { createHash } = require('node:crypto');
const { readFileSync } = require('node:fs');
const { resolve } = require('node:path');

const root = resolve(__dirname, '..');
const bookNookBase = 'https://bananawalnut.github.io/booknook/ontology/';
const castaliaBase = 'https://zenith-research.ca/ontology/v0.6.0/';
const castaliaGraph = castaliaBase + 'ontology.jsonld';
const directoryTerms = 'https://zenith-research.ca/ontology/terms/';
const bn = 'urn:booknook:ontology:';
const owl = 'http://www.w3.org/2002/07/owl#';
const rdfs = 'http://www.w3.org/2000/01/rdf-schema#';
const rdfType = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#type';
const proofId = 'urn:booknook:conformance:jsonld:';
const castaliaDigest = '11d670a57b2052b4de00509559d081b524bf953c6da2544a25f33976857f8f4f';

const moduleDirectory = process.argv[2];
assert(process.argv.length === 3 && moduleDirectory, 'Pass the standalone jsonld.js 9.0.0 module directory');
const jsonld = require(resolve(moduleDirectory));
const processorVersion = require(resolve(moduleDirectory, 'package.json')).version;
assert.equal(processorVersion, '9.0.0');
const sha256 = value => createHash('sha256').update(value).digest('hex');
const offline = name => readFileSync(resolve(root, 'ontology', name));

async function fetchAsset(url, limit = 1024 * 1024) {
  const response = await fetch(url, { redirect: 'error', signal: AbortSignal.timeout(15000) });
  assert.equal(response.status, 200, url);
  assert.equal(response.url, url);
  assert(response.body, url);
  const chunks = [];
  let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    assert(size <= limit, url + ' exceeded the asset size bound');
    chunks.push(chunk);
  }
  return { bytes: Buffer.concat(chunks, size), headers: response.headers };
}

async function verifyBookNookFiles(expected) {
  // Bound concurrent requests. The shipped discovery helper owns traversal and path validation.
  const entries = Object.entries(expected);
  for (let offset = 0; offset < entries.length; offset += 6) {
    await Promise.all(entries.slice(offset, offset + 6).map(async ([name, item]) => {
      const { bytes, headers } = await fetchAsset(bookNookBase + name);
      assert.equal(bytes.length, item.bytes, name);
      assert.equal(sha256(bytes), item.sha256, name);
      assert.equal(headers.get('access-control-allow-origin'), '*', name);
      const mime = name.endsWith('.jsonld') ? 'application/ld+json' : 'application/json';
      assert.equal(headers.get('content-type')?.split(';')[0], mime, name);
    }));
  }
}

function bookNookInventory() {
  const resolver = [
    'import json,sys',
    'sys.path.insert(0,"scripts")',
    'from ontology_artifacts import directory_inventory',
    'print(json.dumps(directory_inventory("ontology")))',
  ].join('\n');
  const result = spawnSync('python3', ['-B', '-c', resolver], { cwd: root, encoding: 'utf8', timeout: 15000 });
  assert.equal(result.status, 0, result.stderr || String(result.error));
  return JSON.parse(result.stdout);
}

async function verifyCastaliaManifest() {
  const manifestUrl = castaliaBase + 'manifest.json';
  const manifestBytes = (await fetchAsset(manifestUrl)).bytes;
  const manifest = JSON.parse(manifestBytes);
  assert.equal(manifest.version, '0.6.0');
  assert.equal(manifest.canonical_base_url + '/', castaliaBase);
  assert.equal(manifest.bundle_digest, castaliaDigest);
  assert.equal(manifest.bundle_digest_algorithm, 'sha256-file-list-v1');
  assert(manifest.files.length > 0 && manifest.files.length <= 64);
  const files = {};
  const jsonldFiles = {};
  for (const file of manifest.files) {
    assert(/^[A-Za-z0-9][A-Za-z0-9_.-]+$/.test(file.path) && !Object.hasOwn(files, file.path));
    assert.equal(file.url, castaliaBase + file.path);
    assert(Number.isSafeInteger(file.bytes) && file.bytes > 0 && file.bytes <= 1024 * 1024);
    const fetched = await fetchAsset(file.url);
    assert.equal(fetched.bytes.length, file.bytes, file.path);
    assert.equal(sha256(fetched.bytes), file.sha256, file.path);
    files[file.path] = { sha256: file.sha256, bytes: file.bytes };
    if (file.path.endsWith('.jsonld')) {
      assert.equal(fetched.headers.get('content-type')?.split(';')[0], 'application/ld+json', file.path);
      jsonldFiles[file.path] = JSON.parse(fetched.bytes);
    }
  }
  return { manifest_sha256: sha256(manifestBytes), advertised_bundle_digest: castaliaDigest,
    aggregate_recomputed: false, files, jsonldFiles };
}

async function main() {
  const catalog = bookNookInventory();
  assert.equal(catalog.current, '0.4.0');
  const discovery = JSON.parse(offline('discovery.json'));
  const releases = discovery.releases.map(({ version, manifest }) => {
    const pin = JSON.parse(offline(manifest)).castalia;
    assert.deepEqual(pin, { version: '0.6.0', canonical_base_url: castaliaBase.slice(0, -1), bundle_digest: castaliaDigest });
    const name = manifest.replace(/manifest\.json$/, 'ontology.jsonld');
    assert(Object.hasOwn(catalog.files, name));
    return { version, name };
  });
  const mapping = JSON.parse(offline('v0.1.0/castalia-mapping.json'));
  assert.equal(mapping.directory_namespace, directoryTerms);
  assert.equal(mapping.upstream_bundle_digest, castaliaDigest);
  const [{ jsonldFiles: castaliaDocuments, ...castalia }] = await Promise.all([
    verifyCastaliaManifest(),
    verifyBookNookFiles(catalog.files),
  ]);

  const ontologyNames = Object.keys(catalog.files).filter(name => name.endsWith('.jsonld'));
  const documents = new Map([
    ...ontologyNames.map(name => [bookNookBase + name, JSON.parse(offline(name))]),
    ...Object.entries(castaliaDocuments).map(([name, document]) => [castaliaBase + name, document]),
  ]);
  const defaultLoader = jsonld.documentLoaders.node({ secure: true, strictSSL: true, maxRedirects: 0 });
  const loaded = new Set();
  const documentLoader = async url => {
    const expected = documents.get(url);
    assert(expected, 'Unexpected JSON-LD document ' + url);
    const result = await defaultLoader(url);
    assert.equal(result.documentUrl, url);
    assert.equal(result.contextUrl, null);
    assert.deepEqual(result.document, expected, url);
    loaded.add(url);
    return result;
  };
  const rdfOptions = { documentLoader, processingMode: 'json-ld-1.1', safe: true };
  const rdf = input => jsonld.toRDF(input, rdfOptions);
  const canonize = input => jsonld.canonize(input, { ...rdfOptions, algorithm: 'RDFC-1.0', format: 'application/n-quads' });
  const localGraphs = {};
  const graphs = {};
  const subjects = (rows, predicate, subject) =>
    rows.filter(row => row.predicate.value === predicate && row.subject.value === subject).map(row => row.object.value).sort();

  for (const release of releases) {
    const url = bookNookBase + release.name;
    const fromNetwork = await canonize(url);
    const fromDisk = await canonize(JSON.parse(offline(release.name)));
    assert.equal(fromNetwork, fromDisk, release.name);
    localGraphs[release.version] = sha256(fromDisk);
    graphs[release.version] = await rdf(url);
  }

  const currentUrn = bn + 'release:' + discovery.current;
  const priorUrn = bn + 'release:0.3.0';
  const currentImports = subjects(graphs[discovery.current], owl + 'imports', currentUrn);
  assert.deepEqual(currentImports, [castaliaGraph, priorUrn].sort());
  assert(releases.some(entry => bn + 'release:' + entry.version === priorUrn));
  for (const type of ['NotebookOrder', 'NotebookAppend', 'NotebookOrderReview']) {
    assert(graphs[discovery.current].some(row => row.graph.value === currentUrn && row.subject.value === bn + type &&
      row.predicate.value === rdfType && row.object.value === owl + 'Class'), type);
  }

  const upstream = await rdf(castaliaGraph);
  assert.deepEqual(subjects(upstream, rdfType, directoryTerms + 'Entity'), [owl + 'Class']);
  for (const type of ['Person', 'Agent', 'Organization']) {
    assert(subjects(upstream, rdfs + 'subClassOf', directoryTerms + type).includes(directoryTerms + 'Entity'), type);
  }
  assert.deepEqual(subjects(upstream, owl + 'equivalentClass', directoryTerms + 'Actor'), [directoryTerms + 'Entity']);
  const combined = [...upstream, ...Object.values(graphs).flat()];
  for (const historical of ['0.1.0', '0.2.0', '0.3.0']) {
    const releaseId = bn + 'release:' + historical;
    assert.deepEqual(subjects(graphs[historical], rdfType, releaseId), [owl + 'Ontology']);
    assert.deepEqual(subjects(graphs[historical], owl + 'imports', releaseId), [castaliaGraph]);
    assert.deepEqual(subjects(graphs[historical], rdfs + 'subClassOf', bn + 'Character'), [bn + 'LiteraryEntity']);
    assert.deepEqual(subjects(graphs[historical], rdfs + 'subClassOf', bn + 'Annotation'), ['http://www.w3.org/ns/oa#Annotation']);
  }
  for (const edge of [rdfs + 'subClassOf', owl + 'equivalentClass']) {
    assert(!subjects(combined, edge, bn + 'Character').some(iri => iri.startsWith(directoryTerms)));
    assert(!subjects(combined, edge, directoryTerms + 'Person').includes(bn + 'Character'));
  }

  const fixture = {
    '@graph': [
      ...['Actor', 'Entity', 'Person', 'Agent', 'Organization', 'Character'].map(type => ({
        '@id': proofId + type, '@type': type,
      })),
      {
        '@id': proofId + 'Interpretation', '@type': 'CharacterInterpretation',
        reader: proofId + 'Person', character_id: proofId + 'Character',
        pass_id: proofId + 'ReadingPass', parents: [proofId + 'Earlier'],
      },
    ],
  };
  const contexts = {};
  for (const name of ontologyNames.filter(name => name.endsWith('/context.jsonld'))) {
    const liveContext = bookNookBase + name;
    const actual = await rdf({ '@context': liveContext, ...fixture });
    const expected = await rdf({ '@context': JSON.parse(offline(name))['@context'], ...fixture });
    assert.equal(await canonize({ '@context': liveContext, ...fixture }),
      await canonize({ '@context': JSON.parse(offline(name))['@context'], ...fixture }));
    assert.equal(actual.length, expected.length);
    for (const type of ['Actor', 'Entity', 'Person', 'Agent', 'Organization']) {
      assert.deepEqual(subjects(actual, rdfType, proofId + type), [directoryTerms + type]);
    }
    assert.deepEqual(subjects(actual, rdfType, proofId + 'Character'), [bn + 'Character']);
    const checks = {
      attributedReader: 'Person', describesCharacter: 'Character',
      readingPass: 'ReadingPass', derivedFromVersion: 'Earlier',
    };
    for (const [property, entity] of Object.entries(checks)) {
      assert(actual.some(row => row.subject.value === proofId + 'Interpretation' && row.predicate.value === bn + property &&
        row.object.termType === 'NamedNode' && row.object.value === proofId + entity), property);
    }
    contexts[name] = { checked_iri_relationships: Object.keys(checks).length, statements: actual.length };
  }
  assert.deepEqual([...loaded].sort(), [...documents.keys()].sort());
  const receipt = {
    schema: 'booknook.remote-jsonld-acceptance.v1',
    result: 'PASS',
    observed_utc: new Date().toISOString(),
    processor: { package: 'jsonld', version: processorVersion, node: process.versions.node, rdf_canonicalization: 'RDFC-1.0' },
    booknook: { discovery: bookNookBase + 'discovery.json', current: catalog.current, exact_files: Object.keys(catalog.files).length,
      jsonld_documents_loaded: ontologyNames.length, graph_nquads_sha256: localGraphs, current_imports: currentImports, contexts },
    castalia: { ontology: castaliaGraph, ...castalia, file_count: Object.keys(castalia.files).length },
    scope: 'Actual HTTPS node document loader and explicitly resolved import catalog. Role equivalence and namespace relationships were checked as RDF statements. The advertised Zenith bundle digest and each individual file hash were checked.',
  };
  process.stdout.write(JSON.stringify(receipt, null, 2) + '\n');
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
