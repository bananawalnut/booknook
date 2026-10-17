// SPDX-License-Identifier: MIT OR Apache-2.0
const assert = require('node:assert/strict');
const {BookNook, validateProviderConnections, validateNativeAuthorityProfile, recommendations, prepareSyncBatch, verifyCastaliaReceipt} = require('@booknook/sdk');
const {genericFixture} = require('./castalia-generic-fixture.cjs');
const {runProviderAcceptance} = require('./provider-service-acceptance.cjs');
const core = require('@booknook/sdk/wasm');
(async () => {
  console.log(await runProviderAcceptance(require('@booknook/sdk'),core));
  const f=genericFixture();
  assert.equal(verifyCastaliaReceipt(core,f.request,f.presentation,f.expectedRequest,f.disclosure,f.policy,f.now).holder_public_key,f.policy.holder_public_key);
  assert.throws(()=>verifyCastaliaReceipt(core,f.request,f.presentation,f.expectedRequest,f.disclosure,{...f.policy,pins:[]},f.now));
  assert.deepEqual(validateProviderConnections(core, '[]'), []);
  assert.throws(() => validateProviderConnections(core, '[{"access_token":"never-a-connection"}]'));
  assert.throws(() => validateNativeAuthorityProfile(core, '{"schema":"castalia.native-authority-profile.v1","membership":"not-authority"}'));
  let value = null;
  const desk = await BookNook.open(core, {
    async load() { return structuredClone(value); },
    async compareAndSave(expected, next) {
      if ((value?.revision ?? null) !== expected) return false;
      value = structuredClone(next); return true;
    }
  });
  const {state} = await desk.execute({op:'add_book',title:'Installed CJS 🌿',author:'Fixture',at:1});
  assert.equal(Object.values(state.books)[0].title, 'Installed CJS 🌿');
  assert.equal(recommendations(core,{candidates:[{book_id:'catalog:one',title:'Forest',subjects:['nature']}],
    preferred_subjects:['nature'],shared_ratings:[],friends:[],excluded:[]})[0].score,2);
  assert.equal(prepareSyncBatch(core,'holder',7n,['device'],new Uint8Array([0,255])).schema,'booknook.sync-batch.v1');
  const archiveSDK=require('@booknook/sdk');
  const archive={schema:'booknook.portable-export.v1',state,assets:{}};
  assert.deepEqual(archiveSDK.validatePortableArchive(core,JSON.stringify(archive)),archive);
  assert.deepEqual(JSON.parse(archiveSDK.roundtripPortableArchive(core,archiveSDK.encodePortableArchive(core,archive))),archive);
  assert.throws(()=>archiveSDK.validatePortableArchive(core,JSON.stringify({...archive,settings:{}})));
  console.log('Installed CommonJS facade and Node WASM work.');
})().catch(error => { console.error(error); process.exitCode = 1; });
