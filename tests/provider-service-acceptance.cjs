// SPDX-License-Identifier: MIT OR Apache-2.0
// Synthetic host providers exercise the actual public facade, shared WASM and atomic storage.
const assert=require('node:assert/strict');
const {createHash}=require('node:crypto');
function descriptor(id='fixture.library',namespace='fixture.books') {
  return {id,source_namespace:namespace,capabilities:['catalog','content','positions','identity','recommendations'],available:true,reason:null,connection:null};
}
function response(request,result) {
  return {schema:'booknook.provider-response.v1',request_id:request.request_id,generation:request.generation,
    provider_id:request.provider_id,connection_id:request.connection_id,result};
}
function deferred() { let resolve; const promise=new Promise(r=>resolve=r);return{promise,resolve}; }

async function checkGoodreadsCorrections(desk) {
  const first = 'Book Id,Title,Author,ISBN,ISBN13,Exclusive Shelf,My Rating,My Review\n42,Orchard,A,111,123,read,4,Imported note\n';
  const importCSV = (csv, at) => desk.execute({op: 'import_goodreads', csv, at});
  const initial = await importCSV(first, 1);
  assert.equal(initial.result.inserted, 1);
  const book = Object.values(initial.state.books).find(row => row.title === 'Orchard');
  const [editionID] = book.edition_ids;
  function identifiers(state, provider) {
    return state.editions[editionID].identifiers.filter(item => item.provider === provider).map(item => item.id);
  }

  const corrected = await importCSV('Book Id,Title,Author,ISBN13\n42,Orchard,B,978-0000000101\n', 2);
  assert.equal(corrected.result.updated, 1);
  const current = corrected.state.books[book.id];
  assert.deepEqual(current.authors, ['B']);
  assert.deepEqual([current.shelf, current.rating, current.review], ['read', 4, 'Imported note']);
  assert.deepEqual(identifiers(corrected.state, 'isbn'), ['111']);
  assert.deepEqual(identifiers(corrected.state, 'isbn13'), ['9780000000101']);

  await desk.execute({op: 'update_book', book_id: book.id, title: 'My orchard', shelf: 'currently-reading',
    rating: 5, review: 'My own note'});
  const cleared = await importCSV('Book Id,Title,Author,ISBN,Exclusive Shelf,My Rating,My Review\n42,Source title,C,,read,2,Source review\n', 3);
  assert.equal(cleared.result.updated, 1);
  const privateBook = cleared.state.books[book.id];
  assert.deepEqual(privateBook.authors, ['C']);
  assert.deepEqual([privateBook.title, privateBook.shelf, privateBook.rating, privateBook.review],
    ['My orchard', 'currently-reading', 5, 'My own note']);
  assert.deepEqual(identifiers(cleared.state, 'isbn'), []);
  assert.deepEqual(identifiers(cleared.state, 'isbn13'), ['9780000000101']);
  assert.deepEqual(identifiers(cleared.state, 'goodreads'), ['42']);

  const replay = await importCSV(first, 4);
  assert.equal(replay.result.unchanged, 1);
  assert.deepEqual(replay.state.books, cleared.state.books);
  assert.deepEqual(replay.state.editions, cleared.state.editions);
}

