import { describe, expect, it } from 'vitest';
import { exportName, exportThemeSuffix, EXPORT_WIDTHS } from './canvasExport';
import { CANVAS_HOSTS, offers } from './hostContract';

describe('canvas export', () => {
  it('keeps identity, revision, size and look in the filename', () => {
    expect(exportName('release-status', 2, 'doc', 'png', false)).toBe('release-status-v2-doc.png');
    expect(exportName('release-status', 1, 'slide', 'pdf', true)).toBe('release-status-v1-slide-dark.pdf');
    expect(EXPORT_WIDTHS).toEqual({ email: 600, doc: 800, slide: 1280 });
    for (const bad of ['../other', 'has/slash', 'x.png', '']) expect(() => exportName(bad, 1, 'doc', 'png', false)).toThrow();
    expect(() => exportName('valid', -1, 'doc', 'png', false)).toThrow();
  });
  it('withholds live-pin exports and accounts for every other canvas host', () => {
    for (const host of CANVAS_HOSTS) expect(offers(host, 'export'), host).toBe(host !== 'pinned');
  });
  it('names a selected palette safely for all three formats', () => {
    for (const format of ['png', 'pdf', 'svg'] as const) {
      expect(exportName('release', 3, 'doc', format, true, 2, 'Solarized Dark'))
        .toBe(`release-v3-doc-dark-solarized-dark${format === 'svg' ? '-flow-2' : ''}.${format}`);
    }
    expect(exportThemeSuffix('../Paper / A')).toBe('-paper-a');
    expect(exportThemeSuffix('日本語')).toBe('-theme');
    expect(exportThemeSuffix('x'.repeat(100))).toHaveLength(49);
    expect(exportThemeSuffix('a'.repeat(47) + ' b')).toBe('-' + 'a'.repeat(47));
  });
});
