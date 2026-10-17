// SPDX-License-Identifier: MIT OR Apache-2.0
// Real native Rust contract probe; no mocked core or signatures.
const path = require("node:path");
const {execFileSync} = require("node:child_process");
const {createPrivateKey, createPublicKey} = require("node:crypto");
function nativeProbe(input) {
  return JSON.parse(execFileSync(path.resolve(__dirname, "../target/debug/examples/contract-probe"), [], {
    input: JSON.stringify(input), encoding: "utf8", maxBuffer: 16 * 1024 * 1024,
  }));
}
function nativeCli(directory, args, input) {
  return execFileSync(path.resolve(__dirname, "../target/debug/booknook"), [directory, ...args], {
    input: input === undefined ? undefined : JSON.stringify(input),
    encoding: "utf8", maxBuffer: 16 * 1024 * 1024,
  });
}
// Portable synthetic strokes and opaque bytes exercise serialization, not PencilKit recognition.
function inkCommand(notebookID, blockID, versionID, parents, at, erase = false) {
  return {op:"set_notebook_ink", notebook_id:notebookID, block_id:blockID,
    version_id:versionID, parents, canvas_width:640, canvas_height:320,
    strokes:erase ? [] : [{points:[{x:12.5,y:20,pressure:0.5,time:0},
      {x:44,y:31.25,pressure:0.75,time:0.125}],color:"#315b48",width:2.5}],
    native_drawing:erase ? null : Buffer.from("Synthetic SDK opaque drawing bytes").toString("base64"), at};
}
// Public, repeatable synthetic Ed25519 identity matching fixtures/rust/social_receipts.rs.
function actor(seed) {
  const privateKey = createPrivateKey({key: Buffer.concat([
    Buffer.from("302e020100300506032b657004220420", "hex"), Buffer.alloc(32, seed),
  ]), format: "der", type: "pkcs8"});
  return createPublicKey(privateKey).export({format:"der", type:"spki"}).subarray(-32).toString("hex");
}
module.exports = {nativeProbe, nativeCli, inkCommand, actor};
