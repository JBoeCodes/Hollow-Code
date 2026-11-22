import { Show, createEffect, createSignal, onCleanup } from 'solid-js';
import { Icon } from '../icons';
import { quoteMarkdown } from '../selectionQuote';
import '../styles/quote-selection.css';

/**
 * A Quote button just above text selected in the transcript (overnight 11-07,
 * D1, audit gap 9; ChatGPT's "Ask ChatGPT" on a selection, Claude's Reply).
 * It puts the passage in the message box as a Markdown quote with the caret
 * under it, so you write your question and send; nothing is sent from here.
 *
 * Built as the file preview's selection bar is (`SelectionActions`): the bar
 * reads the selection when a mouse button comes up anywhere, or a moment after
 * a keyboard change (Shift and the arrows), follows it while the transcript
 * scrolls, and goes when the selection collapses or leaves the view. It sits in
 * the scroll wrap (`.chat-scroll-wrap`, positioned, a size container). A
 * selection that runs past the transcript (released over the composer) counts
 * only its transcript part; one in the composer or the find bar is ignored.
 */
const SKIP = '.chat-find, .chat-input, textarea, input, [contenteditable="true"]';
/** What a drag can cross that is not the passage (review of #1207): controls
 *  and their labels (a code block's wrap and copy, Copy table, Show more), a
 *  code block's or a table's header, a turn's footer and your message's
 *  actions, a canvas (its frame keeps its own selection; its header and live
 *  status are chrome) and words only screen readers get. */
const CHROME = 'button, .chat-codeblock-head, .reply-table-bar, .chat-user-actions, .turn-footer, .chat-artifact-head, .chat-canvas, .sr-only';
/** A folded long message of yours: its lines past the fold are clipped, not
 *  gone, so a selection that reaches them would quote words you can't see. */
const CLIPPED = '.chat-user-clipped';
/** About half the button's width, so it never hangs past the wrap's edge. */
const HALF = 48;

function elementOf(node: Node): Element | null {
  return node.nodeType === 1 ? (node as Element) : node.parentElement;
}

/** The part of `range` inside `el`. */
function within(range: Range, el: Element): Range {
  const r = range.cloneRange();
  if (!el.contains(r.startContainer)) r.setStart(el, 0);
  if (!el.contains(r.endContainer)) r.setEnd(el, el.childNodes.length);
  return r;
}

/** Whether the selection reaches past a folded message's visible lines. */
function reachesClipped(range: Range, el: Element): boolean {
  const box = el.getBoundingClientRect();
  return [...within(range, el).getClientRects()].some(q => q.height > 0 && q.bottom > box.bottom + 1);
}

/** The pieces of `range` that are the passage: everything but the chrome and
 *  the clipped lines it crosses, in document order. */
function passageParts(range: Range): Range[] {
  const host = elementOf(range.commonAncestorContainer);
  if (!host) return [];
  const cuts = (el: Element) => el.matches(CHROME) || (el.matches(CLIPPED) && reachesClipped(range, el));
  // Inside one piece of chrome, or a clipped message, entirely.
  for (let el: Element | null = host; el; el = el.parentElement) if (cuts(el)) return [];
  const parts: Range[] = [];
  const rest = range.cloneRange();
  // Document order, so a cut comes before anything inside it, and only the
  // outermost cut counts: one inside a cut would move the start back into it.
  let last: Element | null = null;
  for (const el of host.querySelectorAll(`${CHROME}, ${CLIPPED}`)) {
    if (last?.contains(el) || !range.intersectsNode(el) || !cuts(el)) continue;
    last = el;
    const part = rest.cloneRange();
    // A cut that holds the start collapses this piece to nothing.
    part.setEndBefore(el);
    if (!part.collapsed) parts.push(part);
    rest.setStartAfter(el);
  }
  if (!rest.collapsed) parts.push(rest);
  return parts;
}