async function checkPortableIsbnHistory(sdk, core) {
  async function reader(seed) {
    let state = structuredClone(seed);
    let csv;
    let saveAttempts = 0;
    const provider = descriptor('fixture.goodreads', 'goodreads');
    const storage = {
      async load() { return structuredClone(state); },
      async compareAndSave(expected, next) {
        saveAttempts++;
        if ((state?.revision ?? null) !== expected) return false;
        state = structuredClone(next);
        return true;
      }
    };
    const host = { descriptorJSON: JSON.stringify(provider), async execute(raw) {
      return {responseJSON: JSON.stringify(response(JSON.parse(raw), {kind: 'catalog', catalog: {format: 'goodreads', csv}}))};
    }};
    let sequence = 0;
    const options = {now: () => 1000n, requestID: () => `isbn-request-${++sequence}`};
    const service = await sdk.BookNookServices.open(core, storage, [host], options);
    return {desk: await sdk.BookNook.open(core, storage), storage, get saveAttempts() { return saveAttempts; }, async importValue(val) {
      csv = `Book Id,Title,Author,ISBN\n42,Orchard,Reader,${val}\n`;
      return (await service.importCatalog(provider.id, 'selected-catalog')).state;
    }};
  }

  const seed = await (await reader(null)).importValue('111');
  const [edition] = Object.keys(seed.editions);
  const history = state => sdk.isbnHistory(core, state, edition, 'isbn');
  const [original] = history(seed).heads;
  const branch = async value => (await reader(seed)).importValue(value);
  const local = await branch('222');
  const clear = await branch('');
  const merge = (a, b) => sdk.reconcile(core, null, a, b).state;
  const merged = merge(local, clear);
  const pending = state => sdk.isbnReviews(core, state);
  const [review] = pending(merged);
  assert.deepEqual(pending(merge(clear, local)), [review]);
  assert.equal(review.edition_id, edition);
  assert.equal(review.namespace, 'isbn');
  assert.deepEqual(review.choices, [[], ['222']]);
  assert.equal(review.heads.length, 2);
  assert.ok(review.heads.every(head => typeof head === 'string' && /^[0-9a-f]{64}$/.test(head)));
  assert.deepEqual(review.heads, [...review.heads].sort());
  const roundtrip = state => sdk.validatePortableArchive(core, sdk.encodePortableArchive(core, {
    schema:'booknook.portable-export.v1', state, assets:{}
  })).state;
  const restored = await reader(roundtrip(merged));
  assert.deepEqual(pending(await restored.storage.load()), [review]);
  const choose = {op:'review_isbn', edition_id:edition, namespace:'isbn', expected_heads:review.heads, values:[]};
  const third = await branch('333');
  const changed = await reader(merge(merged, third));
  const before = await changed.storage.load();
  await assert.rejects(() => changed.desk.execute(choose));
  assert.deepEqual(await changed.storage.load(), before);
  const beforeDecision = restored.saveAttempts;
  const chosen = await restored.desk.execute(choose);
  assert.equal(restored.saveAttempts, beforeDecision + 1);
  assert.deepEqual(pending(chosen.state), []);
  const bookID = Object.keys(chosen.state.books)[0];
  const edited = await restored.desk.execute({op:'update_book', book_id:bookID, title:'Orchard', shelf:'read', rating:4, review:'Private later edit'});
  assert.ok(edited.state.revision > chosen.state.revision);
  const beforeReplay = restored.saveAttempts;
  const replay = await restored.desk.execute(choose);
  assert.equal(replay.result.replayed, true);
  assert.equal(replay.result.node_id, chosen.result.node_id);
  assert.equal(restored.saveAttempts, beforeReplay);
  assert.deepEqual(replay.state, edited.state);
  assert.deepEqual(await restored.storage.load(), edited.state);
  const observed = merge(roundtrip(chosen.state), local);
  assert.deepEqual(history(observed).heads, [chosen.result.node_id]);
  assert.deepEqual(history(observed).nodes[chosen.result.node_id].values, []);
  const reopened = await reader(observed);
  const result = await reopened.desk.execute({op:'reassert_isbn', edition_id:edition, namespace:'isbn',
    expected_heads:[chosen.result.node_id], historical_node:original});
  const final = merge(roundtrip(result.state), clear);
  assert.deepEqual(history(final).heads, [result.result.node_id]);
  assert.deepEqual(history(final).nodes[result.result.node_id].values, ['111']);
  assert.deepEqual(final.editions[edition].identifiers.filter(i => i.provider === 'isbn').map(i => i.id), ['111']);
  assert.throws(() => history({...final, revision: Number.MAX_SAFE_INTEGER + 1}));

  // Equal legacy empties can mean either a proven clear or an unknown absence.
  const asLegacy = state => {
    const legacy = structuredClone(state);
    delete legacy.isbn_history;
    legacy.schema = 'booknook.archive.v4';
    legacy.ontology_version = '0.4.0';
    core.validate_archive(JSON.stringify(legacy));
    return legacy;
  };
  const legacy = [asLegacy(await (await reader(null)).importValue('')), asLegacy(clear), asLegacy(seed)];
  let expectedGraph;
  for (const order of [[0,1,2], [0,2,1], [1,0,2], [1,2,0], [2,0,1], [2,1,0]]) {
    const [a,b,c] = order.map(index => legacy[index]);
    for (const state of [merge(merge(a,b),c), merge(a,merge(b,c))]) {
      assert.deepEqual(pending(state).map(review => review.choices), [[[], ['111']]]);
      assert.equal(history(state).legacy_absence_unknown, true);
      expectedGraph ??= state.isbn_history;
      assert.deepEqual(state.isbn_history, expectedGraph);
    }
  }
}

