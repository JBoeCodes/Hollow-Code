// Inline AI in the editor (overnight 10-11 B5): the parts with no view. The
// suggestion state (a StateField that follows its text through edits), the
// request built from a selection, and the slash menu's matching and items.
// The view (the selection bar, the inline diff, the keys) is
// editor/inlineAi.ts; the model call is the backend's `inline_ai`.
import { StateEffect, StateField, type EditorState, type TransactionSpec } from '@codemirror/state';

export type InlineAction = 'rewrite' | 'shorten' | 'expand' | 'fix' | 'translate' | 'ask';

export const INLINE_ACTIONS: readonly { action: InlineAction; label: string; title: string }[] = [
  { action: 'rewrite', label: 'Rewrite', title: 'Rewrite it to read better' },
  { action: 'shorten', label: 'Shorten', title: 'About half as long, every fact kept' },
  { action: 'expand', label: 'Expand', title: 'More detail, same voice, no invented facts' },
  { action: 'fix', label: 'Fix', title: 'Spelling, grammar and punctuation only' },
  { action: 'translate', label: 'Translate…', title: 'Into a language you name' },
  { action: 'ask', label: 'Ask…', title: 'Your own instruction' },
];

/** The longest selection a request may carry (the backend's limit). */
export const MAX_SELECTION = 20_000;
/** Surrounding text sent for tone and terms, each side. */
export const CONTEXT_CHARS = 600;

export interface InlineRequest { action: InlineAction; text: string; instruction?: string; context?: string }

/** A suggestion for `from`..`to` (an insertion when they're equal). */
export interface Suggestion {
  id: number;
  from: number;
  to: number;
  action: InlineAction;
  /** `expired`: the file changed on disk under it (the editor remounted)
   *  and its text is gone; only a notice is left (overnight S3). */
  status: 'pending' | 'ready' | 'error' | 'expired';
  text: string;
  error?: string;
  request: InlineRequest;
  /** The backend's id for a sent request, for `inline_ai_cancel`. */
  requestId?: string;
}

/** How much text either side of an insertion point finds it again. */
const ANCHOR_CHARS = 40;

/** `s` in `newDoc` after the file changed on disk, or null when its text is
 *  gone: the same selected text (the occurrence nearest where it was), or,
 *  for an insertion, the same text around the cursor. */
export function reanchor(s: Suggestion, oldDoc: string, newDoc: string): Suggestion | null {
  if (oldDoc === newDoc) return s;
  const nearest = (needle: string, near: number): number => {
    let best = -1;
    for (let i = newDoc.indexOf(needle); i !== -1; i = newDoc.indexOf(needle, i + 1)) {
      if (best === -1 || Math.abs(i - near) < Math.abs(best - near)) best = i;
      if (needle === '') break;
    }
    return best;
  };
  if (s.to > s.from) {
    const at = nearest(oldDoc.slice(s.from, s.to), s.from);
    return at === -1 ? null : { ...s, from: at, to: at + (s.to - s.from) };
  }
  const before = oldDoc.slice(Math.max(0, s.from - ANCHOR_CHARS), s.from);
  const after = oldDoc.slice(s.from, s.from + ANCHOR_CHARS);
  if (!before && !after) return newDoc ? null : s;
  const at = nearest(before + after, s.from - before.length);
  return at === -1 ? null : { ...s, from: at + before.length, to: at + before.length };
}

export const setSuggestion = StateEffect.define<Suggestion | null>();

/** The one live suggestion. It follows its text through edits elsewhere and
 *  goes away when its text is deleted. */
export const suggestionField = StateField.define<Suggestion | null>({
  create: () => null,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setSuggestion)) return e.value;
    if (!value || !tr.docChanged) return value;
    // An edit inside the text being worked on makes the answer stale (it
    // was written for the old text): the suggestion goes.
    let inside = false;
    tr.changes.iterChangedRanges((fromA, toA) => {
      if (fromA < value.to && toA > value.from) inside = true;
      else if (fromA === toA && fromA > value.from && fromA < value.to) inside = true;
    });
    if (inside) return null;
    const empty = value.from === value.to;
    const from = tr.changes.mapPos(value.from, empty ? -1 : 1);
    const to = tr.changes.mapPos(value.to, empty ? -1 : -1);
    if (!empty && to <= from) return null;
    return { ...value, from, to: Math.max(from, to) };
  },
});

