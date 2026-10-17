// SPDX-License-Identifier: MIT OR Apache-2.0
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const {nativeProbe,nativeCli,inkCommand} = require('./sdk-native.cjs');
module.exports = function noteWhiteboardParity(core) {
  const fixture = JSON.parse(fs.readFileSync(path.join(__dirname,'../ontology/v0.3.0/note-whiteboard.json'),'utf8'));
  for (const example of fixture.cases) {
    const raw=JSON.stringify(example.input);
    const wasm=()=> example.contract==='state' ? (core.validate_archive(raw), JSON.parse(core.validate_portable_archive(JSON.stringify({schema:'booknook.portable-export.v1',state:example.input,assets:{}}))).state) : JSON.parse(core.execute(JSON.stringify(example.state),raw));
    const native=()=> example.contract==='state' ? nativeProbe({op:'normalize',state:example.input}) : nativeProbe({op:'execute',state:example.state,command:example.input});
    if(example.valid) assert.deepEqual(wasm(),native(),example.id); else {assert.throws(wasm,example.id);assert.throws(native,example.id);}
  }
  const directory=fs.mkdtempSync(path.join(os.tmpdir(),'booknook-note-whiteboard-'));
  const restore=fs.mkdtempSync(path.join(os.tmpdir(),'booknook-note-restore-'));
  try {
    nativeCli(directory,['command'],{op:'add_book',title:'Bounded whiteboard',author:'Synthetic',at:1});
    let state=JSON.parse(nativeCli(directory,['snapshot']));
    const n=Object.values(state.notebooks)[0], b=n.blocks[0];
    assert.equal(state.schema,'booknook.archive.v1');assert.equal(b.note,undefined);
    const capture={pass_id:n.pass_id,positions:[{kind:'provider',provider:'example.reader',source_id:'volume',value:'chapter3'}],position_observations:[]};
    nativeCli(directory,['command'],{op:'add_block',notebook_id:n.id,at:2,capture_context:capture,face:'text'});
    state=JSON.parse(nativeCli(directory,['snapshot']));
    const ordered=state.notebooks[n.id].blocks.map(block=>block.id);
    function both(command) {
      const w=JSON.parse(core.execute(JSON.stringify(state),JSON.stringify(command)));
      const r=JSON.parse(nativeCli(directory,['command'],command));assert.deepEqual(w,r);state=r.state;return r;
    }
    both({op:'set_note_face',notebook_id:n.id,block_id:b.id,face:'whiteboard',at:5});
    assert.equal(state.notebooks[n.id].blocks[0].note.created.source,'pending');
    const first={...inkCommand(n.id,b.id,'world-one',[],20),canvas_width:40000,canvas_height:30000,canvas_space:{kind:'whiteboard',origin_x:-20000,origin_y:-10000},capture_context:capture,note_creation:{at:10,capture_context:capture}};
    first.strokes[0].points[0].x=-18000;first.strokes[0].points[1].y=18000;
    both(first);const initial=structuredClone(state.notebooks[n.id].blocks[0]);
    assert.equal(state.schema,'booknook.archive.v3');assert.equal(initial.note.created.at,10);
    both({...first,version_id:'world-two',parents:['world-one'],at:30,canvas_width:1_500_000,canvas_space:{kind:'whiteboard',origin_x:-750000,origin_y:-10000},capture_context:{...capture,positions:[]}});
    both({op:'set_note_face',notebook_id:n.id,block_id:b.id,face:'text',at:31});
    const latest=state.notebooks[n.id].blocks[0];assert.deepEqual(latest.note.created,initial.note.created);assert.deepEqual(latest.ink.versions[0],initial.ink.versions[0]);assert.equal(latest.text,b.text);assert.equal(latest.update,b.update);
    assert.deepEqual(state.notebooks[n.id].blocks.map(block=>block.id),ordered);
    const prior=structuredClone(state);assert.equal(both(first).result.replayed,true);assert.deepEqual(state,prior);
    for (const invalid of [{...first,capture_context:{...capture,positions:[]}},{...first,version_id:'bad-world',canvas_width:2_000_001}]) {
      assert.throws(()=>core.execute(JSON.stringify(state),JSON.stringify(invalid)));assert.throws(()=>nativeCli(directory,['command'],invalid));assert.deepEqual(JSON.parse(nativeCli(directory,['snapshot'])),state);
    }
    const archivePath=path.join(directory,'original.json');nativeCli(directory,['export',archivePath]);
    const envelope=JSON.parse(fs.readFileSync(archivePath,'utf8'));
    const portable=JSON.parse(core.validate_portable_archive(JSON.stringify(envelope)));assert.deepEqual(portable,envelope);
    const importedPath=path.join(directory,'wasm.json');fs.writeFileSync(importedPath,JSON.stringify(portable));nativeCli(restore,['restore',importedPath]);
    const exportedPath=path.join(restore,'restored.json');nativeCli(restore,['export',exportedPath]);const restored=JSON.parse(fs.readFileSync(exportedPath,'utf8'));assert.deepEqual(restored,{...portable,state:{...portable.state,revision:restored.state.revision}});
    assert.deepEqual(JSON.parse(nativeCli(directory,['snapshot'])),state,'actual SQLite reopen preserves all notes');
    const downgrade=structuredClone(state);downgrade.schema='booknook.archive.v2';downgrade.ontology_version='0.2.0';assert.throws(()=>core.validate_archive(JSON.stringify(downgrade)));
    console.log(`Notes/whiteboards: ${fixture.cases.length} published vectors, actual native/WASM exact results, immutable first context, expanding world, ordering, retry rejection and portable SQLite reopen pass.`);
  } finally {fs.rmSync(directory,{recursive:true,force:true});fs.rmSync(restore,{recursive:true,force:true});}
};
