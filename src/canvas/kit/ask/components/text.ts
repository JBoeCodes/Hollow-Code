/** `<hc-text name="why" label="Why?" long maxlength="500" placeholder="…">`:
 *  one line, or a text area with `long`. `maxlength` defaults to 200 (2,000
 *  with `long`) and is capped at 8,000; a counter shows what is left.
 *  Answer: the trimmed text, or null when empty. */
import { attr, h, uid } from '../../registry';
import { AskField } from '../field';
import { intAttr, TEXT_MAX, type FieldRule } from '../model';

export class HcText extends AskField {
  static get observedAttributes(): string[] { return ['label', 'required', 'long', 'maxlength', 'placeholder']; }
  readonly _kind = 'text';
  _v = '';

  _max(): number {
    const long = this.hasAttribute('long');
    return Math.max(1, Math.min(intAttr(this.getAttribute('maxlength')) ?? (long ? TEXT_MAX.long : TEXT_MAX.short), TEXT_MAX.cap));
  }

  protected render(): void {
    const long = this.hasAttribute('long');
    const max = this._max();
    const id = uid('hcq-text');
    const countId = uid('hcq-count');
    const frame = this._frame(null, id);
    const input = h(long ? 'textarea' : 'input', 'hc-text-input', null, { id, maxlength: String(max), 'aria-describedby': countId }) as HTMLInputElement;
    if (!long) input.type = 'text';
    else input.setAttribute('rows', '3');
    const placeholder = attr(this, 'placeholder');
    if (placeholder) input.placeholder = placeholder;
    if (this.hasAttribute('required')) input.setAttribute('aria-required', 'true');
    this._control(input, true);
    input.value = this._v;
    const count = h('span', 'hc-text-count', null, { id: countId });
    const update = () => { count.textContent = `${input.value.length} of ${max} characters`; };
    input.addEventListener('input', () => { this._v = input.value; update(); this._changed(); });
    update();
    frame.append(input, count);
    this._done(frame);
  }

  _value(): string | null { return this._v.trim() || null; }
  _rule(): FieldRule { return { kind: 'text', required: this.hasAttribute('required'), maxLength: this._max() }; }
}
