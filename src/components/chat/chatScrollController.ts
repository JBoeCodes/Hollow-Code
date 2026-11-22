// The transcript's scroll controller. Extracted from ChatBody.tsx (plan L3
// X17, 2026-10-14; review V06/F11): the scroller and its frame budget, the
// bounded render window, jumps (find, outline, canvas reveals), scroll
// memory, following the stream, canvas focus, settling an opened
// conversation, the person's own scroll intent and the reading anchor. Moved
// unchanged, no behaviour change: these regions share about fifteen closure
// variables, so they stay together behind one interface.
//
// Created once, synchronously, in ChatBody's body (it owns the effects,
// mounts and cleanups it sets up), with the component's props object so
// every read stays as lazy as it was.
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, untrack, type Accessor } from 'solid-js';
import { TURN_WINDOW, scrollSignature, ScrollMemory, windowStart } from '../../conversationTools';
import { AUTO_STEP, TURN_CAP, laterEnd, trimEnd, trimStart, windowFor } from '../../transcriptWindow';
import { motionReduced as reducedMotion } from '../../motion';
import {
  chatTurns,
  chatAgentSessionKey,
  chatBusy,
  chatSessionLoading, chatConversationVersion,
  flashFooter,
  pendingApprovals,
  chatInteractions,
} from '../../store';
import { canvasTurnIndex } from '../../canvasTurn';
import { createCanvasFocus, replyCanvas } from '../../canvasFocus';
import { createReadingAnchor } from '../../chatReadingAnchor';
import { isTranscriptFrameFocus } from '../../chatFrameFocus';
import { SETTLE_MAX_MS, createSettle, type SettleTarget } from '../../chatSettle';
import { canvasReveal, takeCanvasReveal, type CanvasReveal } from '../../store/canvasReveal';
import type { FrameBudgetScope } from '../../canvas/frameBudgetScope';

/** Scroll positions by conversation signature (next-level P04): the latest per conversation, 200 at most (P6). */
const SCROLL_MEMORY = new ScrollMemory<{ top: number; atBottom: boolean }>();

/** A pending follow tolerates a scroll event that moves the view up by less than this: its own layout
 *  clamp (a short tail remounting). Measured as the drop, not as the position: the 11-07 header rows put a
 *  narrow window's transcript at 85, not under 80, so its clamp (85 to 49) released the follow and the
 *  340 px canvas skeleton below went unfollowed (chat-motion.spec.ts:107 failed 21 of 30 on chromium-narrow). */
const FOLLOW_CLAMP_PX = 80;
/** Reader input this recent makes a move up the reader's (a trackpad's momentum keeps sending wheels). */
const READER_INPUT_MS = 1000;

export interface ChatScrollProps { workspace?: boolean }

export interface ChatScrollDeps {
  /** The last user turn's index (ChatBody's memo): it must exist first,
   *  since a mount mid-turn reads it at setup. */
  lastUser: Accessor<number>;
  /** Open the finished run's "Worked for…" fold that holds this turn;
   *  returns the first turn that must render to show it (the fold's head). */
  openFoldAt: (turnIndex: number) => number;
  /** The composer's textarea (assigned during render: a getter). */
  textarea: () => import('../../composerField').ComposerField | undefined;
}

