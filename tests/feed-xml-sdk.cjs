// SPDX-License-Identifier: MIT OR Apache-2.0
// Real WASM + built typed wrapper, no HTML renderer/network/cache is used.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto');
async function main(){
 const modulePath=path.resolve(process.argv[2]||'sdk/typescript/pkg/nodejs/booknook_wasm.js');
 const web=path.basename(path.dirname(modulePath))==='web';
 const core=web?await import(require('node:url').pathToFileURL(modulePath).href):require(modulePath);
 if(web)core.initSync({module:fs.readFileSync(path.join(path.dirname(modulePath),'booknook_wasm_bg.wasm'))});
 const sdk=require(path.resolve(process.argv[3]||'sdk/typescript/dist/cjs/index.js'));
 const corpus=JSON.parse(fs.readFileSync(path.join(__dirname,'../crates/core/tests/fixtures/feed-xml-v1/cases.json'),'utf8'));
 const bytes=s=>new TextEncoder().encode(typeof s==='string'?s:JSON.stringify(s));
 const context=bytes(corpus.context);
 const rejectsNormally=(operation,pattern)=>{
  let rejected=false;try{operation();}catch(error){rejected=true;assert.equal(error instanceof WebAssembly.RuntimeError,false,'Input refusal must not be a WASM trap');assert.equal(typeof error,'string','Rust validation returns a normal string refusal');if(pattern)assert.match(error,pattern);}
  assert.equal(rejected,true,'Expected whole-candidate validation refusal');
 };
 const pointer=(object,p)=>p.split('/').slice(1).reduce((v,k)=>v[k.replace(/~1/g,'/').replace(/~0/g,'~')],object);
 for(const bad of [null,true,1,[],['auto'],{auto:null}])rejectsNormally(()=>sdk.normalizeFeedXmlV1(core,bytes({...corpus.context,expected_format:bad}),bytes("<rss version='2.0'><channel/></rss>")));
 for(const test of corpus.cases){
  const run=()=>sdk.normalizeFeedXmlV1(core,context,bytes(test.xml));
  if(!test.valid){rejectsNormally(run);continue;}
  const result=run();
  assert.equal(result.schema,'booknook.normalized-feed.v1');
  assert.equal(result.normalizer_profile,'booknook.rss2-atom.inert.v1');
  assert.equal(result.provenance.raw_sha256,crypto.createHash('sha256').update(test.xml).digest('hex'));
  assert.deepEqual(result,JSON.parse(core.normalize_feed_xml_v1(context,bytes(test.xml))));
  for(const [p,expected]of Object.entries(test.checks))assert.deepEqual(pointer(result,p),expected,test.id+' '+p);
 }
 for(const [depth,valid]of [[64,true],[65,false]]){
  const xml=bytes("<rss version='2.0'><channel>"+'<x>'.repeat(depth-2)+'</x>'.repeat(depth-2)+'</channel></rss>');
  if(valid)assert.equal(sdk.normalizeFeedXmlV1(core,context,xml).format,'rss');else rejectsNormally(()=>sdk.normalizeFeedXmlV1(core,context,xml),/depth exceeds 64/);
 }
 for(const raw of [Uint8Array.from([0xff,0xfe,60,0]),bytes("<rss version='2.0'><channel>"+'<x>'.repeat(30000)+'</x>'.repeat(30000)+'</channel></rss>'),bytes("<rss version='2.0'><channel>"+'<item/>'.repeat(501)+'</channel></rss>')])rejectsNormally(()=>sdk.normalizeFeedXmlV1(core,context,raw));
 console.log(`Actual ${web?'web-target':'Node'} WASM + typed XML normalizer: ${corpus.cases.length} shared cases, original SHA, hostile depth/UTF8/entry bounds passed; no transport/render/cache qualification.`);
}
main().catch(error=>{console.error(error);process.exitCode=1;});
