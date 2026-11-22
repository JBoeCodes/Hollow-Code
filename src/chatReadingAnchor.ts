/** Hold reading content when an earlier canvas changes height. Unlike the
 * opening settle or finished-canvas focus, this owns no destination: scrolls
 * capture a new place, and higher-priority navigation disables it.
 * `following`: the view follows the bottom. Nothing is held there (`enabled`
 * is false) and leaving it always captures afresh (a scroll, or `reset`), so
 * no place is measured meanwhile: a streamed reply resized the transcript at
 * every flush, and each capture measured every rendered turn (run 15 S7d). */
export function createReadingAnchor(el: HTMLElement, enabled: () => boolean, moved: () => void, following: () => boolean) {
  /** `edge`: which side of `node` is held. A canvas whose top is above the
   *  reading line is held by its bottom, the edge next to what's read. */
  let anchor: { node: HTMLElement; turn: HTMLElement; text: string; tag: string; offset: number; edge: 'top' | 'bottom' } | null = null;
  let frame: number | null = null;
  let sampledTop = NaN;
  let disposed = false;
  const blocks = 'p, pre, li, h1, h2, h3, h4, h5, h6, table, img, button, .turn-footer, .chat-canvas, .chat-canvas-skeleton, .chat-canvas-broken';
  /** Blocks that change height after they appear (frames report late). */
  const growing = '.chat-canvas, .chat-canvas-skeleton, .chat-canvas-broken';
  const edgeOf = (n: HTMLElement, edge: 'top' | 'bottom') => { const r = n.getBoundingClientRect(); return edge === 'top' ? r.top : r.bottom; };

  function capture() {
    anchor = null;
    sampledTop = el.scrollTop;
    if (disposed || !el.offsetParent) return;
    const box = el.getBoundingClientRect();
    // Skip a sliver of the previous block. A canvas is an opaque frame: hold
    // its outer box, never reach into its isolated document.
    const line = box.top + Math.min(48, el.clientHeight / 4);
    // A canvas that starts above the view (scrolling up brings one in from
    // the top) is skipped for the text after it, in this turn or the next:
    // holding its top let its late growth push everything being read down
    // (night ST4). With nothing after it on screen it is held by its bottom
    // edge, next to what's read, or by its top when it runs past the view.
    let fallback: { node: HTMLElement; turn: HTMLElement } | null = null;
    for (const turn of el.querySelectorAll<HTMLElement>('.turn[data-turn-index]')) {
      const r = turn.getBoundingClientRect();
      if (r.bottom <= line || r.top >= box.bottom) continue;
      const visible = [...turn.querySelectorAll<HTMLElement>(blocks)].filter(n => {
        if (n.parentElement?.closest('.chat-canvas, .chat-canvas-broken, pre, table')) return false;
        const b = n.getBoundingClientRect();
        return b.height > 0 && b.bottom > line && b.top < box.bottom;
      });
      // A tall list item can contain the canvas that is still growing. Its
      // visible paragraph/control is the reference, not the ancestor's top.
      const leaves = visible.filter(n => !visible.some(child => child !== n && n.contains(child)));
      const node = leaves.find(n => !(n.matches(growing) && n.getBoundingClientRect().top < box.top));
      if (node) { set(node, turn, 'top'); return; }
      if (!leaves.length) { set(turn, turn, 'top'); return; }
      fallback ??= { node: leaves[0], turn };
    }
    // Its bottom only while it is on screen: a canvas taller than the view is
    // itself what's read, and frames grow at the bottom, so its top is held.
    if (fallback) set(fallback.node, fallback.turn, fallback.node.getBoundingClientRect().bottom < box.bottom ? 'bottom' : 'top');
  }
  function set(node: HTMLElement, turn: HTMLElement, edge: 'top' | 'bottom') {
    anchor = { node, turn, text: (node.textContent ?? '').slice(0, 80), tag: node.tagName, edge, offset: edgeOf(node, edge) - el.getBoundingClientRect().top };
  }
  function resized() {
    if (disposed) return;
    if (following()) { anchor = null; return; }
    if (!enabled() || !el.offsetParent) { capture(); return; }
    // Streamed Markdown may replace its paragraph nodes. Reuse only a unique
    // textual match within the SAME surviving turn; never carry a pixel debt
    // into another turn, a changed paragraph, or a new conversation.
    if (anchor && !el.contains(anchor.node) && el.contains(anchor.turn) && anchor.text.length >= 16) {
      const saved = anchor;
      const matches = [...saved.turn.querySelectorAll<HTMLElement>(blocks)]
        .filter(n => n.tagName === saved.tag && (n.textContent ?? '').startsWith(saved.text));
      if (matches.length === 1) anchor.node = matches[0];
    }
    if (!anchor || !el.contains(anchor.node) || !anchor.node.getClientRects().length) { capture(); return; }
    const offset = edgeOf(anchor.node, anchor.edge) - el.getBoundingClientRect().top;
    const delta = offset - anchor.offset;
    // Native Chromium anchoring may already have corrected the move. Apply
    // only the residual viewport displacement; never add total height growth.
    if (Math.abs(delta) >= 1) {
      el.scrollTop += delta;
      // At a scroll boundary, retain the reachable position, not a debt that
      // would be applied to unrelated future growth.
      anchor.offset = edgeOf(anchor.node, anchor.edge) - el.getBoundingClientRect().top;
      sampledTop = el.scrollTop;
      moved();
    }
  }
  const ro = new ResizeObserver(resized);
  ro.observe(el);
  for (const child of el.children) ro.observe(child);
  const mo = new MutationObserver(records => {
    for (const record of records) {
      record.removedNodes.forEach(n => { if (n instanceof Element) ro.unobserve(n); });
      record.addedNodes.forEach(n => { if (n instanceof Element) ro.observe(n); });
    }
  });
  mo.observe(el, { childList: true });
  return {
    scrolled() {
      // A follow's own scroll holds nothing; a reader leaving the bottom is
      // no longer following by the time the controller reports it.
      if (following()) { anchor = null; return; }
      // Delayed duplicate scroll events may arrive after fresh layout. The
      // same scrollTop is not new intent: retain the pre-growth reference.
      // Native anchoring and actual user moves change top and are sampled.
      if (!anchor || Math.abs(el.scrollTop - sampledTop) >= 1) capture();
    },
    // Drop before input/default scrolling or explicit navigation. Capture
    // after layout, and let any intervening scroll event pick its own place.
    reset() {
      anchor = null;
      if (frame !== null) cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => { frame = null; if (!anchor && !following()) capture(); });
    },
    dispose() {
      disposed = true;
      anchor = null;
      if (frame !== null) cancelAnimationFrame(frame);
      ro.disconnect();
      mo.disconnect();
    },
  };
}
