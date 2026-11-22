/** The base every ask field shares (Canvas C4 A1). A field is a light-DOM
 *  element with a `label` (its visible question), a `name` (what the agent
 *  reads its answer under; made from the label when missing) and an
 *  optional `required`. On upgrade it renders a native, labelled control
 *  group as parts and keeps the author's rows in the hidden source wrapper,
 *  like the display kit. The value lives on the element (`_value()`), so a
 *  re-render keeps what the person chose. Validation rules and texts are
 *  `model.ts`'s; the field only shows them: a message under the control
 *  (`aria-describedby`, `aria-invalid`), cleared live once the answer is
 *  fixed. */
import { attr, h, keepView, KitElement, part, savedView, uid } from '../registry';
import { isIsoDate, validate, type AnswerValue, type FieldKind, type FieldRule } from './model';

/** Text rows of a source wrapper: one per `<li>`, else per child element,
 *  else per line of text. */
export function rowsOf(box: HTMLElement): string[] {
  const items = box.querySelectorAll('li');
  const els = items.length ? Array.from(items) : Array.from(box.children);
  return els.length ? els.map(e => e.textContent || '') : (box.textContent || '').split('\n');
}

/** "★ Recommended": a glyph and the word, never colour alone. */
export function recommended(): HTMLElement {
  const badge = h('span', 'hc-opt-pick');
  badge.append(h('span', 'hc-icon', '★', { 'aria-hidden': 'true' }), h('span', '', 'Recommended'));
  return badge;
}

export abstract class AskField extends KitElement {
  abstract readonly _kind: FieldKind;
  abstract _value(): AnswerValue;
  abstract _rule(): FieldRule;
  /** Where the error message is shown, and the control group it describes. */
  _err: HTMLElement | null = null;
  _group: HTMLElement | null = null;

  _label(): string { return attr(this, 'label'); }
  _unit(): string { return ''; }
  _check(): string | null { return validate(this._rule(), this._value()); }

  /** Focus the first control (after a failed Send). */
  _focus(): void {
    const el = this.querySelector<HTMLElement>('[data-hc-control]:not([disabled])');
    el?.focus();
  }

  /** Show `message` under the field, or clear it. */
  _show(message: string | null): void {
    const err = this._err;
    const group = this._group;
    if (!err || !group) return;
    err.hidden = !message;
    err.replaceChildren();
    if (message) err.append(h('span', 'hc-icon', '!', { 'aria-hidden': 'true' }), h('span', '', message));
    // aria-invalid only where ARIA supports it: a radiogroup and the
    // text, number, date and checkbox controls (marked data-hc-describe).
    const marked = Array.from(group.querySelectorAll('[data-hc-describe]'));
    if (group.hasAttribute('data-hc-describe')) marked.push(group);
    for (const el of marked) {
      if (message) el.setAttribute('aria-invalid', 'true');
      else el.removeAttribute('aria-invalid');
    }
  }

  /** Call when the person changes the answer: an error already showing is
   *  re-checked, so it clears as soon as the answer is fixed. The answer is
   *  kept in the canvas's saved state (P1-33), so a frame that loads again
   *  (scrolled far away and back, Read as text, a grant) shows it. */
  _changed(): void {
    if (this._err && !this._err.hidden) this._show(this._check());
    keepView(this, this._value());
  }

  /** The kept answer back (P1-33): checked against the field's kind, then
   *  drawn. Text keeps its trimmed value; an empty answer keeps the default. */
  _restore(): void {
    // Dot access only: the kit build mangles `_` names, never quoted ones.
    const v = savedView(this) as never, k = this._kind, s = this as unknown as { _v: unknown; _sel: unknown; _order: unknown };
    if (k == 'multi' || k == 'rank' ? !Array.isArray(v) : k == 'matrix' ? !v || typeof v != 'object' || Array.isArray(v) : typeof v != (k == 'range' ? 'number' : 'string') || k == 'date' && !isIsoDate(v)) return;
    if (k == 'multi') s._sel = new Set(v);
    else if (k == 'rank') s._order = v;
    else if (k == 'text' || k == 'date' || k == 'range') s._v = v;
    else s._sel = k == 'matrix' ? Object.assign(Object.create(null), v) : v;
    this.render();
  }

  /** The field's frame as parts: a fieldset with the label as its legend
   *  (`group`), or a plain block whose label is a `<label for>` on the
   *  control (`for`). Returns the frame and the element controls go in. */
  protected _frame(role: string | null, labelFor?: string): HTMLElement {
    this.clearParts();
    const label = this._label();
    const errId = uid('hcq-err');
    const required = this.hasAttribute('required');
    const frame = part(role ? 'fieldset' : 'div', 'hc-field');
    const head = h(role ? 'legend' : 'label', 'hc-field-label', label || attr(this, 'name') || 'Question');
    if (labelFor) head.setAttribute('for', labelFor);
    if (required) head.append(h('span', 'hc-req', ' (required)'));
    frame.append(head);
    if (role) {
      if (role !== 'group') frame.setAttribute('role', role);
      frame.setAttribute('aria-describedby', errId);
      if (role === 'radiogroup') {
        frame.setAttribute('data-hc-describe', '');
        if (required) frame.setAttribute('aria-required', 'true');
      }
    }
    this._err = h('p', 'hc-field-error', null, { id: errId, hidden: '' });
    this._group = frame;
    this.prepend(frame);
    return frame;
  }

  /** Append the error line (last, after the controls). */
  protected _done(frame: HTMLElement): void {
    if (this._err) frame.append(this._err);
  }

  /** Mark a control as one `_focus` reaches; with `describe`, the error
   *  also describes it and marks it invalid (inputs whose role allows it). */
  protected _control<T extends HTMLElement>(el: T, describe = false): T {
    el.setAttribute('data-hc-control', '');
    if (describe && this._err) {
      el.setAttribute('data-hc-describe', '');
      el.setAttribute('aria-describedby', [el.getAttribute('aria-describedby'), this._err.id].filter(Boolean).join(' '));
    }
    return el;
  }
}
