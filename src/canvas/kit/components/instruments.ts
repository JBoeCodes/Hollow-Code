/** SVG instruments: text-first, theme-live, bounded and entirely local. */
import { withUnit, toChartData, parseNum } from '../data';
import { gaugeValue, sparkPaths, timelineRows, heatmapDays, weekday } from '../instruments';
import { attr, h, part, source, sourceRows, tone, toneMark, uid } from '../registry';
import { TextureElement } from '../textureElement';
import { stipple } from '../material';

function s(tag: string, attrs: Record<string, string | number>, text?: string): SVGElement {
  const el = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, String(v));
  if (text !== undefined) el.textContent = text;
  return el;
}
function svg(w: number, height: number): SVGElement {
  return s('svg', { viewBox: `0 0 ${w} ${height}`, width: w, height, 'aria-hidden': 'true', focusable: 'false', class: 'hc-instrument-svg' });
}
function heading(el: HTMLElement, fallback: string): HTMLElement {
  return part('strong', 'hc-instrument-label', attr(el, 'label') || fallback);
}
function table(el: HTMLElement, headers: string[], rows: string[][]): HTMLElement {
  const box = part('details', 'hc-instrument-data');
  box.append(h('summary', '', 'Data table'));
  const t = h('table', '') as HTMLTableElement;
  t.createCaption().textContent = attr(el, 'label') || 'Values';
  const tr = t.createTHead().insertRow();
  for (const name of headers) tr.append(h('th', '', name, { scope: 'col' }));
  const body = t.createTBody();
  for (const row of rows) { const r = body.insertRow(); row.forEach((c, i) => r.append(h(i ? 'td' : 'th', '', c, i ? {} : { scope: 'row' }))); }
  box.append(t);
  return box;
}
function notice(box: HTMLElement, text: string): void { box.append(h('p', 'hc-instrument-note', text)); }

/** A single tab stop per drawing; arrows expose every value, pointer and touch too. */
function inspect(box: HTMLElement, targets: SVGElement[], descriptions: string[]): void {
  if (!targets.length) return;
  const tip = h('div', 'hc-instrument-tip', descriptions[0], { 'aria-live': 'polite', id: uid('hc-value') });
  const control = box.querySelector('svg')!;
  control.removeAttribute('aria-hidden');
  control.setAttribute('role', 'img');
  control.setAttribute('aria-label', 'Values. Use arrow keys to inspect; Data table lists all values.');
  control.setAttribute('aria-describedby', tip.id);
  control.setAttribute('tabindex', '0');
  let at = 0;
  const activate = (i: number) => {
    targets[at].removeAttribute('data-active');
    at = i;
    targets[at].setAttribute('data-active', '');
    tip.textContent = descriptions[at];
    const cell = (targets[at].querySelector('.hc-timeline-point') || targets[at]).getBoundingClientRect(), view = box.getBoundingClientRect();
    if (cell.left < view.left) box.scrollLeft -= view.left - cell.left + 4;
    else if (cell.right > view.right) box.scrollLeft += cell.right - view.right + 4;
  };
  targets.forEach((t, i) => { t.addEventListener('pointerenter', () => activate(i)); t.addEventListener('click', () => activate(i)); });
  control.addEventListener('keydown', e => {
    const key = (e as KeyboardEvent).key;
    const next = key === 'ArrowRight' || key === 'ArrowDown' ? (at + 1) % targets.length
      : key === 'ArrowLeft' || key === 'ArrowUp' ? (at + targets.length - 1) % targets.length
        : key === 'Home' ? 0 : key === 'End' ? targets.length - 1 : -1;
    if (next < 0) return;
    e.preventDefault(); activate(next);
  });
  control.addEventListener('focus', () => activate(at));
  box.append(tip);
}

export class HcGauge extends TextureElement {
  static get observedAttributes(): string[] { return ['label', 'value', 'min', 'max', 'unit', 'tone']; }
  protected render(): void {
    this.clearParts();
    const label = attr(this, 'label') || 'Gauge', unit = attr(this, 'unit');
    const g = gaugeValue(attr(this, 'value'), attr(this, 'min'), attr(this, 'max'));
    const value = g.value === null ? 'No data' : withUnit(attr(this, 'value'), unit);
    const box = part('div', 'hc-gauge-body');
    const graphic = svg(240, 152);
    graphic.append(s('path', { d: 'M24,112 A96,96 0 0 1 216,112', class: 'hc-gauge-track' }));
    if (g.fraction !== null && g.fraction > 0) {
      const a = Math.PI * (1 - g.fraction), x = 120 + 96 * Math.cos(a), y = 112 - 96 * Math.sin(a);
      const d = `M24,112 A96,96 0 0 1 ${x},${y}`;
      const arc = s('path', { d, class: 'hc-gauge-value' });
      graphic.append(arc); stipple(graphic, arc, 'hc-gauge-dots');
    }
    graphic.append(s('text', { x: 120, y: 101, 'text-anchor': 'middle', class: 'hc-gauge-number', ...(value.length > 12 ? { textLength: 160, lengthAdjust: 'spacingAndGlyphs' } : {}) }, value));
    const range = g.min === null || g.max === null || g.max <= g.min ? 'Invalid range' : `${withUnit(String(g.min), unit)} – ${withUnit(String(g.max), unit)}`;
    graphic.append(s('text', { x: 120, y: 136, 'text-anchor': 'middle', class: 'hc-gauge-range' }, range));
    box.append(graphic);
    graphic.removeAttribute('aria-hidden'); graphic.setAttribute('role', 'img'); graphic.setAttribute('aria-label', `${label}: ${value}. ${range}`);
    const head = heading(this, 'Gauge');
    if (tone(this) !== 'neutral') head.append(toneMark(tone(this), true));
    if (g.value !== null && g.min !== null && g.max !== null && (g.value < g.min || g.value > g.max)) notice(box, 'Value is outside the shown range.');
    this.prepend(head, box);
  }
}

