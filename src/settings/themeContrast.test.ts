import { expect, it } from 'vitest';
import { draftContrast, previewPalette } from './themeContrast';
import { contrast, parseColor } from '../canvas/kit/palette';
import appearance from './AppearanceRows.tsx?raw';

it('uses the existing AA derivation for sample text without altering custom colors', () => {
  const swatch = { bg: '#fafafa', fg: '#eeeeee', accent: '#eeeeff' };
  const derived = previewPalette(swatch, false);
  expect(contrast(parseColor(derived['--ink'])!, parseColor(derived['--canvas'])!)).toBeGreaterThanOrEqual(4.5);
  expect(derived['--canvas']).toBe(swatch.bg);
  expect(swatch.fg).toBe('#eeeeee');
});
it('keeps theme samples passive while Use retains the selection action', () => {
  const card = appearance.slice(appearance.indexOf('export function ThemeCard'), appearance.indexOf('export function ThemeEditor'));
  expect(card).not.toMatch(/<button\s+class="theme-card-preview"/);
  expect(card).toContain('Sample');
  expect(card).toContain('onClick={(e) => props.onUse(e)}');
  expect(card).toContain("aria-label={`${props.active ? 'In use:' : 'Use'} ${props.name} theme`}");
  expect(card).toContain('aria-pressed={props.active}');
});

it('measures both documented draft forms without applying or changing the source', () => {
  for (const source of [
    'accent #bbb\nforeground #fff\nbackground #000\ncolor8 #4d4d4d\nis_dark yes',
    'accent = "#bbbbbb" # comment\nforeground = "#ffffff"\nbackground = "#000000"\ncolor8 = "#4d4d4d"\nis_dark = true',
  ]) {
    expect(draftContrast(source)?.ratio).toBeGreaterThanOrEqual(4.5);
    expect(draftContrast(source)?.adjusted).toBe(true);
  }
});
it('withholds a measurement for incomplete, malformed or unsupported draft colors', () => {
  const valid = 'accent #bbb\nforeground #fff\nbackground #000';
  for (const source of ['', 'foreground nope', valid + '\ncolor8 papayawhip', valid + '\ncolor8 "#fff', valid + '\ncolor8 rgba(255,255,255,0.2)']) {
    expect(draftContrast(source)).toBeNull();
  }
});
