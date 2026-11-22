import { describe, it, expect } from 'vitest';
import KIT_JS from './kit/generated/kit.min.js?raw';
import KIT_CSS from './kit/generated/kit.min.css?raw';
import TOKENS_CSS from '../styles/tokens.css?raw';
import {
  baseName, columnCompare, dedent, donutSlices, formatTick, highlight, isNumeric, niceTicks, parseNum, parseRows,
  scale, sharedPrefix, sortOrder, splitNames, toChartData, vaultPath, withUnit,
} from './kit/data';
import {
  MARKER_GLYPHS, MAX_SERIES, SERIES_DARK, SERIES_LIGHT, contrast, parseColor, schemeOf, type Rgb,
} from './kit/palette';

const rgb = (hex: string): Rgb => parseColor(hex)!;
/** Hollow's chart surfaces: --surface-card in the light and dark blocks. */
const token = (block: RegExp, name: string) => {
  const body = block.exec(TOKENS_CSS)![1];
  return new RegExp(`${name}:\\s*(#[0-9a-f]{6})`, 'i').exec(body)![1];
};
const LIGHT_SURFACE = token(/:root \{([\s\S]*?)\n\}/, '--surface-card');
const DARK_SURFACE = token(/:root\[data-theme="dark"\] \{([\s\S]*?)\n\}/, '--surface-card');

describe('kit data rows (Canvas C2 K2)', () => {
  it('reads Label: value lines, pipe rows and markdown rules', () => {
    expect(parseRows(`
      Lane A: 12
      Lane B: 30, 4
      | Suite | Passed |
      |---|:--:|
      Rust | 2,077
      Mon 10:30: 5; 6
    `)).toEqual([
      ['Lane A', '12'],
      ['Lane B', '30', '4'],
      ['Suite', 'Passed'],
      ['Rust', '2,077'],
      ['Mon 10:30', '5', '6'],
    ]);
  });

  it('parses numbers with grouping, signs, currency, percent and short units', () => {
    expect(parseNum('1,200')).toBe(1200);
    expect(parseNum('1,200,000.5')).toBe(1200000.5);
    expect(parseNum('$42')).toBe(42);
    expect(parseNum('−3.5%')).toBe(-3.5);
    expect(parseNum('+2')).toBe(2);
    expect(parseNum('12 h')).toBe(12);
    expect(parseNum('.5')).toBe(0.5);
    for (const text of ['', '—', 'n/a', 'Q3', 'Mon 12', 'Lane A', '1.2.3']) expect(parseNum(text), text).toBeNull();
    expect(isNumeric('2025')).toBe(true);
    expect(isNumeric('Passed')).toBe(false);
  });

  it('a 1,200 is one value, and "12, 8" is two', () => {
    expect(parseRows('Mon: 1,200, 8')).toEqual([['Mon', '1,200', '8']]);
  });

  it('finds a header row, series names and the series attribute', () => {
    const data = toChartData(parseRows('Suite | Passed | Skipped\nRust | 2,077 | 12\nVitest | 1,025 |'));
    expect(data.head).toBe('Suite');
    expect(data.categories).toEqual(['Rust', 'Vitest']);
    expect(data.series.map(s => s.name)).toEqual(['Passed', 'Skipped']);
    expect(data.series[0].values).toEqual([2077, 1025]);
    expect(data.series[1].values).toEqual([12, null]);
    expect(data.series[1].cells).toEqual(['12', '']);

    const named = toChartData(parseRows('Mon: 12, 8\nTue: 15, 9'), splitNames('Claude, Codex'));
    expect(named.head).toBe('');
    expect(named.series.map(s => s.name)).toEqual(['Claude', 'Codex']);
    expect(toChartData(parseRows('A: 1\nB: 2'), [], 'Hours').series.map(s => s.name)).toEqual(['Hours']);
    // A <th> row is a header even when its cells look numeric.
    expect(toChartData([['Year', '2024', '2025'], ['Revenue', '1', '2']], [], 'Value', true).series.map(s => s.name)).toEqual(['2024', '2025']);
    expect(splitNames('A | B, C')).toEqual(['A', 'B, C']);
    expect(splitNames(null)).toEqual([]);
  });

  it('keeps the author\'s cell text and adds a unit only to bare numbers', () => {
    expect(withUnit('12', 'h')).toBe('12 h');
    expect(withUnit('45', '%')).toBe('45%');
    expect(withUnit('12 hours', 'h')).toBe('12 hours');
    expect(withUnit('', 'h')).toBe('—');
    expect(sharedPrefix(['$42', '$18', ''])).toBe('$');
    expect(sharedPrefix(['$42', '€18'])).toBe('');
  });
});

