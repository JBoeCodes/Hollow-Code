// Inline AI in the editor (overnight 10-11 B5): the selection bar, the
// inline diff (the original struck through, the suggestion beneath with
// Accept and Reject) and the slash menu. The suggestion state, request and
// menu items are inlineAiModel.ts; the model call is the host's (`run`).
import { Facet, Prec, StateField, type EditorState, type Extension, Transaction, type TransactionSpec } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin, WidgetType, keymap, showTooltip, type DecorationSet, type Tooltip } from '@codemirror/view';
import { startCompletion, type Completion, type CompletionContext, type CompletionResult } from '@codemirror/autocomplete';
import { syntaxTree } from '@codemirror/language';
import type { SyntaxNode } from '@lezer/common';
import { parseFrontmatter } from '../utils';
import { INLINE_ACTIONS, acceptSpec, reanchor, requestFor, setSuggestion, slashItems, slashStart, suggestionField, type InlineAction, type InlineRequest, type Suggestion } from '../inlineAiModel';

/** What the editor's host supplies: the model call and a one-line prompt. */
export interface InlineAiHost {
  /** The model call, tagged with `requestId` so `cancel` can stop it. */
  run(request: InlineRequest, requestId: string): Promise<string>;
  /** Stop a pending request's model call (Dismiss, Esc, closing the file). */
  cancel?(requestId: string): void;
  ask(title: string, label: string, placeholder?: string): Promise<string | null>;
  today(): string;
  /** The open file. With it, a suggestion outlives the editor remounting
   *  when the file changes on disk (overnight S3). */
  key?: string;
}

const hostFacet = Facet.define<InlineAiHost, InlineAiHost | null>({ combine: v => v[0] ?? null });

let seq = 0;
// A UUID: the backend's registry is shared by every window. WebKit before
// Safari 15.4 (macOS 11 is supported) has no randomUUID: random words instead.
const requestIdFor = (id: number) =>
  (globalThis as { crypto?: { randomUUID?: () => string } }).crypto?.randomUUID?.()
  ?? `ai-${Date.now().toString(36)}-${id}-${Math.random().toString(36).slice(2)}${Math.random().toString(36).slice(2)}`;

/** What the carry needs of an editor (tests pass a plain state holder). */
type ViewLike = { state: EditorState; dispatch(spec: TransactionSpec): void };

/** The editor showing suggestion `id` while its answer is awaited; it moves
 *  to the new editor on a remount. */
const owners = new Map<number, ViewLike>();
/** A suggestion whose editor went away, by file, until the editor for that
 *  file comes back (a disk change remounts it at once). */
const carried = new Map<string, { s: Suggestion; doc: string; timer: ReturnType<typeof setTimeout> }>();
/** How long a carried suggestion waits; after that the file was closed and a
 *  pending request is cancelled. */
export const CARRY_MS = 2000;
export const EXPIRED_NOTICE = 'The suggestion expired because the file changed.';

/** Stop `s`'s model call if it is still being written (a dropped answer is
 *  still metered otherwise). */
function cancelPending(host: InlineAiHost | null, s: Suggestion | null | undefined) {
  if (!s) return;
  owners.delete(s.id);
  if (s.status === 'pending' && s.requestId) host?.cancel?.(s.requestId);
}

/** Start a request for `from`..`to` (or ask for an instruction first). */
async function start(view: EditorView, action: InlineAction, from: number, to: number) {
  const host = view.state.facet(hostFacet);
  if (!host) return;
  let instruction: string | undefined;
  if (action === 'translate') instruction = (await host.ask('Translate into', 'Language', 'English')) ?? undefined;
  if (action === 'ask') instruction = (await host.ask(from === to ? 'Ask Hollow to write' : 'Ask Hollow about the selection', 'Instruction', from === to ? 'A three-row table of owners and dates' : 'Make it friendlier')) ?? undefined;
  if ((action === 'translate' || action === 'ask') && !instruction) { view.focus(); return; }
  const request = requestFor(view.state, from, to, action, instruction);
  const id = ++seq;
  // A new request replaces the old one: stop the old one's call.
  cancelPending(host, view.state.field(suggestionField, false));
  if ('error' in request) {
    view.dispatch({ effects: setSuggestion.of({ id, from, to, action, status: 'error', text: '', error: request.error, request: { action, text: '' } }) });
    return;
  }
  const requestId = requestIdFor(id);
  view.dispatch({ effects: setSuggestion.of({ id, from, to, action, status: 'pending', text: '', request, requestId }) });
  run(view, id, request, requestId);
}

