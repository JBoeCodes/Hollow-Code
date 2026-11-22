/** `<hc-ask title="Release choices" send="Send answers">…fields…</hc-ask>`:
 *  the form of an ask-canvas. It wraps the author's fields and markup and
 *  adds a title and a status line (`role=status`, polite).
 *
 *  Question cards (2026-10-14): the Send button is Hollow's, drawn under the
 *  frame (`send` names it), never inside it. Its click posts `collect
 *  {nonce}`; every form in the frame then checks its fields (`model.ts`
 *  rules). Any error stays in the frame: each field shows its message, the
 *  status counts them, focus moves to the first, and the host hears
 *  `ask-invalid {nonce, count}`. With every answer valid the frame answers
 *  with the bridge `draft` (`{canvasId, answers, summary, nonce}`) and Hollow
 *  sends it as the person's message. A frame can't send on its own: the host
 *  takes a draft only as the reply to its own Send's `collect`.
 *
 *  Locked (C4 A2): once the person sent the answer, the host posts
 *  `answered {summary}` (on every load after that too). Every form in the
 *  frame then makes its fields inert and shows the answer. */
import { attr, h, KitElement, named, part } from '../../registry';
import { KIT_ASK_FIELDS } from '../manifest';
import { buildDraft, fieldName, type FieldAnswer } from '../model';
import type { AskField } from '../field';

/** The host window, captured when the ask kit loads (after the runtime and
 *  the display kit, before any canvas script could shadow `parent`). */
const host = window.parent;

/** In the capture phase, so this listener (added before any canvas script)
 *  runs first, and a host `collect` goes no further (overnight 10 A7): a
 *  canvas script's own listener never sees the nonce, so it can't answer
 *  Hollow's Send ahead of the form. Other messages pass on as before. */
addEventListener('message', e => {
  const d = e.data;
  if (e.source !== host || !d || d.__hc !== 1) return;
  if (d.kind === 'answered') {
    for (const a of forms()) a._lock?.(String(d.summary));
    return;
  }
  if (d.kind === 'collect') {
    e.stopImmediatePropagation();
    if (typeof d.nonce === 'string') collect(d.nonce);
  }
}, true);

interface Init { canvasId: string | null }
const canvasId = (): string | null =>
  ((window as unknown as { hollowCanvas?: { init: Init | null } }).hollowCanvas?.init?.canvasId) ?? null;

const forms = (): HcAsk[] => Array.from(document.querySelectorAll('hc-ask')) as HcAsk[];

/** Hollow's Send: check every form; answer with the draft or the count of
 *  answers that need attention. */
function collect(nonce: string): void {
  const open = forms().filter(f => !f._locked && typeof f._check === 'function');
  let bad = 0;
  for (const f of open) bad += f._check();
  if (bad) {
    host.postMessage({ __hc: 1, kind: 'ask-invalid', nonce, count: bad }, '*');
    return;
  }
  const id = canvasId();
  const taken = new Set<string>();
  const answers = open.flatMap(f => f._answers(taken));
  if (!id) {
    for (const f of open) f._say(NOT_SAVED, 'warning');
    return;
  }
  const built = buildDraft(id, answers);
  if (!built.ok) {
    const say = `These answers are too long to send (${Math.ceil(built.bytes / 1024)} KB of 16 KB). Shorten a text answer.`;
    for (const f of open) f._say(say, 'danger');
    host.postMessage({ __hc: 1, kind: 'ask-invalid', nonce, count: 1 }, '*');
    return;
  }
  for (const f of open) f._say('');
  host.postMessage({ __hc: 1, kind: 'draft', canvasId: built.draft.canvasId, answers: built.draft.answers, summary: built.draft.summary, nonce }, '*');
}

export const NOT_SAVED = 'This canvas isn’t saved, so Hollow can’t send its answers. Reply in the message box instead.';

export class HcAsk extends KitElement {
  static get observedAttributes(): string[] { return ['title', 'send']; }
  _status: HTMLElement | null = null;
  _foot: HTMLElement | null = null;
  _locked = false;

  protected render(): void {
    // As hc-callout: the title moves to data-title so it isn't a tooltip.
    const moved = this.getAttribute('title');
    if (moved !== null) {
      this.setAttribute('data-title', moved);
      this.removeAttribute('title');
    }
    this.clearParts();
    const title = attr(this, 'data-title');
    named(this, 'form', title || 'Questions');
    if (title) this.prepend(part('div', 'hc-ask-title', title, { role: 'heading', 'aria-level': '2' }));
    const foot = part('div', 'hc-ask-foot');
    this._status = h('p', 'hc-ask-status', null, { role: 'status' });
    foot.append(this._status);
    this._foot = foot;
    this.append(foot);
  }

  /** The fields that belong to this form (not to a nested one). */
  _fields(): AskField[] {
    return (Array.from(this.querySelectorAll(KIT_ASK_FIELDS.join(','))) as AskField[])
      .filter(f => f.closest('hc-ask') === this && typeof f._value === 'function');
  }

  _answers(taken: Set<string> = new Set()): FieldAnswer[] {
    return this._fields().map((f, i) => ({
      name: fieldName(attr(f, 'name'), f._label(), i, taken), label: f._label(), kind: f._kind, value: f._value(), unit: f._unit(),
    }));
  }

  _say(text: string, tone = ''): void {
    if (!this._status) return;
    this._status.textContent = text;
    if (tone) this._status.setAttribute('data-tone', tone);
    else this._status.removeAttribute('data-tone');
  }

  /** Show each field's problem; say how many and focus the first. Returns
   *  the count. */
  _check(): number {
    let first: AskField | null = null;
    let bad = 0;
    for (const f of this._fields()) {
      const message = f._check();
      f._show(message);
      if (message) { bad++; first ??= f; }
    }
    if (first) {
      this._say(bad === 1 ? 'One answer needs attention.' : `${bad} answers need attention.`, 'danger');
      first._focus();
    } else this._say('');
    return bad;
  }

  /** The answers as read-only text under the form (one box at a time). */
  _box(value: string): void {
    this._foot?.querySelector('.hc-ask-copy')?.remove();
    const box = h('div', 'hc-ask-copy');
    const label = h('label', 'hc-field-label', 'Your answers');
    const text = h('textarea', 'hc-text-input', null, { readonly: '', rows: '4' }) as HTMLTextAreaElement;
    text.value = value;
    label.append(text);
    box.append(label);
    this._foot?.append(box);
  }

  /** Answered (C4 A2): the fields go inert and the answer shows. */
  _lock(summary: string): void {
    this._locked = true;
    for (const f of this._fields()) f.inert = true;
    this._say('Answered. You sent these answers.', 'success');
    this._box(summary);
  }
}
