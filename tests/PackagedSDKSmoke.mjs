// SPDX-License-Identifier: MIT OR Apache-2.0
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {createRequire} from 'node:module';
import {BookNook, recommendations, prepareSyncBatch, verifyCastaliaReceipt} from '@booknook/sdk';
import * as sdk from '@booknook/sdk';
import * as nodeCore from '@booknook/sdk/wasm/node';
import initWeb, * as webCore from '@booknook/sdk/wasm/web';
const require = createRequire(import.meta.url);
const {genericFixture}=require('./castalia-generic-fixture.cjs');
const {runProviderAcceptance}=require('./provider-service-acceptance.cjs');
await initWeb({module_or_path: await readFile(require.resolve('@booknook/sdk/wasm/booknook_wasm_bg.wasm'))});
for (const core of [nodeCore, webCore]) {
  console.log(await runProviderAcceptance(sdk,core));
  const f=genericFixture();
  assert.equal(verifyCastaliaReceipt(core,f.request,f.presentation,f.expectedRequest,f.disclosure,f.policy,f.now).holder_public_key,f.policy.holder_public_key);
  assert.throws(()=>verifyCastaliaReceipt(core,f.request,f.presentation,f.expectedRequest,f.disclosure,{...f.policy,pins:[]},f.now));
  let value = null;
  const desk = await BookNook.open(core, {
    async load() { return structuredClone(value); },
    async compareAndSave(expected, next) {
      if ((value?.revision ?? null) !== expected) return false;
      value = structuredClone(next); return true;
    }
  });
  const {state} = await desk.execute({op:'add_book',title:'Installed ESM 日本語',author:'Fixture',at:1});
  assert.equal(Object.values(state.books)[0].title, 'Installed ESM 日本語');
  assert.equal(recommendations(core,{candidates:[{book_id:'catalog:one',title:'Forest',subjects:['nature']}],
    preferred_subjects:['nature'],shared_ratings:[],friends:[],excluded:[]})[0].score,2);
  const archive={schema:'booknook.portable-export.v1',state,assets:{}};
  assert.deepEqual(sdk.validatePortableArchive(core,JSON.stringify(archive)),archive);
  assert.deepEqual(JSON.parse(sdk.roundtripPortableArchive(core,sdk.encodePortableArchive(core,archive))),archive);
  assert.throws(()=>sdk.validatePortableArchive(core,JSON.stringify({...archive,settings:{}})));
  assert.equal(prepareSyncBatch(core,'holder',7n,['device'],new Uint8Array([0,255])).schema,'booknook.sync-batch.v1');
}
console.log('Installed ESM facade and both independent Node/browser WASM runtimes work.');
