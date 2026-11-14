// SPDX-License-Identifier: MIT OR Apache-2.0
// Actual generated WASM + actual built TypeScript wrapper; deterministic host
// adapters exercise the contract, not DNS/HTTP/XML/backend qualification.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto');
async function main(){
 const modulePath=path.resolve(process.argv[2]||'sdk/typescript/pkg/nodejs/booknook_wasm.js');
 const web=path.basename(path.dirname(modulePath))==='web';
 const core=web?await import(require('node:url').pathToFileURL(modulePath).href):require(modulePath);
 if(web)core.initSync({module:fs.readFileSync(path.join(path.dirname(modulePath),'booknook_wasm_bg.wasm'))});
 const sdk=require(path.resolve(process.argv[3]||'sdk/typescript/dist/cjs/index.js'));
 const bytes=value=>new TextEncoder().encode(typeof value==='string'?value:JSON.stringify(value));
 const feeds=JSON.parse(fs.readFileSync(path.join(__dirname,'../crates/core/tests/fixtures/feeds-v1/section.json'),'utf8'));
 const config=bytes({schema:'booknook.feed-configuration.v1',revision:0,feeds});const id=feeds.subscriptions[0].id;
 const descriptor={schema:'booknook.feed-source-provider.v1',version:1,id:'reader.public',available:true,protocols:['public-xml-bytes.v1']};
 let calls=0,callback=async()=>{};
 const provider={descriptorBytes:bytes(descriptor),async fetch(request,cancel){calls++;await callback(request,cancel);const body=request.conditional?new Uint8Array():bytes('<rss/>');return {untrustedBody:body,responseBytes:bytes({schema:'booknook.feed-source-response.v1',runtime_id:request.runtime_id,request_id:request.request_id,source_id:request.source_id,provider_id:request.provider_id,configuration_generation:request.configuration_generation,provider_generation:request.provider_generation,activation_generation:request.activation_generation,final_url:request.url,content_type:request.conditional?null:'application/rss+xml',outcome:request.conditional?'not-modified':'body',validators:{etag:null,last_modified:null},body_sha256:request.conditional?request.conditional.body_sha256:crypto.createHash('sha256').update(body).digest('hex')})};}};
 assert.deepEqual(sdk.validateFeedSourceDescriptorV1(core,bytes(descriptor)),descriptor);
 for(const bad of [null,true,1,[],['public-xml-bytes.v1'],{'public-xml-bytes.v1':null}]){
  const malformed=bytes({...descriptor,protocols:[bad]});assert.throws(()=>sdk.validateFeedSourceDescriptorV1(core,malformed));assert.throws(()=>new sdk.BookNookFeedSourcesV1(core,config,{...provider,descriptorBytes:malformed}));
 }
 for(const raw of [JSON.stringify(descriptor).replace('"version":1','"version":1e0'),JSON.stringify(descriptor).replace('{','{"version":1,'),'{"schema":"booknook.provider-registry.v1","providers":[]}'])assert.throws(()=>core.validate_feed_source_descriptor_v1(bytes(raw)));
 const runtime=new sdk.BookNookFeedSourcesV1(core,config,provider);
 await assert.rejects(runtime.fetch(id));assert.equal(calls,0);runtime.activate(id);
 callback=async request=>{assert.equal(Object.keys(request).length,12);for(const privateKey of ['title','collections','collectionIds','disclosure','identity','configurationDigest'])assert.equal(request[privateKey],undefined);};
 const read=await runtime.fetch(id);assert.equal(Buffer.from(read.untrustedBody()).toString(),'<rss/>');
 const condition=read.cacheCondition();assert.equal(condition.source_id,id);
 let writes=0;
 await read.store({async stage(expected,body,cancel){await Promise.resolve();assert.deepEqual(expected,condition);assert.equal(cancel.isCancelled(),false);return {committer:{compare_and_store(current,untrusted){assert.deepEqual(JSON.parse(current),condition);assert.equal(Buffer.from(untrusted).toString(),'<rss/>');assert.throws(()=>runtime.disable(id));writes++;return undefined;}}};}});
 assert.equal(writes,1);assert.throws(()=>read.cacheCondition());await assert.rejects(read.store({async stage(){throw Error('must not stage twice');}}));read.close();
 for(const change of ['configuration','provider','disabled','cancel']){
   runtime.activate(id);const controller=new AbortController();
   callback=async(_request,cancel)=>{await Promise.resolve();if(change==='configuration'){runtime.replaceConfiguration(config);runtime.activate(id);}else if(change==='provider'){runtime.replaceProvider(provider);runtime.activate(id);}else if(change==='disabled'){runtime.disable(id);}else controller.abort();assert.equal(cancel.isCancelled(),true);};
   await assert.rejects(runtime.fetch(id,controller.signal));
 }
 callback=async()=>{};runtime.activate(id);
 const oldCandidate=await runtime.fetch(id),newCandidate=await runtime.fetch(id);assert.throws(()=>oldCandidate.cacheCondition());oldCandidate.close();newCandidate.close();
 let release,first=true;callback=async()=>{if(first){first=false;await new Promise(resolve=>{release=resolve;});}};
 const oldRequest=runtime.fetch(id);assert.equal(typeof release,'function');const newRequest=await runtime.fetch(id);release();await assert.rejects(oldRequest);newRequest.close();
 callback=async()=>{};
 const stale=await runtime.fetch(id);
 await assert.rejects(stale.store({async stage(){await Promise.resolve();runtime.replaceConfiguration(config);return {committer:{compare_and_store(){writes++;return undefined;}}};}}));assert.equal(writes,1);stale.close();
 runtime.activate(id);const cancellable=await runtime.fetch(id);const abort=new AbortController();
 await assert.rejects(cancellable.store({async stage(){abort.abort();await Promise.resolve();return {committer:{compare_and_store(){writes++;return undefined;}}};}},abort.signal));assert.equal(writes,1);cancellable.close();
 runtime.activate(id);
 const hint={schema:'booknook.feed-conditional-hint.v1',source_id:id,provider_id:descriptor.id,url:feeds.subscriptions[0].url,body_sha256:crypto.createHash('sha256').update('<rss/>').digest('hex'),validators:{etag:'"v1"',last_modified:'Wed, 21 Oct 2015 07:28:00 GMT'}};
 for(const existingBody of [undefined,bytes('wrong'),bytes('<rss/>')]){
   const conditional=await runtime.fetch(id,undefined,bytes(hint));assert.equal(conditional.response().outcome,'not-modified');assert.equal(conditional.untrustedBody().length,0);
   const before=writes;const operation=conditional.store({async stage(){await Promise.resolve();return {existingBody,committer:{compare_and_store(raw,body){const c=JSON.parse(raw);assert.equal(c.expected_body_sha256,hint.body_sha256);assert.equal(c.outcome,'not-modified');assert.equal(body.length,0);writes++;return undefined;}}};}});
   if(existingBody&&Buffer.from(existingBody).toString()==='<rss/>'){await operation;assert.equal(writes,before+1);}else{await assert.rejects(operation);assert.equal(writes,before);}conditional.close();
 }
 for(const field of ['source_id','provider_id','url','body_sha256']){const bad={...hint,[field]:'wrong'};await assert.rejects(runtime.fetch(id,undefined,bytes(bad)));}
 for(const change of ['configuration','provider']){callback=async()=>{await Promise.resolve();if(change==='configuration')runtime.replaceConfiguration(config);else runtime.replaceProvider(provider);};runtime.activate(id);await assert.rejects(runtime.fetch(id,undefined,bytes(hint)));}
 callback=async()=>{};runtime.activate(id);
 const pending=await runtime.fetch(id);runtime.close();assert.throws(()=>pending.cacheCondition());pending.close();await assert.rejects(runtime.fetch(id));runtime.close();
 // A response or preview JSON cannot fabricate a live Rust read/candidate.
 assert.throws(()=>new sdk.BookNookFeedReadV1(JSON.parse(JSON.stringify(condition))));
 for(const literal of ['body','not-modified'])for(const bad of [null,false,1,[],[literal],{[literal]:null}]){
  const malformedProvider={...provider,async fetch(request,cancel){const reply=await provider.fetch(request,cancel);return {...reply,responseBytes:bytes({...JSON.parse(new TextDecoder().decode(reply.responseBytes)),outcome:bad})};}};
  const malformed=new sdk.BookNookFeedSourcesV1(core,config,malformedProvider);malformed.activate(id);
  await assert.rejects(malformed.fetch(id,undefined,literal==='body'?undefined:bytes(hint)));malformed.close();
 }
 console.log('Actual '+(web?'web-target':'Node')+' WASM + typed host source SDK: async activation, cancellation, ABA/provider staleness, private-field omission and guarded cache/reentry passed. No network/storage backend qualification claimed.');
}
main().catch(error=>{console.error(error);process.exitCode=1;});
