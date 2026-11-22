/** `<hc-chart type="line" label="Sessions per day" series="Claude, Codex">`
 *  with one row per line (`Mon: 12, 8`, `Mon | 12 | 8`, or a `<table>`).
 *  Types: `line` (default), `bar` (grouped columns), `area`, `donut` (the
 *  first value column, at most six parts; the tail folds into "Other").
 *
 *  Drawn on a `<canvas>` sized by devicePixelRatio, in the `dataviz` skill's
 *  style: 2px lines, ≥8px markers with a 2px surface ring, columns ≤24px with
 *  a 4px rounded data end, a 2px surface gap between fills, hairline grid,
 *  text in text tokens. Identity is never colour alone: each series has its
 *  own marker shape (shown in the legend), two to four lines get direct end
 *  labels, donut parts list their values, and under `prefers-contrast: more`
 *  or forced colours fills get a 45°/135° texture. Hover or arrow keys show
 *  the values; a data table (visually hidden, with a Table toggle that shows
 *  it) is what VoiceOver reads, and the source rows are the no-script
 *  fallback. One transform-only entrance reveals cached marks; axes stay still.
 *
 *  Redraws on the runtime's `hollow-theme` event and on size changes, which
 *  it detects from the window `resize` event and the kit's `hc-reflow` (never
 *  ResizeObserver, which dies in sandboxed WKWebView frames), plus a DPR
 *  change. A chart in a hidden panel draws when the panel is shown. */
import {
  donutSlices, formatTick, niceTicks, scale, sharedPrefix, splitNames, toChartData, withUnit, type ChartData,
} from '../data';
import { chartEntrance, motionAllowed } from '../chartEntrance';
import { MARKER_GLYPHS, MAX_SERIES } from '../palette';
import { attr, h, KitElement, part, source, sourceRows, uid } from '../registry';
import { textureOf, watchTextures } from '../texture';
import { ditherFill, ditherRing, lineTop } from '../ditherFill';

const TYPES = ['line', 'bar', 'area', 'donut'] as const;
type ChartType = typeof TYPES[number];
type Ctx = CanvasRenderingContext2D;

const live = new Set<HcChart>();
let stopTexture: (() => void) | undefined;
let queued = false;
let forced = false;

/** Redraw every live chart on the next frame; `force` repaints even when the
 *  size is unchanged (theme, contrast). */
function schedule(force: boolean): void {
  forced = forced || force;
  if (queued) return;
  queued = true;
  requestAnimationFrame(() => {
    queued = false;
    const f = forced;
    forced = false;
    live.forEach(c => c._draw(f));
  });
}

let wired = false;
function wire(): void {
  if (wired) return;
  wired = true;
  let late: ReturnType<typeof setTimeout> | undefined;
  addEventListener('resize', () => {
    schedule(false);
    clearTimeout(late);
    late = setTimeout(() => schedule(false), 150);
  });
  addEventListener('hc-reflow', () => schedule(false));
  // A native <details> opening reveals a chart too (toggle doesn't bubble).
  addEventListener('toggle', () => schedule(false), true);
  addEventListener('load', () => schedule(false));
  addEventListener('hollow-theme', () => schedule(true));
  addEventListener('hollow-init', () => schedule(true));
  const watchDpr = () => {
    try {
      matchMedia(`(resolution: ${devicePixelRatio}dppx)`).addEventListener('change', () => { schedule(true); watchDpr(); }, { once: true });
    } catch { /* no matchMedia */ }
  };
  watchDpr();
  try { matchMedia(TEXTURE).addEventListener('change', () => schedule(true)); } catch { /* no matchMedia */ }
}

const TEXTURE = '(prefers-contrast: more), (forced-colors: active)';

/** [sides, rotation in half-turns, size] per slot, matching MARKER_GLYPHS:
 *  circle, square, triangle, diamond, triangle down, pentagon, hexagon,
 *  triangle left. */
