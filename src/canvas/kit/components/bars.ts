/** `<hc-bars label="Hours by lane" unit="h" max="40">` with one row per
 *  line (`Lane A: 12`, or a `<table>`): labelled horizontal bars for one
 *  series. Every bar shows its value as text; the bar itself is decoration
 *  (`aria-hidden`), so the list reads the same to VoiceOver. One series wears
 *  one colour (slot 1); bars grow from zero, and a negative value draws no
 *  bar but keeps its number. */
import { parseNum, toChartData, withUnit } from '../data';
import { attr, h, part, source, sourceRows } from '../registry';
import { TextureElement } from '../textureElement';

export class HcBars extends TextureElement {
  static get observedAttributes(): string[] { return ['label', 'unit', 'max']; }

  protected render(): void {
    this.clearParts();
    const label = attr(this, 'label');
    const unit = attr(this, 'unit');
    const { rows, header } = sourceRows(source(this));
    const data = toChartData(rows, [], label, header);
    const first = data.series[0] || { cells: [], values: [] };
    const values = first.values.map(v => v ?? 0);
    const max = Math.max(parseNum(this.getAttribute('max')) ?? 0, ...values, 0) || 1;
    const parts: HTMLElement[] = [];
    if (label) parts.push(part('div', 'hc-bars-label', label));
    const list = part('div', 'hc-bars-list', null, { role: 'list' });
    if (label) list.setAttribute('aria-label', label);
    data.categories.forEach((name, i) => {
      const row = h('div', 'hc-bar', null, { role: 'listitem' });
      const track = h('span', 'hc-bar-track', null, { 'aria-hidden': 'true' });
      const fill = h('span', 'hc-bar-fill');
      fill.style.setProperty('--hc-pct', String(Math.max(0, Math.min(1, values[i] / max))));
      track.append(fill);
      row.append(h('span', 'hc-bar-name', name), track, h('span', 'hc-bar-value', withUnit(first.cells[i] || '', unit)));
      list.append(row);
    });
    parts.push(list);
    this.prepend(...parts);
  }
}
