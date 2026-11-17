// SPDX-License-Identifier: MIT OR Apache-2.0
const assert = require("node:assert/strict");
const {resolve} = require("node:path");
const {nativeProbe, actor} = require("./sdk-native.cjs");
const core = require("../sdk/typescript/pkg/nodejs/booknook_wasm.js");
const {publicGroupPreviews} = require(resolve(process.argv[2]));

const owner = actor(2);
const viewer = actor(4);
const group = owner + ":preview-sdk";
let publisher = core.new_social_state();
let reader = core.new_social_state();

// Separate desks exercise actual signature approval and receiving-side admission.
function deliver(action, at) {
  const prepared = core.social_prepare(publisher, owner, JSON.stringify(action), BigInt(at));
  const event = JSON.parse(prepared);
  const proof = nativeProbe({op:"receipt", event, seed:2});
  const policy = JSON.stringify(proof.policy);
  const approved = JSON.parse(core.social_approve(publisher, prepared, proof.request,
    proof.presentation, policy, BigInt(at)));
  publisher = JSON.stringify(approved.state);
  const received = JSON.parse(core.social_receive(reader, JSON.stringify(approved.publication),
    policy, BigInt(at)));
  assert.equal(received.changed, true);
  reader = JSON.stringify(received.state);
  return event.id;
}

deliver({op:"create_group", id:group, name:"Preview SDK", description:"Discussion", private:false}, 2000);
const post = {kind:"note", book:null, body:"The ending", stars:null, reading_status:null,
  spoiler:true, reply_to:null};
const id = deliver({op:"publish", group_id:group, post}, 2001);
const row = {id, actor:owner, at:2001, group_id:group, post, spoiler_hidden:false};
const state = JSON.parse(reader);
assert.deepEqual(publicGroupPreviews(core, state, viewer),
  [{...row, post:{...post, body:""}, spoiler_hidden:true}]);
assert.deepEqual(publicGroupPreviews(core, state, viewer, true), [row]);
assert.deepEqual(JSON.parse(core.social_feed(reader, viewer, true)), []);
assert.deepEqual(publicGroupPreviews(core, state, owner), []);
deliver({op:"retract", event_id:id}, 2002);
assert.deepEqual(publicGroupPreviews(core, JSON.parse(reader), viewer, true), []);
console.log("Signed public group previews passed through the TypeScript and Node/WASM APIs.");
