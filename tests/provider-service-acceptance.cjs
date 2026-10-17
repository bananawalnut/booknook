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
  return 'Public provider imports/reimports/private edits/reopen, generic source positions, hash/source validation, capability/duplicate rejection, cancellation, concurrent edits, stale generation and failed CAS passed.';
}
module.exports={runProviderAcceptance};
