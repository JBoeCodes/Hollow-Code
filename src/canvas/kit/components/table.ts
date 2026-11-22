/** `<hc-table label="Lanes">` holding a `<table>`, or rows as text (first
 *  row is the header: `Lane | Hours | Status`). Columns sort by click or by
 *  keyboard: each header holds a real button (Enter/Space), and the sorted
 *  column carries `aria-sort`. Numeric columns sort by value, others in
 *  natural order; empty cells stay last. An author table keeps its own cell
 *  content (links, badges); text rows become a generated table. The sort
 *  (column header and direction) persists in the canvas state. */
import { parseRows, sortOrder } from '../data';
import { attr, h, keepView, KitElement, part, savedView, source } from '../registry';

export class HcTable extends KitElement {
  static get observedAttributes(): string[] { return ['label']; }
  private _col = -1;
  private _dir: 'ascending' | 'descending' = 'ascending';

  protected render(): void {
    this.clearParts();
    const label = attr(this, 'label');
    let table = this.querySelector<HTMLTableElement>(':scope > table');
    if (!table) {
      const rows = parseRows(source(this).textContent || '');
      table = part('table', 'hc-table-t') as HTMLTableElement;
      this._col = -1;
      if (rows.length) {
        const head = table.createTHead().insertRow();
        for (const c of rows[0]) head.append(h('th', '', c, { scope: 'col' }));
        const body = table.createTBody();
        for (const r of rows.slice(1)) {
          const tr = body.insertRow();
          for (let i = 0; i < rows[0].length; i++) tr.insertCell().textContent = r[i] ?? '';
        }
      }
      this.append(table);
    }
    if (label && !table.caption) table.createCaption().textContent = label;
    const header = this._header(table);
    if (!header) return;
    Array.from(header.cells).forEach((th, i) => {
      if (!th.querySelector(':scope > .hc-sort')) {
        th.dataset.hcCol = (th.textContent || '').trim();
        const button = h('button', 'hc-sort', null, { type: 'button' });
        button.append(...Array.from(th.childNodes), h('span', 'hc-sort-icon', '', { 'aria-hidden': 'true' }));
        button.addEventListener('click', () => {
          this._sort(i);
          keepView(this, [th.dataset.hcCol, this._dir]);
        });
        th.append(button);
      }
    });
    this._mark(header);
  }

  private _header(table: HTMLTableElement): HTMLTableRowElement | null {
    const first = table.tHead?.rows[0] || table.rows[0];
    return first && Array.from(first.cells).every(c => c.tagName === 'TH') ? first : null;
  }

  private _mark(header: HTMLTableRowElement): void {
    Array.from(header.cells).forEach((th, i) => {
      const sorted = i === this._col;
      th.setAttribute('aria-sort', sorted ? this._dir : 'none');
      const icon = th.querySelector('.hc-sort-icon');
      if (icon) icon.textContent = sorted ? (this._dir === 'ascending' ? '▲' : '▼') : '↕';
    });
  }

  _restore(): void {
    const v = savedView(this);
    const table = this.querySelector<HTMLTableElement>(':scope > table');
    const header = table && this._header(table);
    if (!Array.isArray(v) || !header) return;
    const col = Array.from(header.cells).findIndex(th => th.dataset.hcCol === v[0]);
    if (col >= 0 && (v[1] === 'ascending' || v[1] === 'descending')) this._sort(col, v[1]);
  }

  private _sort(col: number, dir?: 'ascending' | 'descending'): void {
    const table = this.querySelector<HTMLTableElement>(':scope > table');
    const header = table && this._header(table);
    if (!table || !header) return;
    this._dir = dir || (col === this._col && this._dir === 'ascending' ? 'descending' : 'ascending');
    this._col = col;
    const rows = Array.from(table.tBodies).flatMap(b => Array.from(b.rows)).filter(r => r !== header);
    for (const row of rows) Array.from(row.cells).forEach((cell, i) => cell.toggleAttribute('data-hc-sorted', i === col));
    const order = sortOrder(rows.map(r => (r.cells[col]?.textContent || '').trim()), this._dir);
    const body = rows[0]?.parentElement;
    if (body) for (const i of order) body.append(rows[i]);
    this._mark(header);
  }
}