export function QuoteSelection(props: { scroller: () => HTMLElement | undefined; onQuote: (quote: string) => void }) {
  const [bar, setBar] = createSignal<{ x: number; y: number; below: boolean } | null>(null);
  // The selection the bar points at, cut to this transcript.
  let shown: Range | null = null;
  // A mouse button is down: a drag shows the bar when it ends, not on the way.
  let pressed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const hide = () => { shown = null; setBar(null); };

  /** The live selection cut to this transcript, or null. */
  const selected = (): Range | null => {
    const el = props.scroller();
    const sel = window.getSelection();
    if (!el || !sel || sel.isCollapsed || !sel.rangeCount) return null;
    const live = sel.getRangeAt(0);
    if (!live.intersectsNode(el)) return null;
    const range = within(live, el);
    const host = elementOf(range.commonAncestorContainer);
    if (!host || host.closest(SKIP)) return null;
    return range;
  };

  // Above the first selected line in view, else below the last one, else
  // just inside the top of the view (a selection taller than the view).
  const place = () => {
    const el = props.scroller();
    if (!el || !shown) { hide(); return; }
    const view = el.getBoundingClientRect();
    const lines = [...shown.getClientRects()].filter(q => q.width > 0 && q.height > 0 && q.bottom > view.top && q.top < view.bottom);
    // Scrolled out of the view: nothing to point at (it comes back with the text).
    if (!lines.length) { setBar(null); return; }
    const first = lines[0];
    const last = lines[lines.length - 1];
    const wrap = (el.parentElement ?? el).getBoundingClientRect();
    let below = false;
    let y = first.top - 8;
    if (first.top - view.top < 44) {
      below = true;
      y = view.bottom - last.bottom >= 44 ? last.bottom + 8 : view.top + 8;
    }
    const centre = first.left + first.width / 2 - wrap.left;
    const x = wrap.width < HALF * 2 ? wrap.width / 2 : Math.max(HALF, Math.min(centre, wrap.width - HALF));
    setBar({ x, y: y - wrap.top, below });
  };

  const read = () => {
    const range = selected();
    if (!range || !passageParts(range).some(part => part.toString().trim())) { hide(); return; }
    shown = range;
    place();
  };
  const later = (ms: number) => { clearTimeout(timer); timer = setTimeout(read, ms); };

  createEffect(() => {
    const el = props.scroller();
    if (!el) return;
    // While a selection shows the bar, a scroll or a resize moves it with the
    // text. Captured, so a wide table's or code box's own sideways scroll
    // inside the transcript moves it too (scroll doesn't bubble; ENG-93).
    const follow = () => { if (shown) place(); };
    const resized = new ResizeObserver(follow);
    el.addEventListener('scroll', follow, { passive: true, capture: true });
    resized.observe(el);
    onCleanup(() => { el.removeEventListener('scroll', follow, { capture: true }); resized.disconnect(); });
  });
  // A keyboard read still pending waits for the drag to end.
  const onDown = (e: MouseEvent) => { if (e.button === 0) { pressed = true; clearTimeout(timer); } };
  // Anywhere: a drag that starts in the transcript may end over the composer.
  const onUp = () => { pressed = false; later(0); };
  const onSelectionChange = () => {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed) { clearTimeout(timer); hide(); return; }
    // The keyboard (Shift and the arrows, Select All) has no mouse-up.
    if (!pressed) later(120);
  };
  // A press whose mouse-up never reaches the page (the window lost focus in the
  // middle of a drag, or a ctrl-click opened the context menu) would leave
  // `pressed` set and every keyboard selection after it without a bar.
  const onAbandoned = () => { pressed = false; };
  document.addEventListener('mousedown', onDown, true);
  document.addEventListener('mouseup', onUp, true);
  document.addEventListener('contextmenu', onAbandoned, true);
  window.addEventListener('blur', onAbandoned);
  document.addEventListener('selectionchange', onSelectionChange);
  onCleanup(() => {
    clearTimeout(timer);
    document.removeEventListener('mousedown', onDown, true);
    document.removeEventListener('mouseup', onUp, true);
    document.removeEventListener('contextmenu', onAbandoned, true);
    window.removeEventListener('blur', onAbandoned);
    document.removeEventListener('selectionchange', onSelectionChange);
  });

  // Read again at the click, so a selection grown since the bar came quotes
  // all of it. Each piece's text is the selection's own (`toString` keeps the
  // line breaks between blocks), read by selecting that piece alone; the
  // selection is cleared right after.
  const quote = () => {
    const sel = window.getSelection();
    const range = selected();
    hide();
    if (!sel || !range) return;
    const texts: string[] = [];
    for (const part of passageParts(range)) {
      sel.removeAllRanges();
      sel.addRange(part);
      texts.push(sel.toString());
    }
    sel.removeAllRanges();
    const text = quoteMarkdown(texts.join('\n'));
    if (text) props.onQuote(text);
  };
  return <Show when={bar()}>{b =>
    // Pressing it must not clear the selection before the click reads it.
    <div class="quote-selection" classList={{ below: b().below }} role="group" aria-label="Selected text" style={{ left: `${b().x}px`, top: `${b().y}px` }} onMouseDown={e => e.preventDefault()}>
      <button type="button" onClick={quote} title="Put the selected text in your message as a quote" aria-label="Quote the selected text in your message"><Icon name="message" size={13} />Quote</button>
    </div>}
  </Show>;
}
