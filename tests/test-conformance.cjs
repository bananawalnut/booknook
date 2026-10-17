// SPDX-License-Identifier: MIT OR Apache-2.0
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const core = require("../sdk/typescript/pkg/nodejs/booknook_wasm.js");
const {nativeProbe} = require("./sdk-native.cjs");
const bundle = JSON.parse(fs.readFileSync(path.resolve(__dirname, "../ontology/v0.1.0/conformance.json"), "utf8"));
assert.equal(bundle.schema, "booknook.conformance.v1");
let checked = 0;
for (const fixture of bundle.cases) {
  if (fixture.contract === "identity") continue; // Rust checks the typed Rolodex contract.
  const raw = fixture.raw_json ?? JSON.stringify(fixture.input);
  const run = () => fixture.contract === "state"
    ? core.validate_archive(raw)
    : JSON.parse(core.execute(JSON.stringify(fixture.state), raw));
  if (fixture.valid) {
    const result = run();
    if (fixture.contract === "command") assert.deepEqual(result, nativeProbe({op:"execute", state:fixture.state, command:fixture.input}), fixture.id);
    else assert.doesNotThrow(() => nativeProbe({op:"normalize", state:fixture.input}), fixture.id);
  } else assert.throws(run, undefined, fixture.id);
  checked++;
}
assert.equal(checked, 13);
console.log(`Published conformance: ${checked} portable state/command vectors pass WASM; native valid-command results agree. Rust additionally validates all 20 vectors including Rolodex alias and invalid identities.`);
