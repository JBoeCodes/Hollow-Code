/** `<hc-multi name="surfaces" label="Keep which?" min="1" max="3">` with one
 *  option per line (`* ` = recommended). Native checkboxes in a labelled
 *  group. Answer: the checked options in authored order, or null when none
 *  (as every field left empty; Canvas C6 P1, it was `[]`).
 *  `required` means at least one; `min` and `max` bound the count. */
import { h, source } from '../../registry';
import { AskField, recommended, rowsOf } from '../field';
import { intAttr, parseOptions, type FieldRule } from '../model';

export class HcMulti extends AskField {
  static get observedAttributes(): string[] { return ['label', 'required', 'min', 'max']; }
  readonly _kind = 'multi';
  _sel = new Set<string>();
  _all: string[] = [];

  protected render(): void {
    const options = parseOptions(rowsOf(source(this)));
    this._all = options.map(o => o.label);
    const frame = this._frame('group');
    for (const o of options) {
      const input = this._control(h('input', '', null, { type: 'checkbox' }) as HTMLInputElement, true);
      input.checked = this._sel.has(o.label);
      input.addEventListener('change', () => {
        if (input.checked) this._sel.add(o.label);
        else this._sel.delete(o.label);
        this._changed();
      });
      const row = h('label', 'hc-opt');
      row.append(input, h('span', 'hc-opt-text', o.label));
      if (o.recommended) row.append(recommended());
      frame.append(row);
    }
    this._done(frame);
  }

  _value(): string[] | null {
    const v = this._all.filter(l => this._sel.has(l));
    return v.length ? v : null;
  }
  _rule(): FieldRule {
    return { kind: 'multi', required: this.hasAttribute('required'), minCount: intAttr(this.getAttribute('min')), maxCount: intAttr(this.getAttribute('max')) };
  }
}
