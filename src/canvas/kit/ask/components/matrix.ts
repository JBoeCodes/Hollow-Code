/** `<hc-matrix name="fit" label="Rate each option" options="Low | Medium |
 *  High">` with one row per line: a table with one radio group per row
 *  (`options` are the columns; `Low | Medium | High` by default). Each radio
 *  is named "row: option". Answer: `{row: option}` for the rows answered, or
 *  null when none; `required` means every row. */
import { attr, h, source, uid } from '../../registry';
import { AskField, rowsOf } from '../field';
import { parseOptions, splitList, type FieldRule } from '../model';

export class HcMatrix extends AskField {
  static get observedAttributes(): string[] { return ['label', 'required', 'options']; }
  readonly _kind = 'matrix';
  _sel: { [row: string]: string } = Object.create(null);
  _rows: string[] = [];

  protected render(): void {
    const focused = this.contains(document.activeElement) ? document.activeElement?.getAttribute('aria-label') : null;
    const hadError = !!this._err && !this._err.hidden;
    const lines = rowsOf(source(this));
    // Checklist labels are literal; leading '* ' is not a recommendation.
    this._rows = this.hasAttribute('data-hc-checklist')
      ? [...new Set(lines.map(t => t.trim()).filter(Boolean))] : parseOptions(lines).map(o => o.label);
    const cols = splitList(attr(this, 'options'));
    if (!cols.length) cols.push('Low', 'Medium', 'High');
    // A kept answer (P1-33) or changed options: only a current column counts.
    for (const r in this._sel) if (!cols.includes(this._sel[r])) delete this._sel[r];
    const frame = this._frame('group');
    const scroll = h('div', 'hc-matrix-scroll', null, { role: 'region', tabindex: '0', 'aria-label': this._label() || 'Grid' });
    const table = h('table', 'hc-matrix');
    const head = h('tr', '');
    head.append(h('td', ''), ...cols.map(c => h('th', '', c, { scope: 'col' })));
    const thead = h('thead', '');
    const tbody = h('tbody', '');
    thead.append(head);
    table.append(thead, tbody);
    for (const row of this._rows) {
      const tr = h('tr', '');
      tr.append(h('th', '', row, { scope: 'row' }));
      const group = uid('hcq-m');
      for (const c of cols) {
        const input = this._control(h('input', '', null, { type: 'radio', name: group, 'aria-label': `${row}: ${c}` }) as HTMLInputElement);
        input.checked = this._sel[row] === c;
        input.addEventListener('change', () => { this._sel[row] = c; this._changed(); });
        const td = h('td', '');
        td.append(input);
        tr.append(td);
      }
      tbody.append(tr);
    }
    scroll.append(table);
    frame.append(scroll);
    this._done(frame);
    if (hadError) this._show(this._check());
    if (focused) (Array.from(this.querySelectorAll<HTMLInputElement>('input')).find(input => input.getAttribute('aria-label') === focused) ?? scroll).focus({ preventScroll: true });
  }

  _value(): { [row: string]: string } | null {
    const out: { [row: string]: string } = {};
    for (const r of this._rows) {
      if (Object.prototype.hasOwnProperty.call(this._sel, r)) Object.defineProperty(out, r, { value: this._sel[r], enumerable: true, writable: true, configurable: true });
    }
    return Object.keys(out).length ? out : null;
  }
  _rule(): FieldRule { return { kind: 'matrix', required: this.hasAttribute('required'), rows: this._rows }; }
}
