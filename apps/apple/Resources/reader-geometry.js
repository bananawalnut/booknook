// SPDX-License-Identifier: MPL-2.0
// Shared, deterministic coordinate rules. DOM measurements are supplied by reader.js.
(function (root) {
  "use strict";
  const finiteRect = r => r && [r.x, r.y, r.width, r.height].every(Number.isFinite) && r.width >= 0 && r.height >= 0;
  const approximately = (a, b) => Math.abs(a - b) <= 0.75;
  const resolveInk = (saved, current) => {
    if (!saved || !current || saved.space !== current.space || saved.resource !== current.resource || saved.page !== current.page || saved.content_fingerprint !== current.content_fingerprint) return { status: "missing" };
    if (saved.space === "pdf_page") return approximately(saved.width, current.width) && approximately(saved.height, current.height) ? { status: "resolved", dx: 0, dy: 0 } : { status: "changed_layout" };
    const a = saved.text_rects || [], b = current.text_rects || [];
    if (!a.length || a.length !== b.length || !a.every(finiteRect) || !b.every(finiteRect)) return { status: "changed_layout" };
    const dx = b[0].x - a[0].x, dy = b[0].y - a[0].y;
    // A paragraph may move as earlier content loads. A changed line break, font,
    // size or shape is not a translation: retain its marker instead of guessing.
    return a.every((r, i) => approximately(r.width, b[i].width) && approximately(r.height, b[i].height) && approximately(r.x + dx, b[i].x) && approximately(r.y + dy, b[i].y)) ? { status: "resolved", dx, dy } : { status: "changed_layout" };
  };
  const viewportTransform = (viewport, nativeWidth, nativeHeight) => {
    if (![viewport?.x, viewport?.y, viewport?.width, viewport?.height, nativeWidth, nativeHeight].every(Number.isFinite) || viewport.width <= 0 || viewport.height <= 0 || nativeWidth <= 0 || nativeHeight <= 0) return null;
    // visualViewport is measured in CSS pixels, native overlays in view points.
    return { scaleX: nativeWidth / viewport.width, scaleY: nativeWidth / viewport.width, x: viewport.x, y: viewport.y };
  };
  const api = { resolveInk, viewportTransform };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  root.BookNookGeometry = api;
})(typeof globalThis !== "undefined" ? globalThis : this);
