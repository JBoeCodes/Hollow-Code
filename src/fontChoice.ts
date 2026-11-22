/**
 * Font choice (t3code parity 4.7, A15): the interface font and the code font
 * are two settings. Each is Hollow's own default, a system stack, or a font
 * family name the person typed (used only if installed; the default stack
 * follows it as the fallback). Pure, so a test can hold the stacks and the
 * name rules; store/fonts.ts applies them.
 */
export type FontKind = 'ui' | 'code';
export interface FontOption { value: string; label: string; stack: string | null }

/** Hollow's own stacks, as styles/tokens.css writes them (`--ui`, `--mono`). */
export const DEFAULT_STACK: Record<FontKind, string> = {
  ui: "'Inter Variable', 'Inter', -apple-system, BlinkMacSystemFont, system-ui, sans-serif",
  code: "'JetBrains Mono', ui-monospace, 'SF Mono', Menlo, Consolas, monospace",
};

export const FONT_OPTIONS: Record<FontKind, readonly FontOption[]> = {
  ui: [
    { value: 'default', label: 'Hollow default', stack: null },
    { value: 'system', label: 'System', stack: "-apple-system, BlinkMacSystemFont, system-ui, sans-serif" },
    { value: 'serif', label: 'Serif', stack: "'New York', 'Iowan Old Style', Georgia, serif" },
    { value: 'rounded', label: 'Rounded', stack: "ui-rounded, 'SF Pro Rounded', system-ui, sans-serif" },
  ],
  code: [
    { value: 'default', label: 'Hollow default', stack: null },
    { value: 'sf-mono', label: 'SF Mono', stack: "ui-monospace, 'SF Mono', Menlo, monospace" },
    { value: 'menlo', label: 'Menlo', stack: "Menlo, Monaco, monospace" },
    { value: 'courier', label: 'Courier', stack: "'Courier New', Courier, monospace" },
  ],
};

export const CUSTOM_PREFIX = 'custom:';

/** A family name worth trusting: letters, digits, spaces, dots and hyphens, 1 to 60 characters. */
export function cleanFontName(raw: string): string | null {
  const name = raw.replace(/\s+/g, ' ').trim();
  return name.length >= 1 && name.length <= 60 && /^[\p{L}\p{N} .-]+$/u.test(name) ? name : null;
}

/** The saved choice, or `default` when it names nothing known. */
export function normalizeFontChoice(kind: FontKind, raw: string | null): string {
  if (!raw) return 'default';
  if (raw.startsWith(CUSTOM_PREFIX)) { const n = cleanFontName(raw.slice(CUSTOM_PREFIX.length)); return n ? CUSTOM_PREFIX + n : 'default'; }
  return FONT_OPTIONS[kind].some(o => o.value === raw) ? raw : 'default';
}

/** The CSS font stack a choice means, or null for Hollow's default (no override). */
export function fontStack(kind: FontKind, choice: string): string | null {
  const c = normalizeFontChoice(kind, choice);
  if (c.startsWith(CUSTOM_PREFIX)) return `'${c.slice(CUSTOM_PREFIX.length)}', ${DEFAULT_STACK[kind]}`;
  return FONT_OPTIONS[kind].find(o => o.value === c)?.stack ?? null;
}