const SHAPES = [[0, 0, 1], [4, 0.25, 1.15], [3, -0.5, 1.3], [4, 0, 1.3], [3, 0.5, 1.3], [5, -0.5, 1.15], [6, 0, 1.1], [3, 1, 1.3]] as const;

function marker(ctx: Ctx, k: number, x: number, y: number, r: number): void {
  ctx.beginPath();
  const [sides, turn, grow] = SHAPES[k % SHAPES.length];
  if (!sides) ctx.arc(x, y, r, 0, Math.PI * 2);
  for (let i = 0; i < sides; i++) {
    const a = Math.PI * (turn + (2 * i) / sides);
    ctx.lineTo(x + Math.cos(a) * r * grow, y + Math.sin(a) * r * grow);
  }
  ctx.closePath();
}

/** A column from `base` to `top` with a rounded data end only. */
function column(ctx: Ctx, x: number, w: number, base: number, top: number): void {
  const dir = top < base ? 1 : -1;
  const r = Math.min(4, w / 2, Math.abs(base - top));
  ctx.beginPath();
  ctx.moveTo(x, base);
  ctx.lineTo(x, top + dir * r);
  ctx.arcTo(x, top, x + r, top, r);
  ctx.lineTo(x + w - r, top);
  ctx.arcTo(x + w, top, x + w, top + dir * r, r);
  ctx.lineTo(x + w, base);
  ctx.closePath();
}

/**
 * Texture for the current path: 45° or 135° lines in the surface colour,
 * clipped to it. save()/restore() keep no path, so this consumes the current
 * path and leaves an empty one: stroke an outline before calling it.
 */
function hatch(ctx: Ctx, k: number, surface: string, w: number, hgt: number): void {
  ctx.save();
  ctx.clip();
  ctx.strokeStyle = surface;
  ctx.globalAlpha = 0.6;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  for (let d = -hgt; d < w + hgt; d += 6) {
    if (k % 2) { ctx.moveTo(d, 0); ctx.lineTo(d + hgt, hgt); } else { ctx.moveTo(d, hgt); ctx.lineTo(d + hgt, 0); }
  }
  ctx.stroke();
  ctx.restore();
  ctx.beginPath();
}

interface Geometry { kind: ChartType; left: number; right: number; n: number; x: (i: number) => number; cx: number; cy: number; r0: number; r1: number; arcs: number[] }

export class HcChart extends KitElement {
  static get observedAttributes(): string[] { return ['type', 'label', 'series', 'unit', 'height']; }
  private _data: ChartData = { head: '', categories: [], series: [] };
  private _kind: ChartType = 'line';
  private _prefix = '';
  private _unit = '';
  private _cv: HTMLCanvasElement | null = null;
  private _plot: HTMLElement | null = null;
  private _tip: HTMLElement | null = null;
  private _active = -1;
  private _drawn = '';
  private _geo: Geometry | null = null;
  private _entrance: (() => void) | null = null;
  private _entered = false;
  private _baseline = 0;
  /** The dithered data layer, redrawn only when its key or the data changes. */
  private _layer: { key: string; data: ChartData; cv: HTMLCanvasElement } | null = null;

  connectedCallback(): void {
    wire();
    live.add(this);
    if (!stopTexture) stopTexture = watchTextures(() => schedule(true));
    super.connectedCallback();
  }

  disconnectedCallback(): void {
    live.delete(this);
    if (!live.size) { stopTexture?.(); stopTexture = undefined; }
    this._entrance?.();
    this._entrance = null;
    this._layer = null;
  }

  private get _shown(): number {
    return Math.min(this._data.series.length, MAX_SERIES);
  }

  private _slices() {
    const s = this._data.series[0];
    return s ? donutSlices(this._data.categories, s.values) : [];
  }

