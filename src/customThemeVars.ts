import type { CustomTheme } from './types';
import { contrast, parseColor, type Rgb } from './canvas/kit/palette';

export function deriveCustomVars(t: CustomTheme, clampText = true): Record<string, string> {
  const bg = t.background;
  const fg = t.foreground;
  const accent = t.accent;
  const c = t.colors;
  const dark = t.isDark;

  const surfaceSoft = mix(bg, fg, dark ? 0.06 : 0.04);
  const surfaceCard = mix(bg, fg, dark ? 0.10 : 0.08);
  const surfaceStrong = mix(bg, fg, dark ? 0.18 : 0.14);
  // Raised = the only surface lighter than its surroundings (transient
  // popovers/modals). Dark themes lift toward fg; light themes toward white.
  const surfaceRaised = dark ? mix(bg, fg, 0.13) : mix(bg, '#ffffff', 0.55);
  const hairline = c[8] ?? mix(bg, fg, dark ? 0.28 : 0.32);
  const hairlineSoft = mix(bg, fg, dark ? 0.12 : 0.14);
  const ink = fg;
  const bodyStrong = fg;
  const body = mix(fg, bg, 0.10);
  const muted = c[8] ?? mix(fg, bg, 0.45);
  const mutedSoft = mix(fg, bg, 0.65);
  const primary = accent;
  const primaryActive = dark ? lighten(accent, 0.10) : darken(accent, 0.12);
  const primaryDisabled = mix(accent, bg, 0.60);
  const onPrimary = bestContrast(accent, [bg, fg]);
  const success = t.success ?? c[10] ?? c[2] ?? '#4a8f3f';
  const warning = t.warning ?? c[11] ?? c[3] ?? '#b8841a';
  // One accent: "teal" is a quiet neutral for info marks and "amber" is the
  // warning colour, mirroring the baked tokens in styles/tokens.css.
  const accentTeal = mix(fg, bg, 0.35);
  const accentAmber = warning;
  const error = t.error ?? c[9] ?? c[1] ?? '#b73d3d';
  // Violet from the ANSI magenta slot, picked the same way teal is.
  const risk = c[5] ?? c[13] ?? accent;

  const softA = dark ? 0.18 : 0.14;
  const softAStrong = dark ? 0.34 : 0.24;
  const tealA = dark ? 0.20 : 0.16;
  const amberA = dark ? 0.20 : 0.18;
  const successA = dark ? 0.20 : 0.16;
  const errorA = dark ? 0.20 : 0.14;
  const riskA = dark ? 0.18 : 0.14;

  // Syntax: the palette's own ANSI hues (magenta keywords, green strings,
  // blue calls, cyan types), falling back to the baked tokens per slot.
  const syn: Record<string, string | null | undefined> = {
    '--syn-keyword': c[5] ?? c[13],
    '--syn-string': c[2] ?? c[10],
    '--syn-comment': mutedSoft,
    '--syn-number': c[13] ?? c[5],
    '--syn-function': c[4] ?? c[12],
    '--syn-type': c[6] ?? c[14],
    '--syn-builtin': c[13] ?? c[5],
    '--syn-operator': body,
    '--syn-punct': mutedSoft,
    '--syn-property': c[4] ?? c[12],
    '--syn-tag': c[5] ?? c[13],
    '--syn-attr': c[6] ?? c[14],
    '--syn-meta': c[13] ?? c[5],
  };
  const synVars = Object.fromEntries(Object.entries(syn).filter((e): e is [string, string] => !!e[1]));

  const vars: Record<string, string> = {
    '--canvas': bg,
    '--surface-soft': surfaceSoft,
    '--surface-card': surfaceCard,
    '--surface-cream-strong': surfaceStrong,
    '--surface-raised': surfaceRaised,
    '--hairline': hairline,
    '--hairline-soft': hairlineSoft,
    '--ink': ink,
    '--body-strong': bodyStrong,
    '--body': body,
    '--muted': muted,
    '--muted-soft': mutedSoft,
    '--primary': primary,
    '--primary-active': primaryActive,
    '--primary-disabled': primaryDisabled,
    '--accent-teal': accentTeal,
    '--accent-amber': accentAmber,
    '--on-primary': onPrimary,
    // The label on an error fill: whichever of bg/fg reads on it (overnight F5b).
    '--on-error': bestContrast(error, [bg, fg]),
    '--success': success,
    '--warning': warning,
    '--error': error,
    '--risk': risk,
    '--primary-soft': withAlpha(accent, softA),
    '--primary-soft-strong': withAlpha(accent, softAStrong),
    '--teal-soft': withAlpha(accentTeal, tealA),
    '--amber-soft': withAlpha(accentAmber, amberA),
    '--success-soft': withAlpha(success, successA),
    '--error-soft': withAlpha(error, errorA),
    '--risk-soft': withAlpha(risk, riskA),
    ...synVars,
  };
  if (clampText) {
    // Keep a single readable polarity even for mid-tone palettes whose raised
    // mixes otherwise straddle the black/white AA boundary. Source stays intact.
    const background = opaqueRgb(bg) ? bg : dark ? '#181719' : '#f7f5f1';
    vars['--canvas'] = background;
    const end = bestContrast(background, ['#000000', '#ffffff']);
    for (const key of TEXT_SURFACES) {
      vars[key] = toward(vars[key], background, color => ratio(color, end) >= 4.5);
    }
    const backs = TEXT_SURFACES.map(key => vars[key]);
    for (const key of CORE_TEXT) {
      vars[key] = toward(vars[key], end, color => backs.every(back => ratio(color, back) >= 4.5));
    }
    vars['--syn-comment'] = vars['--syn-punct'] = vars['--muted-soft'];
    vars['--syn-operator'] = vars['--body'];
  }
  return vars;
}