export function createChatScroll(props: ChatScrollProps, deps: ChatScrollDeps) {
  const { lastUser, openFoldAt, textarea } = deps;
  let scrollEl: HTMLDivElement | undefined;
  const frameScope: FrameBudgetScope = { root: () => scrollEl };
  // ── Long-thread navigation (next-level P04) ────────────────────────────
  // Bounded rendering: the latest TURN_WINDOW turns render; earlier ones load
  // on request (or when find / the outline jumps to them). Totals, search and
  // copy still cover every turn.
  const [renderFloor, setRenderFloor] = createSignal<number | null>(null);
  const renderFrom = () => windowStart(chatTurns.length, renderFloor());
  // The window's end (parity 0.3): `null` reaches the last turn. Set only while the person reads
  // back through a long thread, so the drawn turns stay within TURN_CAP (transcriptWindow.ts).
  const [renderCeil, setRenderCeil] = createSignal<number | null>(null);
  const renderTo = () => { const c = renderCeil(); return c === null ? chatTurns.length : Math.min(c, chatTurns.length); };
  createEffect(on(() => [chatAgentSessionKey(), chatConversationVersion()], () => {
    readingPin = false;
    setRenderFloor(null);
    setRenderCeil(null);
    // Switched while scrolled up (the turns swap in one step, so `atBottom`
    // doesn't change): pin the new window once its turns are in (P2 review).
    queueMicrotask(() => { if (!untrack(atBottom)) pinForReading(); });
  }, { defer: true }));
  // The window pinned while the person reads further up (see below).
  let readingPin = false;
  // Deferred navigation must yield to a later send or reader action.
  let scrollIntentVersion = 0;
  let readingAnchor: ReturnType<typeof createReadingAnchor> | undefined;
  let readingNavigation = false;
  let readingNavigationTimer: ReturnType<typeof setTimeout> | undefined;
  function finishReadingNavigation() {
    clearTimeout(readingNavigationTimer);
    readingNavigation = false;
    readingAnchor?.reset();
  }
  function holdReadingForNavigation() {
    clearTimeout(readingNavigationTimer);
    readingNavigation = true;
    readingAnchor?.reset();
    readingNavigationTimer = setTimeout(finishReadingNavigation, 700);
  }
  onCleanup(() => clearTimeout(readingNavigationTimer));
  /** Make the window hold turn `idx` (and `need`, the first turn its fold needs) by recentering it,
   *  never by loading every turn between (windowFor). Nothing moves when both are already drawn. */
  function includeTurn(idx: number, need: number) {
    const total = chatTurns.length;
    const from = renderFrom();
    const to = renderTo();
    if (need >= from && idx < to) return;
    readingPin = false;
    const w = windowFor({ from, to }, total, idx, need);
    setRenderFloor(w.from);
    setRenderCeil(w.to >= total ? null : w.to);
  }
  /** The first and last turn index on screen, from the DOM; null when nothing is drawn or shown. */
  function visibleTurns(): { first: number; last: number } | null {
    const el = scrollEl;
    if (!el || !el.offsetParent) return null;
    const box = el.getBoundingClientRect();
    let first = -1;
    let last = -1;
    for (const t of el.querySelectorAll<HTMLElement>('.turn[data-turn-index]')) {
      const r = t.getBoundingClientRect();
      if (r.bottom <= box.top) continue;
      if (r.top >= box.bottom) break;
      const i = Number(t.dataset.turnIndex);
      if (first < 0) first = i;
      last = i;
    }
    return first < 0 ? null : { first, last };
  }
  /** Run a change to the window and keep what the person reads where it was: the first turn on
   *  screen keeps its offset (WebKit has no scroll anchoring, and turns drawn above or dropped
   *  above move everything under the view). */
  function holdPlace(change: () => void) {
    const el = scrollEl;
    const turn = el ? [...el.querySelectorAll<HTMLElement>('.turn[data-turn-index]')].find(t => t.getBoundingClientRect().bottom > el.getBoundingClientRect().top + 1) : undefined;
    const before = turn?.getBoundingClientRect().top;
    change();
    if (el && turn && before !== undefined && turn.isConnected) {
      const moved = turn.getBoundingClientRect().top - before;
      if (Math.abs(moved) >= 1) { el.scrollTop += moved; lastTop = el.scrollTop; readingAnchor?.reset(); }
    }
  }
  /** After the start moved up: cut the end so the window stays within TURN_CAP (never while the person is at the end). */
  function trimTheEnd() {
    const v = visibleTurns();
    const to = trimEnd({ from: renderFrom(), to: renderTo() }, chatTurns.length, v?.last ?? null, atBottom() && renderCeil() === null);
    if (to !== null) setRenderCeil(to >= chatTurns.length ? null : to);
  }
  /** Scroll a turn into view, rendering earlier turns first if needed. A
   *  turn inside a closed "Worked for…" fold opens it first (P1-27): its
   *  step carries the turn's index, so the jump lands on the step. */
  function jumpToTurn(idx: number) {
    const intent = ++scrollIntentVersion;
    holdReadingForNavigation();
    if (pendingScrollForce) cancelAutoScroll();
    const from = openFoldAt(idx);
    includeTurn(idx, from);
    requestAnimationFrame(() => requestAnimationFrame(() => {
      if (intent !== scrollIntentVersion) return;
      // The step inside an open fold before the turn row that holds the fold.
      const el = scrollEl?.querySelector<HTMLElement>(`.chat-run-step[data-turn-index="${idx}"]`)
        ?? scrollEl?.querySelector<HTMLElement>(`[data-turn-index="${idx}"]`);
      if (!el) return;
      stopSettle();
      readingAnchor?.reset();
      setAtBottom(false);
      canvasFocus.userScrolled();
      // A fold that just opened is still settling its height: a smooth jump's
      // first small step would read as "still at the bottom", and the follow
      // would pin it back there. Jump there at once.
      const settling = el.classList.contains('chat-run-step');
      el.scrollIntoView({ block: 'center', behavior: reducedMotion() || settling ? 'auto' : 'smooth' });
    }));
  }
  // "Waiting on you" reopens an ask-canvas by its exact id (Canvas C4 A3):
  // once this conversation is on screen with its transcript loaded, take the
  // request and bring that canvas revision into view. Its turn renders first
  // (it may sit above the render window); the revision is the frame's own
  // `data-revision`, which appears once its store read settles.
  createEffect(() => {
    const want = canvasReveal();
    if (!want || chatSessionLoading() || !chatTurns.length || !scrollEl?.offsetParent) return;
    const taken = takeCanvasReveal(chatAgentSessionKey());
    if (taken) revealCanvas(taken);
  });
  function revealCanvas(want: CanvasReveal) {
    const intent = ++scrollIntentVersion;
    holdReadingForNavigation();
    if (pendingScrollForce) cancelAutoScroll();
    const idx = canvasTurnIndex(chatTurns, want.canvasId);
    if (idx < 0) { flashFooter('× That canvas is no longer in this conversation.', 4000); return; }
    includeTurn(idx, idx);
    let tries = 0;
    const find = () => {
      const all = [...(scrollEl?.querySelectorAll<HTMLElement>(`.turn .chat-canvas[data-canvas-id="${want.canvasId}"]`) ?? [])];
      return all.find(el => el.dataset.revision === String(want.revision)) ?? (tries >= 40 ? all[all.length - 1] : undefined);
    };
    const step = () => {
      if (intent !== scrollIntentVersion) return;
      const el = find();
      if (!el) { if (++tries <= 40) setTimeout(step, 50); return; }
      stopSettle();
      readingAnchor?.reset();
      setAtBottom(false);
      canvasFocus.userScrolled();
      el.scrollIntoView({ block: 'center', behavior: 'auto' });
      lastTop = scrollEl?.scrollTop ?? lastTop;
      // Frames above it settle their heights after load: look again once.
      setTimeout(() => {
        if (intent !== scrollIntentVersion) return;
        const r = el.getBoundingClientRect(), box = scrollEl?.getBoundingClientRect();
        if (box && el.isConnected && (r.bottom < box.top || r.top > box.bottom)) el.scrollIntoView({ block: 'center', behavior: 'auto' });
      }, 500);
    };
    requestAnimationFrame(() => requestAnimationFrame(step));
  }
  // Scroll memory: coming back to a conversation returns to where you were,
  // as long as it is the same conversation with the same turns.
  const scrollMemory = SCROLL_MEMORY;
  createEffect(on(() => scrollSignature(chatAgentSessionKey(), chatTurns), sig => {
    if (!sig || chatSessionLoading()) return;
    const saved = scrollMemory.get(sig);
    if (!saved) return;
    requestAnimationFrame(() => requestAnimationFrame(() => {
      if (!scrollEl || scrollSignature(chatAgentSessionKey(), chatTurns) !== sig) return;
      if (saved.atBottom) { scrollToBottom(); startSettle({ kind: 'bottom' }); return; }
      scrollEl.scrollTop = saved.top; lastTop = scrollEl.scrollTop; setAtBottom(false);
      // Frames above settle their heights after this: keep the place (chatSettle.ts).
      startSettle({ kind: 'top', top: saved.top });
    }));
  }, { defer: true }));
  const [atBottom, setAtBottom] = createSignal(true);
  /** A run ended below the view (P1-27); cleared at the bottom, by a new run
   *  or another conversation. */
  const [finishedBelow, setFinishedBelow] = createSignal(false);
  createEffect(on(atBottom, bottom => { if (bottom) setFinishedBelow(false); }, { defer: true }));
  createEffect(on(() => [chatAgentSessionKey(), chatConversationVersion()], () => setFinishedBelow(false), { defer: true }));
  function nearBottom(): boolean {
    const el = scrollEl;
    if (!el) return true;
    return el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  }
  // Frontier F5: follow the stream only while the user is at the bottom. Any
  // upward scroll of theirs releases it at once (not only past the 80 px
  // "near bottom" band), and the "Jump to latest" pill brings it back. A
  // smooth jump's own intermediate scroll events don't count as the user's.
  let lastTop = 0;
  let jumping = false;
  // What a layout clamp is told apart by (`layoutMove`): whether the
  // transcript's DOM changed since the last frame; when the reader last did
  // anything in the transcript (a press, any key, a wheel, a touch, focus
  // moving in); and whether a press is still down (a scrollbar or a selection
  // drag).
  let changedSinceFrame = false;
  let readerInputAt = -Infinity;
  let pressing = false;
  // The pointer last went onto a canvas frame and hasn't come back to this
  // window since: a scroll then chains out of the frame (no wheel reaches this
  // scroller), and it may have carried the frame out from under the pointer
  // by the time its scroll event arrives, so `:hover` can't tell.
  let pointerOnFrame = false;
  /** An upward move is the layout's: the transcript's DOM changed since the
   *  last frame (a frame's scroll events come before its animation frames, so
   *  the clamp's event always sees its change, however late the frame), with
   *  no reader input in the last second, no press held, and neither
   *  the pointer nor focus on a canvas frame (a scroll chained out of a frame,
   *  by wheel or keys, reaches no listener here). Any other move up (an
   *  assistive scroll in a quiet chat) is still the reader's. The DOM change
   *  is only timed, never measured: reading layout while the transcript
   *  changes would itself clamp it mid-update. */
  function layoutMove(el: HTMLElement): boolean {
    if (!changedSinceFrame || pressing || pointerOnFrame || performance.now() - readerInputAt < READER_INPUT_MS) return false;
    // Focus left in a frame counts only while this window has it (keys can't
    // scroll a window in the background).
    return !el.querySelector('iframe:hover') && !(document.hasFocus() && isTranscriptFrameFocus<Element>(document.activeElement, el));
  }
  function onScroll() {
    const el = scrollEl;
    // Hidden behind another page (U4), its offsets are zeros: not a reader's
    // scroll, and not a place to remember.
    if (!el || el.clientHeight === 0) return;
    // Just after a show at the end (U4), frames unparking above re-anchor
    // the view in script: the show's settle keeps the end, and that scroll
    // is neither the reader's nor a place to remember. A reader's own wheel,
    // key or press still ends the settle first (`onScrollIntent`), and a
    // scroll with the pointer over a canvas frame (it chains out of the
    // frame, so no wheel reaches this scroller) is the reader's too, once
    // the pointer has moved since the show (`readerOverFrame`, lane C's
    // medium, 11-04).
    if (settleAt && performance.now() - shownAt < SHOW_MS && untrack(atBottom) && settle.active() && !readerOverFrame()) {
      settle.resized(); lastTop = el.scrollTop; settleAt = { top: el.scrollTop, height: el.scrollHeight };
      return;
    }
    // A layout clamp at the end is not the reader leaving it (chat jump,
    // 11-10). WebKit clamps `scrollTop` whenever the transcript is shorter
    // for a moment inside one task: a finished run folding its steps into
    // "Worked for…" while the composer drops its busy row, or every reply
    // swapping its loading view on the first open. The view then sits short
    // of the end, the move up read as the reader's, and nothing followed
    // again: each later turn (a check-in) left the view another reply
    // further up, under "Finished". An upward move while following, right
    // after the transcript changed and with no reader input (`layoutMove`),
    // goes back to the end and keeps following.
    if (el.scrollTop < lastTop - 2 && untrack(atBottom) && !jumping && !readingNavigation && !canvasFocus.held() && layoutMove(el)) {
      el.scrollTop = el.scrollHeight;
      if (settleAt) settleAt = { top: el.scrollTop, height: el.scrollHeight };
    }
    const top = el.scrollTop;
    if (chatTurns.length && !chatSessionLoading()) { const sig = scrollSignature(chatAgentSessionKey(), chatTurns); if (sig) SCROLL_MEMORY.set(chatAgentSessionKey()!, sig, { top, atBottom: el.scrollHeight - top - el.clientHeight < 80 }); }
    if (jumping) { if (nearBottom()) { jumping = false; setAtBottom(true); } }
    // A programmatic follow's scroll event can arrive after the next text/
    // frame resize. An unchanged top is not a reader leaving the bottom:
    // preserve following so the resize observer can catch up with that growth.
    else if (top !== lastTop && (scrollFrame === null || top !== scheduledScrollTop) && !(scrollFrame !== null && top < lastTop && lastTop - top < FOLLOW_CLAMP_PX && !scrollEl?.querySelector('iframe:hover'))) {
      // Preserve only the layout clamp already observed when follow was
      // requested, or a clamp of under FOLLOW_CLAMP_PX as a short tail remounts.
      // Larger unannounced/assistive moves still release following; physical
      // intent and focus cancel the frame before any of these exceptions. A scroll
      // with the pointer over a canvas frame (chained out of it, so no wheel
      // listener here saw it) is the reader's, whatever its size; unlike the settle's
      // `readerOverFrame` this needs no pointer move since the show, which can stay
      // false for as long as the pointer rests there.
      if (top < lastTop - 2) setAtBottom(nearBottom() && el.scrollHeight - top - el.clientHeight < 4);
      else setAtBottom(nearBottom());
    }
    lastTop = top;
    // A settling view moved by something other than the settle (a scroll
    // chained out of a canvas frame, which no wheel or key listener here
    // sees) is the person's: let them have it. Content getting shorter only
    // clamps it.
    if (settleAt && Math.abs(top - settleAt.top) > 4 && !(el.scrollHeight < settleAt.height && top >= el.scrollHeight - el.clientHeight - 1)) stopSettle();
    canvasFocus.scrolled();
    if (readingNavigation) {
      clearTimeout(readingNavigationTimer);
      readingNavigationTimer = setTimeout(finishReadingNavigation, 120);
    }
    readingAnchor?.scrolled();
  }
  // Smart follow: only stick to the bottom while the user is already there.
  // `force` overrides for user-initiated actions (sending, opening).
  // Coalesced to one scroll per frame: the follow effect fires on every
  // streamed delta, and each scrollHeight read forces a layout — per-token
  // that adds up to visible jank on long answers.
  let scrollFrame: number | null = null;
  let scheduledScrollTop: number | null = null;
  let pendingScrollForce = false;
  let pendingCardReveal = false;
  function cancelAutoScroll(keepCard = false) {
    if (keepCard && pendingCardReveal) return;
    if (scrollFrame !== null) cancelAnimationFrame(scrollFrame);
    scrollFrame = null;
    scheduledScrollTop = null;
    pendingScrollForce = false;
    pendingCardReveal = false;
  }
  onCleanup(() => cancelAutoScroll());
  function autoScroll(force?: boolean, cardReveal = false) {
    if (chatTurns.length === 0 && pendingApprovals().length === 0 && chatInteractions().length === 0) return;
    if (!force && !atBottom()) return;
    // A held canvas (canvasFocus.ts) stays put; a forced scroll (an approval
    // or question card the person must see, a send) lets it go.
    if (force) { readingAnchor?.reset(); canvasFocus.released(); }
    else if (canvasFocus.held()) return;
    // A later send promotes an already queued ordinary follow. Keeping the
    // first closure's `force` dropped the send when layout changed meanwhile.
    pendingScrollForce ||= !!force;
    pendingCardReveal ||= cardReveal;
    if (scrollFrame !== null) return;
    scheduledScrollTop = scrollEl?.scrollTop ?? null;
    scrollFrame = requestAnimationFrame(() => {
      const force = pendingScrollForce;
      scrollFrame = null;
      scheduledScrollTop = null;
      pendingScrollForce = false;
      pendingCardReveal = false;
      // Re-checked here, not only when scheduled: a jump (find, outline, a
      // canvas reveal, scroll memory) may have let go of the bottom since.
      if (!force && (!atBottom() || canvasFocus.held())) return;
      if (scrollEl && !jumping) { scrollEl.scrollTop = scrollEl.scrollHeight; lastTop = scrollEl.scrollTop; setAtBottom(true); }
    });
  }
  // Overnight 10-11/12 P2: while the person reads further up a long thread,
  // the window of rendered turns keeps its start, so turns arriving below
  // (a queued turn, notices, a long run's replies) never drop the turns above
  // and move what they read (WebKit has no scroll anchoring). Meanwhile the
  // window grows by what arrives (few turns: a run's tool calls join its
  // reply); back at the bottom it trims to the latest turns again. A floor a
  // jump or "Show earlier" set is theirs and stays.
  function pinForReading() {
    if (renderFloor() !== null || chatSessionLoading() || chatTurns.length === 0) return;
    readingPin = true;
    setRenderFloor(renderFrom());
  }
  createEffect(on(() => [atBottom(), chatSessionLoading()] as const, ([bottom, loading]) => {
    if (!bottom && !loading) untrack(pinForReading);
    else if (bottom && readingPin) { readingPin = false; setRenderFloor(null); }
    else if (bottom && !loading) untrack(backToLatest);
  }, { defer: true }));
  /** At the end of the thread again: a window that grew past TURN_CAP while reading back draws the
   *  latest turns once more (a window within the cap, the person's own, stays: next-level P04). */
  function backToLatest() {
    if (renderCeil() !== null || chatTurns.length - renderFrom() <= TURN_CAP) return;
    readingPin = false;
    setRenderFloor(null);
    queueMicrotask(() => { if (scrollEl && atBottom()) { scrollEl.scrollTop = scrollEl.scrollHeight; lastTop = scrollEl.scrollTop; } });
  }
  // Follow what the stream renders, not only its deltas (overnight 10-11/12
  // A1): a message draws its text up to 80 ms after the delta
  // (ChatMessage's throttle) and a canvas frame settles its height later
  // still, so a scroll made on the delta pins a stale height. While a turn
  // runs, any growth of the scroller or a rendered turn re-follows (same
  // rules: only at the bottom, never past a held canvas or a smooth jump).
  createEffect(on(chatBusy, busy => {
    if (!busy || !scrollEl || typeof ResizeObserver === 'undefined') return;
    const el = scrollEl;
    let height = el.scrollHeight;
    const ro = new ResizeObserver(() => {
      const h = el.scrollHeight;
      if (h === height) return;
      height = h;
      if (!jumping) autoScroll();
    });
    ro.observe(el);
    for (const child of el.children) ro.observe(child);
    const mo = new MutationObserver(records => {
      for (const r of records) {
        r.addedNodes.forEach(n => { if (n instanceof Element) ro.observe(n); });
        r.removedNodes.forEach(n => { if (n instanceof Element) ro.unobserve(n); });
      }
    });
    mo.observe(el, { childList: true });
    onCleanup(() => { ro.disconnect(); mo.disconnect(); });
  }));
  /** The "Jump to latest" pill: a smooth scroll (instant under reduced motion). */
  function scrollToBottom(smooth = false) {
    readingAnchor?.reset();
    const el = scrollEl;
    if (!el) return;
    if (smooth && !reducedMotion()) {
      jumping = true;
      el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' });
      // A smooth scroll that never reports the bottom (content grew) still ends.
      setTimeout(() => { if (jumping) { jumping = false; if (scrollEl) scrollEl.scrollTop = scrollEl.scrollHeight; setAtBottom(true); } }, 700);
    } else { el.scrollTop = el.scrollHeight; lastTop = el.scrollTop; setAtBottom(true); }
  }

  // ── Canvas focus (10-11, canvasFocus.ts) ───────────────────────────────
  // A finished reply with a canvas keeps the top of its last canvas in view
  // instead of ending at the closing prose. Held until the next message or
  // the person's own scroll; size changes re-anchor it meanwhile, and the
  // "Back to canvas" chip brings it back once let go.
  const [focusTick, setFocusTick] = createSignal(0);
  const [focusTitle, setFocusTitle] = createSignal<string | null>(null);
  /** The latest reply's last canvas as rendered: its outer frame, or the
   *  skeleton or broken view that stands in for it. */
  let focusEl: HTMLElement | null = null;
  function replyCanvasEl(): HTMLElement | null {
    if (!scrollEl) return null;
    // Cached between scrolls; looked up again once it leaves the page (the
    // skeleton swaps for the frame, a fold closes, the window moves).
    if (focusEl?.isConnected && scrollEl.contains(focusEl)) return focusEl;
    const from = lastUser();
    const all = [...scrollEl.querySelectorAll<HTMLElement>('.turn[data-turn-index] :is(.chat-canvas, .chat-canvas-skeleton, .chat-canvas-broken)')]
      .filter(el => !el.parentElement?.closest('.chat-canvas') && Number(el.closest<HTMLElement>('.turn')?.dataset.turnIndex) > from);
    focusEl = all[all.length - 1] ?? null;
    return focusEl;
  }
  const canvasFocus = createCanvasFocus({
    scrollTop: () => scrollEl?.scrollTop ?? 0,
    setScrollTop: (top, smooth) => {
      const el = scrollEl;
      if (!el) return;
      if (smooth && !reducedMotion()) el.scrollTo({ top, behavior: 'smooth' });
      else el.scrollTop = top;
    },
    clientHeight: () => scrollEl?.clientHeight ?? 0,
    scrollHeight: () => scrollEl?.scrollHeight ?? 0,
    canvasBox: () => {
      const el = replyCanvasEl();
      if (!el || !scrollEl) return null;
      const r = el.getBoundingClientRect(), box = scrollEl.getBoundingClientRect();
      return { top: r.top - box.top, bottom: r.bottom - box.top };
    },
  }, () => setFocusTick(n => n + 1));
  const focusHeld = createMemo(() => { focusTick(); return canvasFocus.held(); });
  const focusChip = () => { focusTick(); return canvasFocus.chip() && chatTurns.length > 0 && !chatBusy(); };
  /** A smooth move ends by itself when it arrives, or after this long. */
  const settleSoon = () => setTimeout(() => canvasFocus.settle(), 700);
  // While held: the scroller and every rendered turn are watched for size
  // changes (frames resizing, charts drawing, fonts loading).
  createEffect(on(focusHeld, held => {
    if (!held || !scrollEl || typeof ResizeObserver === 'undefined') return;
    const el = scrollEl;
    const ro = new ResizeObserver(() => canvasFocus.resized());
    ro.observe(el);
    for (const child of el.children) ro.observe(child);
    const mo = new MutationObserver(records => {
      for (const r of records) r.addedNodes.forEach(n => { if (n instanceof Element) ro.observe(n); });
    });
    mo.observe(el, { childList: true });
    onCleanup(() => { ro.disconnect(); mo.disconnect(); });
  }));
  // The turn's end: anchor its canvas, or keep today's behaviour. Only a turn
  // that ended in this same conversation counts: switching conversations, or
  // sending a running one to the background, also clears the busy flag. A
  // new conversation gets its key mid-turn, so a key set since then is the
  // same conversation.
  // `from`: the first turn the run could have written, so a failed send (its
  // message taken back, a notice posted) never lands on an earlier reply's
  // canvas. Mounted mid-turn (a page switch, an adopted run): the whole
  // current reply is this run's.
  let busyIn: { key: string | null; version: number; from: number } | null =
    untrack(chatBusy) ? { key: untrack(chatAgentSessionKey), version: untrack(chatConversationVersion), from: untrack(lastUser) + 1 } : null;
  const conversationId = () => `${chatAgentSessionKey() ?? ''}#${chatConversationVersion()}`;
  createEffect(on(chatBusy, (busy, prev) => {
    if (busy) { setFinishedBelow(false); busyIn = { key: chatAgentSessionKey(), version: chatConversationVersion(), from: chatTurns.length }; canvasFocus.released(); return; }
    const started = busyIn;
    busyIn = null;
    const same = !!prev && !!started && started.version === chatConversationVersion() && (started.key === null || started.key === chatAgentSessionKey());
    // P1-27: a run that ended while the person read further up says so on
    // Jump to latest ("Finished") until they're back at the bottom.
    if (same && !atBottom()) setFinishedBelow(true);
    if (!same || chatTurns[chatTurns.length - 1]?.partial || !scrollEl?.offsetParent) return;
    const found = replyCanvas(chatTurns, started.from);
    setFocusTitle(found?.title ?? null);
    focusEl = null;
    if (canvasFocus.turnEnded({ hasCanvas: !!found, following: atBottom() }) !== 'anchor') return;
    // A canvas drawn before a tool call sits in the finished run's
    // "Worked for…" fold, which renders nothing while closed: open it.
    if (found) openFoldAt(found.turnIndex);
    // Two frames: the settled reply (its canvas frame or skeleton) renders
    // first. It may re-mount as it settles, so look again for up to ~1 s
    // before ending at the bottom as before.
    let tries = 0;
    const place = () => {
      if (!canvasFocus.held()) return;
      if (canvasFocus.anchor(true)) { settleSoon(); return; }
      if (++tries <= 20) { setTimeout(place, 50); return; }
      canvasFocus.abandon();
      scrollToBottom();
    };
    requestAnimationFrame(() => requestAnimationFrame(place));
  }, { defer: true }));
  // A new message, or another conversation, lets the canvas go.
  createEffect(on(lastUser, () => canvasFocus.released(), { defer: true }));
  createEffect(on(conversationId, () => canvasFocus.released(), { defer: true }));
  // ── Settling an opened conversation (10-12, chatSettle.ts) ─────────────
  // Opening a conversation (or coming back to it) scrolls once, while its
  // canvases are still short placeholders; their growth above the view used
  // to leave it a canvas or two back from the end. Until the content is quiet,
  // each size change puts the view back on its target. Anything the person
  // does in the transcript, a jump, a send or a new turn ends it.
  const settle = createSettle({
    scrollTop: () => scrollEl?.scrollTop ?? 0,
    setScrollTop: top => { if (scrollEl) scrollEl.scrollTop = top; },
    clientHeight: () => scrollEl?.clientHeight ?? 0,
    scrollHeight: () => scrollEl?.scrollHeight ?? 0,
  });
  let stopSettle = () => settle.cancel();
  /** When the transcript last showed again at its end, and how long after
   *  it focus moving in and script re-anchoring are taken as the app's (U4). */
  let shownAt = -Infinity;
  const SHOW_MS = 1000;
  // Whether the pointer really moved over this window since the transcript
  // last showed: its screen position differs from the last move this window
  // saw or, when it may not have seen the last one, movementX or movementY
  // says so: none seen yet, the pointer went into a frame or out of the
  // window, a press (a scrollbar drag's moves never reach the page), or the
  // window lost focus. WebKit sends a still pointer's synthetic move after
  // every scroll, the settle's own jump included, at the pointer's real spot
  // with no movement, so it never counts; a position doesn't depend on how a
  // real move's movement rounds. In the show's first second a canvas
  // frame under a pointer that hasn't moved is the pointer resting where it
  // was, not the reader: frames unpark and grow after a show, so a frame can
  // come to sit under a still pointer, and its own re-anchor then counted as
  // the reader's scroll and the view stopped short of the end (lane C's
  // medium, 11-04: back through a narrow window's workspace panel Close, or a
  // shortcut with the mouse mid-window). A frame keeps its own pointer moves
  // (moves inside it never reach this window), so the trade-off is a wheel
  // over a frame the pointer already rested on, in that second, with no move
  // over the window first: it is put back at the end.
  let movedSinceShow = true;
  let lastPointer: { x: number; y: number } | null = null;
  onMount(() => {
    const onMove = (e: PointerEvent) => {
      // A move over a frame that has no view yet comes here: by movement too.
      const position = (e.target as Element | null)?.tagName !== 'IFRAME';
      if (lastPointer && position ? e.screenX !== lastPointer.x || e.screenY !== lastPointer.y : e.movementX || e.movementY) movedSinceShow = true;
      lastPointer = position ? { x: e.screenX, y: e.screenY } : null;
      pointerOnFrame = !position;
    };
    const forget = () => { lastPointer = null; };
    const onOver = (e: PointerEvent) => { pointerOnFrame = (e.target as Element | null)?.tagName === 'IFRAME'; if (pointerOnFrame) forget(); };
    // Out to nowhere: out of the window, or into a frame (Chromium's frames
    // are other documents, so the parent sees neither `pointerover` on the
    // frame nor `:hover`). Its point says which.
    const onOut = (e: PointerEvent) => {
      if (e.relatedTarget) return;
      forget();
      const box = scrollEl?.getBoundingClientRect();
      pointerOnFrame = !!box && [...(scrollEl?.querySelectorAll('iframe') ?? [])].some(f => {
        const r = f.getBoundingClientRect();
        const left = Math.max(r.left, box.left), right = Math.min(r.right, box.right), top = Math.max(r.top, box.top), bottom = Math.min(r.bottom, box.bottom);
        return e.clientX >= left - 2 && e.clientX <= right + 2 && e.clientY >= top - 2 && e.clientY <= bottom + 2;
      });
    };
    window.addEventListener('pointermove', onMove, { passive: true, capture: true });
    window.addEventListener('pointerover', onOver, { passive: true, capture: true });
    window.addEventListener('pointerout', onOut, { passive: true, capture: true });
    window.addEventListener('pointerdown', forget, { passive: true, capture: true });
    window.addEventListener('blur', forget);
    // Off to another app (not into a frame): the pointer left with it.
    let blurCheck: ReturnType<typeof setTimeout> | undefined;
    const leftWindow = () => { clearTimeout(blurCheck); blurCheck = setTimeout(() => { if (document.activeElement?.tagName !== 'IFRAME') pointerOnFrame = false; }, 0); };
    window.addEventListener('blur', leftWindow);
    onCleanup(() => {
      window.removeEventListener('pointermove', onMove, { capture: true });
      window.removeEventListener('pointerover', onOver, { capture: true });
      window.removeEventListener('pointerout', onOut, { capture: true });
      window.removeEventListener('pointerdown', forget, { capture: true });
      window.removeEventListener('blur', forget);
      window.removeEventListener('blur', leftWindow);
      clearTimeout(blurCheck);
    });
  });
  /** A canvas frame under the pointer after a move of the reader's since the
   *  show: a scroll chained out of it is the reader's. */
  function readerOverFrame(): boolean {
    return movedSinceShow && !!scrollEl?.querySelector('iframe:hover');
  }
  /** Where the settle last put the view, and the content height then. */
  let settleAt: { top: number; height: number } | null = null;
  function startSettle(target: SettleTarget) {
    stopSettle();
    readingAnchor?.reset();
    const el = scrollEl;
    if (!el || chatBusy() || canvasFocus.held() || typeof ResizeObserver === 'undefined') return;
    const intent = scrollIntentVersion;
    const followed = () => {
      lastTop = el.scrollTop;
      settleAt = { top: el.scrollTop, height: el.scrollHeight };
      if (target.kind === 'bottom') setAtBottom(true);
    };
    settle.start(target);
    followed();
    const ro = new ResizeObserver(() => {
      // Something else let go of the end meanwhile (a reveal, a find jump):
      // the end is no longer where the view belongs.
      const away = target.kind === 'bottom' && !untrack(atBottom);
      if (away || intent !== scrollIntentVersion || jumping || canvasFocus.held() || chatBusy() || !settle.resized()) { stop(); return; }
      followed();
    });
    ro.observe(el);
    for (const child of el.children) ro.observe(child);
    const mo = new MutationObserver(records => {
      for (const r of records) r.addedNodes.forEach(n => { if (n instanceof Element) ro.observe(n); });
    });
    mo.observe(el, { childList: true });
    const timer = setTimeout(() => stop(), SETTLE_MAX_MS + 100);
    function stop() { ro.disconnect(); mo.disconnect(); clearTimeout(timer); settle.cancel(); settleAt = null; readingAnchor?.reset(); if (stopSettle === stop) stopSettle = () => settle.cancel(); }
    stopSettle = stop;
  }
  onCleanup(() => stopSettle());
  // A conversation's turns landing on screen (opened, switched to, or loaded):
  // at its end, stay there while it settles. A restored reading place takes
  // over (scroll memory, above).
  createEffect(on(() => [conversationId(), chatSessionLoading()] as const, ([, loading]) => {
    if (loading) { stopSettle(); return; }
    if (untrack(atBottom)) requestAnimationFrame(() => { if (untrack(atBottom) && !untrack(chatSessionLoading)) startSettle({ kind: 'bottom' }); });
  }, { defer: true }));
  createEffect(on(chatBusy, busy => { if (busy) stopSettle(); }, { defer: true }));
  // The loading status is in the transcript. Sample the reading place again
  // after it leaves, so later output cannot restore the status row's offset.
  createEffect(on(chatSessionLoading, loading => {
    if (!loading) readingAnchor?.reset();
  }, { defer: true }));
  /** The person's own scroll: wheel, touch, scroll keys, or the scrollbar. */
  const SCROLL_KEYS = new Set(['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End', ' ']);
  function onScrollIntent(e: Event) {
    // Focus the page moves in as the conversation shows again (U4) is the
    // app's, not the reader's: it leaves that show's settle running.
    if (e.type === 'focusin' && performance.now() - shownAt < SHOW_MS) return;
    // Anything the reader does here makes the next move up theirs (`layoutMove`),
    // a press on a control or the text and any key included.
    readerInputAt = performance.now();
    if (e.type === 'pointerdown') pressing = true;
    // Any interaction in the transcript (a click on a fold, a key, a wheel)
    // ends an opened conversation's settling: from here the place is theirs.
    stopSettle();
    finishReadingNavigation();
    // Tab, native focus and assistive focus can scroll without wheel/key
    // events. A preventScroll focus keeps the canvas anchor itself intact.
    if (e.type === 'focusin') { ++scrollIntentVersion; cancelAutoScroll(true); return; }
    if (e instanceof KeyboardEvent) {
      const t = e.target as HTMLElement | null;
      if (!SCROLL_KEYS.has(e.key) || (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable))) return;
      // Space on a button, link or summary presses it; it doesn't scroll.
      if (e.key === ' ' && t !== scrollEl && t?.closest('button, a[href], summary, [role="button"], [role="menuitem"], [tabindex]:not([tabindex="-1"])')) return;
    }
    // A sideways wheel (a wide code block or table) doesn't move the transcript.
    if (e instanceof WheelEvent && e.deltaY === 0) return;
    // A press on the scroller itself is its scrollbar (or its bare margin).
    if (e.type === 'pointerdown' && e.target !== scrollEl) return;
    ++scrollIntentVersion;
    cancelAutoScroll(true); // a newer human gesture outranks a pending send's follow
    canvasFocus.userScrolled();
  }
  onMount(() => {
    const el = scrollEl;
    if (!el) return;
    const kinds = ['wheel', 'touchmove', 'keydown', 'pointerdown', 'focusin'] as const;
    for (const k of kinds) el.addEventListener(k, onScrollIntent, { passive: true });
    // A press ends anywhere (a scrollbar drag lets go outside the scroller);
    // its last moves still count as the reader's for READER_INPUT_MS.
    const released = () => { if (pressing) { pressing = false; readerInputAt = performance.now(); } };
    // A context menu eats its pointerup; a move with no button down ends a
    // press whose release went elsewhere.
    const moved = (e: PointerEvent) => { if (pressing && e.buttons === 0) released(); };
    window.addEventListener('pointerup', released, { capture: true });
    window.addEventListener('pointercancel', released, { capture: true });
    window.addEventListener('contextmenu', released, { capture: true });
    window.addEventListener('pointermove', moved, { passive: true, capture: true });
    window.addEventListener('blur', released);
    // The transcript changed since the last frame (`layoutMove`): one
    // callback per batch, at its task's microtask checkpoint, before the
    // clamp's scroll event in the next frame. It reads no layout (see
    // `layoutMove`). A change made in this frame's own animation frames
    // (after its scroll events) keeps the flag one more frame.
    let changeFrame: number | null = null;
    let lastChangeAt = -Infinity;
    const clearChange = (frameAt: number) => {
      changeFrame = null;
      if (lastChangeAt >= frameAt) { changeFrame = requestAnimationFrame(clearChange); return; }
      changedSinceFrame = false;
    };
    const changed = new MutationObserver(() => {
      lastChangeAt = performance.now();
      changedSinceFrame = true;
      if (changeFrame === null) changeFrame = requestAnimationFrame(clearChange);
    });
    changed.observe(el, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ['class', 'style', 'open', 'hidden'] });
    onCleanup(() => {
      for (const k of kinds) el.removeEventListener(k, onScrollIntent);
      window.removeEventListener('pointerup', released, { capture: true });
      window.removeEventListener('pointercancel', released, { capture: true });
      window.removeEventListener('contextmenu', released, { capture: true });
      window.removeEventListener('pointermove', moved, { capture: true });
      window.removeEventListener('blur', released);
      changed.disconnect();
      if (changeFrame !== null) cancelAnimationFrame(changeFrame);
    });
  });
  // A click inside a canvas or HTML frame (H10, 2026-10-15): the frame is
  // sandboxed without same-origin, so its events never reach the listeners
  // above, but focus moving into it blurs this window with the frame left as
  // the active element (chatFrameFocus.ts). Handled as focus moving within
  // the transcript (`focusin` above): from here the place is theirs. A frame
  // reached by Tab may also send `focusin`; doing this twice is harmless.
  function onFrameFocus() {
    readerInputAt = performance.now();
    stopSettle();
    finishReadingNavigation();
    ++scrollIntentVersion;
    cancelAutoScroll(true);
  }
  onMount(() => {
    let again: ReturnType<typeof setTimeout> | undefined;
    const framed = () => isTranscriptFrameFocus<Element>(document.activeElement, scrollEl);
    // WebKit can report the active element late (App.tsx's blur handler):
    // look once more after the blur's task.
    const onBlur = () => {
      clearTimeout(again);
      if (framed()) onFrameFocus();
      else again = setTimeout(() => { if (framed()) onFrameFocus(); }, 0);
    };
    window.addEventListener('blur', onBlur);
    onCleanup(() => { window.removeEventListener('blur', onBlur); clearTimeout(again); });
  });
  function backToCanvas(fromKeyboard: boolean) {
    readingAnchor?.reset();
    canvasFocus.backToCanvas(true);
    settleSoon();
    // The chip goes away: from the keyboard, focus lands on the canvas asked for.
    const el = replyCanvasEl();
    if (fromKeyboard && el) {
      if (!el.hasAttribute('tabindex')) el.setAttribute('tabindex', '-1');
      el.focus({ preventScroll: true });
    }
  }
  // Reading has the lowest scroll priority: opening/restoration, sends,
  // explicit jumps and a held canvas keep their existing destinations.
  onMount(() => {
    if (!scrollEl || typeof ResizeObserver === 'undefined') return;
    readingAnchor = createReadingAnchor(scrollEl,
      () => !untrack(atBottom) && !untrack(chatSessionLoading) && !jumping
        && !readingNavigation && !pendingScrollForce && !canvasFocus.held() && !settle.active(),
      () => { lastTop = scrollEl?.scrollTop ?? lastTop; },
      () => untrack(atBottom));
    onCleanup(() => { readingAnchor?.dispose(); readingAnchor = undefined; });
  });
  createEffect(on(conversationId, finishReadingNavigation, { defer: true }));
  // The chip hid while it had focus (the canvas scrolled into view, the next
  // turn began): focus goes to the composer, not the page.
  let chipFocused = false;
  createEffect(on(focusChip, (shown, was) => {
    if (was && !shown && chipFocused) {
      chipFocused = false;
      if (!document.activeElement || document.activeElement === document.body) textarea()?.focus({ preventScroll: true });
    }
  }, { defer: true }));

  onMount(() => {
    autoScroll(true);
    // Mounted on a conversation already loaded (back from another page):
    // its frames are placeholders again, so stay at the end while they draw.
    if (chatTurns.length && !chatSessionLoading()) requestAnimationFrame(() => { if (untrack(atBottom)) startSettle({ kind: 'bottom' }); });
  });
  // Coming back to a retained conversation is a show, not a mount (U4: the
  // workspace transcript stays mounted but hidden while another page shows),
  // so the mount above doesn't run: do what it does on the show. Where the
  // reader was is taken as it hides, since a scroll event as it hides or
  // shows can move it. At the end: back to the end, and stay there while the
  // frames draw. Further up: their place, as left. A held canvas, a busy turn
  // (the follow has it) or a loading conversation are left alone.
  onMount(() => {
    const el = scrollEl;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const isHidden = () => !el.offsetParent || el.clientHeight === 0;
    let hidden = isHidden();
    let left = { end: untrack(atBottom), top: lastTop, key: untrack(chatAgentSessionKey) };
    const ro = new ResizeObserver(() => {
      const now = isHidden();
      if (now === hidden) return;
      hidden = now;
      if (now) { left = { end: untrack(atBottom), top: lastTop, key: untrack(chatAgentSessionKey) }; stopSettle(); return; }
      if (!chatTurns.length || untrack(chatSessionLoading) || untrack(chatBusy) || canvasFocus.held() || jumping) return;
      // Another conversation opened while hidden has its own place already.
      const end = left.key === untrack(chatAgentSessionKey) ? left.end : untrack(atBottom);
      if (end) { shownAt = performance.now(); movedSinceShow = false; startSettle({ kind: 'bottom' }); return; }
      if (left.key === untrack(chatAgentSessionKey) && Math.abs(el.scrollTop - left.top) > 1) { el.scrollTop = left.top; lastTop = el.scrollTop; }
      setAtBottom(false);
    });
    ro.observe(el);
    onCleanup(() => ro.disconnect());
  });

  // Auto-follow the stream while pinned to the bottom; re-runs on any turn
  // mutation (new turn, streamed token, tool call, partial→final flip).
  createEffect(() => {
    const n = chatTurns.length;
    const last = chatTurns[n - 1];
    void n; void last?.text; void last?.tools.length; void last?.partial; void last?.receipt?.id; void chatBusy();
    autoScroll();
  });

  // An approval card appearing is always something the user must see — it
  // blocks the agent — so force-scroll it into view.
  createEffect(() => {
    if (pendingApprovals().length > 0) { untrack(showLatest); autoScroll(true, true); }
  });
  // So is a question card (Paperclip PR 13): the agent ended its turn to ask.
  createEffect(() => {
    if (chatInteractions().length > 0) { untrack(showLatest); autoScroll(true, true); }
  });

  /** "Show N earlier messages": the window grows upward, and the floor is the person's. */
  function showEarlier(auto = false) {
    readingPin = false;
    const load = () => setRenderFloor(Math.max(0, renderFrom() - (auto ? AUTO_STEP : TURN_WINDOW)));
    // Scrolled to: the reader's place holds, so they read on upward. Pressed: as before, the new turns
    // sit under the view's top.
    if (auto) holdPlace(load); else load();
    trimTheEnd();
  }
  /** "Show N later messages": the end of the window moves down a step, then the oldest turns go if the
   *  window passed TURN_CAP (never one on screen). */
  function showLater(auto = false) {
    const total = chatTurns.length;
    const to = laterEnd(renderTo(), total, auto ? AUTO_STEP : undefined);
    readingPin = false;
    const grow = () => setRenderCeil(to >= total ? null : to);
    if (auto) holdPlace(grow); else grow();
    const v = visibleTurns();
    const from = trimStart({ from: renderFrom(), to: renderTo() }, v?.first ?? null);
    if (from !== null) holdPlace(() => setRenderFloor(from));
  }
  /** Reader input this recent: a wheel, touch, key or scrollbar drag. Loading by scroll needs it, so a
   *  scroll the app made (a jump, a settle) never loads turns the person did not ask for. */
  const readerRecently = () => pressing || performance.now() - readerInputAt < READER_INPUT_MS;
  /** `ref` for the window's two edge buttons: scrolling one near the view presses it, as a click would. */
  function loadAtEdge(kind: 'earlier' | 'later') {
    return (button: HTMLElement) => {
      if (typeof IntersectionObserver === 'undefined') return;
      let watch: IntersectionObserver | undefined;
      queueMicrotask(() => {
        if (!button.isConnected) return;
        watch = new IntersectionObserver(hits => {
          if (!hits.some(h => h.isIntersecting) || !readerRecently()) return;
          if (kind === 'earlier') showEarlier(true); else showLater(true);
        }, { root: scrollEl ?? null, rootMargin: '600px 0px' });
        watch.observe(button);
      });
      onCleanup(() => watch?.disconnect());
    };
  }
  /** A send (or a local command's output) goes to the end: it outranks any
   *  deferred navigation still waiting. */
  function followSend() { ++scrollIntentVersion; showLatest(); autoScroll(true); }
  /** The window reaches the end of the thread again (a send, Jump to latest), the latest turns only when it had grown past TURN_CAP. */
  function showLatest() {
    if (renderCeil() === null && chatTurns.length - renderFrom() <= TURN_CAP) return;
    readingPin = false;
    setRenderCeil(null);
    if (chatTurns.length - renderFrom() > TURN_CAP) setRenderFloor(null);
  }
  /** A control under what is read opens or closes it in place (a long
   *  message's Show more and Show less, overnight 11-04). Either is the
   *  person's own action, as a scroll is (a press on a button doesn't count
   *  as one in `onScrollIntent`): a pending follow, a held canvas and any
   *  deferred jump let go, so none of them pulls the view back to the end
   *  over what just opened or closed, a streaming reply's growth included.
   *  Opening grows the words down from where the person reads: the reading
   *  place is taken again once they have opened (the place held when the
   *  button was pressed can be that button, and holding it still would
   *  scroll the words just shown off the top), and when the growth leaves
   *  the bottom the view stops following, so Jump to latest shows and the
   *  next turn doesn't pull the reader down. Closing pulls the button up
   *  with the end of the words, so the view moves with it: it stays where
   *  it was, under the pointer and in view with keyboard focus on it,
   *  however the press came (a key's or a pointer's reset leaves no hold to
   *  rely on). */
  function toggleInPlace(control: HTMLElement, opening: boolean, toggle: () => void) {
    ++scrollIntentVersion;
    cancelAutoScroll(true);
    canvasFocus.userScrolled();
    if (opening) {
      readingAnchor?.reset();
      toggle();
      if (!nearBottom()) setAtBottom(false);
      return;
    }
    const before = control.getBoundingClientRect().top;
    toggle();
    if (scrollEl) scrollEl.scrollTop -= before - control.getBoundingClientRect().top;
    // Closing can bring the end back into view with no scroll event (it all
    // fits again): follow again then, so Jump to latest doesn't linger.
    if (nearBottom()) setAtBottom(true);
  }
  /** The "Jump to latest" pill. */
  function jumpToLatest() {
    canvasFocus.userScrolled();
    // From far back the window is the latest turns again first, and the view is at the end at once:
    // a smooth scroll over hundreds of drawn turns is what the window exists to avoid.
    const far = renderCeil() !== null || chatTurns.length - renderFrom() > TURN_CAP;
    showLatest();
    scrollToBottom(!far);
  }

  return {
    frameScope,
    scrollEl: () => scrollEl,
    setScrollEl: (el: HTMLDivElement) => { scrollEl = el; },
    onScroll,
    // Not at the bottom while the window's end gives way: the last turns are not drawn, so Jump to latest stays.
    atBottom: () => atBottom() && renderCeil() === null,
    finishedBelow,
    renderFrom,
    renderTo,
    /** The last turn on screen (the visit watermark's record of how far the person read). */
    lastOnScreen: () => visibleTurns()?.last ?? null,
    // A press loads once: the pointer events that came with it must not count as reading input for the edge loaders.
    showEarlier: () => { showEarlier(false); readerInputAt = -Infinity; },
    showLater: () => { showLater(false); readerInputAt = -Infinity; },
    loadAtEdge,
    jumpToTurn,
    jumpToLatest,
    followSend,
    toggleInPlace,
    focusChip,
    focusTitle,
    backToCanvas,
    dismissFocusChip: () => canvasFocus.dismiss(),
    setChipFocused: (focused: boolean) => { chipFocused = focused; },
  };
}

export type ChatScroll = ReturnType<typeof createChatScroll>;