  protected render(): void {
    this._entrance?.();
    this._entrance = null;
    this._entered = false;
    this.clearParts();
    const t = attr(this, 'type').toLowerCase();
    this._kind = (TYPES as readonly string[]).includes(t) ? t as ChartType : 'line';
    const label = attr(this, 'label');
    this._unit = attr(this, 'unit');
    const { rows, header } = sourceRows(source(this));
    const data = toChartData(rows, splitNames(this.getAttribute('series')), label || 'Value', header);
    this._data = data;
    this._prefix = sharedPrefix(data.series.flatMap(s => s.cells));
    this._active = -1;
    const donut = this._kind === 'donut';

    const tableId = uid('hc-chart-table');
    const head = part('div', 'hc-chart-head');
    if (label) head.append(h('strong', 'hc-chart-title', label));
    const toggle = h('button', 'hc-chart-toggle', 'Table', { type: 'button', 'aria-pressed': 'false', 'aria-controls': tableId });
    head.append(toggle);

    const parts: HTMLElement[] = [head];
    if (donut || data.series.length > 1) {
      const legend = part('div', 'hc-chart-legend', null, { role: 'list', 'aria-label': 'Legend' });
      const item = (k: number, glyph: string, text: string, slot = k) => {
        const li = h('span', 'hc-chart-key', null, { role: 'listitem' });
        const sw = h('span', 'hc-sw', glyph, { 'aria-hidden': 'true' });
        this._look(sw, k, slot);
        li.append(sw, h('span', '', text));
        legend.append(li);
      };
      if (donut) {
        const slices = this._slices();
        const total = slices.reduce((a, s) => a + s.value, 0) || 1;
        slices.forEach((s, i) => item(s.other ? -1 : i, '■', `${s.label} ${formatTick(s.value, this._prefix, this._unit)} (${Math.round((s.value / total) * 100)}%)`, i));
        legend.append(h('span', 'hc-chart-more', 'Positive values shown; all values in Table.'));
      } else {
        data.series.slice(0, MAX_SERIES).forEach((s, i) => item(i, this._kind === 'bar' ? '■' : MARKER_GLYPHS[i], s.name));
      }
      const hidden = donut ? data.series.length - 1 : data.series.length - MAX_SERIES;
      if (hidden > 0) legend.append(h('span', 'hc-chart-more', `${hidden} more ${hidden === 1 ? 'column' : 'columns'} in the table`));
      parts.push(legend);
    }

    const n = donut ? this._slices().length : data.categories.length;
    const what = donut ? `donut chart with ${n} parts` : `${this._kind} chart, ${this._shown} series over ${n} categories`;
    const plot = part('div', 'hc-chart-plot');
    const cv = document.createElement('canvas');
    cv.tabIndex = 0;
    cv.setAttribute('role', 'img');
    cv.setAttribute('aria-label', `${label || 'Chart'}: ${what}. Arrow keys read values; the data table lists every value.`);
    const tip = h('div', 'hc-chart-tip', null, { 'aria-live': 'polite' });
    tip.hidden = true;
    plot.append(cv, tip);
    this._cv = cv;
    this._plot = plot;
    this._tip = tip;
    parts.push(plot);

    // The table sits in a visually hidden wrapper (a table ignores the 1px
    // box itself); the Table toggle shows it.
    const wrap = part('div', 'hc-chart-data hc-sr', null, { id: tableId });
    const table = h('table', '') as HTMLTableElement;
    wrap.append(table);
    if (label) table.createCaption().textContent = label;
    const hr = table.createTHead().insertRow();
    hr.append(h('th', '', data.head, { scope: 'col' }));
    for (const s of data.series) hr.append(h('th', '', s.name, { scope: 'col' }));
    const body = table.createTBody();
    data.categories.forEach((c, i) => {
      const tr = body.insertRow();
      tr.append(h('th', '', c, { scope: 'row' }));
      for (const s of data.series) tr.insertCell().textContent = withUnit(s.cells[i] || '', this._unit);
    });
    parts.push(wrap);

    toggle.addEventListener('click', () => {
      const on = toggle.getAttribute('aria-pressed') !== 'true';
      toggle.setAttribute('aria-pressed', String(on));
      wrap.classList.toggle('hc-sr', !on);
    });
    cv.addEventListener('mousemove', e => {
      const r = cv.getBoundingClientRect();
      this._activate(this._hit(e.clientX - r.left, e.clientY - r.top));
    });
    cv.addEventListener('mouseleave', () => { if (document.activeElement !== cv) this._activate(-1); });
    cv.addEventListener('blur', () => this._activate(-1));
    cv.addEventListener('keydown', e => {
      const count = this._kind === 'donut' ? this._slices().length : this._data.categories.length;
      if (!count) return;
      const a = this._active;
      const next = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? (a + 1) % count
        : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? (a <= 0 ? count - 1 : a - 1)
          : e.key === 'Home' ? 0 : e.key === 'End' ? count - 1 : e.key === 'Escape' ? -2 : -3;
      if (next === -3) return;
      e.preventDefault();
      this._activate(next === -2 ? -1 : next);
    });

    this.prepend(...parts);
    this._drawn = '';
    this._draw(true);
  }