export class HcSparkline extends TextureElement {
  static get observedAttributes(): string[] { return ['label', 'unit']; }
  protected render(): void {
    this.clearParts();
    const { rows, header } = sourceRows(source(this));
    const hasHeader = header ?? !!rows[0]?.slice(1).some(c => parseNum(c) === null && !/^(—|-|n\/a|null|)$/i.test(c));
    const data = toChartData(rows, [], 'Value', hasHeader);
    const series = data.series[0];
    const vals = series.values.slice(0, 366), unit = attr(this, 'unit');
    const paths = sparkPaths(vals, 280, 64);
    const box = part('div', 'hc-sparkline-body');
    const graphic = svg(280, 64); graphic.setAttribute('preserveAspectRatio', 'none');
    for (const d of paths) {
      const start = d.slice(1).split(',')[0], end = d.slice(d.lastIndexOf('L') + 1).split(',')[0];
      if (d.includes('L')) {
        const closed = `${d} L${end},64 L${start},64 Z`;
        // Opacity-only fade, dense at the floor, under the stipple.
        const fade = s('linearGradient', { id: uid('hc-fade'), x1: 0, y1: 1, x2: 0, y2: 0 });
        fade.append(s('stop', { offset: 0, 'stop-opacity': 0.5, class: 'hc-fade' }), s('stop', { offset: 1, 'stop-opacity': 0.08, class: 'hc-fade' }));
        graphic.append(fade, s('path', { d: closed, fill: `url(#${fade.id})`, class: 'hc-fade-area' }));
        stipple(graphic, s('path', { d: closed }), 'hc-sparkline-area');
      }
      graphic.append(s('path', { d, class: 'hc-sparkline-path' }));
      if (!d.includes('L')) { const [cx, cy] = d.slice(1).split(','); graphic.append(s('circle', { cx, cy, r: 3, class: 'hc-sparkline-dot' })); }
    }
    box.append(graphic);
    const known = vals.filter(v => v !== null);
    const summary = known.length ? `${withUnit(String(known[0]), unit)} → ${withUnit(String(known[known.length - 1]), unit)} · ${known.length} known of ${vals.length}` : 'No data';
    graphic.removeAttribute('aria-hidden'); graphic.setAttribute('role', 'img'); graphic.setAttribute('aria-label', `${attr(this, 'label') || 'Trend'}: ${summary}`);
    box.append(h('div', 'hc-instrument-note', summary));
    if (series.values.length > 366) notice(box, `${series.values.length - 366} more samples in the table.`);
    if (data.series.length > 1) notice(box, 'First series shown; all series in the table.');
    this.prepend(heading(this, 'Trend'), box, table(this, [data.head || 'Sample', ...data.series.map(s => s.name)], data.categories.map((c, i) => [c, ...data.series.map(s => withUnit(s.cells[i], unit))])));
  }
}

