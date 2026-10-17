// SPDX-License-Identifier: MIT OR Apache-2.0
// Build the native contract-probe example and Node WASM package, then compile index.ts to CommonJS.
const assert = require("node:assert/strict");
const path = require("node:path");
const {BookNook, BookNookSocial, audioEditionCandidates, reviewedAudioObservation,
  recommendations, socialRecommendations, prepareSyncBatch, sealSyncBatch, openSyncBatch} = require(path.resolve(process.argv[2]));
const core = require("../sdk/typescript/pkg/nodejs/booknook_wasm.js");
const {nativeProbe, inkCommand, actor} = require("./sdk-native.cjs");

function atomicStorage() {
  let value = null;
  return {
    rejectNext: false, writes: 0,
    async load() { return structuredClone(value); },
    async compareAndSave(expected, next) {
      if (this.rejectNext) { this.rejectNext = false; return false; }
      if ((value?.revision ?? null) !== expected) return false;
      value = structuredClone(next); this.writes++; return true;
    }
  };
}

(async () => {
  let stored = null;
  const storage = {
    async load() { return structuredClone(stored); },
    async compareAndSave(expected, value) {
      if ((stored?.revision ?? null) !== expected) return false;
      stored = structuredClone(value); return true;
    }
  };
  const desk = await BookNook.open(core, storage);
  const results = await Promise.allSettled([
    desk.execute({op:"add_book", title:"First", author:"Test", at:1}),
    desk.execute({op:"add_book", title:"Second", author:"Test", at:1})
  ]);
  assert.equal(results.filter(r => r.status === "fulfilled").length, 1);
  assert.equal(Object.keys(stored.books).length, 1);
  await assert.rejects(desk.execute({op:"add_book", title:"Unsafe number", author:"Test", at:Number.MAX_SAFE_INTEGER + 1}), /cannot represent exactly/);
  assert.equal(Object.keys(stored.books).length, 1);
  const passID = Object.keys(stored.passes)[0];
  const startSession = await desk.execute({op:"start_session", pass_id:passID, at:10});
  const sessionID = startSession.result.session_id;
  const reopenedDesk = await BookNook.open(core, storage);
  assert.equal(stored.sessions[sessionID].ended_at, null);
  await assert.rejects(reopenedDesk.execute({op:"end_session", session_id:sessionID, at:9}), /interval/);
  assert.equal(stored.sessions[sessionID].ended_at, null);
  await reopenedDesk.execute({op:"end_session", session_id:sessionID, at:20});
  assert.equal(stored.sessions[sessionID].ended_at, 20);
  const place = await reopenedDesk.execute({op:"create_literary_entity", book_id:Object.keys(stored.books)[0], kind:"place", name:"The café"});
  assert.equal(stored.entities[place.result.entity_id].kind, "place");
  const metadataBook = Object.values(stored.books)[0];
  const audio = {asin:"B012345678",position_milliseconds:12345,source_updated_at:10,fetched_at:20,
    duration_milliseconds:50000,edition_identifier:null,alignment:"unmatched",metadata:{title:metadataBook.title,authors:metadataBook.authors}};
  const candidates = audioEditionCandidates(core,stored,audio,20n);
  assert.ok(candidates.length > 0);
  const reviewed = reviewedAudioObservation(core,stored,audio,passID,candidates[0].edition_id,20n);
  assert.equal(reviewed.location.milliseconds,12345);
  assert.equal(reviewed.alignment.status,"confirmed");
  await reopenedDesk.execute({op:"observe_position",observation:reviewed});
  assert.deepEqual(stored.positions[reviewed.id].audio_metadata,audio.metadata);
  assert.throws(()=>audioEditionCandidates(core,stored,audio,-1n),/unsigned/);
  assert.throws(()=>reviewedAudioObservation(core,stored,audio,passID,undefined,1n<<64n),/unsigned/);
  // Exercise the actual WASM-backed facade with atomic storage failures, then reopen it.
  const inkStore = atomicStorage();
  const inkDesk = await BookNook.open(core,inkStore);
  let inkState = (await inkDesk.execute({op:"add_book",title:"Canvas beside prose",author:"Fixture",at:1})).state;
  const inkNotebook = Object.values(inkState.notebooks)[0];
  const blockID = inkNotebook.blocks[0].id;
  inkState = (await inkDesk.execute({op:"edit_block",notebook_id:inkNotebook.id,block_id:blockID,
    base_update:inkNotebook.blocks[0].update,text:"Typed words remain 🌿",at:2})).state;
  const originalProse = structuredClone(inkState.notebooks[inkNotebook.id].blocks[0]);
  assert.equal(inkState.schema,"booknook.archive.v1");
  const draw = inkCommand(inkNotebook.id,blockID,"storage-ink",[],3);
  await inkDesk.execute(draw);
  const firstSaved = await inkStore.load();
  const erase = inkCommand(inkNotebook.id,blockID,"storage-erase",[draw.version_id],4,true);
  inkStore.rejectNext = true;
  await assert.rejects(inkDesk.execute(erase),/Storage conflict/);
  assert.deepEqual(await inkStore.load(),firstSaved,"Rejected storage commit retains original ink and no ghost erase version");
  const inkReopened = await BookNook.open(core,inkStore);
  await inkReopened.execute(erase);
  const transcript = {op:"save_notebook_transcription",notebook_id:inkNotebook.id,block_id:blockID,
    transcription_id:"storage-transcript",ink_version_id:draw.version_id,text:"  Reviewed café\n日本語  ",method:"manual",at:5};
  await inkReopened.execute(transcript);
  const inkCommitted = await inkStore.load();
  assert.equal(inkCommitted.schema,"booknook.archive.v2");
  const {ink:committedInk,...preservedProse} = inkCommitted.notebooks[inkNotebook.id].blocks[0];
  assert.deepEqual(preservedProse,originalProse);
  assert.equal(committedInk.versions.length,2);
  assert.deepEqual(committedInk.versions[0],firstSaved.notebooks[inkNotebook.id].blocks[0].ink.versions[0]);
  assert.deepEqual(committedInk.versions[1].strokes,[]);
  assert.equal(committedInk.transcriptions[0].text,transcript.text);
  const reopenedInk = await BookNook.open(core,inkStore);
  assert.equal((await reopenedInk.execute(draw)).result.replayed,true);
  assert.equal((await reopenedInk.execute(transcript)).result.replayed,true);
  assert.deepEqual(await inkStore.load(),inkCommitted);
  await assert.rejects(reopenedInk.execute({...draw,canvas_height:321}),/reused/);
  assert.deepEqual(await inkStore.load(),inkCommitted);
  console.log("Typed SDK ink: failed CAS recovery, immutable erase, exact reviewed transcription, unchanged prose and idempotent reopen passed.");
  const socialStore = atomicStorage();
  const social = new BookNookSocial(core, socialStore);
  // Independent hosts race to create one store, then race on its same revision.
  const otherHost = new BookNookSocial(core, socialStore);
  await Promise.all([social.state(), otherHost.state()]);
  assert.equal(socialStore.writes, 1);
  const changes = await Promise.allSettled([
    social.local({op:"set_subjects", subjects:["history"]}),
    otherHost.local({op:"set_subjects", subjects:["poetry"]}),
  ]);
  assert.equal(changes.filter(r => r.status === "fulfilled").length, 1);
  assert.equal(changes.filter(r => r.status === "rejected").length, 1);
  assert.equal((await social.state()).revision, 1);

  const now = 1000n;
  const action = {op:"publish", group_id:null, post:{kind:"note", book:null,
    body:"A deliberately shared 🌿 passage", stars:null, reading_status:null, spoiler:false, reply_to:null}};
  const beforePrepare = await social.state();
  const event = await social.prepare(actor(2), action, now);
  assert.deepEqual(await social.state(), beforePrepare, "Preparing does not publish or persist");
  await social.local({op:"save_draft", event});
  const proof = nativeProbe({op:"receipt", event, seed:2});
  const beforeApproval = await social.state();
  socialStore.rejectNext = true;
  await assert.rejects(social.approve(event, proof.request, proof.presentation, proof.policy, now), /store changed/);
  assert.deepEqual(await social.state(), beforeApproval, "Failed CAS retains draft and consumes no receipt");
  assert.equal(Object.keys(beforeApproval.outbox).length, 0);

  const publication = await social.approve(event, proof.request, proof.presentation, proof.policy, now);
  const committed = await social.state();
  assert.deepEqual(committed.outbox[event.id].publication, publication);
  assert.deepEqual(committed.publications[event.id], publication);
  assert.equal(Object.keys(committed.replay).length, 1);
  assert.equal(committed.drafts[event.id], undefined);

  const receiverStore = atomicStorage();
  const receiver = new BookNookSocial(core, receiverStore);
  assert.equal(await receiver.receive(publication, proof.policy, now), true);
  const writes = receiverStore.writes;
  assert.equal(await receiver.receive(publication, proof.policy, now + 600n), false);
  assert.equal(receiverStore.writes, writes, "Historical duplicate performs no storage write");
  const receiverState = await receiver.state();
  const altered = structuredClone(publication);
  altered.event.action.post.body = "A forged selection";
  await assert.rejects(receiver.receive(altered, proof.policy, now));
  assert.deepEqual(await receiver.state(), receiverState);
  const revoked = structuredClone(proof.policy); revoked.pins = [];
  await assert.rejects(receiver.receive(publication, revoked, now), undefined, "Duplicate still checks current issuer trust");

  // wasm-bindgen would otherwise wrap u64 BigInt arguments modulo 2^64.
  for (const invalid of [-1n, (1n << 64n) + now, 1000]) {
    await assert.rejects(social.prepare(actor(2), action, invalid), /unsigned 64-bit BigInt/);
    await assert.rejects(social.approve(event, proof.request, proof.presentation, proof.policy, invalid), /unsigned 64-bit BigInt/);
    await assert.rejects(receiver.receive(publication, proof.policy, invalid), /unsigned 64-bit BigInt/);
  }
  await assert.rejects(social.local({op:"save_draft", event:{...event, at:Number.MAX_SAFE_INTEGER + 1}}), /cannot represent exactly/);
  assert.deepEqual(await social.state(), committed);
  socialStore.rejectNext = true;
  await assert.rejects(social.local({op:"cancel_delivery", event_id:event.id}), /store changed/);
  assert.deepEqual(await social.state(), committed, "Failed cancellation CAS leaves the queue intact");
  const cancelled = await social.local({op:"cancel_delivery", event_id:event.id});
  assert.equal(cancelled.outbox[event.id], undefined);
  assert.deepEqual(cancelled.publications, committed.publications, "Cancellation preserves published history");
  assert.deepEqual(cancelled.replay, committed.replay, "Cancellation preserves consumed receipts");
  const catalog = [{book_id:'catalog:one',title:'Forest',subjects:['nature']}];
  const input = {candidates:catalog,preferred_subjects:['nature'],shared_ratings:[],friends:[],excluded:[]};
  assert.equal(recommendations(core,input)[0].score,2);
  assert.throws(()=>recommendations(core,{...input,notebooks:['private']}));
  const socialWithInterests = await social.local({op:'set_subjects',subjects:['nature']});
  assert.equal(socialRecommendations(core,socialWithInterests,catalog,[],proof.policy,2000n)[0].score,2);
  assert.throws(()=>socialRecommendations(core,socialWithInterests,catalog,[],{...proof.policy,pins:[]},2000n));
  const bytes = Uint8Array.from([0,255,...Buffer.from('Exact private 🌿')]);
  const key = new Uint8Array(32).fill(8);
  const batch = prepareSyncBatch(core,actor(2),7n,['device'],bytes);
  const syncProof = nativeProbe({op:'sync_receipt',batch});
  const sealed = sealSyncBatch(core,batch,bytes,key,syncProof.request,syncProof.presentation,syncProof.policy,110n);
  const opened = openSyncBatch(core,sealed.packet,key,'device',syncProof.policy,10000n);
  assert.ok(opened.plaintext instanceof Uint8Array);
  assert.deepEqual(opened.plaintext,bytes);
  assert.equal(opened.replay_key,sealed.replay_key);
  assert.match(opened.packet_digest,/^[0-9a-f]{64}$/);
  assert.throws(()=>sealSyncBatch(core,batch,bytes,key,syncProof.request,syncProof.presentation,syncProof.policy,161n));
  assert.throws(()=>openSyncBatch(core,sealed.packet,new Uint8Array(31),'device',syncProof.policy,10000n),/32 bytes/);
  for (const invalid of [-1n,1n<<64n,110]) {
    assert.throws(()=>prepareSyncBatch(core,actor(2),invalid,['device'],bytes),/unsigned 64-bit BigInt/);
    assert.throws(()=>sealSyncBatch(core,batch,bytes,key,syncProof.request,syncProof.presentation,syncProof.policy,invalid),/unsigned 64-bit BigInt/);
    assert.throws(()=>openSyncBatch(core,sealed.packet,key,'device',syncProof.policy,invalid),/unsigned 64-bit BigInt/);
    assert.throws(()=>socialRecommendations(core,socialWithInterests,catalog,[],proof.policy,invalid),/unsigned 64-bit BigInt/);
  }
  assert.throws(()=>prepareSyncBatch(core,actor(2),1n<<53n,['device'],bytes),/cannot represent exactly/);
  console.log('Typed recommendation and approved encrypted sync facade validates receipts, binary bytes and numeric/key bounds.');
  console.log("Typed SDK notebook/social CAS, durable publication, replay, issuer revocation and numeric-boundary checks passed.");
})().catch(error => { console.error(error); process.exitCode = 1; });