function run(view: EditorView, id: number, request: InlineRequest, requestId: string) {
  const host = view.state.facet(hostFacet);
  if (!host) return;
  owners.set(id, view);
  host.run(request, requestId).then(
    text => update(id, s => ({ ...s, status: 'ready', text })),
    err => update(id, s => ({ ...s, status: 'error', error: err instanceof Error ? err.message : String(err) })),
  );
}

/** Apply an answer only to the request that asked for it, in whichever
 *  editor shows it now (a newer request, or a dismissal, drops it). An
 *  answer that lands while the editor remounts is kept for the new one. */
function update(id: number, next: (s: Suggestion) => Suggestion) {
  const view = owners.get(id);
  owners.delete(id);
  if (!view) {
    for (const c of carried.values()) if (c.s.id === id && c.s.status === 'pending') c.s = next(c.s);
    return;
  }
  const s = view.state.field(suggestionField, false);
  if (!s || s.id !== id) return;
  try { view.dispatch({ effects: setSuggestion.of(next(s)) }); } catch { /* the editor is gone */ }
}

/** The editor came back for the same file: show the carried suggestion where
 *  its text now is, or say plainly that it expired. */
function reattach(view: ViewLike, c: { s: Suggestion; doc: string }) {
  const host = view.state.facet(hostFacet);
  if (view.state.field(suggestionField, false)) { cancelPending(host, c.s); return; }
  const doc = view.state.doc.toString();
  const moved = reanchor(c.s, c.doc, doc);
  if (!moved) cancelPending(host, c.s);
  else if (moved.status === 'pending') owners.set(moved.id, view);
  const at = Math.min(c.s.from, doc.length);
  const s: Suggestion = moved ?? { ...c.s, from: at, to: at, status: 'expired', text: '', error: undefined };
  try { view.dispatch({ effects: setSuggestion.of(s) }); } catch { cancelPending(host, s); }
}

/** Carries the live suggestion across a remount (overnight S3): on destroy
 *  it waits under the file's key; the next editor for that file takes it. If
 *  none comes (the file was closed), a pending request is cancelled. And the
 *  one place a pending request that leaves the field any other way (mapped
 *  away by an edit, replaced, dismissed) has its call cancelled; one lost
 *  to a change that wasn't the person's own (the document replaced in place)
 *  leaves the expired notice. Exported for the state-level tests. */
export function carrier(view: ViewLike) {
  const host = view.state.facet(hostFacet);
  const key = host?.key;
  const waiting = key ? carried.get(key) : undefined;
  if (key && waiting) {
    carried.delete(key);
    clearTimeout(waiting.timer);
    // Not inside the editor's own construction.
    queueMicrotask(() => reattach(view, waiting));
  }
  return {
    update(u: { startState: EditorState; state: EditorState; transactions: readonly Transaction[] }) {
      const before = u.startState.field(suggestionField, false);
      const after = u.state.field(suggestionField, false);
      if (!before || before.status === 'expired' || after?.id === before.id) return;
      cancelPending(host, before);
      const outside = !after && u.transactions.every(tr => tr.docChanged && !tr.annotation(Transaction.userEvent) && !tr.effects.some(e => e.is(setSuggestion)));
      if (!outside) return;
      const at = Math.min(before.from, u.state.doc.length);
      const notice: Suggestion = { ...before, from: at, to: at, status: 'expired', text: '', error: undefined };
      // Not inside the update itself.
      queueMicrotask(() => {
        if (view.state.field(suggestionField, false)) return;
        try { view.dispatch({ effects: setSuggestion.of(notice) }); } catch { /* the editor is gone */ }
      });
    },
    destroy() {
      const s = view.state.field(suggestionField, false);
      if (!s) return;
      owners.delete(s.id);
      if (!key || s.status === 'expired') { cancelPending(host, s); return; }
      const prev = carried.get(key);
      if (prev) { clearTimeout(prev.timer); cancelPending(host, prev.s); }
      const entry = {
        s,
        doc: view.state.doc.toString(),
        timer: setTimeout(() => {
          if (carried.get(key) !== entry) return;
          carried.delete(key);
          cancelPending(host, entry.s);
        }, CARRY_MS),
      };
      carried.set(key, entry);
    },
  };
}
const carry = ViewPlugin.define(view => carrier(view));

