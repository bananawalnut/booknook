// SPDX-License-Identifier: MIT OR Apache-2.0
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const {pathToFileURL} = require("node:url");

async function loadWasm(file) {
  const absolute = path.resolve(file);
  const pkg = JSON.parse(fs.readFileSync(path.join(path.dirname(absolute), "package.json"), "utf8"));
  if (pkg.type !== "module") return require(absolute);
  const wasm = await import(pathToFileURL(absolute).href);
  const module = fs.readFileSync(absolute.replace(/\.js$/, "_bg.wasm"));
  wasm.initSync({module});
  return wasm;
}

function runNotebookOrderConformance(wasm, corpus) {
  assert.equal(corpus.ontology_version, "0.4.0");
  assert.ok(corpus.cases.some(test => test.contract === "state" && test.raw_json));
  for (const test of corpus.cases) {
    const raw = test.raw_json ?? JSON.stringify(test.input);
    const verify = {
      state: () => wasm.validate_archive(raw),
      portable: () => wasm.validate_portable_archive(raw),
      command: () => wasm.execute(JSON.stringify(test.state), raw),
    }[test.contract];
    assert.equal(typeof verify, "function", test.contract);
    if (test.valid) assert.doesNotThrow(verify, test.id);
    else assert.throws(verify, test.id);
  }
}

async function runNotebookOrderSmoke(wasm, sdk, corpus) {
  runNotebookOrderConformance(wasm, corpus);
  const create = (state, command) => JSON.parse(wasm.execute(JSON.stringify(state), JSON.stringify(command)));
  const first = create(JSON.parse(wasm.new_state()), {
    op: "add_book", title: "Two readings", author: "Book Nook QA", at: 1,
  }).state;
  const notebookId = Object.keys(first.notebooks)[0];
  const legacy = create(first, {op: "add_block", notebook_id: notebookId, at: 2}).state;
  const ids = legacy.notebooks[notebookId].blocks.map(block => block.id);
  assert.equal(ids.length, 2);
  assert.deepEqual(sdk.notebookOrderReviews(wasm, legacy), []);
  const other = structuredClone(legacy);
  other.notebooks[notebookId].blocks.reverse();
  const merged = sdk.reconcile(wasm, null, legacy, other).state;
  assert.equal(merged.schema, "booknook.archive.v4");
  assert.equal(merged.ontology_version, "0.4.0");
  const [review] = sdk.notebookOrderReviews(wasm, merged);
  assert.equal(review.notebook_id, notebookId);
  assert.deepEqual(review.heads, []);
  assert.deepEqual(review.choices.map(JSON.stringify).sort(),
    [ids, ids.slice().reverse()].map(JSON.stringify).sort());
  const reviewCommand = {
    op: "review_notebook_order", notebook_id: notebookId,
    review_id: "portable-order-review", evidence: review.evidence, heads: review.heads,
    selected: ids.slice().reverse(),
  };
  let stored = merged;
  let saves = 0;
  const desk = await sdk.BookNook.open(wasm, {
    async load() { return structuredClone(stored); },
    async compareAndSave(expected, state) {
      if (stored.revision !== expected) return false;
      stored = structuredClone(state);
      saves++;
      return true;
    },
  });
  await assert.rejects(desk.execute({...reviewCommand, evidence: "stale"}));
  assert.equal(saves, 0);
  assert.deepEqual(stored, merged);
  const {state: settled} = await desk.execute(reviewCommand);
  assert.deepEqual(settled.notebooks[notebookId].blocks.map(block => block.id), ids.slice().reverse());
  assert.deepEqual(sdk.notebookOrderReviews(wasm, settled), []);
  const replayed = await desk.execute(reviewCommand);
  assert.equal(replayed.result.replayed, true);
  assert.equal(replayed.state.revision, settled.revision);

  const visible = settled.notebooks[notebookId].blocks.map(block => block.id);
  const append = {
    op: "append_ordered_note", notebook_id: notebookId, block_id: "portable-whiteboard",
    visible_block_ids: visible, at: 3, face: "whiteboard",
  };
  const {state: appended} = await desk.execute(append);
  assert.deepEqual(appended.notebooks[notebookId].blocks.map(block => block.id), [...visible, append.block_id]);
  assert.equal(appended.notebooks[notebookId].blocks.at(-1).note.face, "whiteboard");
  const archived = {schema: "booknook.portable-export.v1", state: appended, assets: {}};
  assert.deepEqual(sdk.validatePortableArchive(wasm, JSON.stringify(archived)).state, appended);
  assert.deepEqual(stored, appended);
  assert.deepEqual(sdk.notebookOrderReviews(wasm, appended), []);
  console.log("Portable review retrieval, exact replay, ordered Scribble append, and v4 archive round trip passed.");
}
module.exports = {runNotebookOrderSmoke};
if (require.main === module) {
  (async () => {
    const [wasmPath, sdkPath] = process.argv.slice(2);
    assert(wasmPath && sdkPath, "Pass a built WASM entry point and the emitted TypeScript SDK");
    globalThis.crypto ??= require("node:crypto").webcrypto;
    const corpus = require("../crates/core/tests/fixtures/notebook-order.json");
    await runNotebookOrderSmoke(await loadWasm(wasmPath), require(path.resolve(sdkPath)), corpus);
  })().catch(error => { console.error(error); process.exitCode = 1; });
}
