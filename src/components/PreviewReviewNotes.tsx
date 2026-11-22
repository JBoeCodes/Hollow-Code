import { Index, Show, createEffect, createMemo, createSignal, on } from 'solid-js';
import { Icon } from '../icons';
import { flashFooter } from '../store';
import '../styles/review-notes.css';
import {
  MAX_NOTE_CHARS, MAX_REVIEW_LINES, composeBatch, loadReviewNotes, makeNote, markSent, newNoteId, pendingNotes,
  reanchor, saveReviewNotes, sameNotes, sendAgain, setResolved, splitLines, type ReviewNote,
} from '../reviewNotes';

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

/** One note: its words, Resolve or Reopen, Delete, and whether it is already in a draft. */
function NoteItem(props: {
  note: ReviewNote;
  /** Why the note is listed apart from its line (the line moved, was removed or is not shown). */
  flag?: string;
  onResolve: (resolved: boolean) => void;
  onDelete: () => void;
  onSendAgain: () => void;
}) {
  const line = () => props.note.line;
  return (
    <li class="rn-note" classList={{ 'is-resolved': props.note.resolved, 'is-lost': props.note.lost }}>
      <Show when={props.flag}>
        <p class="rn-flag"><Icon name="alert" size={12} /> {props.flag}: <span class="rn-quote">{`"${props.note.anchorText}"`}</span></p>
      </Show>
      <p class="rn-note-text">{props.note.text}</p>
      <div class="rn-note-actions">
        <button type="button" class="btn ghost small" aria-label={`${props.note.resolved ? 'Reopen' : 'Resolve'} the note on line ${line()}`}
          onClick={() => props.onResolve(!props.note.resolved)}>{props.note.resolved ? 'Reopen' : 'Resolve'}</button>
        <button type="button" class="btn ghost small" aria-label={`Delete the note on line ${line()}`} onClick={() => props.onDelete()}>Delete</button>
        <Show when={props.note.resolved}><span class="rn-chip">Resolved: not sent</span></Show>
        <Show when={!props.note.resolved && props.note.sentAt !== undefined}>
          <span class="rn-chip">In your message</span>
          <button type="button" class="btn ghost small" aria-label={`Send the note on line ${line()} again`} onClick={() => props.onSendAgain()}>Send again</button>
        </Show>
      </div>
    </li>
  );
}

/**
 * Review notes on a text file in the Preview pane (overnight 11-17, row OC9):
 * a line-numbered view of the file's first 1500 lines where a person leaves a
 * note on any line, then sends every open note to the chat as ONE message
 * (`composeBatch`). The notes are kept per file in this browser's storage and
 * follow their lines when the file changes (`reanchor`). The draft is filled,
 * never sent: the person reads it and presses Send. Loaded on demand from
 * ChatPreview.
 */