function accept(view: EditorView): boolean {
  const spec = acceptSpec(view.state);
  if (!spec) return false;
  view.dispatch(spec);
  view.focus();
  return true;
}

/** Tab accepts only with the cursor on the suggestion's lines; anywhere else
 *  it stays indent. */
function acceptByTab(view: EditorView): boolean {
  const s = view.state.field(suggestionField, false);
  if (!s || s.status !== 'ready') return false;
  const head = view.state.selection.main.head;
  const doc = view.state.doc;
  if (head < doc.lineAt(s.from).from || head > doc.lineAt(s.to).to) return false;
  return accept(view);
}

/** ⌘⇧Enter: reach the selection bar from the keyboard (←/→ move, Esc goes
 *  back to the text). */
function focusBar(view: EditorView): boolean {
  const first = view.dom.querySelector<HTMLButtonElement>('.cm-ai-bar .cm-ai-btn');
  if (!first) return false;
  first.focus();
  return true;
}

function reject(view: EditorView): boolean {
  const s = view.state.field(suggestionField, false);
  if (!s) return false;
  cancelPending(view.state.facet(hostFacet), s);
  view.dispatch({ effects: setSuggestion.of(null) });
  view.focus();
  return true;
}

function button(label: string, title: string, onClick: () => void, cls = ''): HTMLButtonElement {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = `cm-ai-btn${cls ? ` ${cls}` : ''}`;
  b.textContent = label;
  b.title = title;
  // Keep the selection: a mousedown on the bar would move the cursor.
  b.addEventListener('mousedown', e => e.preventDefault());
  b.addEventListener('click', e => { e.preventDefault(); onClick(); });
  return b;
}

/** The suggestion under the edited text: writing, the answer, or the error. */
class SuggestionWidget extends WidgetType {
  constructor(readonly s: Suggestion) { super(); }
  eq(other: SuggestionWidget) { return other.s.id === this.s.id && other.s.status === this.s.status && other.s.text === this.s.text && other.s.error === this.s.error; }
  toDOM(view: EditorView) {
    const box = document.createElement('div');
    box.className = `cm-ai-suggestion is-${this.s.status}`;
    box.setAttribute('role', 'group');
    box.setAttribute('aria-label', 'Inline AI suggestion');
    const label = INLINE_ACTIONS.find(a => a.action === this.s.action)?.label.replace('…', '') ?? 'Suggestion';
    if (this.s.status === 'pending') {
      const p = document.createElement('div');
      p.className = 'cm-ai-writing';
      p.setAttribute('role', 'status');
      p.textContent = `${label}: writing…`;
      // Dismiss stops the model call (`inline_ai_cancel`, overnight S3).
      box.append(p, button('Dismiss', 'Dismiss (Esc)', () => reject(view)));
      return box;
    }
    if (this.s.status === 'expired') {
      const p = document.createElement('div');
      p.className = 'cm-ai-expired';
      p.setAttribute('role', 'status');
      p.textContent = EXPIRED_NOTICE;
      box.append(p, button('Dismiss', 'Dismiss (Esc)', () => reject(view)));
      return box;
    }
    if (this.s.status === 'error') {
      const p = document.createElement('div');
      p.className = 'cm-ai-error';
      p.setAttribute('role', 'alert');
      p.textContent = this.s.error ?? 'It didn’t work.';
      const actions = document.createElement('div');
      actions.className = 'cm-ai-actions';
      if (this.s.request.text || this.s.request.instruction) actions.append(button('Retry', 'Ask again', () => {
        // The live suggestion, not this widget's copy: its range may have moved.
        const live = view.state.field(suggestionField, false);
        if (!live) return;
        const id = ++seq;
        const requestId = requestIdFor(id);
        view.dispatch({ effects: setSuggestion.of({ ...live, id, status: 'pending', error: undefined, requestId }) });
        run(view, id, live.request, requestId);
      }));
      actions.append(button('Dismiss', 'Dismiss (Esc)', () => reject(view)));
      box.append(p, actions);
      return box;
    }
    const text = document.createElement('div');
    text.className = 'cm-ai-new';
    text.textContent = this.s.text;
    const actions = document.createElement('div');
    actions.className = 'cm-ai-actions';
    actions.append(button('Accept', 'Accept (Tab or ⌘Enter)', () => accept(view), 'is-primary'), button('Reject', 'Reject (Esc)', () => reject(view)));
    const hint = document.createElement('span');
    hint.className = 'cm-ai-hint';
    hint.textContent = `${label} · Tab accepts · Esc rejects`;
    actions.append(hint);
    box.append(text, actions);
    return box;
  }
  ignoreEvent() { return true; }
}

