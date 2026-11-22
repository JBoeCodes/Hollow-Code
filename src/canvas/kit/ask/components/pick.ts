/** `<hc-pick name="week" label="Which week?" unit="tasks">` with one data
 *  row per line (`Week 1: 12` or `Week 1 | 12`, like `hc-bars`): each row is
 *  a radio option drawn as a labelled bar, so the person picks an item off
 *  the chart. Rows without a number are plain options. Answer: the picked
 *  row's label, or null. */
import { attr, h, source, uid } from '../../registry';
import { AskField, rowsOf } from '../field';
import { pickRows, type FieldRule } from '../model';

export class HcPick extends AskField {
  static get observedAttributes(): string[] { return ['label', 'required', 'unit']; }
  readonly _kind = 'pick';
  _sel: string | null = null;

  protected render(): void {
    const items = pickRows(rowsOf(source(this)));
    if (this._sel !== null && !items.some(i => i.label === this._sel)) this._sel = null;
    const max = Math.max(0, ...items.map(i => i.n ?? 0)) || 1;
    const unit = attr(this, 'unit');
    const frame = this._frame('radiogroup');
    const group = uid('hcq-p');
    for (const item of items) {
      const input = this._control(h('input', '', null, { type: 'radio', name: group }) as HTMLInputElement);
      input.checked = item.label === this._sel;
      input.addEventListener('change', () => { this._sel = item.label; this._changed(); });
      const row = h('label', 'hc-opt hc-pick-row');
      row.append(input, h('span', 'hc-opt-text', item.label));
      if (item.n !== null) {
        const track = h('span', 'hc-bar-track', null, { 'aria-hidden': 'true' });
        const fill = h('span', 'hc-bar-fill');
        fill.style.setProperty('--hc-pct', String(Math.max(0, Math.min(1, item.n / max))));
        track.append(fill);
        row.append(track, h('span', 'hc-bar-value', unit && /\d$/.test(item.cell) ? `${item.cell} ${unit}` : item.cell));
      }
      frame.append(row);
    }
    this._done(frame);
  }

  _value(): string | null { return this._sel; }
  _rule(): FieldRule { return { kind: 'pick', required: this.hasAttribute('required') }; }
}
