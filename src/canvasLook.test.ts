import { describe, expect, it, vi } from 'vitest';
vi.mock('./api', () => ({ api: {} }));
import { lookPairs } from './canvasLook';
import { deriveCustomVars } from './customThemeVars';
import { THEME_TOKENS } from './components/htmlSandbox';
import { contrast, parseColor } from './canvas/kit/palette';
import type { CustomTheme } from './types';
const theme = (dark = false): CustomTheme => ({
  name: 'Low contrast', source: '', accent: '#989898', foreground: '#999999',
  background: dark ? '#202020' : '#ffffff', isDark: dark, colors: Array(16).fill(null),
  cursor: null, selectionForeground: null, selectionBackground: null, success: null, warning: null, error: null,
});
describe('workspace canvas look', () => {
  it('normalizes short hex and reports unsupported colours without calling them missing', () => {
    const t = { ...theme(true), foreground: '#eee', accent: '#abc', background: '#222' };
    const look = lookPairs(t);
    expect(look.warnings).toEqual([]);
    expect(Object.fromEntries(look.pairs)['--app-canvas']).toBe('#222222');
    expect(lookPairs({ ...t, foreground: 'papayawhip' }).warnings?.length).toBeGreaterThan(0);
    expect(lookPairs({ ...t, foreground: '#eeeeee55' }).warnings?.length).toBeGreaterThan(0);
    expect(lookPairs({ ...t, foreground: 'rgba(255, 255, 255, 0.2)' }).warnings?.length).toBeGreaterThan(0);
    expect(lookPairs({ ...t, foreground: 'rgb(255, 255, 255, 0.2)' }).warnings).toContain('--app-ink: contrast could not be checked for this colour.');
    expect(lookPairs({ ...t, foreground: 'rgb(255, 255, 255)' }).warnings).toEqual([]);
    expect(lookPairs({ ...t, foreground: 'rgba(255, 255, 255, 1)' }).warnings).toEqual([]);
    const hostile = lookPairs({ ...t, foreground: '</style><script>bad()</script>' });
    expect(hostile.pairs.every(([, value]) => !value.includes('<'))).toBe(true);
    expect(hostile.warnings?.length).toBeGreaterThan(0);
    expect(lookPairs({ ...t, background: '#777777', foreground: '#000000' }).warnings).toContain('--app-ink: no adjustment meets 4.5:1 on both backgrounds.');
  });
  it('covers every frame token and leaves tone colours unchanged', () => {
    const t = theme(), look = lookPairs(t), values = Object.fromEntries(look.pairs), derived = deriveCustomVars(t);
    expect(look.pairs.map(([k]) => k)).toEqual(THEME_TOKENS.map(([k]) => k));
    for (const [key, source] of THEME_TOKENS.filter(([k]) => /info|success|warning|danger|risk/.test(k))) expect(values[key]).toBe(derived[source]);
    expect(t.foreground).toBe('#999999');
    expect(look.adjusted.length).toBeGreaterThan(0);
  });
  for (const dark of [false, true]) it(`meets foreground targets (${dark ? 'dark' : 'light'})`, () => {
    const look = lookPairs(theme(dark)), v = Object.fromEntries(look.pairs);
    expect(look.scheme).toBe(dark ? 'dark' : 'light');
    const ratio = (a: string, b: string) => contrast(parseColor(v[a])!, parseColor(v[b])!);
    expect(ratio('--app-ink', '--app-canvas')).toBeGreaterThanOrEqual(4.5);
    for (const key of ['--app-ink', '--app-body', '--app-body-strong', '--app-muted', '--app-muted-soft']) expect(ratio(key, '--app-surface')).toBeGreaterThanOrEqual(4.5);
    expect(ratio('--app-primary', '--app-surface')).toBeGreaterThanOrEqual(3);
    expect(ratio('--app-on-primary', '--app-primary')).toBeGreaterThanOrEqual(4.5);
  });
});
