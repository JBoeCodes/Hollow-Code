/**
 * The docked question cards (2026-10-14), the pure half: one question at a
 * time over the composer, as Claude Code asks. Which rows a question shows,
 * where the keyboard cursor starts, what a key does, and how many questions
 * are waiting. `QuestionDock.tsx` draws it; `questionCards.ts` still owns
 * validity and the answer `resolve_interaction` receives.
 *
 * Keys are plain (no ⌘, ⌃ or ⌥), so none can overlap a macOS shortcut, and a
 * key with any of those held is left alone. Every key has a button too.
 *
 * The node vitest suite loads this module directly (solid-js signals only).
 */
import { createSignal } from 'solid-js';
import type { ChatInteraction, InteractionQuestion, InteractionRequest } from './types';
import { formErrors, optionOutcome, type QuestionFormAnswer } from './questionCards';

export type DockRow =
  | { kind: 'option'; id: string; label: string; description: string | null; outcome: string | null; recommended: boolean }
  | { kind: 'other'; label: string };

/** A choice question's rows: its options, then "Other…" when it takes the
 *  person's own words. A text question has none. */
export function dockRows(q: InteractionQuestion): DockRow[] {
  if (q.answerMode === 'text') return [];
  const rows: DockRow[] = (q.options ?? []).map(o => ({
    kind: 'option' as const, id: o.id, label: o.label, description: o.description ?? null, outcome: optionOutcome(o) || null, recommended: !!o.recommended,
  }));
  if (q.allowCustom) rows.push({ kind: 'other', label: q.customLabel?.trim() || 'Other…' });
  return rows;
}

/** Where the cursor starts: on the recommended option, else the first row. */
export function startCursor(q: InteractionQuestion): number {
  const at = dockRows(q).findIndex(r => r.kind === 'option' && r.recommended);
  return at < 0 ? 0 : at;
}

export type DockAction =
  | { type: 'move'; to: number }
  /** Choose row `index` (single: and go on; multi: tick or untick it). */
  | { type: 'pick'; index: number }
  /** Go on with what is chosen (multi), or choose the cursor's row (single). */
  | { type: 'confirm' }
  | { type: 'back' }
  | { type: 'hide' };

export interface DockKey { key: string; shiftKey?: boolean; metaKey?: boolean; ctrlKey?: boolean; altKey?: boolean; isComposing?: boolean }

/** What a key does in a choice question's list, or null to leave it alone. */
export function dockKey(e: DockKey, ctx: { mode: 'single' | 'multi'; rows: number; cursor: number }): DockAction | null {
  if (e.metaKey || e.ctrlKey || e.altKey || e.isComposing) return null;
  const last = ctx.rows - 1;
  switch (e.key) {
    case 'ArrowDown': return { type: 'move', to: ctx.rows ? (ctx.cursor >= last ? 0 : ctx.cursor + 1) : 0 };
    case 'ArrowUp': return { type: 'move', to: ctx.rows ? (ctx.cursor <= 0 ? last : ctx.cursor - 1) : 0 };
    case 'Home': return { type: 'move', to: 0 };
    case 'End': return { type: 'move', to: Math.max(0, last) };
    case 'ArrowLeft': return { type: 'back' };
    case 'Escape': return { type: 'hide' };
    case 'Enter': return e.shiftKey ? null : ctx.mode === 'single' ? { type: 'pick', index: ctx.cursor } : { type: 'confirm' };
    case ' ': return { type: 'pick', index: ctx.cursor };
  }
  if (!e.shiftKey && /^[1-9]$/.test(e.key)) {
    const index = Number(e.key) - 1;
    return index < ctx.rows ? { type: 'pick', index } : null;
  }
  return null;
}

/** How long after something moves under the person a key or click can't
 *  answer (ms): the dock just showed, so a key meant for the message box a
 *  moment ago lands in it; or an ask-canvas's frame just changed height, so
 *  Hollow's Send slid under the pointer. */
export const GRACE_MS = 500;

/** Whether an answering key or click comes too soon after `since` (ms, when
 *  the thing last moved; null: never). The dock's choosing keys, Enter in its
 *  fields, and a click on an ask-canvas's Send all wait it out (2026-10-15). */
export function tooSoon(since: number | null, now: number, ms = GRACE_MS): boolean {
  return since !== null && now - since < ms;
}

/** Why this one question can't go on yet, or null. */
export function stepError(q: InteractionQuestion, a: QuestionFormAnswer | undefined): string | null {
  const answers = { [q.id]: a ?? { selected: [], otherOn: false, other: '', text: '' } };
  const request: InteractionRequest = { mode: 'questions', supersedeOnUserMessage: true, questions: [q] };
  return formErrors(request, { answers, accepted: null, note: '', selected: [], verdicts: {} })[q.id] ?? null;
}

/** One line for a step already answered (the dock's "1 of 3" trail). */
export function answerWords(q: InteractionQuestion, a: QuestionFormAnswer | undefined): string {
  if (!a) return 'No answer';
  if (q.answerMode === 'text') return a.text.trim() || 'No answer';
  const picked = (q.options ?? []).filter(o => a.selected.includes(o.id)).map(o => o.label);
  if (a.otherOn && a.other.trim()) picked.push(a.other.trim());
  return picked.length ? picked.join(', ') : 'No answer';
}

/** How many questions the cards hold: a questions card counts each, any
 *  other card counts one. */
export function questionCount(cards: Pick<ChatInteraction, 'request'>[]): number {
  return cards.reduce((n, c) => n + (c.request.mode === 'questions' ? Math.max(1, c.request.questions?.length ?? 0) : 1), 0);
}

/** The chip over the composer while the dock is hidden. */
export function waitingWords(n: number): string {
  return n === 1 ? '1 question waiting' : `${n} questions waiting`;
}

/** Whether the dock shows: the first waiting card, until the person hides
 *  it. A new card opens it again. */
export function createQuestionDock(cards: () => ChatInteraction[]) {
  const [hiddenFor, setHiddenFor] = createSignal<string | null>(null);
  // An out-of-date card can't be answered: it waits behind the chip and
  // opens only when the person asks for it.
  const [shown, setShown] = createSignal<string | null>(null);
  const first = () => cards()[0] ?? null;
  const open = () => {
    const c = first();
    if (!c || hiddenFor() === c.id) return false;
    return c.status === 'pending' || shown() === c.id;
  };
  return {
    first,
    open,
    /** Keep this card behind the chip (the person was typing when it came). */
    hideFor: (id: string) => setHiddenFor(id),
    /** The first card's id (or null): what the dock is keyed by, so a fresh
     *  copy of the same card, or hiding and showing it, never restarts the
     *  person's answers (hidden, it stays mounted). */
    cardId: () => first()?.id ?? null,
    hide: () => setHiddenFor(first()?.id ?? null),
    show: () => { setHiddenFor(null); setShown(first()?.id ?? null); },
  };
}
