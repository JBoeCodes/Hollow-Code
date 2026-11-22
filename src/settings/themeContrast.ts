import type { CustomTheme } from '../types';
import { CORE_TEXT, TEXT_SURFACES, deriveCustomVars, opaqueRgb, textContrast } from '../customThemeVars';

/** Listed palette metadata only: no read or selection. Use the app's existing
 * text clamp so a low-contrast custom palette stays readable in its sample. */
export function previewPalette(swatch: { bg: string; fg: string; accent: string }, isDark: boolean) {
  return deriveCustomVars({
    name: '', source: '', isDark, colors: Array(16).fill(null),
    background: swatch.bg, foreground: swatch.fg, accent: swatch.accent,
    cursor: null, selectionForeground: null, selectionBackground: null,
    success: null, warning: null, error: null,
  });
}

/** Advisory only: never writes or applies the editor draft. Rust owns Save.
 * Unsupported syntax/colours remain explicitly unmeasured. */
export function draftContrast(source: string): { ratio: number; adjusted: boolean } | null {
  const fields: Record<string, string> = {};
  for (const raw of source.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith('#') || line.startsWith('//')) continue;
    const entry = /^([\w]+)\s*(?:=\s*|\s+)(.*)$/.exec(line);
    if (!entry) return null;
    const value = /^(?:"([^"]*)"|'([^']*)'|([^\s]+))(?:\s+(?:#|\/\/).*)?$/.exec(entry[2].trim());
    if (!value) return null;
    fields[entry[1].toLowerCase()] = value[1] ?? value[2] ?? value[3];
  }
  for (const key of ['accent', 'foreground', 'background']) if (!opaqueRgb(fields[key] ?? '')) return null;
  // These are the documented hex forms. Other CSS colours remain unmeasured
  // here rather than approximating Rust's background-based polarity inference.
  if (!/^#[0-9a-f]{3}(?:[0-9a-f]{3})?$/i.test(fields.background)) return null;
  const bg = opaqueRgb(fields.background)!;
  const explicit = fields.is_dark?.toLowerCase();
  const isDark = ['true', '1', 'yes'].includes(explicit) ? true : ['false', '0', 'no'].includes(explicit) ? false
    : (0.299 * bg[0] + 0.587 * bg[1] + 0.114 * bg[2]) / 255 < 0.5;
  const colors = Array.from({ length: 16 }, (_, i) => fields[`color${i}`] ?? null);
  if (colors.some(color => color !== null && !opaqueRgb(color))) return null;
  const theme: CustomTheme = {
    name: '', source, isDark, colors, accent: fields.accent, foreground: fields.foreground, background: fields.background,
    cursor: null, selectionForeground: null, selectionBackground: null, success: null, warning: null, error: null,
  };
  const raw = deriveCustomVars(theme, false), derived = deriveCustomVars(theme);
  const ratio = textContrast(derived);
  return ratio === null ? null : { ratio, adjusted: [...CORE_TEXT, ...TEXT_SURFACES].some(key => raw[key] !== derived[key]) };
}