async function runProviderAcceptance(sdk,core,nativeProbe) {
  let value=null,failCAS=false,sourceTitle='Provider café',calls=0,transform=v=>v,beforeReply=async()=>{},lastRequest,commitPause;
  const bytes=new Uint8Array([37,80,68,70,45,49]); // Deliberately only hash/media validation; not a valid PDF parser fixture.
  const storage={async load(){return structuredClone(value);},async compareAndSave(expected,next){
    if(commitPause)await commitPause();
    if(failCAS || (value?.revision??null)!==expected)return false;value=structuredClone(next);return true;
  }};
  const host={descriptorJSON:JSON.stringify(descriptor()),async execute(raw){
    calls++;const request=JSON.parse(raw);lastRequest=request;await beforeReply();
    let result;
    switch(request.operation.kind) {
      case 'catalog':result={kind:'catalog',catalog:{format:'normalized',entries:[{source_id:'one',title:sourceTitle,
        authors:['A reader'],subjects:['fiction'],edition_identifiers:[],language:null}]}};break;
      case 'content':result={kind:'content',source_id:request.operation.source_id,media_type:request.operation.media_type,
        sha256:createHash('sha256').update(bytes).digest('hex')};break;
      case 'identity':result={kind:'identity',reference:{entity_iri:request.operation.entity_iri,kind:'Actor',display_name:'Reader'}};break;
      case 'source_positions':result={kind:'source_positions',observations:[{source_id:request.operation.source_ids[0],value:'chapter-two:paragraph-seven',observed_at:990}]};break;
      case 'recommendations':result={kind:'recommendations',recommendations:core.recommendations?
        JSON.parse(core.recommendations(JSON.stringify(request.operation.input))):[]};break;
      default:throw new Error('Unsupported synthetic operation');
    }
    const reply=transform(response(request,result));
    return{responseJSON:typeof reply==='string'?reply:JSON.stringify(reply),contentBytes:request.operation.kind==='content'?bytes:undefined};
  }};
  let sequence=0;
  const options={now:()=>1000n,requestID:()=>`fixture-request-${++sequence}`};
  const service=await sdk.BookNookServices.open(core,storage,[host],options);
  const initial=structuredClone(value);
  const fetched=await service.fetch(descriptor().id,{kind:'catalog',source_id:'selected-catalog'});
  const registry=JSON.stringify({schema:'booknook.provider-registry.v1',providers:[descriptor()]});
  const direct=JSON.parse(core.provider_apply_catalog(JSON.stringify(initial),registry,0n,fetched.executionJSON,1000n));
  if(nativeProbe)assert.deepEqual(direct,nativeProbe({op:'provider_catalog',state:JSON.stringify(initial),registry,
    generation:0,execution:fetched.executionJSON,now:1000}));
  const first=await service.importCatalog(descriptor().id,'selected-catalog');
  assert.deepEqual(first,direct);assert.equal(first.result.inserted,1);
  const id=Object.keys(first.state.books)[0];
  const desk=await sdk.BookNook.open(core,storage);
  await desk.execute({op:'update_book',book_id:id,title:'My title',shelf:'currently-reading',rating:4,review:'Private interpretation 🌿'});
  sourceTitle='Provider second title';
  const second=await service.importCatalog(descriptor().id,'selected-catalog');
  assert.equal(second.result.updated,1);assert.equal(Object.keys(second.state.books).length,1);
  assert.equal(second.state.books[id].title,'My title');assert.equal(second.state.books[id].rating,4);
  assert.equal(second.state.books[id].review,'Private interpretation 🌿');
  const repeat=await service.importCatalog(descriptor().id,'selected-catalog');assert.equal(repeat.result.unchanged,1);
  const reopened=await sdk.BookNookServices.open(core,storage,[host],options);
  assert.equal((await reopened.importCatalog(descriptor().id,'selected-catalog')).state.books[id].review,'Private interpretation 🌿');
  const identity=await service.fetch(descriptor().id,{kind:'identity',entity_iri:'urn:fixture:reader'});
  assert.equal(identity.execution.response.result.reference.kind,'Entity');
  const notebook=await desk.execute({op:'start_pass',book_id:id,at:980});
  const passID=notebook.result.pass_id;
  const positionRead=await service.sourcePositions(descriptor().id,['one']);
  const observation=positionRead.execution.response.result.observations[0];
  const initialPositionState=structuredClone(value);
  const expectedPosition=JSON.parse(core.provider_apply_source_position(JSON.stringify(initialPositionState),registry,0n,
    positionRead.executionJSON,JSON.stringify(observation),passID,1000n));
  if(nativeProbe)assert.deepEqual(expectedPosition,nativeProbe({op:'provider_position',state:JSON.stringify(initialPositionState),
    registry,generation:0,execution:positionRead.executionJSON,observation:JSON.stringify(observation),pass_id:passID,now:1000}));
  const attached=await service.attachPosition(positionRead,observation,passID);
  assert.deepEqual(attached,expectedPosition);
  assert.equal(JSON.stringify(attached.state.positions).includes('chapter-two:paragraph-seven'),true);
  assert.equal(JSON.stringify(attached.state.positions).includes('fixture.books'),true);
  const savedPosition=JSON.stringify(value);
  await assert.rejects(()=>service.attachPosition(positionRead,{...observation,value:'forged-location'},passID));
  assert.equal(JSON.stringify(value),savedPosition);
  const suggestions=await service.recommendations(descriptor().id,{candidates:[{book_id:'catalog:one',title:'Other book',subjects:['fiction']}],
    preferred_subjects:['fiction'],shared_ratings:[],friends:[],excluded:[]});
  assert.equal(suggestions[0].score,2);assert.equal(JSON.stringify(lastRequest).includes('Private interpretation'),false);
  const content=await service.acquireContent(descriptor().id,'asset:one','application/pdf');
  assert.deepEqual(content.contentBytes,bytes);bytes[0]=1;assert.equal(content.contentBytes[0],37);bytes[0]=37;
  const unchanged=()=>JSON.stringify(value);
  let before=unchanged();
  for(const mutation of [
    r=>({...r,provider_id:'wrong-provider'}),
    r=>{const v={...r};delete v.connection_id;return v;},
    r=>JSON.stringify(r).replace('{','{"schema":"duplicate",'),
    r=>({...r,result:{kind:'identity',reference:{entity_iri:'urn:wrong',kind:'Entity',display_name:null}}}),
    r=>({...r,result:{...r.result,catalog:{format:'normalized',entries:[...r.result.catalog.entries,...r.result.catalog.entries]}}}),
  ]) {
    transform=mutation;await assert.rejects(()=>service.importCatalog(descriptor().id,'selected-catalog'));assert.equal(unchanged(),before);
  }
  transform=v=>v;
  const previousCalls=calls;
  await assert.rejects(()=>service.fetch(descriptor().id,{kind:'recommendations',input:{candidates:[],preferred_subjects:[],shared_ratings:[],friends:[],excluded:[],notebook:'private'}}));
  assert.equal(calls,previousCalls);
  failCAS=true;sourceTitle='Third title';
  await assert.rejects(()=>service.importCatalog(descriptor().id,'selected-catalog'),/Storage conflict/);assert.equal(unchanged(),before);failCAS=false;
  let started=deferred(),release=deferred();beforeReply=async()=>{started.resolve();await release.promise;};
  const controller=new AbortController();
  const cancelled=service.importCatalog(descriptor().id,'selected-catalog',controller.signal);await started.promise;
  controller.abort();release.resolve();await assert.rejects(cancelled,/cancelled/);assert.equal(unchanged(),before);
  started=deferred();release=deferred();
  const stale=service.importCatalog(descriptor().id,'selected-catalog');await started.promise;
  service.replaceProviders([host]);release.resolve();await assert.rejects(stale,/registration changed/);assert.equal(unchanged(),before);
  beforeReply=async()=>{};
  assert.throws(()=>core.provider_apply_catalog(JSON.stringify(value),registry,1n,fetched.executionJSON,1000n));
  const unavailable={...host,descriptorJSON:JSON.stringify({...descriptor(),available:false,reason:'Test outage'})};
  service.replaceProviders([unavailable]);
  await assert.rejects(()=>service.importCatalog(descriptor().id,'selected-catalog'));assert.equal(unchanged(),before);
  service.replaceProviders([host]);
  const invalid={...host,descriptorJSON:host.descriptorJSON.replace('{','{"id":"duplicate",')};
  assert.throws(()=>service.replaceProviders([invalid]));
  assert.equal(service.descriptors()[0].id,descriptor().id);
  await service.importCatalog(descriptor().id,'selected-catalog');
  assert.equal(value.books[id].review,'Private interpretation 🌿');
  started=deferred();release=deferred();beforeReply=async()=>{started.resolve();await release.promise;};
  const concurrent=service.importCatalog(descriptor().id,'selected-catalog');await started.promise;
  await desk.execute({op:'update_book',book_id:id,title:'My latest title',shelf:'read',rating:5,review:'Newer personal edit'});
  release.resolve();assert.equal((await concurrent).state.books[id].review,'Newer personal edit');
  beforeReply=async()=>{};
  const committing=deferred(),finishCommit=deferred();
  commitPause=async()=>{committing.resolve();await finishCommit.promise;};
  const lateCancel=new AbortController();
  const durable=service.importCatalog(descriptor().id,'selected-catalog',lateCancel.signal);await committing.promise;
  assert.throws(()=>service.replaceProviders([host]),/commit is in progress/);
  lateCancel.abort();finishCommit.resolve();const committed=await durable;commitPause=undefined;
  assert.deepEqual(committed.state,value,'An acknowledged commit must not be reported as cancellation');
  await checkGoodreadsCorrections(desk);
  await checkPortableIsbnHistory(sdk, core);
  return 'Public provider imports/reimports/private edits/reopen, Goodreads author/ISBN corrections and historical replay, portable v5 ISBN review/reassert/reopen and acknowledged replay without CAS, generic source positions, hash/source validation, capability/duplicate rejection, cancellation, concurrent edits, stale generation and failed CAS passed.';
}
module.exports={runProviderAcceptance};
