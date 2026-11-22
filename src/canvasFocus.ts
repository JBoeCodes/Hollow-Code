/**
 * Canvas focus (10-11): when a turn finishes with a canvas, the chat keeps
 * that canvas in view instead of scrolling to the closing prose below it.
 *
 * The rules, decided here so the node Vitest suite covers them (ChatBody only
 * supplies the geometry and the events):
 * - A finished reply with a `hollow-canvas` fence (ask-canvases and broken
 *   ones too) anchors the TOP of its last canvas, `CANVAS_FOCUS_PAD` below the
 *   scroller's top. Tall canvases align their top, never their centre.
 * - Only someone who was following the stream is moved. Someone who scrolled
 *   away mid-stream stays put and gets the "Back to canvas" chip instead.
 * - While the anchor is held, size changes (frames resizing, charts drawing,
 *   fonts) re-anchor it. Any scroll of the person's own releases it at once;
 *   the chip then offers the way back until the canvas is in view again.
 * - The next message (or any new turn) releases the anchor and the chip.
 * Turns without a canvas keep the follow-the-stream behaviour.
 */
import { parseSegments } from './chatSegments';
import { CANVAS_HEAD, rootAttr } from './chatVisuals';

/** Space above an anchored canvas, in px. */
export const CANVAS_FOCUS_PAD = 16;
/** How much of the canvas must show for it to count as in view, in px. */
const IN_VIEW = 24;
/** A held anchor this far off its place was moved by someone else: a scroll
 *  no wheel, touch or key listener saw (scroll chaining out of a canvas
 *  frame, assistive tech), so it is the person's and releases the anchor. */
const DRIFT = 48;

/** The last canvas of the latest reply (after the last user turn): the turn
 *  that shows it and its title (null when it has none yet), or null. `from`:
 *  the first turn of the run that just ended; a canvas before it belongs to an
 *  earlier reply (a send that failed, or a queued turn withdrawn, ends a busy
 *  spell with nothing new), so it doesn't count. */
export function replyCanvas(turns: ReadonlyArray<{ role: string; text: string }>, from = 0): { turnIndex: number; title: string | null } | null {
  for (let i = turns.length - 1; i >= Math.max(0, from); i--) {
    const t = turns[i];
    if (t.role === 'user') return null;
    if (t.role !== 'assistant' || !t.text.includes('hollow-canvas')) continue;
    const canvases = parseSegments(t.text).filter(s => s.kind === 'canvas');
    const last = canvases[canvases.length - 1];
    if (!last || last.kind !== 'canvas') continue;
    const head = CANVAS_HEAD.exec(last.code)?.[1];
    const m = head === undefined ? null : rootAttr(head, 'title');
    const title = m ? (m[1] ?? m[2] ?? m[3] ?? '').trim().slice(0, 180) : '';
    return { turnIndex: i, title: title || null };
  }
  return null;
}

/** What the controller reads and moves: the scroller and the anchored
 *  canvas, whose box is relative to the scroller's visible top (null when it
 *  isn't rendered). */
export interface FocusView {
  scrollTop(): number;
  setScrollTop(top: number, smooth: boolean): void;
  clientHeight(): number;
  scrollHeight(): number;
  canvasBox(): { top: number; bottom: number } | null;
}

/** `anchor`: hold the canvas (call `anchor` once it has rendered).
 *  `follow`: no canvas, today's behaviour. `stay`: the person had scrolled
 *  away, so nothing moves (the chip may show). */
export type TurnEndFocus = 'anchor' | 'follow' | 'stay';

