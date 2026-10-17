// SPDX-License-Identifier: MPL-2.0
// Bridge only: book scripts never run. Native code owns persistence and consent.
(() => {
  "use strict";
  const post = body => window.webkit?.messageHandlers?.booknook?.postMessage(body);
  let annotations = [], markerLayer;
  const textNodes = () => {
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {
      acceptNode: node => node.parentElement?.closest("script,style,noscript,[data-booknook-overlay]") ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT
    });
    const nodes = []; while (walker.nextNode()) nodes.push(walker.currentNode); return nodes;
  };
  const hash = value => { let h = 2166136261; for (let i = 0; i < value.length; i++) h = Math.imul(h ^ value.charCodeAt(i), 16777619); return (h >>> 0).toString(16); };
  let cachedContext;
  const textContext = () => cachedContext || (cachedContext = (() => { const nodes = textNodes(); return { nodes, text: nodes.map(n => n.textContent).join("") }; })());
  const viewport = () => ({x: visualViewport?.pageLeft ?? scrollX, y: visualViewport?.pageTop ?? scrollY, width: visualViewport?.width ?? innerWidth, height: visualViewport?.height ?? innerHeight});
  const docRect = r => ({x: r.x + scrollX, y: r.y + scrollY, width:r.width, height:r.height});
  const position = (range, context) => {
    let start = 0, end = 0, offset = 0;
    for (const node of context.nodes) { if (node === range.startContainer) start = offset + range.startOffset; if (node === range.endContainer) end = offset + range.endOffset; offset += node.textContent.length; }
    return end > start ? {quote: context.text.slice(start,end), prefix: context.text.slice(Math.max(0,start-64),start), suffix: context.text.slice(end,end+64)} : null;
  };
  const resolve = anchor => {
    const {nodes,text} = textContext(), matches = [];
    if (!anchor.quote) return {status:"missing"};
    for (let at = text.indexOf(anchor.quote); at >= 0; at = text.indexOf(anchor.quote, at + 1)) {
      if (text.slice(0,at).endsWith(anchor.prefix || "") && text.slice(at+anchor.quote.length).startsWith(anchor.suffix || "")) matches.push(at);
    }
    if (matches.length !== 1) return {status:matches.length ? "ambiguous" : "missing",context:anchor.quote};
    const start = matches[0], end = start + anchor.quote.length; let offset = 0; const range = document.createRange();
    for (const node of nodes) { const next = offset + node.textContent.length; if (start >= offset && start < next) range.setStart(node,start-offset); if (end > offset && end <= next) range.setEnd(node,end-offset); offset = next; }
    return {status:"resolved",range,rects:Array.from(range.getClientRects(),docRect).filter(r=>r.width>0 && r.height>0)};
  };
  const geometryFor = (anchor, resource) => {
    const found = resolve(anchor); if (found.status !== "resolved") return {status:found.status};
    return {status:"resolved",geometry:{space:"epub_document",resource,page:null,content_fingerprint:hash(textContext().text),width:document.documentElement.scrollWidth,height:document.documentElement.scrollHeight,origin_x:0,origin_y:0,text_quote:anchor.quote,text_prefix:anchor.prefix || "",text_suffix:anchor.suffix || "",text_rects:found.rects}};
  };
  const nearestText = (x, y) => {
    // Selection ranges provide glyph bounds, including for non-Latin and RTL text.
    const context = textContext(); let best, distance = Infinity;
    for (const node of context.nodes) {
      if (!node.textContent.trim()) continue;
      const range = document.createRange(); range.selectNodeContents(node);
      for (const r of range.getClientRects()) {
        const dx = Math.max(r.left - x, 0, x - r.right), dy = Math.max(r.top - y, 0, y - r.bottom), d = dx * dx + dy * dy;
        if (d < distance) { distance = d; best = {node, range}; }
      }
    }
    if (!best) return null;
    // Anchor an individual text run. This is deliberately conservative when the
    // run reflows: its saved line geometry is retained as an unresolved marker.
    return position(best.range, context);
  };
  window.booknookInkAnchor = (bounds, resource) => {
    const anchor = nearestText(bounds.x + bounds.width / 2 - scrollX, bounds.y + bounds.height / 2 - scrollY);
    if (!anchor) return {status:"missing"};
    return {...geometryFor(anchor, resource), anchor};
  };
  const renderAnnotations = () => {
    if (!markerLayer) { markerLayer = document.createElement("div"); markerLayer.dataset.booknookOverlay = "true"; markerLayer.style.cssText="position:absolute;inset:0;pointer-events:none;overflow:hidden;z-index:2147483640"; document.body.appendChild(markerLayer); }
    markerLayer.replaceChildren();
    const ranges = [], placements = [];
    for (const item of annotations) {
      const resolved = resolve(item.anchor);
      if (!item.ink_geometry) {
        if (!item.legacy_ink && resolved.status === "resolved") ranges.push(resolved.range);
        else if (item.legacy_ink) placements.push({id:item.id,status:"legacy_layout"});
      } else {
        const current = geometryFor(item.anchor, item.ink_geometry.resource);
        const placement = current.status === "resolved" ? BookNookGeometry.resolveInk(item.ink_geometry,current.geometry) : {status:current.status};
        placements.push({id:item.id,...placement});
        if (placement.status === "resolved" && item.render_portable) {
          const svg=document.createElementNS("http://www.w3.org/2000/svg","svg");
          svg.setAttribute("width",String(document.documentElement.scrollWidth));svg.setAttribute("height",String(document.documentElement.scrollHeight));svg.style.cssText="position:absolute;left:0;top:0;overflow:visible";svg.setAttribute("aria-hidden","true");
          for (const stroke of item.strokes || []) { const line=document.createElementNS(svg.namespaceURI,"polyline");line.setAttribute("points",stroke.points.map(p=>`${p.x+placement.dx},${p.y+placement.dy}`).join(" "));line.setAttribute("fill","none");line.setAttribute("stroke",/^#[0-9a-f]{6}$/i.test(stroke.color)?stroke.color:"#4d7050");line.setAttribute("stroke-width",String(stroke.width));line.setAttribute("stroke-linecap","round");line.setAttribute("stroke-linejoin","round");svg.appendChild(line); }
          markerLayer.appendChild(svg);
        }
        if (placement.status !== "resolved" && resolved.status === "resolved" && resolved.rects[0]) {
          const marker=document.createElement("span"), r=resolved.rects[0]; marker.textContent="✎"; marker.title="Saved markup: its original text layout has changed"; marker.setAttribute("aria-label",marker.title); marker.style.cssText=`position:absolute;left:${Math.max(0,r.x-24)}px;top:${r.y}px;color:#936221;font:18px system-ui`; markerLayer.appendChild(marker);
        }
      }
    }
    if (window.CSS?.highlights && window.Highlight) CSS.highlights.set("booknook",new Highlight(...ranges));
    post({type:"ink_placements",placements});
  };
  window.booknookResolve = resolve;
  window.booknookSetFontSize = size => { if (Number.isFinite(size) && size>=13 && size<=34) {document.body.style.setProperty("font-size",`${size}px`,"important");measure();} };
  window.booknookRestorePosition = anchor => { if (!anchor) return {status:"missing"}; const found=resolve(anchor); if (found.status==="resolved" && found.rects[0]) {scrollTo({top:Math.max(0,found.rects[0].y-40),behavior:"instant"});measure();} return {status:found.status}; };
  window.booknookNavigateFragment = fragment => {
    if (!fragment) {scrollTo({top:0,behavior:"instant"});measure();return {status:"resolved"};}
    const target=document.getElementById(fragment) || document.getElementsByName(fragment)[0];
    if (!target) return {status:"missing"};
    target.scrollIntoView({block:"start",behavior:"instant"});measure();return {status:"resolved"};
  };
  window.booknookAnnotations = value => { annotations = value; renderAnnotations(); };
  window.booknookHighlights = anchors => { annotations = anchors.map((anchor,i)=>({id:String(i),anchor})); renderAnnotations(); };
  let lastPositionAt = 0, currentPosition = null;
  const geometry = () => {
    const text = textContext().text, v = viewport();
    if (Date.now() - lastPositionAt > 750) { currentPosition = nearestText(v.width/2,Math.min(80,v.height/4)); lastPositionAt = Date.now(); }
    post({type:"geometry",fingerprint:[location.pathname,innerWidth,innerHeight,getComputedStyle(document.body).fontSize,document.documentElement.scrollHeight,hash(text)].join("|"),contentFingerprint:hash(text),viewport:v,position:currentPosition,width:document.documentElement.scrollWidth,height:document.documentElement.scrollHeight});
    renderAnnotations();
  };
  let scheduled = false, trailingPosition;
  const measure = () => { clearTimeout(trailingPosition); trailingPosition=setTimeout(geometry,760); if (!scheduled) { scheduled=true; requestAnimationFrame(()=>{scheduled=false;geometry();}); } };
  document.addEventListener("click", event => {
    const link=event.target?.closest?.("a[href]");
    if (!link || event.defaultPrevented) return;
    event.preventDefault();
    post({type:"reader_link",href:link.href});
  });
  document.addEventListener("selectionchange", () => {
    const selection=getSelection(); if (!selection || selection.isCollapsed || !selection.rangeCount) {post({type:"selection_cleared"});return;}
    const range=selection.getRangeAt(0), context=textContext();
    if (!context.nodes.includes(range.startContainer) || !context.nodes.includes(range.endContainer)) return;
    const anchor=position(range,context); if (anchor) post({type:"selection",...anchor,rects:Array.from(range.getClientRects(),docRect)});
  });
  // Reflowable EPUBs often omit a mobile viewport. Without one WebKit can
  // retain a desktop-width layout and shrink the text after a pane is reopened.
  // Keep pinch zoom available; the reader owns only the initial layout scale.
  const viewportTags = [...document.querySelectorAll('meta[name="viewport" i]')];
  const viewportTag = viewportTags.shift() ?? document.createElement("meta");
  viewportTags.forEach(tag => tag.remove());
  viewportTag.name = "viewport";
  viewportTag.content = "width=device-width,initial-scale=1";
  if (!viewportTag.parentNode) document.head.appendChild(viewportTag);
  const style=document.createElement("style");
  style.textContent="html{-webkit-text-size-adjust:100%;text-size-adjust:100%;color-scheme:only light!important;background:#f7f2e5!important}body{font-family:Georgia,serif!important;font-size:19px!important;line-height:1.8!important;max-width:42em;margin:0 auto!important;padding:28px clamp(16px,5vw,30px)!important;color:#293f38!important;background:#f7f2e5!important;overflow-wrap:break-word}body :where(article,section,main,div,p,span,h1,h2,h3,h4,h5,h6,blockquote,ul,ol,li,table,tr,td,th){color:inherit!important;background-color:transparent!important}img{max-width:100%;height:auto}::highlight(booknook){background:#ebd99b;color:#293f38}::selection{background:#d7e6cb}a{color:#39684e!important}";
  document.head.appendChild(style);
  addEventListener("scroll",measure,{passive:true});addEventListener("resize",measure);visualViewport?.addEventListener("resize",measure);visualViewport?.addEventListener("scroll",measure);
  new ResizeObserver(measure).observe(document.documentElement);document.fonts?.ready.then(measure);measure();
})();