  /** A swatch's colour and, in dither, its series look (`data-look`, the fill variant). Fill charts show the look alone. */
  private _look(sw: HTMLElement, k: number, slot: number): void {
    sw.style.setProperty('--hc-c', k < 0 ? 'var(--hc-muted)' : `var(--hc-s${k + 1})`);
    sw.dataset.look = String(slot % 4);
    if (this._kind !== 'line') sw.dataset.solo = '';
  }

  private _activate(i: number): void {
    if (i === this._active) return;
    this._entered = true;
    this._active = i;
    this._draw(true);
    this._showTip();
  }

  /** The category (or donut part) under a point in CSS px, or -1. */
  private _hit(mx: number, my: number): number {
    const g = this._geo;
    if (!g) return -1;
    if (g.kind === 'donut') {
      const dx = mx - g.cx;
      const dy = my - g.cy;
      const d = Math.hypot(dx, dy);
      if (d < g.r0 - 4 || d > g.r1 + 6) return -1;
      let a = Math.atan2(dy, dx) + Math.PI / 2;
      if (a < 0) a += Math.PI * 2;
      return g.arcs.findIndex((end, i) => a >= (g.arcs[i - 1] ?? 0) && a < end);
    }
    if (mx < g.left - 12 || mx > g.right + 12 || !g.n) return -1;
    let best = 0;
    for (let i = 1; i < g.n; i++) if (Math.abs(g.x(i) - mx) < Math.abs(g.x(best) - mx)) best = i;
    return best;
  }

  private _showTip(): void {
    const tip = this._tip;
    const g = this._geo;
    if (!tip) return;
    tip.replaceChildren();
    if (this._active < 0 || !g) { tip.hidden = true; return; }
    const line = (k: number, name: string, value: string, slot = k) => {
      const row = h('div', 'hc-tip-row');
      const sw = h('span', 'hc-sw', this._kind === 'bar' || this._kind === 'donut' ? '■' : MARKER_GLYPHS[k], { 'aria-hidden': 'true' });
      this._look(sw, k, slot);
      row.append(sw, h('span', '', `${name}: ${value}`));
      tip.append(row);
    };
    let x: number;
    if (g.kind === 'donut') {
      const slices = this._slices();
      const s = slices[this._active];
      const total = slices.reduce((a, p) => a + p.value, 0) || 1;
      line(s.other ? -1 : this._active, s.label, `${formatTick(s.value, this._prefix, this._unit)} (${Math.round((s.value / total) * 100)}%)`, this._active);
      x = g.cx + g.r1;
    } else {
      tip.append(h('strong', '', this._data.categories[this._active] || ''));
      this._data.series.slice(0, MAX_SERIES).forEach((s, k) => line(k, s.name, withUnit(s.cells[this._active] || '', this._unit)));
      x = g.x(this._active);
    }
    tip.hidden = false;
    const w = this._plot?.clientWidth || 0;
    const tw = tip.offsetWidth;
    tip.style.left = `${Math.max(0, x + 12 + tw > w ? x - 12 - tw : x + 12)}px`;
  }