export function createCanvasFocus(view: FocusView, onChange?: () => void) {
  let held = false;
  // A canvas the chip can bring back: from a turn's end to the next turn.
  let target = false;
  let chip = false;
  // A smooth anchor is on its way: its own scroll events aren't drift, and a
  // size change meanwhile waits for it to arrive (`pending`), so it isn't cut
  // short by an instant jump.
  let moving = false;
  let pending = false;
  // The first anchor has landed: size changes before it (a ResizeObserver's
  // first report) are the anchor's own business.
  let placed = false;
  const setChip = (on: boolean) => { if (chip !== on) { chip = on; onChange?.(); } };
  const setHeld = (on: boolean) => { if (held !== on) { held = on; onChange?.(); } };

  function inView(): boolean {
    const box = view.canvasBox();
    if (!box) return true;
    return box.bottom > IN_VIEW && box.top < view.clientHeight() - IN_VIEW;
  }
  /** Where the scroller must be for the canvas top to sit at the pad,
   *  within its scroll range, or null when the canvas isn't rendered. */
  function anchorTop(): number | null {
    const box = view.canvasBox();
    if (!box) return null;
    const max = Math.max(0, view.scrollHeight() - view.clientHeight());
    return Math.min(max, Math.max(0, Math.round(view.scrollTop() + box.top - CANVAS_FOCUS_PAD)));
  }
  /** Put the canvas top in view; false when it isn't rendered (yet: it may
   *  be re-mounting as the reply settles). The caller retries or gives up. */
  function anchor(smooth: boolean): boolean {
    const top = anchorTop();
    if (top === null) return false;
    placed = true;
    if (Math.abs(top - view.scrollTop()) >= 1) { moving = smooth; view.setScrollTop(top, smooth); }
    return true;
  }
  /** A smooth move is over: catch up with any size change it waited out. */
  function arrived() {
    moving = false;
    if (pending && held) { pending = false; const top = anchorTop(); if (top !== null && Math.abs(top - view.scrollTop()) >= 1) view.setScrollTop(top, false); }
    pending = false;
  }
  function release() { setHeld(false); moving = false; pending = false; placed = false; }
  return {
    held: () => held,
    chip: () => chip,
    turnEnded(o: { hasCanvas: boolean; following: boolean }): TurnEndFocus {
      if (!o.hasCanvas) { release(); target = false; setChip(false); return 'follow'; }
      target = true;
      if (o.following) { placed = false; setHeld(true); setChip(false); return 'anchor'; }
      release();
      setChip(!inView());
      return 'stay';
    },
    anchor,
    /** Something changed size: keep the held canvas where it was. A canvas
     *  briefly missing (its skeleton swapping for the frame) is skipped, not
     *  let go. */
    resized() {
      if (!held || !placed) return;
      if (moving) { pending = true; return; }
      const top = anchorTop();
      if (top !== null && Math.abs(top - view.scrollTop()) >= 1) view.setScrollTop(top, false);
    },
    /** The person scrolled (wheel, touch, keys, scrollbar, a jump of theirs). */
    userScrolled() { release(); if (target) setChip(!inView()); },
    /** Any scroll. Held: a smooth anchor arriving ends its move, and a drift
     *  nobody announced is the person's. Released: the chip hides once the
     *  canvas is in view again. */
    scrolled() {
      if (held) {
        if (!placed) return;
        const top = anchorTop();
        if (top === null) return;
        const off = Math.abs(top - view.scrollTop());
        if (moving) { if (off <= 2) arrived(); return; }
        if (off > DRIFT) { release(); if (target) setChip(!inView()); }
        return;
      }
      if (target) setChip(!inView());
    },
    /** A smooth move's time is up (it may never report arriving). */
    settle() { if (moving) arrived(); },
    /** The chip: back to the canvas, held again. */
    backToCanvas(smooth: boolean) { if (!target) return; setHeld(true); setChip(false); if (!anchor(smooth)) release(); },
    /** The canvas never rendered: let go (the caller ends at the bottom). */
    abandon() { release(); target = false; setChip(false); },
    /** The chip's Escape: hide it until the next turn's canvas. */
    dismiss() { target = false; setChip(false); },
    /** A new message or turn, another conversation, or a card that must show. */
    released() { release(); target = false; setChip(false); },
  };
}

export type CanvasFocus = ReturnType<typeof createCanvasFocus>;
