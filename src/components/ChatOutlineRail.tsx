import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from 'solid-js';
import { conversationOutline, type TurnLike } from '../conversationTools';
import { currentPrompt, dashFor, railDashes, railTabStop } from '../chatOutlineRail';
import '../styles/chat-outline-rail.css';

// The chat outline, redone like ChatGPT's (overnight 11-12, E9). A strip of
// short dashes in the chat's left gutter, one per prompt you sent (sampled
// evenly past 40), the one for the part in view brighter and longer.
// Hovering or focusing the strip opens a card listing the prompts; a dash
// or a row jumps there through ChatBody's `jumpToTurn`. The Outline button
// in the chat's tools opens the same card (`open`), under the tools, and is
// the click path when the window is too narrow for the strip.
//
// Read only: the current dash follows the transcript's scroll (a passive
// listener, measured once per frame). It writes the chat's scroll only to pass
// on a wheel over the strip, which sits outside the scroller (forwardWheel).

export interface ChatOutlineRailProps {
  turns: readonly TurnLike[];
  scrollEl: () => HTMLElement | undefined;
  jump: (index: number) => void;
  /** The Outline button opened the card. */
  open: boolean;
  onClose: () => void;
}

type CardMode = 'rail' | 'button' | null;

export default function ChatOutlineRail(props: ChatOutlineRailProps) {
  const outline = createMemo(() => conversationOutline(props.turns));
  const dashes = createMemo(() => railDashes(outline().length));
  const [current, setCurrent] = createSignal(-1);
  const currentPos = createMemo(() => outline().findIndex(e => e.index === current()));
  const currentDash = createMemo(() => dashFor(dashes(), currentPos()));
  const [mode, setMode] = createSignal<CardMode>(null);
  let root: HTMLDivElement | undefined, strip: HTMLElement | undefined, card: HTMLDivElement | undefined, list: HTMLOListElement | undefined;

  // Which prompt is in view: the last one above a line a third down the
  // transcript (its bottom once scrolled to the end).
  let frame = 0;
  function measure() {
    frame = 0;
    const s = props.scrollEl();
    if (!s) return;
    // Show earlier adds turns without changing the count of turns: catch up.
    if (s.childElementCount !== watched) observeTurns();
    const box = s.getBoundingClientRect();
    const tops: { index: number; top: number }[] = [];
    for (const el of s.querySelectorAll<HTMLElement>('.turn.user[data-turn-index]')) {
      const index = Number(el.dataset.turnIndex);
      if (Number.isFinite(index)) tops.push({ index, top: el.getBoundingClientRect().top - box.top });
    }
    const atEnd = s.scrollHeight - s.scrollTop - s.clientHeight < 8;
    setCurrent(currentPrompt(outline().map(e => e.index), tops, atEnd ? s.clientHeight : s.clientHeight / 3));
  }
  const schedule = () => { if (!frame) frame = requestAnimationFrame(measure); };
  // Re-measured on scroll, on the scroller's own size, and on each rendered
  // turn's size (the turns are the scroller's children, so a streaming reply
  // or a late image moving the prompts counts too). Still once per frame.
  let ro: ResizeObserver | undefined, watched = -1;
  function observeTurns() {
    const s = props.scrollEl();
    if (!ro || !s) return;
    watched = s.childElementCount;
    ro.disconnect();
    ro.observe(s);
    for (const child of s.children) ro.observe(child);
  }
  onMount(() => {
    const s = props.scrollEl();
    s?.addEventListener('scroll', schedule, { passive: true });
    ro = typeof ResizeObserver === 'undefined' || !s ? undefined : new ResizeObserver(schedule);
    observeTurns();
    onCleanup(() => { s?.removeEventListener('scroll', schedule); ro?.disconnect(); if (frame) cancelAnimationFrame(frame); });
  });
  createEffect(on(() => props.turns.length, schedule));

  // The card: the Outline button's (anchored under the chat's tools) or the
  // strip's (beside it, while hovered or focused).
  let closeTimer: ReturnType<typeof setTimeout> | undefined;
  const cancelClose = () => { if (closeTimer) clearTimeout(closeTimer); closeTimer = undefined; };
  onCleanup(cancelClose);
  createEffect(on(() => props.open, open => {
    cancelClose();
    if (open) setMode('button');
    else if (mode() === 'button') setMode(null);
  }));
  // Escape hands focus back to the strip without opening the card again.
  let quiet = false;
  function peek() { if (quiet) return; cancelClose(); if (mode() !== 'button') setMode('rail'); }
  function leave() {
    if (mode() !== 'rail') return;
    cancelClose();
    closeTimer = setTimeout(() => {
      closeTimer = undefined;
      if (mode() === 'rail' && !root?.contains(document.activeElement)) setMode(null);
    }, 160);
  }
  function focusOut(e: FocusEvent) {
    const next = e.relatedTarget as Node | null;
    if (!(next && strip?.contains(next))) setFocused(null);
    if (mode() === 'rail' && !(next && root?.contains(next)) && !strip?.matches(':hover') && !card?.matches(':hover')) setMode(null);
  }
  function close() {
    const was = mode();
    cancelClose();
    setMode(null);
    if (was === 'button') {
      props.onClose();
      (root?.closest('.chat-scroll-wrap')?.querySelector<HTMLElement>('.chat-actions-menu') ?? document.querySelector<HTMLElement>('.chat-topbar-controls .chat-actions-menu'))?.focus();
    } else { quiet = true; focusDash(tabStop()); quiet = false; }
  }
  function onKeyDown(e: KeyboardEvent) {
    if (e.key === 'Escape' && mode()) { e.preventDefault(); e.stopPropagation(); close(); }
  }
  // The card opens with the current prompt in its list's view (the list's
  // own scroll, never the transcript's).
  createEffect(on(mode, m => {
    if (!m) return;
    queueMicrotask(() => {
      const row = list?.querySelector<HTMLElement>('[aria-current="location"]');
      if (list && row) list.scrollTop = Math.max(0, row.offsetTop - list.clientHeight / 2 + row.offsetHeight / 2);
    });
  }));

  // One tab stop for the strip (roving tabindex): the dash ↑, ↓, Home or End
  // moved focus to while focus stays in the strip, else the current dash
  // (#1322 re-review: Tab went back to the current dash). It follows the
  // current dash again once focus leaves the strip or the dashes change.
  const [focused, setFocused] = createSignal<number | null>(null);
  createEffect(on(() => dashes().length, () => setFocused(null), { defer: true }));
  const dashButtons = () => [...(strip?.querySelectorAll<HTMLButtonElement>('button') ?? [])];
  function focusDash(i: number) { dashButtons()[i]?.focus(); }
  function dashKeys(e: KeyboardEvent, i: number) {
    const step = e.key === 'ArrowDown' ? 1 : e.key === 'ArrowUp' ? -1 : 0;
    if (e.key === 'Home' || e.key === 'End') { e.preventDefault(); focusDash(e.key === 'Home' ? 0 : dashes().length - 1); return; }
    if (!step) return;
    e.preventDefault();
    focusDash(Math.min(dashes().length - 1, Math.max(0, i + step)));
  }
  const tabStop = createMemo(() => railTabStop(dashes().length, currentDash(), focused()));

  // The strip sits over the transcript's left edge but outside its scroller,
  // so a wheel over it would scroll nothing (overnight 11-12, C FIX-2). Hand
  // it to the scroller: the same wheel event first, so the chat's own
  // listeners count it as the person's scroll (settle, reading anchor, follow),
  // then the move itself, since a dispatched wheel scrolls nothing.
  function forwardWheel(e: WheelEvent) {
    const s = props.scrollEl();
    if (!s || e.ctrlKey) return;
    e.preventDefault();
    s.dispatchEvent(new WheelEvent('wheel', { deltaX: e.deltaX, deltaY: e.deltaY, deltaZ: e.deltaZ, deltaMode: e.deltaMode, clientX: e.clientX, clientY: e.clientY, shiftKey: e.shiftKey }));
    const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? s.clientHeight : 1;
    s.scrollBy({ left: e.deltaX * unit, top: e.deltaY * unit, behavior: 'instant' });
  }

  return (
    <div class="chat-outline-rail" ref={root} onKeyDown={onKeyDown}>
      <Show when={outline().length >= 2}>
        <nav class="chat-outline-strip" ref={strip} aria-label="Conversation outline"
          onMouseEnter={peek} onMouseLeave={leave} onFocusIn={peek} onFocusOut={focusOut} onWheel={forwardWheel}>
          <For each={dashes()}>{(pos, i) => {
            const entry = () => outline()[pos];
            return (
              <button type="button" class="chat-outline-dash" classList={{ current: i() === currentDash() }}
                aria-label={entry()?.text ?? ''} aria-current={i() === currentDash() ? 'location' : undefined}
                tabindex={i() === tabStop() ? 0 : -1}
                onKeyDown={e => dashKeys(e, i())} onFocus={() => setFocused(i())}
                onClick={() => { const at = entry(); if (at) props.jump(at.index); }}>
                <span aria-hidden="true" />
              </button>
            );
          }}</For>
        </nav>
      </Show>
      <Show when={mode()}>
        <div class="chat-outline" ref={card} classList={{ 'chat-outline-by-rail': mode() === 'rail' }} role="region" aria-label="Outline"
          onMouseEnter={cancelClose} onMouseLeave={leave} onFocusOut={focusOut}>
          <div class="chat-outline-head"><strong>Outline</strong><small>{outline().length} prompt{outline().length === 1 ? '' : 's'}</small></div>
          <ol ref={list}>
            <For each={outline()}>{(entry, i) => <li><button type="button" classList={{ current: i() === currentPos() }} aria-current={i() === currentPos() ? 'location' : undefined}
              onClick={() => props.jump(entry.index)}>
              <span class="chat-outline-n">{i() + 1}</span><span>{entry.text}</span>
            </button></li>}</For>
          </ol>
        </div>
      </Show>
    </div>
  );
}