/** The request for `from`..`to` of `state`: the text and a little context. */
export function requestFor(state: EditorState, from: number, to: number, action: InlineAction, instruction?: string): InlineRequest | { error: string } {
  const text = state.sliceDoc(from, to);
  if (text.length > MAX_SELECTION) return { error: `That's too long for inline AI (over ${MAX_SELECTION.toLocaleString('en-US')} characters). Select less.` };
  // Never cut an emoji (a UTF-16 pair) in half: a lone surrogate doesn't
  // survive the trip to the backend.
  let start = Math.max(0, from - CONTEXT_CHARS);
  if (start > 0 && isLowSurrogate(state.sliceDoc(start, start + 1))) start += 1;
  let end = Math.min(state.doc.length, to + CONTEXT_CHARS);
  if (end < state.doc.length && isHighSurrogate(state.sliceDoc(end - 1, end))) end -= 1;
  const before = state.sliceDoc(start, from);
  const after = state.sliceDoc(to, end);
  const context = `${before}[…]${after}`.trim();
  return { action, text, ...(instruction ? { instruction } : {}), ...(context && context !== '[…]' ? { context } : {}) };
}

const isLowSurrogate = (c: string) => { const n = c.charCodeAt(0); return n >= 0xdc00 && n <= 0xdfff; };
const isHighSurrogate = (c: string) => { const n = c.charCodeAt(0); return n >= 0xd800 && n <= 0xdbff; };

/** The reply as it replaces the selection: the selection's own leading and
 *  trailing whitespace (a line's newline, a list item's indent) put back,
 *  since the model's reply comes trimmed. */
export function replacementText(selected: string, reply: string): string {
  if (!selected.trim()) return reply;
  const lead = /^\s*/.exec(selected)![0];
  const trail = /\s*$/.exec(selected)![0];
  return `${lead}${reply.trim()}${trail}`;
}

/** Accepting: the suggestion replaces its text in one undoable step. */
export function acceptSpec(state: EditorState): TransactionSpec | null {
  const s = state.field(suggestionField, false);
  if (!s || s.status !== 'ready') return null;
  const insert = replacementText(state.sliceDoc(s.from, s.to), s.text);
  return {
    changes: { from: s.from, to: s.to, insert },
    selection: { anchor: s.from + insert.length - (/\s*$/.exec(insert)![0].length) },
    effects: setSuggestion.of(null),
    userEvent: 'input.ai',
    scrollIntoView: true,
  };
}

// ── The slash menu ──────────────────────────────────────────────────────

export type SlashItem = { id: string; label: string; detail: string; insert?: string };

/** `today`: the date the Date item inserts (YYYY-MM-DD). */
export function slashItems(today: string): SlashItem[] {
  return [
    { id: 'task', label: 'Task', detail: 'A checkbox line', insert: '- [ ] ' },
    { id: 'table', label: 'Table', detail: 'Three columns', insert: '| Column | Column | Column |\n| --- | --- | --- |\n|  |  |  |\n' },
    { id: 'callout', label: 'Callout', detail: 'A note box', insert: '> [!note] ' },
    { id: 'date', label: 'Date', detail: today, insert: today },
    { id: 'link', label: 'Link a note', detail: '[[ then pick a note', insert: '[[' },
    { id: 'ask', label: 'Ask Hollow…', detail: 'Write it for me' },
  ];
}

/** Where a slash command starts on this line, when the text before the
 *  cursor is only `/word` after indentation; otherwise null. */
export function slashStart(lineBefore: string): number | null {
  const m = /^(\s*)\/([a-z-]*)$/i.exec(lineBefore);
  return m ? m[1].length : null;
}
