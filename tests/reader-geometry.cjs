// SPDX-License-Identifier: MPL-2.0
const assert = require('node:assert/strict');
const {resolveInk,viewportTransform} = require('../apps/apple/Resources/reader-geometry.js');
const saved={space:'epub_document',resource:'chapter.xhtml',page:null,content_fingerprint:'original',text_rects:[{x:30,y:500,width:220,height:32},{x:30,y:532,width:100,height:32}]};
assert.deepEqual(resolveInk(saved,{...saved}),{status:'resolved',dx:0,dy:0});
assert.deepEqual(resolveInk(saved,{...saved,text_rects:saved.text_rects.map(r=>({...r,x:r.x+14,y:r.y-80}))}),{status:'resolved',dx:14,dy:-80});
assert.equal(resolveInk(saved,{...saved,text_rects:[{x:30,y:500,width:150,height:32},{x:30,y:532,width:170,height:32}]}).status,'changed_layout');
assert.equal(resolveInk(saved,{...saved,text_rects:[saved.text_rects[0]]}).status,'changed_layout');
assert.equal(resolveInk(saved,{...saved,content_fingerprint:'different-edition'}).status,'missing');
assert.equal(resolveInk(saved,{...saved,resource:'other.xhtml'}).status,'missing');
assert.equal(resolveInk(saved,{...saved,text_rects:[{x:NaN,y:0,width:0,height:0}]}).status,'changed_layout');
assert.deepEqual(viewportTransform({x:20,y:800,width:400,height:300},800,600),{scaleX:2,scaleY:2,x:20,y:800});
assert.equal(viewportTransform({x:0,y:0,width:0,height:300},800,600),null);
assert.deepEqual(viewportTransform({x:0,y:200,width:400,height:200},800,600),{scaleX:2,scaleY:2,x:0,y:200});
const pdf={space:'pdf_page',resource:'book.pdf',page:2,content_fingerprint:'pdf',width:600,height:800};
assert.equal(resolveInk(pdf,{...pdf}).status,'resolved');
assert.equal(resolveInk(pdf,{...pdf,page:3}).status,'missing');
assert.equal(resolveInk(pdf,{...pdf,width:800,height:600}).status,'changed_layout');
console.log('Reader geometry: scroll/zoom transforms, translation, reflow ambiguity, edition and page isolation passed.');