  /** Size the canvas to the plot's width × devicePixelRatio and paint, unless
   *  nothing changed. A hidden chart (zero width) waits for `hc-reflow`. */
  _draw(force = false): void {
    const cv = this._cv;
    const w = this._plot?.clientWidth || 0;
    if (!cv || !w) return;
    const dpr = Math.max(1, devicePixelRatio || 1);
    const asked = Number(this.getAttribute('height'));
    const hgt = this._kind === 'donut' ? Math.round(Math.min(240, Math.max(160, w * 0.5)))
      : Math.round(Math.min(480, Math.max(120, asked || 220)));
    const key = `${w}x${hgt}@${dpr}`;
    if (!force && key === this._drawn) return;
    this._entrance?.();
    this._entrance = null;
    const resized = key !== this._drawn;
    this._drawn = key;
    if (resized) {
      cv.style.height = `${hgt}px`;
      cv.width = Math.round(w * dpr);
      cv.height = Math.round(hgt * dpr);
    }
    const ctx = cv.getContext('2d');
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    if (!resized) ctx.clearRect(0, 0, w, hgt);
    this._paint(ctx, w, hgt);
    if (!this._entered && this._data.categories.length && motionAllowed()) {
      this._entered = true;
      // Encode only on first visibility, never for unread/offscreen charts.
      if (this._plot) this._entrance = chartEntrance(this, this._plot, () => {
        const data = document.createElement('canvas'); data.width = cv.width; data.height = cv.height;
        const dc = data.getContext('2d');
        if (dc) { dc.setTransform(dpr, 0, 0, dpr, 0, 0); this._paint(dc, w, hgt, 'data'); }
        return data.toDataURL();
      }, w, hgt, this._kind, this._baseline,
        () => { ctx.clearRect(0, 0, w, hgt); this._paint(ctx, w, hgt, 'axes'); },
        () => { this._entrance = null; this._draw(true); });
    }
  }