export class HcTimeline extends TextureElement {
  static get observedAttributes(): string[] { return ['label']; }
  protected render(): void {
    this.clearParts();
    const { rows, header } = sourceRows(source(this));
    const data = timelineRows(rows, header);
    const box = part('div', 'hc-timeline-body');
    const n = data.items.length;
    if (n) {
      const lo = Math.min(...data.items.map(x => x.start)), hi = Math.max(...data.items.map(x => x.end));
      const x = (day: number) => hi === lo ? 412 : 160 + (day - lo) * 504 / (hi - lo);
      const graphic = svg(700, 52 + n * 34), targets: SVGElement[] = [], descriptions: string[] = [];
      graphic.append(s('line', { x1: 160, x2: 664, y1: 25, y2: 25, class: 'hc-timeline-axis' }));
      graphic.append(s('text', { x: 160, y: 14, class: 'hc-timeline-date' }, data.items[0].date));
      graphic.append(s('text', { x: 664, y: 14, 'text-anchor': 'end', class: 'hc-timeline-date' }, new Date(hi * 86400000).toISOString().slice(0, 10)));
      data.items.forEach((item, i) => {
        const y = 48 + i * 34, g = s('g', { class: 'hc-timeline-event' });
        graphic.append(g);
        const description = `${item.label}: ${item.date}${item.end !== item.start ? ` to ${item.until}` : ''}`;
        g.append(s('title', {}, description));
        g.append(s('rect', { x: 0, y: y - 14, width: 700, height: 28, class: 'hc-instrument-hit' }));
        g.append(s('text', { x: 0, y: y + 4, class: 'hc-timeline-name' }, [...item.label].length > 20 ? `${[...item.label].slice(0, 19).join('')}…` : item.label));
        const span = s('line', { x1: x(item.start), x2: x(item.end), y1: y, y2: y, class: 'hc-timeline-span' });
        g.append(span); stipple(g, span, 'hc-timeline-dots');
        g.append(s('circle', { cx: x(item.start), cy: y, r: 4, class: 'hc-timeline-point' }));
        if (item.end !== item.start) g.append(s('circle', { cx: x(item.end), cy: y, r: 4, class: 'hc-timeline-point' }));
        graphic.append(g); targets.push(g); descriptions.push(description);
      });
      box.append(graphic); inspect(box, targets, descriptions);
    } else notice(box, 'No dated events');
    if (data.invalid) notice(box, `${data.invalid} invalid date or span rows not plotted.`);
    if (data.omitted) notice(box, `${data.omitted} more events in the source table.`);
    this.prepend(heading(this, 'Timeline'), box, table(this, ['Date', 'Event', 'End'], data.rows));
  }
}

export class HcHeatmap extends TextureElement {
  static get observedAttributes(): string[] { return ['label', 'unit', 'start', 'end']; }
  protected render(): void {
    this.clearParts();
    const { rows, header } = sourceRows(source(this));
    const data = heatmapDays(rows, attr(this, 'start'), attr(this, 'end'), header), unit = attr(this, 'unit');
    const box = part('div', 'hc-heatmap-body');
    if (data.days.length) {
      const offset = weekday(data.days[0].day), weeks = Math.ceil((offset + data.days.length) / 7);
      const graphic = svg(40 + weeks * 16, 152), targets: SVGElement[] = [], descriptions: string[] = [];
      const nums = data.days.map(x => x.value).filter((v): v is number => v !== null), low = Math.min(...nums), high = Math.max(...nums);
      ['Mon', 'Wed', 'Fri', 'Sun'].forEach((name, i) => graphic.append(s('text', { x: 0, y: 36 + i * 32, class: 'hc-heatmap-day' }, name)));
      let month = '';
      data.days.forEach((d, i) => {
        const col = Math.floor((i + offset) / 7), row = (i + offset) % 7;
        // Month names get room even when a range begins at month end.
        if (d.date.slice(0, 7) !== month && (i === 0 || row === 0)) {
          month = d.date.slice(0, 7);
          graphic.append(s('text', { x: 38 + col * 16, y: 14, class: 'hc-heatmap-month' }, d.date.slice(5, 7)));
        }
        const level = d.value === null ? 'missing' : d.value === 0 ? 'zero' : String(high === low ? 3 : 1 + Math.round((d.value - low) * 3 / (high - low)));
        const description = `${d.date}: ${d.value === null ? 'No data' : withUnit(String(d.value), unit)}`;
        const cell = s('rect', { x: 38 + col * 16, y: 24 + row * 16, width: 12, height: 12, rx: 3, class: 'hc-heatmap-cell', 'data-level': level });
        cell.append(s('title', {}, description)); graphic.append(cell); targets.push(cell); descriptions.push(description);
        if (d.value !== null && d.value !== 0) stipple(graphic, cell, 'hc-heatmap-dots');
        if (d.value !== null && d.value !== 0) graphic.append(s('text', {
          x: 44 + col * 16, y: 34 + row * 16, class: 'hc-heatmap-glyph', 'text-anchor': 'middle',
        }, '.:+#'[Number(level) - 1]));
      });
      box.append(graphic); inspect(box, targets, descriptions);
      notice(box, 'Outlined: no data · Empty: zero · Filled: lower to higher values');
      box.append(h('p', 'hc-heatmap-glyph-key', '. : + # = lower to higher values'));
    } else notice(box, data.invalidRange ? 'Invalid date range' : 'No dated data');
    if (data.invalid) notice(box, `${data.invalid} invalid date rows not plotted.`);
    if (data.duplicates) notice(box, `${data.duplicates} repeated dates; last value used.`);
    if (data.truncated) notice(box, 'Calendar limited to the first 366 days.');
    if (data.outside) notice(box, `${data.outside} source dates outside the shown range.`);
    this.prepend(heading(this, 'Calendar'), box, table(this, ['Date', 'Value'], data.rows));
  }
}