// ── Color math helpers ────────────────────────────────────────────────

export const CORE_TEXT = ['--ink', '--body-strong', '--body', '--muted', '--muted-soft'] as const;
export const TEXT_SURFACES = ['--canvas', '--surface-soft', '--surface-card', '--surface-cream-strong', '--surface-raised'] as const;
export function opaqueRgb(value: string): Rgb | null {
  if (/^#[0-9a-f]{3}$/i.test(value)) value = '#' + value.slice(1).split('').map(c => c + c).join('');
  if (!/^(#[0-9a-f]{6}|rgb\(\s*(?:\d+(?:\.\d+)?\s*,\s*){2}\d+(?:\.\d+)?\s*\)|rgba\(\s*(?:\d+(?:\.\d+)?\s*,\s*){3}1(?:\.0*)?\s*\))$/i.test(value)) return null;
  const rgb = parseColor(value);
  return rgb?.every(n => n >= 0 && n <= 255) ? rgb : null;
}
const ratio = (a: string, b: string) => contrast(opaqueRgb(a) ?? [0, 0, 0], opaqueRgb(b) ?? [0, 0, 0]);
function toward(from: string, end: string, passes: (color: string) => boolean): string {
  if (!opaqueRgb(from)) return end;
  if (passes(from)) return from;
  for (let step = 1; step <= 100; step++) {
    const value = mix(from, end, step / 100);
    if (passes(value)) return value;
  }
  return end;
}
export function textContrast(vars: Record<string, string>): number | null {
  if (![...CORE_TEXT, ...TEXT_SURFACES].every(key => opaqueRgb(vars[key] ?? ''))) return null;
  return Math.min(...CORE_TEXT.flatMap(key => TEXT_SURFACES.map(surface => ratio(vars[key], vars[surface]))));
}

function parseHex(h: string): [number, number, number] {
  const opaque = opaqueRgb(h);
  if (opaque) return opaque;
  let s = h.trim().replace('#', '');
  if (s.length === 3) s = s.split('').map((ch) => ch + ch).join('');
  const r = parseInt(s.slice(0, 2), 16);
  const g = parseInt(s.slice(2, 4), 16);
  const b = parseInt(s.slice(4, 6), 16);
  return [
    Number.isFinite(r) ? r : 0,
    Number.isFinite(g) ? g : 0,
    Number.isFinite(b) ? b : 0,
  ];
}

function toHex(r: number, g: number, b: number): string {
  const c = (n: number) => Math.max(0, Math.min(255, Math.round(n))).toString(16).padStart(2, '0');
  return `#${c(r)}${c(g)}${c(b)}`;
}

function mix(a: string, b: string, ratio: number): string {
  const [r1, g1, b1] = parseHex(a);
  const [r2, g2, b2] = parseHex(b);
  return toHex(r1 + (r2 - r1) * ratio, g1 + (g2 - g1) * ratio, b1 + (b2 - b1) * ratio);
}

function lighten(c: string, amt: number): string { return mix(c, '#ffffff', amt); }
function darken(c: string, amt: number): string  { return mix(c, '#000000', amt); }

function withAlpha(hex: string, alpha: number): string {
  const [r, g, b] = parseHex(hex);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/// Pick the higher WCAG contrast ratio, not weighted sRGB distance.
function bestContrast(against: string, options: string[]): string {
  let best = options[0];
  let bestDelta = -1;
  for (const o of options) {
    const d = ratio(against, o);
    if (d > bestDelta) { bestDelta = d; best = o; }
  }
  return best;
}
