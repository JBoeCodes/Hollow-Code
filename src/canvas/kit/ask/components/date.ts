/** `<hc-date name="start" label="Start date" min="2026-10-06"
 *  max="2026-12-31" value="2026-10-12">`: the platform date input. Answer:
 *  the date as `YYYY-MM-DD`, or null. */
import { attr, h, uid } from '../../registry';
import { AskField } from '../field';
import { isIsoDate, type FieldRule } from '../model';

const iso = (v: string) => (isIsoDate(v) ? v : '');

export class HcDate extends AskField {
  static get observedAttributes(): string[] { return ['label', 'required', 'min', 'max']; }
  readonly _kind = 'date';
  _v: string | undefined;

  protected render(): void {
    if (this._v === undefined) this._v = iso(attr(this, 'value'));
    const id = uid('hcq-date');
    const frame = this._frame(null, id);
    const input = this._control(h('input', 'hc-date-input', null, { type: 'date', id }) as HTMLInputElement, true);
    const min = iso(attr(this, 'min'));
    const max = iso(attr(this, 'max'));
    if (min) input.min = min;
    if (max) input.max = max;
    if (this.hasAttribute('required')) input.setAttribute('aria-required', 'true');
    input.value = this._v;
    input.addEventListener('input', () => { this._v = input.value; this._changed(); });
    input.addEventListener('change', () => { this._v = input.value; this._changed(); });
    frame.append(input);
    this._done(frame);
  }

  _value(): string | null { return this._v || null; }
  _rule(): FieldRule {
    return { kind: 'date', required: this.hasAttribute('required'), minDate: iso(attr(this, 'min')), maxDate: iso(attr(this, 'max')) };
  }
}