describe('kit scales and ticks', () => {
  it('nice ticks cover the data with round steps', () => {
    expect(niceTicks(0, 21, 4)).toEqual({ lo: 0, hi: 25, step: 5, ticks: [0, 5, 10, 15, 20, 25] });
    expect(niceTicks(-3, 8, 4).ticks).toEqual([-4, -2, 0, 2, 4, 6, 8]);
    expect(niceTicks(0, 0.3, 3).ticks).toEqual([0, 0.1, 0.2, 0.3]);
    expect(niceTicks(5, 5).lo).toBe(0);
    expect(niceTicks(NaN, Infinity).ticks.length).toBeGreaterThan(1);
  });

  it('linear scale maps a domain onto pixels (inverted for y)', () => {
    const y = scale(0, 25, 200, 10);
    expect(y(0)).toBe(200);
    expect(y(25)).toBe(10);
    expect(scale(1, 1, 0, 100)(1)).toBe(0);
  });

  it('compact tick labels carry a shared prefix and percent', () => {
    expect(formatTick(1200)).toMatch(/^1\.2\s?K$/);
    expect(formatTick(40, '$')).toBe('$40');
    expect(formatTick(40, '', '%')).toBe('40%');
  });

  it('donut folds the tail into Other in authored order, never by rank', () => {
    const cats = ['a', 'b', 'c', 'd', 'e', 'f', 'g'];
    const slices = donutSlices(cats, [1, 50, 2, null, 3, 4, 5]);
    expect(slices.map(s => s.label)).toEqual(['a', 'b', 'c', 'e', 'f', 'g']);
    const folded = donutSlices([...cats, 'h'], [1, 2, 3, 4, 5, 6, 7, 8]);
    expect(folded.map(s => s.label)).toEqual(['a', 'b', 'c', 'd', 'e', 'Other']);
    expect(folded[5]).toEqual({ label: 'Other', value: 6 + 7 + 8, other: true });
  });
});

describe('kit table sorting', () => {
  it('sorts numeric columns by value and text naturally, empties last both ways', () => {
    const hours = ['12', '30', '7.5', ''];
    expect(sortOrder(hours, 'ascending')).toEqual([2, 0, 1, 3]);
    expect(sortOrder(hours, 'descending')).toEqual([1, 0, 2, 3]);
    const names = ['Lane 10', 'lane 2', 'Lane 1'];
    expect(sortOrder(names, 'ascending')).toEqual([2, 1, 0]);
    expect(columnCompare(['$1,200', '$900'])('$1,200', '$900')).toBeGreaterThan(0);
    // Stable for ties.
    expect(sortOrder(['b', 'a', 'b', 'a'], 'ascending')).toEqual([1, 3, 0, 2]);
  });
});

describe('hc-file paths', () => {
  it('passes vault paths and refuses anything that could be a web link', () => {
    expect(vaultPath(' outputs/report.md ')).toBe('outputs/report.md');
    expect(vaultPath('Notes/Daily 2026-10-04')).toBe('Notes/Daily 2026-10-04');
    for (const bad of ['', 'https://example.com/x', 'HTTP://x', 'mailto:a@b.c', 'javascript:alert(1)', 'file:///etc/passwd',
      'data:text/html,x', '//evil.example/x', '\\\\server\\share', 'C:\\Users', 'a\nb', 'x'.repeat(1025)]) {
      expect(vaultPath(bad), bad).toBeNull();
    }
    expect(baseName('outputs/canvases/report.md')).toBe('report.md');
    expect(baseName('dir/')).toBe('dir');
  });
});

