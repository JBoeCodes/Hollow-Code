import { expect, it } from 'vitest';
import { deriveCustomVars } from './customThemeVars';
import { contrast, parseColor } from './canvas/kit/palette';
import type { CustomTheme } from './types';

const palette = (dark: boolean): CustomTheme => ({
  name: 'Low contrast fixture', source: '', accent: dark ? '#bbbbbb' : '#555555',
  foreground: dark ? '#ffffff' : '#000000', background: dark ? '#000000' : '#ffffff',
  isDark: dark, colors: Array.from({ length: 16 }, (_, i) => i === 8 ? (dark ? '#4d4d4d' : '#cccccc') : null),
  cursor: null, selectionForeground: null, selectionBackground: null,
  success: null, warning: null, error: null,
});

for (const dark of [false, true]) it(`custom text remains AA on every app surface (${dark ? 'dark' : 'light'})`, () => {
  const theme = palette(dark), original = structuredClone(theme), vars = deriveCustomVars(theme);
  for (const token of ['--ink', '--body-strong', '--body', '--muted', '--muted-soft']) {
    for (const surface of ['--canvas', '--surface-soft', '--surface-card', '--surface-cream-strong', '--surface-raised']) {
      expect(contrast(parseColor(vars[token])!, parseColor(vars[surface])!), `${token} on ${surface}`).toBeGreaterThanOrEqual(4.5);
    }
  }
  expect(theme).toEqual(original);
});

it('mid-tone surfaces retain one readable text polarity without changing the theme file', () => {
  const theme = { ...palette(true), background: '#666666' }, original = structuredClone(theme);
  const vars = deriveCustomVars(theme);
  for (const token of ['--ink', '--body-strong', '--body', '--muted', '--muted-soft']) {
    for (const surface of ['--canvas', '--surface-soft', '--surface-card', '--surface-cream-strong', '--surface-raised']) {
      expect(contrast(parseColor(vars[token])!, parseColor(vars[surface])!), `${token} on ${surface}`).toBeGreaterThanOrEqual(4.5);
    }
  }
  expect(theme).toEqual(original);
});