export default function PreviewReviewNotes(props: { path: string; text: string; truncated?: boolean; onSend: (text: string) => Promise<boolean> | boolean }) {
  let root: HTMLDivElement | undefined;
  const [notes, setNotes] = createSignal<ReviewNote[]>(loadReviewNotes(props.path));
  const [editing, setEditing] = createSignal<number | null>(null);
  const [draft, setDraft] = createSignal('');
  const [active, setActive] = createSignal(1);

  function commit(next: ReviewNote[]) {
    setNotes(next);
    saveReviewNotes(props.path, next);
  }
  // The file was read again (an agent's edit, Refresh): notes follow their lines.
  createEffect(on(() => props.text, text => {
    const next = reanchor(notes(), text);
    if (!sameNotes(next, notes())) commit(next);
  }));

  const allLines = createMemo(() => splitLines(props.text));
  const shown = createMemo(() => allLines().slice(0, MAX_REVIEW_LINES));
  const cut = () => allLines().length > MAX_REVIEW_LINES;
  const byLine = createMemo(() => {
    const map = new Map<number, ReviewNote[]>();
    for (const n of [...notes()].sort((a, b) => a.createdAt - b.createdAt)) {
      if (n.lost || n.line > shown().length) continue;
      map.set(n.line, [...(map.get(n.line) ?? []), n]);
    }
    return map;
  });
  const outside = createMemo(() => notes().filter(n => n.lost || n.line > shown().length).sort((a, b) => a.line - b.line || a.createdAt - b.createdAt));
  const pending = createMemo(() => pendingNotes(notes()));
  const inDraft = createMemo(() => notes().filter(n => !n.resolved && n.sentAt !== undefined).length);
  const resolved = createMemo(() => notes().filter(n => n.resolved).length);

  const focusAdd = (line: number) => {
    const at = Math.min(Math.max(1, line), Math.max(1, shown().length));
    setActive(at);
    queueMicrotask(() => root?.querySelector<HTMLElement>(`[data-add-line="${at}"]`)?.focus());
  };
  const flagFor = (n: ReviewNote) => (n.lost ? 'Line moved or removed' : `Line ${n.line} is past the first ${MAX_REVIEW_LINES} lines`);

  function openEditor(line: number) {
    setDraft('');
    setEditing(line);
  }
  function save() {
    const line = editing();
    const text = draft().trim();
    if (line === null || !text) return;
    if (line > allLines().length) { setEditing(null); return; }
    commit([...notes(), makeNote(props.path, allLines(), line, text, Date.now(), newNoteId())]);
    setEditing(null);
    focusAdd(line);
  }
  function cancel() {
    const line = editing();
    setEditing(null);
    if (line !== null) focusAdd(line);
  }
  function remove(note: ReviewNote) {
    commit(notes().filter(n => n.id !== note.id));
    if (!note.lost && note.line <= shown().length) focusAdd(note.line);
  }
  function onEditorKey(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); cancel(); }
    else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); save(); }
  }
  // One tab stop for all the lines; the arrow keys, Home, End and Page keys
  // move between their "Add a note" buttons.
  function onListKey(e: KeyboardEvent) {
    const target = e.target as HTMLElement;
    if (!target.matches?.('.rn-add')) return;
    const at = Number(target.dataset.addLine);
    const step = ({ ArrowDown: 1, ArrowUp: -1, PageDown: 10, PageUp: -10 } as Record<string, number | undefined>)[e.key];
    if (step !== undefined) { e.preventDefault(); focusAdd(at + step); }
    else if (e.key === 'Home') { e.preventDefault(); focusAdd(1); }
    else if (e.key === 'End') { e.preventDefault(); focusAdd(shown().length); }
  }

  const [sending, setSending] = createSignal(false);
  async function send() {
    const batch = composeBatch(props.path, pending());
    if (!batch.text || sending()) return;
    setSending(true);
    try {
      if (!(await props.onSend(batch.text))) return;
      commit(markSent(notes(), new Set(batch.included.map(n => n.id)), Date.now()));
      const more = batch.omitted ? ` ${plural(batch.omitted, 'more note')} did not fit in one message: send this one, then press the button again.` : '';
      flashFooter(`Added ${plural(batch.included.length, 'review note')} to your message. Nothing is sent until you press Send.${more}`, 6000);
    } finally { setSending(false); }
  }

  const noteItem = (note: () => ReviewNote, flag?: () => string | undefined) => (
    <NoteItem note={note()} flag={flag?.()}
      onResolve={r => commit(setResolved(notes(), note().id, r))}
      onDelete={() => remove(note())}
      onSendAgain={() => commit(sendAgain(notes(), note().id))} />
  );

  return (
    <div class="rn" ref={root}>
      <p class="hint chat-preview-hint rn-hint">
        {cut() ? `Showing the first ${MAX_REVIEW_LINES} of ${allLines().length} lines. ` : ''}
        {props.truncated ? 'Only the first 1.5 MB of the file was read. ' : ''}
        Choose the plus beside a line to leave a note on it.
      </p>

      <Show when={outside().length}>
        <section class="rn-outside" aria-label="Notes whose line moved, was removed or is not shown">
          <h3 class="rn-outside-title">Notes without a line on screen</h3>
          <ul class="rn-notes">
            <Index each={outside()}>{note => noteItem(note, () => flagFor(note()))}</Index>
          </ul>
        </section>
      </Show>

      <div class="rn-list" role="group" aria-label="File lines with review notes" onKeyDown={onListKey}>
        <Index each={shown()}>{(text, i) => {
          const n = i + 1;
          return (
            <div class="rn-row" data-row-line={n}>
              <div class="rn-line">
                <span class="rn-gutter" aria-hidden="true">{n}</span>
                <span class="rn-text">{text() || ' '}</span>
                <button type="button" class="rn-add" data-add-line={n} tabIndex={active() === n || (n === 1 && active() > shown().length) ? 0 : -1}
                  aria-label={`Add a note on line ${n}`} title={`Add a note on line ${n}`}
                  onFocus={() => setActive(n)} onClick={() => openEditor(n)}><Icon name="plus" size={14} /></button>
              </div>
              <Show when={byLine().get(n)}>{list =>
                <ul class="rn-notes"><Index each={list()}>{note => noteItem(note)}</Index></ul>
              }</Show>
              <Show when={editing() === n}>
                <form class="rn-editor" onSubmit={e => { e.preventDefault(); save(); }}>
                  <label class="rn-editor-label" for="rn-editor-input">Note on line {n}</label>
                  <textarea id="rn-editor-input" class="rn-editor-input" rows={3} maxLength={MAX_NOTE_CHARS} value={draft()}
                    aria-describedby="rn-editor-keys" onInput={e => setDraft(e.currentTarget.value)} onKeyDown={onEditorKey}
                    ref={el => queueMicrotask(() => el.focus())} />
                  <div class="rn-editor-actions">
                    <button type="submit" class="btn primary small" disabled={!draft().trim()}>Save note</button>
                    <button type="button" class="btn ghost small" onClick={cancel}>Cancel</button>
                    <span id="rn-editor-keys" class="rn-keys">Ctrl or Cmd+Enter saves, Esc cancels</span>
                  </div>
                </form>
              </Show>
            </div>
          );
        }}</Index>
      </div>

      <div class="rn-footer" role="group" aria-label="Review notes">
        <span class="rn-count" role="status">
          {plural(pending().length, 'note')}
          <Show when={inDraft() > 0}>{`, ${inDraft()} in your message`}</Show>
          <Show when={resolved() > 0}>{`, ${resolved()} resolved`}</Show>
        </span>
        <button type="button" class="btn primary small" disabled={pending().length === 0 || sending()} onClick={() => void send()}>
          <Icon name="send" size={13} /> {pending().length ? `Send ${plural(pending().length, 'note')} to chat` : 'Send notes to chat'}
        </button>
      </div>
    </div>
  );
}