const decorations = EditorView.decorations.compute([suggestionField], state => {
  const s = state.field(suggestionField, false);
  if (!s) return Decoration.none;
  const ranges = [];
  if (s.to > s.from) ranges.push(Decoration.mark({ class: s.status === 'ready' ? 'cm-ai-old' : 'cm-ai-target' }).range(s.from, s.to));
  // A block under the line the text ends on.
  const lineEnd = state.doc.lineAt(s.to).to;
  ranges.push(Decoration.widget({ widget: new SuggestionWidget(s), block: true, side: 1 }).range(lineEnd));
  return Decoration.set(ranges, true) as DecorationSet;
});

/** The selection bar: above a non-empty selection, while nothing is being
 *  suggested. */
const barField = StateField.define<readonly Tooltip[]>({
  create: state => bar(state),
  update: (tooltips, tr) => (tr.docChanged || tr.selection || tr.effects.some(e => e.is(setSuggestion)) ? bar(tr.state) : tooltips),
  provide: f => showTooltip.computeN([f], state => state.field(f)),
});

function bar(state: EditorState): readonly Tooltip[] {
  const sel = state.selection.main;
  if (sel.empty || !state.facet(hostFacet) || state.field(suggestionField, false) || state.readOnly) return [];
  return [{
    pos: sel.from,
    above: true,
    strictSide: false,
    arrow: false,
    create: view => {
      const dom = document.createElement('div');
      dom.className = 'cm-ai-bar';
      dom.setAttribute('role', 'toolbar');
      dom.setAttribute('aria-label', 'Inline AI (⇧⌘Enter from the text)');
      dom.addEventListener('keydown', e => {
        const buttons = [...dom.querySelectorAll<HTMLButtonElement>('.cm-ai-btn')];
        const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
        if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
          e.preventDefault();
          buttons[(at + (e.key === 'ArrowRight' ? 1 : -1) + buttons.length) % buttons.length]?.focus();
        } else if (e.key === 'Escape') {
          e.preventDefault(); e.stopPropagation(); view.focus();
        }
      });
      for (const a of INLINE_ACTIONS) {
        dom.append(button(a.label, a.title, () => { const s = view.state.selection.main; void start(view, a.action, s.from, s.to); }));
      }
      return { dom };
    },
  }];
}

/** The slash menu: `/` at the start of a line. */
function slashComplete(context: CompletionContext): CompletionResult | null {
  const host = context.state.facet(hostFacet);
  if (!host) return null;
  const line = context.state.doc.lineAt(context.pos);
  const before = context.state.sliceDoc(line.from, context.pos);
  const at = slashStart(before);
  if (at === null) return null;
  // Not inside code, nor in the frontmatter at the top.
  for (let node: SyntaxNode | null = syntaxTree(context.state).resolveInner(context.pos, -1); node; node = node.parent) {
    if (/Code/.test(node.name)) return null;
  }
  const fm = parseFrontmatter(context.state.doc.toString());
  if (fm && context.pos < fm.bodyStart) return null;
  const from = line.from + at;
  // In the menu's own order (a boost per place), with no type icon.
  const items = slashItems(host.today());
  const options: Completion[] = items.map((item, i) => ({
    label: `/${item.label}`,
    displayLabel: item.label,
    detail: item.detail,
    boost: items.length - i,
    apply: (view: EditorView, _c: Completion, f: number, t: number) => {
      if (item.id === 'ask') {
        view.dispatch({ changes: { from: f, to: t, insert: '' } });
        void start(view, 'ask', f, f);
        return;
      }
      const insert = item.insert ?? '';
      view.dispatch({ changes: { from: f, to: t, insert }, selection: { anchor: f + insert.length }, userEvent: 'input.complete' });
      if (item.id === 'link') startCompletion(view);
    },
  }));
  return { from, options, validFor: /^\/[a-z-]*$/i, filter: true };
}

/** The extension, with the host's model call. */
export function inlineAi(host: InlineAiHost): Extension {
  return [
    hostFacet.of(host),
    suggestionField,
    decorations,
    barField,
    carry,
    Prec.highest(keymap.of([
      { key: 'Tab', run: acceptByTab },
      { key: 'Mod-Shift-Enter', run: focusBar },
      { key: 'Mod-Enter', run: accept },
      { key: 'Escape', run: reject },
    ])),
  ];
}

export { slashComplete };
