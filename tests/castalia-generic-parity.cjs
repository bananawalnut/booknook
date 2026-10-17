// SPDX-License-Identifier: MIT OR Apache-2.0
const assert=require('node:assert/strict');
const path=require('node:path');
const sdk=require(path.resolve(process.argv[2] ?? 'target/sdk-check/index.js'));
const core=require('../sdk/typescript/pkg/nodejs/booknook_wasm.js');
const {nativeProbe}=require('./sdk-native.cjs');
const {genericFixture}=require('./castalia-generic-fixture.cjs');
const f=genericFixture();
const verified=sdk.verifyCastaliaReceipt(core,f.request,f.presentation,f.expectedRequest,f.disclosure,f.policy,f.now);
assert.equal(verified.holder_public_key,f.policy.holder_public_key);
assert.equal(verified.expires_at,161);
assert.equal(verified.replay_key,f.policy.holder_public_key+':'+ '22'.repeat(16));
const probe={op:'castalia_verify',request:f.request,presentation:f.presentation,expected_request:f.expectedRequest,
  disclosure:JSON.stringify(f.disclosure),policy:JSON.stringify(f.policy),now:Number(f.now)};
assert.deepEqual(verified,nativeProbe(probe));
assert.equal(sdk.validateCastaliaRequest(core,f.request,f.expectedRequest,f.disclosure,f.policy,f.now).request_bytes_base64,
  Buffer.from(f.expectedRequest).toString('base64'));
let checked=0;
for (const change of [
  {expected_request:f.expectedRequest.replace('test:one','test:two')},
  {expected_request:' '+f.expectedRequest},
  {disclosure:JSON.stringify({...f.disclosure,title:'Different disclosure'})},
  {policy:JSON.stringify({...f.policy,holder_public_key:'00'.repeat(32)})},
  {policy:JSON.stringify({...f.policy,expected_caller:{kind:'terminal',id:'another-app'}})},
  {policy:JSON.stringify({...f.policy,pins:[]})},
  {policy:JSON.stringify({...f.policy,policy_digest:'00'.repeat(32)})},
  {now:161},
  {presentation:f.presentation.replace(/"signature":"[a-f0-9]+"/,'"signature":"'+ '00'.repeat(64)+'"')},
  {request:f.request.replace('{','{"schema":"duplicate",')},
]) {
  const v={...probe,...change};
  assert.throws(()=>core.verify_castalia_receipt(v.request,v.presentation,v.expected_request,v.disclosure,v.policy,BigInt(v.now)));
  assert.throws(()=>nativeProbe(v));checked++;
}
assert.throws(()=>sdk.verifyCastaliaReceipt(core,f.request,f.presentation,f.expectedRequest,f.disclosure,f.policy,1n<<64n));
assert.throws(()=>sdk.verifyCastaliaReceipt(core,f.request,f.presentation,f.expectedRequest,f.disclosure,f.policy,-1n));
console.log(`Generic Castalia second-application verification agrees across native/WASM; ${checked} changed-binding/expiry/forgery cases rejected. No custody secret enters WASM.`);
