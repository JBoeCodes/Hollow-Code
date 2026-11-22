/** `<hc-range name="weeks" label="How many weeks?" min="1" max="12" step="1"
 *  value="4" unit="weeks">`: a slider and a number box for the same value.
 *  Without `value` it starts unset ("Not set") until the person moves it.
 *  Answer: the number, or null. `min` 0, `max` 100 and `step` 1 by default. */
import { attr, h, uid } from '../../registry';
import { AskField } from '../field';
import { numAttr, rangeSpec, type FieldRule } from '../model';

export class HcRange extends AskField {
  static get observedAttributes(): string[] { return ['label', 'required', 'min', 'max', 'step', 'unit']; }
  readonly _kind = 'range';
  _v: number | null | undefined;

  protected render(): void {
    const spec = rangeSpec(this.getAttribute('min'), this.getAttribute('max'), this.getAttribute('step'));
    if (this._v === undefined) this._v = numAttr(this.getAttribute('value'));
    const unit = this._unit();
    const id = uid('hcq-range');
    const frame = this._frame(null, id);
    const bounds = { min: String(spec.min), max: String(spec.max), step: String(spec.step) };
    const slider = this._control(h('input', 'hc-range-slider', null, { type: 'range', id, ...bounds }) as HTMLInputElement);
    const box = this._control(h('input', 'hc-range-number', null, {
      type: 'number', inputmode: 'decimal', placeholder: 'Not set', 'aria-label': `${this._label() || 'Value'}${unit ? ` (${unit})` : ''}`, ...bounds,
    }) as HTMLInputElement, true);
    const sync = () => {
      const v = this._v;
      slider.value = String(v ?? spec.min);
      slider.setAttribute('aria-valuetext', v == null ? 'Not set' : `${v}${unit ? ` ${unit}` : ''}`);
      this.toggleAttribute('data-hc-unset', v == null);
    };
    slider.addEventListener('input', () => { this._v = Number(slider.value); box.value = slider.value; sync(); this._changed(); });
    box.addEventListener('input', () => {
      const t = box.value.trim();
      this._v = t === '' ? null : Number(t);
      if (this._v !== null && !Number.isFinite(this._v)) this._v = NaN;
      sync();
      this._changed();
    });
    if (this._v != null) box.value = String(this._v);
    sync();
    const row = h('div', 'hc-range-row');
    row.append(slider, box);
    if (unit) row.append(h('span', 'hc-range-unit', unit));
    frame.append(row);
    this._done(frame);
  }

  _unit(): string { return attr(this, 'unit'); }
  _value(): number | null { return this._v ?? null; }
  _rule(): FieldRule {
    return { kind: 'range', required: this.hasAttribute('required'), range: rangeSpec(this.getAttribute('min'), this.getAttribute('max'), this.getAttribute('step')) };
  }
}
