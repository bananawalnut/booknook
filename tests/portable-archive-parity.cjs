// SPDX-License-Identifier: MIT OR Apache-2.0
// Whole native archive crosses WASM, then restores through the real native content/SQL path.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {execFileSync,spawnSync} = require('node:child_process');
const core = require('../sdk/typescript/pkg/nodejs/booknook_wasm.js');
const sdk = require(path.resolve(process.argv[2] ?? 'target/sdk-check/index.js'));
const root=path.resolve(__dirname,'..');
const temp=fs.mkdtempSync(path.join(os.tmpdir(),'booknook-full-archive-'));
const cli=(desk,...args)=>execFileSync(path.join(root,'target/debug/booknook'),[desk,...args],{encoding:'utf8',maxBuffer:16*1024*1024});
try {
  const fixturePath=execFileSync(path.join(root,'target/debug/examples/portable-fixture'),[temp],{encoding:'utf8'}).trim();
  const original=fs.readFileSync(fixturePath,'utf8');
  const value=JSON.parse(original);
  assert.equal(value.state.schema,'booknook.archive.v2');
  const notebook=Object.values(value.state.notebooks)[0], ink=notebook.blocks[0].ink;
  assert.equal(ink.versions.length,3); assert.equal(ink.transcriptions.length,1);
  assert.equal(Object.keys(value.assets).length,2);
  assert.equal(value.social.bindings.length,1); assert.equal(value.social.pending.length,1);
  assert.ok(Object.keys(value.social.state.outbox).length>0);
  assert.ok(Object.keys(value.social.state.lists).length>0);
  const raw=sdk.roundtripPortableArchive(core,original);
  assert.deepEqual(JSON.parse(raw),value,'whole envelope including assets/social must survive WASM');
  assert.deepEqual(sdk.validatePortableArchive(core,raw),value);
  assert.deepEqual(JSON.parse(sdk.encodePortableArchive(core,value)),value);
  const candidate=path.join(temp,'wasm.json'); fs.writeFileSync(candidate,raw);
  const restored=path.join(temp,'restored'); cli(restored,'restore',candidate);
  const reexport=path.join(temp,'reexport.json');cli(restored,'export',reexport);
  const restoredValue=JSON.parse(fs.readFileSync(reexport,'utf8'));
  restoredValue.state.revision=value.state.revision;
  assert.deepEqual(restoredValue,value,'native/WASM/native includes exact native drawing, CRDT, cards, proofs, queue and bytes');
  for (const edition of Object.values(value.state.editions)) for (const asset of edition.assets) {
    const name=asset.media_type==='application/pdf'?'book.pdf':'original.epub';
    assert.deepEqual(fs.readFileSync(path.join(restored,'assets',asset.storage_key,name)),Buffer.from(value.assets[asset.id],'base64'));
  }
  const legacy={schema:'booknook.portable-export.v1',state:JSON.parse(core.new_state()),assets:{}};
  assert.deepEqual(JSON.parse(sdk.roundtripPortableArchive(core,JSON.stringify(legacy))),legacy);
  const wide=JSON.stringify(legacy).replace('"revision":0','"revision":18446744073709551615');
  assert.ok(sdk.roundtripPortableArchive(core,wide).includes('18446744073709551615'));
  assert.throws(()=>sdk.validatePortableArchive(core,wide),/cannot represent exactly/);
  const duplicate=original.replace('"schema":','"schema":"booknook.portable-export.v1","schema":');
  const failures=[['duplicate-key',duplicate]];
  for (const [name,mutate] of [
    ['unknown-envelope',x=>x.settings={issuer:'must not be imported'}],
    ['missing-asset',x=>delete x.assets[Object.keys(x.assets)[0]]],
    ['changed-asset',x=>x.assets[Object.keys(x.assets)[0]]=Buffer.from('tampered').toString('base64')],
    ['invalid-base64',x=>x.assets[Object.keys(x.assets)[0]]='%%%'],
    ['unknown-social',x=>x.social.private_key='must not be accepted'],
    ['forged-pending',x=>{const event=JSON.parse(x.social.pending[0].event_json);event.sig='00'.repeat(64);x.social.pending[0].event_json=JSON.stringify(event);}],
    ['pending-id',x=>x.social.pending[0].id='00'.repeat(32)],
    ['duplicate-binding',x=>x.social.bindings.push(structuredClone(x.social.bindings[0]))],
    ['oversize-version',x=>x.social.bindings[0].minimum_version=0],
    ['wrong-transcript-source',x=>Object.values(x.state.notebooks)[0].blocks[0].ink.transcriptions[0].ink_version_id='absent'],
  ]) {const bad=structuredClone(value);mutate(bad);failures.push([name,JSON.stringify(bad)]);}
  for(const [name,bad] of failures){
    assert.throws(()=>sdk.roundtripPortableArchive(core,bad),undefined,name);
    const dir=path.join(temp,'rejected-'+name), file=path.join(temp,name+'.json');fs.writeFileSync(file,bad);
    const before=cli(dir,'snapshot');
    const result=spawnSync(path.join(root,'target/debug/booknook'),[dir,'restore',file],{encoding:'utf8'});
    assert.notEqual(result.status,0,name);assert.equal(cli(dir,'snapshot'),before,name+' rejection must not change native state');
  }
  const fixture=fs.readFileSync(path.join(root,'ontology/portable/v1/conformance.json'),'utf8');
  assert.equal(sdk.validatePortableArchive(core,fixture).schema,'booknook.portable-export.v1');
  console.log('Full native → WASM → native assets/ink/card/social/proof/queue roundtrip, legacy v1/u64 preservation, and 11 atomic rejection cases passed.');
} finally {fs.rmSync(temp,{recursive:true,force:true});}