  private _paint(ctx: Ctx, w: number, hgt: number, layer: 'all' | 'axes' | 'data' | 'top' = 'all'): void {
    const cs = getComputedStyle(this);
    const tok = (name: string, fallback: string) => cs.getPropertyValue(name).trim() || fallback;
    const ink = tok('--hc-ink', '#27252c');
    const muted = tok('--hc-muted', '#6c6575');
    const grid = tok('--hc-line', '#dfddda');
    const surface = tok('--hc-surface', '#ffffff');
    const colour = (k: number) => tok(`--hc-s${k + 1}`, '#2a78d6');
    const font = cs.fontFamily || 'system-ui, sans-serif';
    let texture = false;
    try { texture = matchMedia(TEXTURE).matches; } catch { /* no matchMedia */ }
    const dither = textureOf(this) === 'dither';
    const gain = (window as unknown as { hollowCanvas?: { textureLevel?: string } }).hollowCanvas?.textureLevel === 'quiet' ? 0.6 : 1;
    const { categories: cats, series } = this._data;
    const S = this._shown;
    const n = cats.length;
    ctx.font = `11px ${font}`;
    ctx.lineJoin = 'round';
    ctx.lineCap = 'round';

    // A dithered chart keeps its fills in a layer that is rebuilt only when
    // its size, colours, Quiet level or data change, so a hover repaints just
    // the axes, one image copy and the active mark.
    if (layer === 'all' && dither && n && S) {
      const dpr = Math.max(1, devicePixelRatio || 1);
      const key = [w, hgt, dpr, this._kind, gain, texture, ink, surface, muted, font, ...Array.from({ length: 8 }, (_, k) => colour(k))].join('|');
      let l = this._layer;
      if (!l || l.key !== key || l.data !== this._data) {
        const cv = l?.cv ?? document.createElement('canvas');
        const [cw, ch] = [Math.round(w * dpr), Math.round(hgt * dpr)];
        if (cv.width !== cw || cv.height !== ch) { cv.width = cw; cv.height = ch; }
        const dc = cv.getContext('2d');
        if (dc) { dc.setTransform(dpr, 0, 0, dpr, 0, 0); dc.clearRect(0, 0, w, hgt); this._paint(dc, w, hgt, 'data'); }
        l = this._layer = { key, data: this._data, cv };
      }
      this._paint(ctx, w, hgt, 'axes');
      ctx.drawImage(l.cv, 0, 0, w, hgt);
      this._paint(ctx, w, hgt, 'top');
      return;
    }
    const showAxes = layer === 'all' || layer === 'axes';
    const showData = layer === 'all' || layer === 'data';
    const act = layer === 'data' ? -1 : this._active;

    if (!n || !S) {
      this._geo = null;
      ctx.fillStyle = muted;
      ctx.fillText('No data', 8, 20);
      return;
    }

    if (this._kind === 'donut') {
      const slices = this._slices();
      if (!slices.length) {
        this._geo = null;
        ctx.fillStyle = muted;
        ctx.fillText(series[0].values.some(v => v !== null) ? 'No positive values' : 'No data', 8, 20);
        return;
      }
      const total = slices.reduce((a, s) => a + s.value, 0);
      const cx = w / 2;
      const cy = hgt / 2;
      const r1 = Math.min(w, hgt) / 2 - 6;
      const r0 = r1 * 0.62;
      const arcs: number[] = [];
      let a = 0;
      slices.forEach((s, i) => {
        const b = a + (total ? (s.value / total) * Math.PI * 2 : 0);
        arcs.push(b);
        const r = i === act ? r1 + 4 : r1;
        if (showData || (layer === 'top' && i === act)) {
        ctx.beginPath();
        ctx.arc(cx, cy, r, a - Math.PI / 2, b - Math.PI / 2);
        ctx.arc(cx, cy, r0, b - Math.PI / 2, a - Math.PI / 2, true);
        ctx.closePath();
        ctx.fillStyle = s.other ? muted : colour(i);
        // The grown sector covers the cached one (opacity-only cells would show its ink edge through).
        if (layer === 'top') { ctx.save(); ctx.fillStyle = surface; ctx.fill(); ctx.restore(); }
        if (dither) ditherRing(ctx, ctx.fillStyle as string, i, cx, cy, r0, r, gain);
        else ctx.fill();
        if (dither) { ctx.save(); ctx.clip(); ctx.strokeStyle = ink; ctx.lineWidth = 4; ctx.stroke(); ctx.restore(); }
        // The 2px gap first: hatch() replaces the current path.
        ctx.strokeStyle = surface;
        ctx.lineWidth = 2;
        ctx.stroke();
        if (texture && i) hatch(ctx, i, surface, w, hgt);
        }
        a = b;
      });
      this._geo = { kind: 'donut', left: 0, right: w, n: slices.length, x: () => cx, cx, cy, r0, r1, arcs };
      if (!showAxes) return;
      ctx.textAlign = 'center';
      ctx.fillStyle = ink;
      ctx.font = `600 18px ${font}`;
      ctx.textBaseline = 'alphabetic';
      ctx.fillText(formatTick(total, this._prefix, this._unit), cx, cy + 2);
      ctx.fillStyle = muted;
      ctx.font = `11px ${font}`;
      ctx.textBaseline = 'top';
      ctx.fillText('Positive total', cx, cy + 6);
      return;
    }

    const bar = this._kind === 'bar';
    const values = series.slice(0, S).flatMap(s => s.values).filter((v): v is number => v !== null);
    let lo = values.length ? Math.min(...values) : 0;
    let hi = values.length ? Math.max(...values) : 1;
    if (this._kind !== 'line' || (lo >= 0 && lo < hi / 2)) { lo = Math.min(lo, 0); hi = Math.max(hi, 0); }
    const tk = niceTicks(lo, hi, Math.max(2, Math.min(5, Math.floor(hgt / 50))));
    const labels = tk.ticks.map(v => formatTick(v, this._prefix, this._unit));
    const left = Math.ceil(Math.max(...labels.map(t => ctx.measureText(t).width))) + 10;

    // Direct end labels: a single line's last value, or two to four names.
    const ends: Array<{ k: number; text: string; i: number }> = [];
    if (!bar && S <= 4) {
      series.slice(0, S).forEach((s, k) => {
        let i = s.values.length - 1;
        while (i >= 0 && s.values[i] === null) i--;
        if (i >= 0) ends.push({ k, i, text: S === 1 ? withUnit(s.cells[i], this._unit) : s.name });
      });
    }
    let pad = ends.length ? Math.ceil(Math.max(...ends.map(e => ctx.measureText(e.text).width))) + 14 : 10;
    if (pad > w * 0.3) { ends.length = 0; pad = 10; }
    const right = w - pad;
    const top = 10;
    const bottom = hgt - 24;
    const y = scale(tk.lo, tk.hi, bottom, top);
    const base = y(Math.min(Math.max(0, tk.lo), tk.hi));
    this._baseline = base;
    const band = (right - left) / n;
    const x = bar ? (i: number) => left + band * (i + 0.5)
      : (i: number) => (n === 1 ? (left + right) / 2 : left + 6 + (i * (right - left - 12)) / (n - 1));
    this._geo = { kind: this._kind, left, right, n, x, cx: 0, cy: 0, r0: 0, r1: 0, arcs: [] };

    if (showAxes) {
    // Grid and y ticks: solid hairlines; the zero line a step stronger.
    ctx.lineWidth = 1;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    tk.ticks.forEach((v, i) => {
      const yy = Math.round(y(v)) + 0.5;
      ctx.strokeStyle = v === 0 ? muted : grid;
      ctx.globalAlpha = v === 0 ? 0.5 : 1;
      ctx.beginPath();
      ctx.moveTo(left, yy);
      ctx.lineTo(right, yy);
      ctx.stroke();
      ctx.globalAlpha = 1;
      ctx.fillStyle = muted;
      ctx.fillText(labels[i], left - 6, yy);
    });

    // Category labels, thinned so they never overlap.
    ctx.textAlign = 'center';
    ctx.textBaseline = 'top';
    const widest = Math.max(...cats.map(c => ctx.measureText(c).width)) + 8;
    const every = Math.max(1, Math.ceil(widest / Math.max(1, (right - left) / n)));
    cats.forEach((c, i) => { if (i % every === 0) ctx.fillText(c, x(i), bottom + 6); });

    // Hover / keyboard focus: a crosshair, or a wash behind the column group.
    if (this._active >= 0) {
      if (bar) {
        ctx.fillStyle = ink;
        ctx.globalAlpha = 0.06;
        ctx.fillRect(x(this._active) - band / 2, top, band, bottom - top);
        ctx.globalAlpha = 1;
      } else {
        const xx = Math.round(x(this._active)) + 0.5;
        ctx.strokeStyle = muted;
        ctx.beginPath();
        ctx.moveTo(xx, top);
        ctx.lineTo(xx, bottom);
        ctx.stroke();
      }
    }

    }

    const mark = (k: number, c: string, i: number, v: number) => {
      const r = i === act ? 5 : 4;
      ctx.fillStyle = surface;
      marker(ctx, k, x(i), y(v), r + 2);
      ctx.fill();
      ctx.fillStyle = c;
      marker(ctx, k, x(i), y(v), r);
      ctx.fill();
      if (dither) { ctx.strokeStyle = ink; ctx.lineWidth = 1; ctx.stroke(); }
    };
    if (layer === 'top') {
      if (!bar && this._active >= 0) for (let k = 0; k < S; k++) { const v = series[k].values[this._active]; if (v !== null && v !== undefined) mark(k, colour(k), this._active, v); }
      return;
    }
    if (bar) {
      const group = Math.min(band * 0.72, S * 24 + (S - 1) * 2);
      const bw = Math.max(1, (group - (S - 1) * 2) / S);
      if (showData) for (let k = 0; k < S; k++) {
        series[k].values.forEach((v, i) => {
          if (v === null) return;
          const bx = x(i) - group / 2 + k * (bw + 2);
          column(ctx, bx, bw, base, y(v));
          if (dither) {
            ditherFill(ctx, colour(k), k, bx, bx + bw, () => y(v), base, gain);
            // Clip the solid boundary inside the exact data geometry.
            ctx.save(); ctx.clip(); ctx.strokeStyle = ink; ctx.lineWidth = 2; ctx.stroke(); ctx.restore();
          } else { ctx.fillStyle = colour(k); ctx.fill(); }
          if (texture && k) hatch(ctx, k, surface, w, hgt);
        });
      }
      // One series: its value on each cap, when every label fits its band.
      if (S === 1 && showAxes) {
        const texts = series[0].cells.map(c => withUnit(c, this._unit));
        if (texts.every(t => ctx.measureText(t).width <= band - 4)) {
          ctx.fillStyle = ink;
          series[0].values.forEach((v, i) => {
            if (v === null) return;
            ctx.textBaseline = v < 0 ? 'top' : 'bottom';
            ctx.fillText(texts[i], x(i), y(v) + (v < 0 ? 4 : -4));
          });
        }
      }
      return;
    }

    const dense = n > 24;
    // Decorative fills sit below every series' strokes and markers. Preserve
    // the legacy ordering for plain/ASCII charts.
    if (showData) for (let pass = 0; pass < (dither ? 2 : 1); pass++) for (let k = 0; k < S; k++) {
      const vals = series[k].values;
      const c = colour(k);
      const path = (close: boolean) => {
        let run: number[] = [];
        const flush = () => {
          if (!run.length) return;
          ctx.moveTo(x(run[0]), close ? base : y(vals[run[0]]!));
          for (const i of run) ctx.lineTo(x(i), y(vals[i]!));
          if (close) { ctx.lineTo(x(run[run.length - 1]), base); ctx.closePath(); }
          run = [];
        };
        ctx.beginPath();
        vals.forEach((v, i) => { if (v === null) flush(); else run.push(i); });
        flush();
      };
      if (dither ? !pass : this._kind === 'area') {
        path(true);
        if (dither) ditherFill(ctx, c, k, x(0), x(n - 1), lineTop(vals, x, y), base, this._kind === 'area' ? gain : gain * 0.55);
        else { ctx.fillStyle = c; ctx.globalAlpha = 0.1; ctx.fill(); ctx.globalAlpha = 1; }
      }
      if (dither && !pass) continue;
      path(false);
      if (dither) { ctx.strokeStyle = ink; ctx.lineWidth = 4; ctx.stroke(); }
      ctx.strokeStyle = c;
      ctx.lineWidth = 2;
      ctx.stroke();
      let last = vals.length - 1;
      while (last >= 0 && vals[last] === null) last--;
      vals.forEach((v, i) => {
        if (v === null || (dense && i !== last && i !== act)) return;
        mark(k, c, i, v);
      });
    }

    // End labels sit beside their line; if two would collide, the legend
    // carries identity instead of stacking them.
    const placed = ends.map(e => ({ ...e, y: y(series[e.k].values[e.i]!) })).sort((a, b) => a.y - b.y);
    if (showAxes && placed.every((p, i) => !i || p.y - placed[i - 1].y >= 13)) {
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillStyle = ink;
      for (const p of placed) ctx.fillText(p.text, x(p.i) + 8, p.y);
    }
  }
}