describe('hc-code helpers', () => {
  it('dedents and trims blank edge lines', () => {
    expect(dedent('\n    // a\n      b();\n\n    c\n  ')).toBe('// a\n  b();\n\nc');
  });

  it('highlights comments, strings, numbers and keywords as text tokens', () => {
    expect(highlight('const x = "a"; // hi\nreturn 42', 'ts')).toEqual([
      ['keyword', 'const'], ['', ' x = '], ['string', '"a"'], ['', '; '], ['comment', '// hi'], ['', '\n'],
      ['keyword', 'return'], ['', ' '], ['number', '42'],
    ]);
    expect(highlight('x = 1 # note', 'py')).toEqual([['', 'x = '], ['number', '1'], ['', ' '], ['comment', '# note']]);
    expect(highlight('<b>const</b>', 'text')).toEqual([['', '<b>const</b>']]);
    expect(highlight('a', '')).toEqual([['', 'a']]);
  });
});

describe('kit chart palette (dataviz rules)', () => {
  it('is the dataviz reference order: eight fixed slots in each mode, plus a marker per slot', () => {
    expect(SERIES_LIGHT).toEqual(['#2a78d6', '#eb6834', '#1baf7a', '#eda100', '#e87ba4', '#008300', '#4a3aa7', '#e34948']);
    expect(SERIES_DARK).toHaveLength(SERIES_LIGHT.length);
    expect(MAX_SERIES).toBe(SERIES_LIGHT.length);
    expect(new Set(MARKER_GLYPHS).size).toBe(MAX_SERIES);
  });

  it('kit.css carries the same steps, light on :root and dark under data-hc-scheme', () => {
    const light = SERIES_LIGHT.map((c, i) => `--hc-s${i + 1}: ${c}`).join(';');
    const dark = SERIES_DARK.map((c, i) => `--hc-s${i + 1}: ${c}`).join(';');
    expect(KIT_CSS).toContain(light);
    expect(KIT_CSS).toContain(`:root[data-hc-scheme=dark]{${dark}}`);
  });

  it('every dark step clears 3:1 on Hollow\'s dark card; light steps under 3:1 are the known relief slots', () => {
    expect(LIGHT_SURFACE.toLowerCase()).toBe('#ffffff');
    for (const c of SERIES_DARK) expect(contrast(rgb(c), rgb(DARK_SURFACE)), c).toBeGreaterThanOrEqual(3);
    const low = SERIES_LIGHT.filter(c => contrast(rgb(c), rgb(LIGHT_SURFACE)) < 3);
    // aqua, yellow, magenta: charts always ship a legend, end labels and a table view.
    expect(low).toEqual(['#1baf7a', '#eda100', '#e87ba4']);
  });

  it('chooses the palette column from the surface the host sends', () => {
    expect(schemeOf(LIGHT_SURFACE)).toBe('light');
    expect(schemeOf(DARK_SURFACE)).toBe('dark');
    expect(schemeOf('rgba(35, 33, 32, 0.72)')).toBe('dark');
    expect(schemeOf('rgb(247, 245, 241)')).toBe('light');
    expect(schemeOf('not a colour')).toBe('light');
    expect(contrast(rgb('#000000'), rgb('#ffffff'))).toBeCloseTo(21, 5);
  });
});

describe('kit bundle constraints for K2', () => {
  it('never relies on ResizeObserver (dead in sandboxed WKWebView frames), and redraws on theme and reflow', () => {
    expect(KIT_JS).not.toContain('ResizeObserver');
    expect(KIT_JS).toContain('"hollow-theme"');
    expect(KIT_JS).toContain('"hc-reflow"');
    expect(KIT_JS).toContain('devicePixelRatio');
  });

  it('posts nothing over the bridge itself: hc-file is an a[data-vault] the runtime handles', () => {
    expect(KIT_JS).not.toMatch(/postMessage|__hc/);
    expect(KIT_JS).toContain('"data-vault"');
  });

  it('every operable kit part has a visible focus style', () => {
    expect(KIT_CSS).toMatch(/\.hc-chart-toggle,\.hc-sort,\.hc-tab,\.hc-file-link,\.hc-code-pre,hc-chart canvas,\[role=tabpanel\]\):focus-visible\{outline:2px solid/);
  });
});
