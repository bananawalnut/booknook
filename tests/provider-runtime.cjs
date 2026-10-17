// SPDX-License-Identifier: MIT OR Apache-2.0
const path=require('node:path');
const sdk=require(path.resolve(process.argv[2]??'target/sdk-check/index.js'));
const core=require('../sdk/typescript/pkg/nodejs/booknook_wasm.js');
const {nativeProbe}=require('./sdk-native.cjs');
const {runProviderAcceptance}=require('./provider-service-acceptance.cjs');
const assert=require('node:assert/strict');
const fixtures=require('../ontology/providers/v1/conformance.json');
for(const test of fixtures.cases) {
  const validate=()=>core.provider_validate_reply(JSON.stringify(fixtures.registry),1n,JSON.stringify(test.request),
    test.response_json,Buffer.from(test.content_base64,'base64'),BigInt(test.now));
  if(test.valid)assert.doesNotThrow(validate,test.name);else assert.throws(validate,undefined,test.name);
}
console.log(`${fixtures.cases.length} published provider fixtures passed the actual WASM parser.`);
runProviderAcceptance(sdk,core,nativeProbe).then(console.log).catch(error=>{console.error(error);process.exitCode=1;});
