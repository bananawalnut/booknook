// SPDX-License-Identifier: MIT OR Apache-2.0
// Executes actual generated WASM, optionally also the built public TypeScript SDK.
const assert=require('node:assert/strict');
const fs=require('node:fs');const path=require('node:path');
async function main(){
const modulePath=path.resolve(process.argv[2]||'sdk/typescript/pkg/nodejs/booknook_wasm.js');
const web=path.basename(path.dirname(modulePath))==='web';
const core=web?await import(require('node:url').pathToFileURL(modulePath).href):require(modulePath);
if(web)core.initSync({module:fs.readFileSync(path.join(path.dirname(modulePath),'booknook_wasm_bg.wasm'))});
const bytes=s=>new TextEncoder().encode(typeof s==='string'?s:JSON.stringify(s));
const object=JSON.parse;
assert.equal(typeof core.validate_feed_section_v1,'function');
const corpus=object(fs.readFileSync(path.join(__dirname,'../crates/core/tests/fixtures/feeds-v1/conformance.json'),'utf8'));
assert.equal(corpus.cases.length,102);
for(const c of corpus.cases){if(c.valid)assert.deepEqual(object(core.validate_feed_section_v1(bytes(c.raw))),object(c.raw),c.id);else assert.throws(()=>core.validate_feed_section_v1(bytes(c.raw)),undefined,c.id);}
assert.throws(()=>core.validate_feed_section_v1(new Uint8Array([0xc3,0x28])));
const section=object(fs.readFileSync(path.join(__dirname,'../crates/core/tests/fixtures/feeds-v1/section.json'),'utf8'));
const configuration={schema:'booknook.feed-configuration.v1',revision:0,feeds:section};
const current=bytes(configuration);const incoming=structuredClone(section);incoming.subscriptions[0].title='Reviewed change';
const raw=bytes(incoming);const choices=bytes([{target:'subscription',id:incoming.subscriptions[0].id,choice:'incoming'}]);
assert.deepEqual(object(core.validate_feed_configuration_v1(current)),configuration);
const review=new core.FeedImportReviewV1(current,raw);
let resolved;
try{
 const preview=object(review.preview());assert.equal(preview.schema,'booknook.feed-import-review.v1');assert.equal(preview.conflicts.length,1);assert.equal(preview.candidate,null);
 assert.throws(()=>review.confirm(current,raw));
 resolved=review.resolve(current,raw,choices);
 const confirmed=object(resolved.confirm(current,raw));assert.equal(confirmed.revision,1);assert.equal(confirmed.feeds.subscriptions[0].title,'Reviewed change');
 assert.throws(()=>review.resolve(current,bytes(JSON.stringify(incoming)+' '),choices));
 assert.throws(()=>resolved.confirm(current,bytes(JSON.stringify(incoming)+' ')));
 assert.throws(()=>resolved.confirm(bytes({...configuration,revision:1}),raw));
 assert.throws(()=>review.resolve(current,raw,bytes('[{"target":"subscription","target":"collection"}]')));
 assert.throws(()=>new core.FeedImportReviewV1(bytes(preview),raw));
}finally{if(resolved)resolved.free();review.free();}
assert.throws(()=>new core.FeedImportReviewV1(current,new Uint8Array()));
const absent=new core.FeedImportReviewV1(current,undefined);try{assert.deepEqual(object(absent.confirm(current,undefined)),configuration);}finally{absent.free();}
const orphan='b41ef5e8-0e06-43e0-aa47-000000000999';const expanded=structuredClone(configuration);expanded.feeds.collections.push({id:orphan,title:'Private orphan'});
const selection={schema:'booknook.feed-disclosure-selection.v1',origin:'https://reader.example.org',subscriptionIds:[section.subscriptions[0].id],collectionIds:[orphan],memberships:[{subscriptionId:section.subscriptions[0].id,collectionIds:[]}]};
const projection=new core.FeedProjectionReviewV1(bytes(expanded),bytes(selection));
try{const selected=object(projection.confirm(bytes(expanded),selection.origin));assert.equal(selected.feeds.collections.length,0);assert.deepEqual(selected.feeds.subscriptions[0].collectionIds,[]);assert.ok(!JSON.stringify(selected).includes(orphan));assert.ok(!JSON.stringify(selected).includes('Private orphan'));assert.deepEqual(object(projection.preview()),selected);assert.throws(()=>projection.confirm(bytes(expanded),selection.origin+'/'));assert.throws(()=>projection.confirm(bytes({...expanded,revision:1}),selection.origin));}finally{projection.free();}
assert.equal(expanded.feeds.collections.length,2);
const exportSelection={schema:'booknook.feed-export-selection.v1',subscriptionIds:[section.subscriptions[0].id],collectionIds:[section.collections[0].id],memberships:[{subscriptionId:section.subscriptions[0].id,collectionIds:[section.collections[0].id]}]};
const portable=new core.FeedExportReviewV1(current,bytes(exportSelection));
let exported;
try{
 exported=portable.confirm(bytes(JSON.stringify(configuration)+' \n'));assert.deepEqual(object(exported),section);assert.equal(portable.preview(),exported);
 assert.deepEqual(object(core.validate_feed_section_v1(bytes(exported))),section);
 assert.throws(()=>portable.confirm(bytes({...configuration,revision:1})));
 const edited=structuredClone(configuration);edited.feeds.subscriptions[0].title+='!';assert.throws(()=>portable.confirm(bytes(edited)));
}finally{portable.free();}
const emptyCurrent=bytes(object(core.empty_feed_configuration_v1()));
const reimport=new core.FeedImportReviewV1(emptyCurrent,bytes(exported));try{assert.deepEqual(object(reimport.confirm(emptyCurrent,bytes(exported))).feeds,section);}finally{reimport.free();}
const privateCurrent=structuredClone(configuration);privateCurrent.feeds.subscriptions[0].disclosure='private';
assert.throws(()=>new core.FeedExportReviewV1(bytes(privateCurrent),bytes(exportSelection)));assert.equal(privateCurrent.feeds.subscriptions[0].disclosure,'private');
for(const invalid of [
 JSON.stringify(exportSelection).replace('{','{"origin":"https://reader.example.org",'),
 JSON.stringify(exportSelection).replace('{','{"subscriptionIds":[],'),
 JSON.stringify(exportSelection).replace('{','{"subscription\\u0049ds":[],'),
 bytes({...exportSelection,schema:{'booknook.feed-export-selection.v1':null}}),
 new Uint8Array([0xc3,0x28]),
 bytes({...exportSelection,memberships:[]}),
 bytes({...exportSelection,collectionIds:[]}),
 bytes({...exportSelection,subscriptionIds:[section.subscriptions[0].id,section.subscriptions[0].id]}),
])assert.throws(()=>new core.FeedExportReviewV1(current,typeof invalid==='string'?bytes(invalid):invalid));
const orphanChoice={...exportSelection,collectionIds:[orphan],memberships:[{subscriptionId:section.subscriptions[0].id,collectionIds:[]}]};
for(const chosen of [orphanChoice,{...orphanChoice,subscriptionIds:[],memberships:[]}]){
 const p=new core.FeedExportReviewV1(bytes(expanded),bytes(chosen));try{const output=p.confirm(bytes(expanded));const value=object(output);assert.equal(value.collections.length,0);assert.ok(!output.includes(orphan));assert.equal(value.subscriptions.length,chosen.subscriptionIds.length);}finally{p.free();}
}
assert.equal(expanded.feeds.collections.length,2);
const disclosure=new core.FeedProjectionReviewV1(current,bytes({...exportSelection,schema:'booknook.feed-disclosure-selection.v1',origin:'https://reader.example.org'}));
try{assert.throws(()=>core.validate_feed_section_v1(bytes(disclosure.preview())));}finally{disclosure.free();}
if(process.argv[3]){
 const sdk=require(path.resolve(process.argv[3]));
 for(const c of corpus.cases){if(c.valid)assert.deepEqual(sdk.validateFeedSectionV1(core,bytes(c.raw)),object(c.raw),c.id);else assert.throws(()=>sdk.validateFeedSectionV1(core,bytes(c.raw)),undefined,c.id);}
 const wrapped=sdk.BookNookFeedImportReviewV1.prepare(core,current,raw);
 const changed=wrapped.resolve(current,raw,choices);
 assert.equal(changed.confirm(current,raw).revision,1);assert.throws(()=>wrapped.resolve(current,bytes(JSON.stringify(incoming)+' '),choices));
 changed.close();assert.throws(()=>changed.confirm(current,raw),/closed/i);changed.close();wrapped.close();assert.throws(()=>wrapped.preview(),/closed/i);
 const selected=sdk.BookNookFeedProjectionReviewV1.prepare(core,bytes(expanded),bytes(selection));assert.equal(selected.confirm(bytes(expanded),selection.origin).feeds.collections.length,0);selected.close();assert.throws(()=>selected.preview(),/closed/i);
 const exportedReview=sdk.BookNookFeedExportReviewV1.prepare(core,current,bytes(exportSelection));
 assert.deepEqual(exportedReview.preview(),section);assert.deepEqual(exportedReview.confirm(bytes(JSON.stringify(configuration)+' ')),section);
 assert.throws(()=>sdk.BookNookFeedExportReviewV1.prepare(core,current,bytes(JSON.stringify(exportSelection).replace('{','{"subscriptionIds":[],'))));
 exportedReview.close();assert.throws(()=>exportedReview.preview(),/closed/i);assert.throws(()=>exportedReview.confirm(current),/closed/i);exportedReview.close();
}
console.log('Actual WASM feed SDK: 102 raw vectors, exact-review conflict/stale checks, inert absence, origin/collection closure, origin-free export/reimport'+(process.argv[3]?' and real TypeScript wrapper lifecycle':'')+' passed.');

}
main().catch(error=>{console.error(error);process.exitCode=1;});
