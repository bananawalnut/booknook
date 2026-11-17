// SPDX-License-Identifier: MPL-2.0
// Bridge only: book scripts never run. Native code owns persistence and consent.
(() => {
  "use strict";
  const post = body => window.webkit?.messageHandlers?.booknook?.postMessage(body);
  const svgNamespace = "http://www.w3.org/2000/svg";
  let annotations = [], markerLayer, highlightLayer;
  let reportedHighlightPlacements = new Map();
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
  const scalarBoundary = (text, offset) => {
    const char = text.charCodeAt(offset), previous = text.charCodeAt(offset - 1);
    return char >= 0xdc00 && char <= 0xdfff && previous >= 0xd800 && previous <= 0xdbff ? offset - 1 : offset;
  };
  const position = (range, context, preserveScalars = false) => {
    let start = 0, end = 0, offset = 0;
    for (const node of context.nodes) { if (node === range.startContainer) start = offset + range.startOffset; if (node === range.endContainer) end = offset + range.endOffset; offset += node.textContent.length; }
    if (end <= start) return null;
    let prefixStart = Math.max(0, start - 64), suffixEnd = end + 64;
    if (preserveScalars) {
      prefixStart = scalarBoundary(context.text, prefixStart);
      suffixEnd = scalarBoundary(context.text, suffixEnd);
    }
    return {quote:context.text.slice(start,end), prefix:context.text.slice(prefixStart,start), suffix:context.text.slice(end,suffixEnd)};
  };
  const resolve = anchor => {
    const {nodes,text} = textContext();
    if (!anchor.quote) return {status:"missing"};
    let start = -1;
    for (let at = text.indexOf(anchor.quote); at >= 0; at = text.indexOf(anchor.quote, at + 1)) {
      if (!text.endsWith(anchor.prefix || "",at) || !text.startsWith(anchor.suffix || "",at+anchor.quote.length)) continue;
      if (start >= 0) return {status:"ambiguous",context:anchor.quote};
      start = at;
    }
    if (start < 0) return {status:"missing",context:anchor.quote};
    const end = start + anchor.quote.length; let offset = 0; const range = document.createRange();
    for (const node of nodes) { const next = offset + node.textContent.length; if (start >= offset && start < next) range.setStart(node,start-offset); if (end > offset && end <= next) range.setEnd(node,end-offset); offset = next; }
    return {status:"resolved",range,rects:Array.from(range.getClientRects(),docRect).filter(r=>r.width>0 && r.height>0)};
  };
  const textPlacement = found => found.status === "resolved" && !found.rects.length ? "not_visible" : found.status;
  const geometryFor = (anchor, resource) => {
    const found = resolve(anchor); if (found.status !== "resolved") return {status:found.status};
    return {status:"resolved",geometry:{space:"epub_document",resource,page:null,content_fingerprint:hash(textContext().text),width:document.documentElement.scrollWidth,height:document.documentElement.scrollHeight,origin_x:0,origin_y:0,text_quote:anchor.quote,text_prefix:anchor.prefix || "",text_suffix:anchor.suffix || "",text_rects:found.rects}};
  };
  const nearestText = (x, y, visible) => {
    // Selection ranges provide glyph bounds, including for non-Latin and RTL text.
    let best, distance = Infinity;
    for (const node of textContext().nodes) {
      if (!node.textContent.trim()) continue;
      const range = document.createRange(); range.selectNodeContents(node);
      for (const rect of range.getClientRects()) {
        if (visible && (rect.width <= 0 || rect.height <= 0 || rect.right <= visible.left ||
            rect.bottom <= visible.top || rect.left >= visible.right || rect.top >= visible.bottom)) continue;
        const dx = Math.max(rect.left - x, 0, x - rect.right);
        const dy = Math.max(rect.top - y, 0, y - rect.bottom);
        const candidateDistance = dx * dx + dy * dy;
        if (candidateDistance < distance) {
          distance = candidateDistance;
          best = {node, range, rect};
        }
      }
    }
    return best;
  };
  const readingInset = height => Math.min(80, height / 4);
  const readingPosition = v => {
    const left = v.x - scrollX, top = v.y - scrollY;
    const visible = {left, top, right:left + v.width, bottom:top + v.height};
    const x = left + v.width / 2;
    const nearest = nearestText(x, top + readingInset(v.height), visible);
    if (!nearest) return null;
    const {node, rect} = nearest;
    const lineLeft = Math.max(rect.left, visible.left), lineRight = Math.min(rect.right, visible.right);
    const padding = Math.min(1, (lineRight - lineLeft) / 2);
    const probeX = Math.max(lineLeft + padding, Math.min(x, lineRight - padding));
    const probeY = (Math.max(rect.top, visible.top) + Math.min(rect.bottom, visible.bottom)) / 2;
    const caret = document.caretRangeFromPoint?.(probeX, probeY);
    if (caret?.startContainer !== node) return null;

    const data = node.data, start = scalarBoundary(data, caret.startOffset);
    // WebKit may put a caret inside several source spaces that render as one.
    let forward = start, backward = start - 1;
    while (forward < data.length && forward - start < 72 && !data[forward].trim()) forward++;
    while (backward >= 0 && start - backward < 72 && !data[backward].trim()) backward--;
    for (const offset of [forward, scalarBoundary(data, backward)]) {
      if (offset < 0 || offset >= data.length) continue;
      const length = data.codePointAt(offset) > 0xffff ? 2 : 1;
      if (!data.slice(offset, offset + length).trim()) continue;
      const range = document.createRange();
      range.setStart(node, offset);
      range.setEnd(node, offset + length);
      if (![...range.getClientRects()].some(r => r.width > 0 && r.height > 0 &&
          r.bottom > Math.max(rect.top, visible.top) && r.top < Math.min(rect.bottom, visible.bottom) &&
          r.right > lineLeft && r.left < lineRight)) continue;

      range.setEnd(node, scalarBoundary(data, Math.min(data.length, offset + 72)));
      // Keep exact source whitespace and never split a surrogate pair in the context.
      const exact = position(range, textContext(), true);
      if (!exact) continue;
      const resolved = resolve(exact);
      if (resolved.status === "resolved" && resolved.rects.length) return exact;
    }
    return null;
  };
  window.booknookInkAnchor = (bounds, resource) => {
    // The full text run still defines the captured layout for portable ink.
    const match = nearestText(bounds.x + bounds.width / 2 - scrollX, bounds.y + bounds.height / 2 - scrollY);
    const anchor = match && position(match.range, textContext());
    if (!anchor) return {status:"missing"};
    return {...geometryFor(anchor, resource), anchor};
  };
  // Older WebKit has no CSS Highlight registry. Paint measured text rectangles
  // in a viewport-sized layer so it cannot affect line layout or selection.
  const renderFallbackHighlights = rects => {
    if (!highlightLayer) {
      if (!rects.length) return;
      highlightLayer = document.createElementNS(svgNamespace, "svg");
      highlightLayer.setAttribute("data-booknook-overlay", "true");
      highlightLayer.setAttribute("data-booknook-highlight-fallback", "");
      highlightLayer.setAttribute("aria-hidden", "true");
      highlightLayer.style.cssText = "position:fixed;inset:0;width:100%;height:100%;overflow:hidden;pointer-events:none;mix-blend-mode:multiply;z-index:2147483639";
      document.body.appendChild(highlightLayer);
    }
    const bounds = docRect(highlightLayer.getBoundingClientRect());
    const marks = document.createDocumentFragment();
    for (const rect of rects) {
      const x = rect.x - bounds.x, y = rect.y - bounds.y;
      if (x >= bounds.width || y >= bounds.height || x + rect.width <= 0 || y + rect.height <= 0) continue;
      const mark = document.createElementNS(svgNamespace, "rect");
      mark.setAttribute("x", String(x));
      mark.setAttribute("y", String(y));
      mark.setAttribute("width", String(rect.width));
      mark.setAttribute("height", String(rect.height));
      mark.setAttribute("fill", "#dfc362");
      mark.setAttribute("fill-opacity", "0.5");
      marks.appendChild(mark);
    }
    highlightLayer.replaceChildren(marks);
  };
  const renderAnnotations = () => {
    if (!markerLayer) { markerLayer = document.createElement("div"); markerLayer.dataset.booknookOverlay = "true"; markerLayer.style.cssText="position:absolute;inset:0;pointer-events:none;overflow:hidden;z-index:2147483640"; document.body.appendChild(markerLayer); }
    markerLayer.replaceChildren();
    const nativeHighlights = window.CSS?.highlights && typeof window.Highlight === "function";
    const ranges = [], highlightRects = [], placements = [];
    const highlightPlacements = new Map();
    for (const item of annotations) {
      const resolved = resolve(item.anchor);
      if (!item.ink_geometry) {
        if (item.legacy_ink) placements.push({id:item.id,status:"legacy_layout"});
        else {
          const status = textPlacement(resolved);
          highlightPlacements.set(item.id, status);
          if (status === "resolved") {
            if (nativeHighlights) ranges.push(resolved.range);
            else highlightRects.push(...resolved.rects);
          }
        }
      } else {
        const current = geometryFor(item.anchor, item.ink_geometry.resource);
        const placement = current.status === "resolved" ? BookNookGeometry.resolveInk(item.ink_geometry,current.geometry) : {status:current.status};
        placements.push({id:item.id,...placement});
        if (placement.status === "resolved" && item.render_portable) {
          const svg=document.createElementNS(svgNamespace,"svg");
          svg.setAttribute("width",String(document.documentElement.scrollWidth));svg.setAttribute("height",String(document.documentElement.scrollHeight));svg.style.cssText="position:absolute;left:0;top:0;overflow:visible";svg.setAttribute("aria-hidden","true");
          for (const stroke of item.strokes || []) { const line=document.createElementNS(svg.namespaceURI,"polyline");line.setAttribute("points",stroke.points.map(p=>`${p.x+placement.dx},${p.y+placement.dy}`).join(" "));line.setAttribute("fill","none");line.setAttribute("stroke",/^#[0-9a-f]{6}$/i.test(stroke.color)?stroke.color:"#4d7050");line.setAttribute("stroke-width",String(stroke.width));line.setAttribute("stroke-linecap","round");line.setAttribute("stroke-linejoin","round");svg.appendChild(line); }
          markerLayer.appendChild(svg);
        }
        if (placement.status !== "resolved" && resolved.status === "resolved" && resolved.rects[0]) {
          const marker=document.createElement("span"), r=resolved.rects[0]; marker.textContent="✎"; marker.title="Saved markup: its original text layout has changed"; marker.setAttribute("aria-label",marker.title); marker.style.cssText=`position:absolute;left:${Math.max(0,r.x-24)}px;top:${r.y}px;color:#936221;font:18px system-ui`; markerLayer.appendChild(marker);
        }
      }
    }
    if (nativeHighlights) window.CSS.highlights.set("booknook", new window.Highlight(...ranges));
    else renderFallbackHighlights(highlightRects);
    post({type:"ink_placements",placements});
    if (highlightPlacements.size !== reportedHighlightPlacements.size ||
        [...highlightPlacements].some(([id, status]) => reportedHighlightPlacements.get(id) !== status)) {
      reportedHighlightPlacements = highlightPlacements;
      post({type:"highlight_placements",placements:Object.fromEntries(highlightPlacements)});
    }
  };
  window.booknookResolve = resolve;
  window.booknookSetFontSize = size => { if (Number.isFinite(size) && size>=13 && size<=34) {document.body.style.setProperty("font-size",`${size}px`,"important");measure();} };
  window.booknookRestorePosition = anchor => {
    if (!anchor) return {status:"missing"};
    const found = resolve(anchor);
    const status = textPlacement(found);
    if (status === "resolved") {
      const v = viewport();
      const top = found.rects[0].y + found.rects[0].height / 2 - readingInset(v.height) - (v.y - scrollY);
      scrollTo({top:Math.max(0,top),behavior:"instant"});
      lastPositionAt = 0;
      currentPosition = null;
      measure();
    }
    return {status};
  };
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
    if (Date.now() - lastPositionAt > 750) { currentPosition = readingPosition(v); lastPositionAt = Date.now(); }
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
