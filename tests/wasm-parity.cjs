// SPDX-License-Identifier: MIT OR Apache-2.0
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const {execFileSync} = require("node:child_process");
const core = require("../sdk/typescript/pkg/nodejs/booknook_wasm.js");
const {nativeProbe, nativeCli, inkCommand, actor} = require("./sdk-native.cjs");
const root = path.resolve(__dirname,"..");
const temp = fs.mkdtempSync(path.join(os.tmpdir(),"booknook-parity-"));
const cli = (...args) => execFileSync(path.join(root,"target/debug/booknook"),[temp,...args],{encoding:"utf8"});
try {
  require('./portable-services-parity.cjs')(core, nativeProbe, actor);
  require('./note-whiteboard-parity.cjs')(core);
  for (const [fixture, operation, validate, wrap] of [
    ['native-authority-profile-v1-cases.json', 'castalia_profile', raw => core.validate_native_authority_profile(raw), value => value],
    ['provider-connections-v1-cases.json', 'provider_connections', raw => core.validate_provider_connections(raw), value => [value]],
  ]) {
    const cases = JSON.parse(fs.readFileSync(path.join(root, 'crates/core/tests/fixtures/castalia', fixture), 'utf8')).cases;
    for (const example of cases) {
      const raw = JSON.stringify(wrap(example.value));
      if (example.valid) assert.deepEqual(JSON.parse(validate(raw)), nativeProbe({op: operation, raw}), example.name);
      else {
        assert.throws(() => validate(raw), example.name);
        assert.throws(() => nativeProbe({op: operation, raw}), example.name);
      }
    }
  }
  const repeated = '[{"schema":"castalia.provider-connection.v1","schema":"castalia.provider-connection.v1"}]';
  assert.throws(() => core.validate_provider_connections(repeated));
  assert.throws(() => nativeProbe({op:'provider_connections',raw:repeated}));
  console.log('70 shared Castalia fixtures and raw duplicate-key rejection agree across native/WASM.');
  let native = JSON.parse(cli("snapshot"));
  let wasm = JSON.parse(JSON.stringify(native));
  core.validate_archive(JSON.stringify(wasm));
  const commands = [{op:"import_goodreads",csv:fs.readFileSync(path.join(root,"fixtures/goodreads.csv"),"utf8"),at:100}];
  for (const command of commands) {
    native = JSON.parse(execFileSync(path.join(root,"target/debug/booknook"),[temp,"command"],{input:JSON.stringify(command),encoding:"utf8"})).state;
    wasm = JSON.parse(core.execute(JSON.stringify(wasm),JSON.stringify(command))).state;
    assert.deepEqual(wasm,native);
  }
  const book = Object.values(native.books)[0];
  const update = {op:"update_book",book_id:book.id,title:book.title,shelf:"read",rating:5,review:"Unicode 🌿 こんにちは"};
  native = JSON.parse(execFileSync(path.join(root,"target/debug/booknook"),[temp,"command"],{input:JSON.stringify(update),encoding:"utf8"})).state;
  wasm = JSON.parse(core.execute(JSON.stringify(wasm),JSON.stringify(update))).state;
  assert.deepEqual(wasm,native);
  const subjectUpdate = {op:"set_book_subjects", book_id:book.id, subjects:[" Literature ", "literature", "History"]};
  native = JSON.parse(execFileSync(path.join(root,"target/debug/booknook"),[temp,"command"],{input:JSON.stringify(subjectUpdate),encoding:"utf8"})).state;
  wasm = JSON.parse(core.execute(JSON.stringify(wasm),JSON.stringify(subjectUpdate))).state;
  assert.deepEqual(wasm,native);
  assert.deepEqual(new Set(wasm.books[book.id].subjects), new Set(["literature", "history"]));
  assert.ok(wasm.books[book.id].edited_fields.includes("subjects"));
  assert.throws(()=>core.execute(JSON.stringify(wasm),JSON.stringify({...update,admin:true})));
  core.validate_archive(JSON.stringify(JSON.parse(JSON.stringify(wasm))));
  // Both runtimes consume the same CRDT updates; random identity generation is not hidden
  // by rewriting document IDs or history. Only newly prepared event IDs vary by runtime.
  const execute = (state, command) => JSON.parse(core.execute(JSON.stringify(state), JSON.stringify(command))).state;
  let base = execute(JSON.parse(core.new_state()), {op:"add_book", title:"Concurrent readings", author:"Writer", at:1});
  const n = Object.values(base.notebooks)[0];
  const edit = (state, text, at) => execute(state, {op:"edit_block", notebook_id:n.id, block_id:n.blocks[0].id,
    base_update:state.notebooks[n.id].blocks[0].update, text, at});
  base = edit(base, "An impression", 2);
  base = execute(base, {op:"add_character", notebook_id:n.id, block_id:n.blocks[0].id, name:"Mara", at:3});
  const card = Object.values(base.cards)[0];
  const saveCard = (state, description, at) => execute(state, {op:"save_card", card_id:card.id,
    parents:[card.versions[0].id], interpretation:{...card.versions[0].interpretation, description}, positions:[], at});
  let left = saveCard(edit(base, "An impression from the beginning", 4), "Trustworthy in chapter 3", 6);
  let right = saveCard(edit(base, "My impression", 5), "Unreliable by chapter 12", 7);
  const bookID = Object.keys(base.books)[0];
  left.books[bookID].review = "Local interpretation";
  right.books[bookID].review = "Remote interpretation";
  const merged = JSON.parse(core.reconcile_states(JSON.stringify(base), JSON.stringify(left), JSON.stringify(right)));
  assert.deepEqual(merged, nativeProbe({op:"reconcile", base, local:left, incoming:right}));
  assert.match(merged.state.notebooks[n.id].blocks[0].text, /My/);
  assert.match(merged.state.notebooks[n.id].blocks[0].text, /from the beginning/);
  assert.equal(merged.state.cards[card.id].versions.length, 3);
  const reviewConflict = merged.conflicts.find(c => c.field === "review");
  assert.ok(reviewConflict);
  const resolved = JSON.parse(core.resolve_conflict(JSON.stringify(merged.state), JSON.stringify(reviewConflict), true));
  assert.deepEqual(resolved, nativeProbe({op:"resolve", state:merged.state, conflict:reviewConflict, use_incoming:true}));
  assert.equal(resolved.books[bookID].review, "Remote interpretation");
  const restored = JSON.parse(JSON.stringify(resolved));
  core.validate_archive(JSON.stringify(restored));
  assert.deepEqual(nativeProbe({op:"normalize", state:restored}), resolved, "CRDT and card histories survive export");

  // Selecting one interpretation creates an independent text draft, never a card/history export.
  for (const chosen of restored.cards[card.id].versions.filter(v => v.interpretation.description)) {
    const command = {op:"prepare_character_share", card_id:card.id, version_id:chosen.id, at:200};
    const w = JSON.parse(core.execute(JSON.stringify(restored),JSON.stringify(command)));
    const r = nativeProbe({op:"execute",state:restored,command});
    const wd = w.state.drafts[w.result.draft_id], rd = r.state.drafts[r.result.draft_id];
    assert.deepEqual({...wd,id:null},{...rd,id:null});
    assert.equal(wd.kind,"selected_note"); assert.equal(wd.book_id,null); assert.equal(wd.spoiler,true);
    assert.ok(wd.body.includes(chosen.interpretation.description)); assert.ok(wd.body.includes(base.books[bookID].title));
    for (const other of restored.cards[card.id].versions.filter(v => v.id !== chosen.id && v.interpretation.description)) assert.ok(!wd.body.includes(other.interpretation.description));
    for (const privateID of [card.id, n.id, n.pass_id, n.blocks[0].id, chosen.id, restored.local_reader]) assert.ok(!JSON.stringify(wd).includes(privateID));
    delete w.state.drafts[w.result.draft_id]; delete r.state.drafts[r.result.draft_id];
    assert.deepEqual(w.state,r.state); assert.deepEqual(w.state.cards,restored.cards);
  }
  assert.throws(() => core.execute(JSON.stringify(restored),JSON.stringify({op:"prepare_character_share",card_id:card.id,version_id:"unknown",at:200})));

  // The original v1 shape lacked rich-text styles and card position observations.
  const legacy = structuredClone(base);
  delete legacy.notebooks[n.id].blocks[0].styles;
  for (const version of legacy.cards[card.id].versions) delete version.position_observations;
  core.validate_archive(JSON.stringify(legacy));
  assert.deepEqual(nativeProbe({op:"normalize", state:legacy}), base);

  // New records legitimately get different random IDs. Compare that single generated
  // record independently, then compare every unchanged state field without rewriting IDs.
  let readingModel = structuredClone(base);
  const createReadingRecord = (command, collection, resultKey) => {
    const w = JSON.parse(core.execute(JSON.stringify(readingModel), JSON.stringify(command)));
    const r = nativeProbe({op:"execute", state:readingModel, command});
    const wid = w.result[resultKey], rid = r.result[resultKey];
    assert.match(wid, /^[0-9a-f]{32}$/); assert.match(rid, /^[0-9a-f]{32}$/);
    assert.deepEqual({...w.state[collection][wid], id:null}, {...r.state[collection][rid], id:null});
    const wRest = structuredClone(w.state), rRest = structuredClone(r.state);
    delete wRest[collection][wid]; delete rRest[collection][rid];
    assert.deepEqual(wRest, rRest);
    readingModel = w.state;
    return wid;
  };
  const sessionID = createReadingRecord({op:"start_session", pass_id:n.pass_id, at:20}, "sessions", "session_id");
  const endSession = {op:"end_session", session_id:sessionID, at:40};
  const ended = JSON.parse(core.execute(JSON.stringify(readingModel), JSON.stringify(endSession)));
  assert.deepEqual(ended, nativeProbe({op:"execute", state:readingModel, command:endSession}));
  readingModel = ended.state;
  assert.equal(readingModel.sessions[sessionID].ended_at, 40);
  for (const kind of ["place", "theme", "event", "concept"]) {
    const entityID = createReadingRecord({op:"create_literary_entity", book_id:bookID, kind, name:`  ${kind} café 日本語  `}, "entities", "entity_id");
    assert.equal(readingModel.entities[entityID].name, `${kind} café 日本語`);
  }
  const invalidInterval = structuredClone(readingModel);
  invalidInterval.sessions[sessionID].ended_at = 19;
  assert.throws(() => core.validate_archive(JSON.stringify(invalidInterval)));
  assert.throws(() => nativeProbe({op:"normalize", state:invalidInterval}));
  core.validate_archive(JSON.stringify(readingModel));
  assert.deepEqual(nativeProbe({op:"normalize", state:readingModel}), readingModel);
  const legacyUpdate = {op:"update_book", book_id:bookID, title:base.books[bookID].title, shelf:"reading", rating:null, review:"Legacy preserved"};
  assert.deepEqual(JSON.parse(core.execute(JSON.stringify(legacy), JSON.stringify(legacyUpdate))), nativeProbe({op:"execute", state:legacy, command:legacyUpdate}));

  // Native SQLite and WASM use exactly the same explicit immutable IDs. Native drawing
  // bytes here are an opaque synthetic payload; actual PencilKit rendering is tested separately.
  const inkDesk = path.join(temp,"ink-desk");
  let inkNative = JSON.parse(nativeCli(inkDesk,["command"],{op:"add_book",title:"Handwritten reading",author:"Fixture",at:1})).state;
  const inkNotebook = Object.values(inkNative.notebooks)[0];
  const inkBlock = inkNotebook.blocks[0];
  inkNative = JSON.parse(nativeCli(inkDesk,["command"],{op:"edit_block",notebook_id:inkNotebook.id,
    block_id:inkBlock.id,base_update:inkBlock.update,text:"  Typed café 🌿 stays.\n",at:2})).state;
  assert.equal(inkNative.schema,"booknook.archive.v1");
  assert.equal(inkNative.ontology_version,"0.1.0");
  assert.equal(inkNative.notebooks[inkNotebook.id].blocks[0].ink,undefined);
  core.validate_archive(JSON.stringify(inkNative));
  assert.deepEqual(nativeProbe({op:"normalize",state:inkNative}),inkNative);
  let inkWasm = structuredClone(inkNative);
  const prose = structuredClone(inkNative.notebooks[inkNotebook.id].blocks[0]);
  const currentInk = state => state.notebooks[inkNotebook.id].blocks[0].ink;
  const heads = ink => ink.versions.filter(v => !ink.versions.some(other => other.parents.includes(v.id))).map(v => v.id).sort();
  const applyInkBoth = command => {
    const w = JSON.parse(core.execute(JSON.stringify(inkWasm),JSON.stringify(command)));
    const r = JSON.parse(nativeCli(inkDesk,["command"],command));
    assert.deepEqual(w,r);
    inkWasm = w.state; inkNative = r.state;
    const {ink, ...stillTyped} = inkWasm.notebooks[inkNotebook.id].blocks[0];
    assert.deepEqual(stillTyped,prose,"Ink and transcription never change the prose CRDT or styles");
    return w.result;
  };
  const firstInk = inkCommand(inkNotebook.id,inkBlock.id,"sdk-ink-first",[],10);
  assert.deepEqual(applyInkBoth(firstInk),{ink_version_id:"sdk-ink-first",heads:["sdk-ink-first"],conflict:false,replayed:false});
  assert.equal(inkWasm.schema,"booknook.archive.v2");
  assert.equal(inkWasm.ontology_version,"0.2.0");
  const transcription = {op:"save_notebook_transcription",notebook_id:inkNotebook.id,block_id:inkBlock.id,
    transcription_id:"sdk-transcript",ink_version_id:"sdk-ink-first",text:"  Mara’s café — 日本語\nSecond line.\n",method:"on_device",at:11};
  applyInkBoth(transcription);
  assert.equal(currentInk(inkWasm).transcriptions[0].text,transcription.text,"Review text is not trimmed or rewritten");
  const inkBranchBase = structuredClone(inkWasm);
  const firstVersion = structuredClone(currentInk(inkWasm).versions[0]);
  const erasedInk = inkCommand(inkNotebook.id,inkBlock.id,"sdk-ink-erased",["sdk-ink-first"],12,true);
  applyInkBoth(erasedInk);
  assert.deepEqual(currentInk(inkWasm).versions[0],firstVersion,"Erasing is a new snapshot, not history deletion");
  assert.equal(currentInk(inkWasm).versions.length,2);
  assert.deepEqual(currentInk(inkWasm).versions[1].strokes,[]);
  const beforeRetries = structuredClone(inkWasm);
  assert.equal(applyInkBoth(firstInk).replayed,true);
  assert.equal(applyInkBoth(transcription).replayed,true);
  assert.deepEqual(inkWasm,beforeRetries,"Retries do not increment revision or duplicate history");
  for (const changed of [{...firstInk,canvas_width:641},{...transcription,text:"Reused ID must not overwrite"}]) {
    assert.throws(()=>core.execute(JSON.stringify(inkWasm),JSON.stringify(changed)),/reused/);
    assert.throws(()=>nativeCli(inkDesk,["command"],changed),/reused/);
    assert.deepEqual(JSON.parse(nativeCli(inkDesk,["snapshot"])),beforeRetries,"Native rejection is durable and atomic");
  }
  const inkArchivePath = path.join(temp,"ink-export.json");
  nativeCli(inkDesk,["export",inkArchivePath]);
  const inkArchive = JSON.parse(fs.readFileSync(inkArchivePath,"utf8"));
  assert.deepEqual(inkArchive.state,inkWasm);
  core.validate_archive(JSON.stringify(inkArchive.state));
  const restoredInkDesk = path.join(temp,"ink-restored");
  const restoredInk = JSON.parse(nativeCli(restoredInkDesk,["restore",inkArchivePath]));
  assert.deepEqual({...restoredInk,revision:inkWasm.revision},inkWasm,"Native export/reimport preserves both versions, transcript and original bytes");
  assert.deepEqual(JSON.parse(nativeCli(restoredInkDesk,["snapshot"])),restoredInk,"A new native process reopens the same ink");

  // Concurrent full-canvas edits remain independently visible heads, including an erase.
  const incomingInk = execute(inkBranchBase,inkCommand(inkNotebook.id,inkBlock.id,"sdk-ink-branch",["sdk-ink-first"],13));
  const inkMerge = JSON.parse(core.reconcile_states(JSON.stringify(inkBranchBase),JSON.stringify(inkWasm),JSON.stringify(incomingInk)));
  assert.deepEqual(inkMerge,nativeProbe({op:"reconcile",base:inkBranchBase,local:inkWasm,incoming:incomingInk}));
  assert.deepEqual(heads(currentInk(inkMerge.state)),["sdk-ink-branch","sdk-ink-erased"]);
  assert.equal(currentInk(inkMerge.state).versions.length,3);
  assert.deepEqual(currentInk(inkMerge.state).transcriptions,currentInk(inkWasm).transcriptions);
  const branchResult = JSON.parse(core.execute(JSON.stringify(inkMerge.state),JSON.stringify(erasedInk))).result;
  assert.equal(branchResult.conflict,true,"The public command result exposes unresolved branches");
  assert.deepEqual([...branchResult.heads].sort(),["sdk-ink-branch","sdk-ink-erased"]);
  const branchArchivePath = path.join(temp,"ink-branches.json");
  fs.writeFileSync(branchArchivePath,JSON.stringify({...inkArchive,state:inkMerge.state}));
  const branchDesk = path.join(temp,"ink-branches-restored");
  nativeCli(branchDesk,["restore",branchArchivePath]);
  const reopenedBranches = JSON.parse(nativeCli(branchDesk,["snapshot"]));
  assert.deepEqual({...reopenedBranches,revision:inkMerge.state.revision},inkMerge.state);
  core.validate_archive(JSON.stringify(reopenedBranches));
  assert.deepEqual(nativeProbe({op:"normalize",state:reopenedBranches}),reopenedBranches);
  console.log("Notebook ink native SQLite/WASM parity: legacy typed state, immutable erase/history, exact transcription, retries, export/reopen and exposed merge branches passed.");

  const audio = {asin:"B012345678",position_milliseconds:12345,source_updated_at:100,fetched_at:200,
    duration_milliseconds:50000,edition_identifier:null,alignment:"unmatched",
    metadata:{title:base.books[bookID].title,authors:base.books[bookID].authors}};
  const audioCandidates = JSON.parse(core.audio_edition_candidates(JSON.stringify(base),JSON.stringify(audio),200n));
  assert.deepEqual(audioCandidates,nativeProbe({op:"audio_candidates",state:base,observation:audio,now:200}));
  assert.ok(audioCandidates.length > 0);
  const candidateEdition = audioCandidates[0].edition_id;
  const reviewedAudio = JSON.parse(core.reviewed_audio_observation(JSON.stringify(base),JSON.stringify(audio),n.pass_id,candidateEdition,200n));
  assert.deepEqual(reviewedAudio,nativeProbe({op:"audio_review",state:base,observation:audio,pass_id:n.pass_id,confirmed_edition_id:candidateEdition,now:200}));
  assert.equal(reviewedAudio.location.milliseconds,12345);
  assert.equal(reviewedAudio.alignment.status,"confirmed");
  assert.deepEqual(reviewedAudio.audio_metadata,audio.metadata);
  const audioState = execute(base,{op:"observe_position",observation:reviewedAudio});
  assert.deepEqual(audioState,nativeProbe({op:"execute",state:base,command:{op:"observe_position",observation:reviewedAudio}}).state);
  assert.deepEqual(nativeProbe({op:"normalize",state:audioState}),audioState);
  const legacyAudio = {...audio}; delete legacyAudio.metadata;
  assert.deepEqual(JSON.parse(core.audio_edition_candidates(JSON.stringify(base),JSON.stringify(legacyAudio),200n)),[]);
  assert.throws(()=>core.reviewed_audio_observation(JSON.stringify(base),JSON.stringify(legacyAudio),n.pass_id,candidateEdition,200n));

  let social = JSON.parse(core.new_social_state());
  assert.deepEqual(social, nativeProbe({op:"social_new"}));
  for (const command of [{op:"set_subjects", subjects:["poetry","history"]}, {op:"block", actor:actor(3), blocked:true}]) {
    const next = JSON.parse(core.social_local(JSON.stringify(social), JSON.stringify(command)));
    assert.deepEqual(next, nativeProbe({op:"social_local", state:social, command})); social = next;
  }
  const action = {op:"publish", group_id:null, post:{kind:"note", book:null, body:"Selected 🌿 passage",
    stars:null, reading_status:null, spoiler:true, reply_to:null}};
  const prepared = JSON.parse(core.social_prepare(JSON.stringify(social), actor(2), JSON.stringify(action), 1000n));
  const nativePrepared = nativeProbe({op:"social_prepare", state:social, actor:actor(2), action, now:1000});
  assert.match(prepared.id, /^[0-9a-f]{32}$/); assert.match(nativePrepared.id, /^[0-9a-f]{32}$/);
  assert.notEqual(prepared.id, nativePrepared.id);
  assert.deepEqual({...prepared, id:null}, {...nativePrepared, id:null});
  const proof = nativeProbe({op:"receipt", event:prepared, seed:2});
  const approved = JSON.parse(core.social_approve(JSON.stringify(social), JSON.stringify(prepared), proof.request, proof.presentation, JSON.stringify(proof.policy), 1000n));
  assert.deepEqual(approved, nativeProbe({op:"social_approve", state:social, event:prepared, ...proof, now:1000}));
  const received = JSON.parse(core.social_receive(core.new_social_state(), JSON.stringify(approved.publication), JSON.stringify(proof.policy), 1600n));
  assert.deepEqual(received, nativeProbe({op:"social_receive", state:JSON.parse(core.new_social_state()), publication:approved.publication, policy:proof.policy, now:1600}));
  assert.equal(received.changed, true);
  const cancelDelivery = {op:"cancel_delivery", event_id:prepared.id};
  const cancelled = JSON.parse(core.social_local(JSON.stringify(approved.state), JSON.stringify(cancelDelivery)));
  assert.deepEqual(cancelled, nativeProbe({op:"social_local", state:approved.state, command:cancelDelivery}));
  assert.equal(cancelled.outbox[prepared.id], undefined);
  assert.deepEqual(cancelled.publications, approved.state.publications);
  const legacySocial = JSON.parse(core.new_social_state());
  delete legacySocial.join_requests;
  const subjects = {op:"set_subjects", subjects:["poetry"]};
  const migratedSocial = JSON.parse(core.social_local(JSON.stringify(legacySocial), JSON.stringify(subjects)));
  assert.deepEqual(migratedSocial, nativeProbe({op:"social_local", state:legacySocial, command:subjects}));
  assert.deepEqual(migratedSocial.join_requests, {});

  const approveBoth = (state, action, seed, now) => {
    const event = JSON.parse(core.social_prepare(JSON.stringify(state), actor(seed), JSON.stringify(action), BigInt(now)));
    const nativeEvent = nativeProbe({op:"social_prepare", state, actor:actor(seed), action, now});
    assert.deepEqual({...event, id:null}, {...nativeEvent, id:null});
    const proof = nativeProbe({op:"receipt", event, seed});
    const result = JSON.parse(core.social_approve(JSON.stringify(state), JSON.stringify(event), proof.request, proof.presentation, JSON.stringify(proof.policy), BigInt(now)));
    assert.deepEqual(result, nativeProbe({op:"social_approve", state, event, ...proof, now}));
    return result.state;
  };
  const retracted = approveBoth(approved.state, {op:"retract", event_id:prepared.id}, 2, 1001);
  assert.equal(retracted.outbox[prepared.id], undefined, "Retracting an offline publication removes its queued delivery");
  assert.ok(retracted.retracted.includes(prepared.id));
  assert.deepEqual(retracted.publications[prepared.id], approved.publication, "Retraction retains signed history");
  const groupID = `${actor(2)}:public-parity`;
  let publicGroup = approveBoth(JSON.parse(core.new_social_state()), {op:"create_group", id:groupID,
    name:"Public reading circle", description:"Shared evidence", private:false}, 2, 2000);
  publicGroup = approveBoth(publicGroup, {op:"request_join", group_id:groupID}, 4, 2001);
  assert.equal(Object.values(publicGroup.join_requests)[0].reader, actor(4));
  publicGroup = approveBoth(publicGroup, {op:"set_members", group_id:groupID,
    members:[actor(2), actor(4)].sort(), moderators:[actor(2)]}, 2, 2002);
  assert.ok(publicGroup.groups[groupID].members.includes(actor(4)));
  const privateID = `${actor(2)}:private-parity`;
  const privateDesks = new Map([2,3,4].map(seed => [seed, JSON.parse(core.new_social_state())]));
  const directedBoth = (seed, action, now) => {
    const before = privateDesks.get(seed);
    const after = approveBoth(before, action, seed, now);
    privateDesks.set(seed, after);
    const publication = Object.values(after.publications).find(p => !before.publications[p.event.id]);
    const proof = nativeProbe({op:"receipt", event:publication.event, seed});
    for (const [recipient, state] of privateDesks) {
      if (recipient === seed || !publication.event.delivery.recipients.includes(actor(recipient))) continue;
      const result = JSON.parse(core.social_receive(JSON.stringify(state), JSON.stringify(publication), JSON.stringify(proof.policy), BigInt(now)));
      assert.deepEqual(result, nativeProbe({op:"social_receive", state, publication, policy:proof.policy, now}));
      privateDesks.set(recipient, result.state);
    }
    return publication;
  };
  directedBoth(2, {op:"create_group", id:privateID, name:"Private circle", description:"Synthetic parity", private:true}, 3000);
  for (const seed of [3,4]) {
    directedBoth(2, {op:"invite", id:`private-invite-${seed}`, group:privateDesks.get(2).groups[privateID], invitee:actor(seed), expires_at:4000}, 3000+seed);
    const acceptance = directedBoth(seed, {op:"accept_invitation", invitation_id:`private-invite-${seed}`}, 3010+seed);
    assert.equal(acceptance.event.action.admission_consent.disclosure, "current_and_future_group_members");
  }
  assert.equal(privateDesks.get(3).invitations["private-invite-4"], undefined);
  assert.equal(privateDesks.get(4).invitations["private-invite-3"], undefined);
  const admission = directedBoth(2, {op:"set_members", group_id:privateID, members:[actor(2),actor(3),actor(4)].sort(), moderators:[]}, 3040);
  assert.equal(admission.event.action.admission_proofs.length, 2);
  for (const state of privateDesks.values()) assert.equal(state.groups[privateID].members.length, 3);
  console.log("Native/WASM imports, Unicode, Yrs reconciliation, metadata conflicts, legacy archives, card history, signed social, public admission and independent private batch-proof parity passed.");
} finally { fs.rmSync(temp,{recursive:true,force:true}); }
