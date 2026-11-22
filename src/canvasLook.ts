import { deriveCustomVars } from './customThemeVars';
import { THEME_TOKENS } from './components/htmlSandbox';
import { contrast, parseColor, type Rgb } from './canvas/kit/palette';
import type { CustomTheme } from './types';
import { isSafeThemeValue } from './canvasBridge';

export interface Look {
  scheme: 'light' | 'dark';
  pairs: Array<[string, string]>;
  adjusted: Array<{ token: string; from: string; to: string; ratio: number; before: number }>;
  warnings?: string[];
}

// The kit parser also accepts alpha/prefixes. Only opaque, complete colours
// have the contrast measured here; unsupported forms get an honest warning.
const opaqueColor = (value: string): Rgb | null => /^(#[0-9a-f]{6}|rgb\(\s*(?:\d+(?:\.\d+)?\s*,\s*){2}\d+(?:\.\d+)?\s*\)|rgba\(\s*(?:\d+(?:\.\d+)?\s*,\s*){3}1(?:\.0*)?\s*\))$/i.test(value)
  ? parseColor(value)?.map(v => Math.max(0, Math.min(255, v))) as Rgb ?? null : null;

/** Host-only palette. Never changes the theme file, tone or series colours. */
export function lookPairs(theme: CustomTheme): Look {
  // Named Looks keep their own adjustment receipts and unsupported-colour warnings.
  const vars = deriveCustomVars(theme, false);
  const warnings: string[] = [];
  const pairs: Array<[string, string]> = THEME_TOKENS.map(([key, token, fallback]) => {
    const raw = vars[token] ?? fallback;
    const value = /^#[0-9a-f]{3}$/i.test(raw) ? '#' + raw.slice(1).split('').map(c => c + c).join('') : raw;
    if (!isSafeThemeValue(value)) { warnings.push(`${key}: invalid colour; using the default.`); return [key, fallback]; }
    return [key, value];
  });
  const values = Object.fromEntries(pairs);
  const adjusted: Look['adjusted'] = [];
  const fix = (token: string, backgrounds: string[], target: number) => {
    const from = values[token], rgb = opaqueColor(from);
    const parsed = backgrounds.map(k => opaqueColor(values[k]));
    if (!rgb || parsed.some(c => !c)) { warnings.push(`${token}: contrast could not be checked for this colour.`); return; }
    const backs = parsed as Rgb[];
    const ratio = (color: Rgb) => Math.min(...backs.map(bg => contrast(color, bg)));
    const before = ratio(rgb);
    if (before >= target) return;
    for (let step = 1; step <= 20; step++) {
      const options = [0, 255].map(end => rgb.map(v => Math.round(v + (end - v) * step / 20)) as Rgb);
      const chosen = options.find(c => ratio(c) >= target);
      if (!chosen) continue;
      const to = '#' + chosen.map(v => v.toString(16).padStart(2, '0')).join('');
      values[token] = to;
      adjusted.push({ token, from, to, before, ratio: ratio(chosen) });
      return;
    }
    warnings.push(`${token}: no adjustment meets ${target}:1 on both backgrounds.`);
  };
  fix('--app-ink', ['--app-canvas', '--app-surface'], 4.5);
  for (const t of ['--app-body', '--app-body-strong', '--app-muted', '--app-muted-soft']) fix(t, ['--app-surface'], 4.5);
  fix('--app-primary', ['--app-surface'], 3);
  fix('--app-on-primary', ['--app-primary'], 4.5);
  return { scheme: theme.isDark ? 'dark' : 'light', pairs: pairs.map(([k]) => [k, values[k]]), adjusted, warnings };
}
