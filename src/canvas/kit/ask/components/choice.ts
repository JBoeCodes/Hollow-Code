/** `<hc-choice name="plan" label="Which plan?" required>` with one option
 *  per line; `* ` marks the recommended one. Native radio buttons in a
 *  labelled radiogroup (arrow keys move and select). Answer: the chosen
 *  option's text, or null. */
import { h, source, uid } from '../../registry';
import { AskField, recommended, rowsOf } from '../field';
import { parseOptions, type FieldRule } from '../model';

export class HcChoice extends AskField {
  static get observedAttributes(): string[] { return ['label', 'required']; }
  readonly _kind = 'choice';
  _sel: string | null = null;

  protected render(): void {
    const options = parseOptions(rowsOf(source(this)));
    if (this._sel !== null && !options.some(o => o.label === this._sel)) this._sel = null;
    const frame = this._frame('radiogroup');
    const group = uid('hcq-r');
    for (const o of options) {
      const input = this._control(h('input', '', null, { type: 'radio', name: group }) as HTMLInputElement);
      input.checked = o.label === this._sel;
      input.addEventListener('change', () => { this._sel = o.label; this._changed(); });
      const row = h('label', 'hc-opt');
      row.append(input, h('span', 'hc-opt-text', o.label));
      if (o.recommended) row.append(recommended());
      frame.append(row);
    }
    this._done(frame);
  }

  _value(): string | null { return this._sel; }
  _rule(): FieldRule { return { kind: 'choice', required: this.hasAttribute('required') }; }
}
