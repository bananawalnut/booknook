// SPDX-License-Identifier: MIT OR Apache-2.0
const assert = require('node:assert/strict');
module.exports = function portableServicesParity(core, nativeProbe, actor) {
  const input = {candidates:[{book_id:'catalog:one',title:'A forest',subjects:['Nature','nature']}],
    preferred_subjects:['NATURE'],shared_ratings:[],friends:[],excluded:[]};
  const recommend = raw => JSON.parse(core.recommendations(raw));
  assert.deepEqual(recommend(JSON.stringify(input)), nativeProbe({op:'recommendations',raw:JSON.stringify(input)}));
  assert.deepEqual(recommend(JSON.stringify(input)), [{book_id:'catalog:one',score:2,reasons:['Matches your interest in nature']}]);
  for (const raw of [JSON.stringify({...input,notebooks:['private text']}), JSON.stringify(input).replace('{','{"candidates":[],')]) {
    assert.throws(()=>recommend(raw)); assert.throws(()=>nativeProbe({op:'recommendations',raw}));
  }
  let state = JSON.parse(core.new_social_state());
  state = JSON.parse(core.social_local(JSON.stringify(state), JSON.stringify({op:'set_subjects',subjects:['nature']})));
  let viewerPolicy;
  for (const [seed,action,at] of [
    [2,{op:'friendship',peer:actor(3),requested:true},1000],
    [3,{op:'friendship',peer:actor(2),requested:true},1001],
    [3,{op:'publish',group_id:null,post:{kind:'rating',book:{id:'catalog:one',title:'A forest',author:'Fixture'},
      body:'',stars:5,reading_status:null,spoiler:false,reply_to:null}},1002],
  ]) {
    const event = JSON.parse(core.social_prepare(JSON.stringify(state),actor(seed),JSON.stringify(action),BigInt(at)));
    const proof = nativeProbe({op:'receipt',event,seed});
    if (seed === 2) viewerPolicy = proof.policy;
    state = JSON.parse(core.social_approve(JSON.stringify(state),JSON.stringify(event),proof.request,proof.presentation,JSON.stringify(proof.policy),BigInt(at))).state;
  }
  const socialRecommend = (s, policy) => JSON.parse(core.social_recommendations(JSON.stringify(s),JSON.stringify(input.candidates),'[]',JSON.stringify(policy),5000n));
  const nativeSocial = (s,policy) => nativeProbe({op:'social_recommendations',state:s,candidates:input.candidates,excluded:[],policy,now:5000});
  assert.deepEqual(socialRecommend(state,viewerPolicy),nativeSocial(state,viewerPolicy));
  assert.equal(socialRecommend(state,viewerPolicy)[0].score,7);
  const altered = structuredClone(state); altered.friendship_intents = {};
  assert.throws(()=>socialRecommend(altered,viewerPolicy)); assert.throws(()=>nativeSocial(altered,viewerPolicy));
  const revoked = {...viewerPolicy,pins:[]};
  assert.throws(()=>socialRecommend(state,revoked)); assert.throws(()=>nativeSocial(state,revoked));
  const blocked = {...state,blocked:[actor(3)]};
  assert.deepEqual(socialRecommend(blocked,viewerPolicy),[{book_id:'catalog:one',score:2,reasons:['Matches your interest in nature']}]);
  assert.deepEqual(socialRecommend(blocked,viewerPolicy),nativeSocial(blocked,viewerPolicy));

  const plaintext = Uint8Array.from([0,255,...Buffer.from('Private café 🌿\n')]);
  const key = new Uint8Array(32).fill(8);
  const recipient = 'device-日本語';
  const batch = JSON.parse(core.prepare_sync_batch(actor(2),7n,JSON.stringify([recipient]),plaintext));
  const nativeBatch = nativeProbe({op:'sync_prepare',actor:actor(2),revision:7,recipients:[recipient],plaintext:Array.from(plaintext)});
  assert.notEqual(batch.id,nativeBatch.id);
  assert.deepEqual({...batch,id:null},{...nativeBatch,id:null});
  const proof = nativeProbe({op:'sync_receipt',batch});
  const sealed = JSON.parse(core.seal_sync_batch(JSON.stringify(batch),plaintext,key,proof.request,proof.presentation,JSON.stringify(proof.policy),110n));
  const nativeSealed = nativeProbe({op:'sync_seal',batch,plaintext:Array.from(plaintext),key:Array.from(key),...proof,now:110});
  assert.deepEqual(Object.keys(sealed.packet).sort(),['schema','envelope','approved_at','request_json','presentation_json'].sort());
  assert.equal(sealed.packet.schema,'booknook.sync-packet.v1');
  assert.equal(sealed.replay_key,nativeSealed.replay_key);
  assert.notEqual(sealed.packet.envelope.nonce,nativeSealed.packet.envelope.nonce);
  const wasmOpen = (raw,k=key,p=proof.policy,who=recipient,at=10000n) => JSON.parse(core.open_sync_batch(raw,k,who,JSON.stringify(p),at));
  const nativeOpen = (raw,k=key,p=proof.policy,who=recipient,at=10000) => nativeProbe({op:'sync_open',raw,key:Array.from(k),recipient:who,policy:p,now:at});
  for (const packet of [sealed.packet,nativeSealed.packet]) {
    const raw = JSON.stringify(packet);
    assert.deepEqual(wasmOpen(raw),nativeOpen(raw));
    assert.deepEqual(wasmOpen(raw).plaintext,Array.from(plaintext));
    assert.equal(wasmOpen(raw).replay_key,sealed.replay_key);
    assert.deepEqual(wasmOpen(raw),wasmOpen(raw),'Decode is pure; the host must commit replay consumption with applying state');
  }
  const raw = JSON.stringify(sealed.packet);
  const mutations = [
    {...sealed.packet,schema:'unrecognized'},
    {...sealed.packet,approved_at:161},
    {...sealed.packet,envelope:{...sealed.packet.envelope,ciphertext:'AA=='}},
    {...sealed.packet,envelope:{...sealed.packet.envelope,batch:{...batch,recipients:['stranger']}}},
    {...sealed.packet,request_json:proof.request.replace('{','{"schema":"duplicate",')},
    {...sealed.packet,presentation_json:JSON.stringify({...JSON.parse(proof.presentation),signature:'00'.repeat(64)})},
    {...sealed.packet,unknown:true},
  ].map(JSON.stringify);
  mutations.push(raw.replace('{','{"approved_at":110,'));
  for (const changed of mutations) { assert.throws(()=>wasmOpen(changed)); assert.throws(()=>nativeOpen(changed)); }
  for (const [k,p,who,at] of [
    [new Uint8Array(31),proof.policy,recipient,10000],
    [new Uint8Array(32),proof.policy,recipient,10000],
    [key,{...proof.policy,pins:[]},recipient,10000],
    [key,proof.policy,'stranger',10000],
    [key,proof.policy,recipient,109],
  ]) { assert.throws(()=>wasmOpen(raw,k,p,who,BigInt(at))); assert.throws(()=>nativeOpen(raw,k,p,who,at)); }
  for (const [bytes,at] of [[Uint8Array.from([1]),110],[plaintext,161]]) {
    assert.throws(()=>core.seal_sync_batch(JSON.stringify(batch),bytes,key,proof.request,proof.presentation,JSON.stringify(proof.policy),BigInt(at)));
    assert.throws(()=>nativeProbe({op:'sync_seal',batch,plaintext:Array.from(bytes),key:Array.from(key),...proof,now:at}));
  }
  assert.throws(()=>core.prepare_sync_batch(actor(2),7n,'[]',plaintext));
  assert.throws(()=>core.prepare_sync_batch(actor(2),7n,JSON.stringify([recipient,recipient]),plaintext));
  console.log('Native/WASM recommendation audit and approved encrypted packet parity, revocation, tampering and binary roundtrip passed.');
};
