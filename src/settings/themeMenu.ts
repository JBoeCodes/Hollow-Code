// The General → Theme menu (Settings PR 5, plan 3.2): one row, one menu.
// Match macOS, Light, Dark, Glass light, Glass dark, then the person's own
// themes. Pure: AppearanceRows maps a choice onto `setActiveTheme`.
import type { SelectOption } from '../selectModel';

export const CUSTOM_PREFIX = 'custom:';

export type ThemeChoice =
  | { kind: 'system' }
  | { kind: 'stock'; mode: 'light' | 'dark' }
  | { kind: 'glass'; base: 'light' | 'dark' }
  | { kind: 'custom'; name: string };

const STOCK: SelectOption[] = [
  { value: 'system', label: 'Match macOS', detail: 'Follows the system’s light or dark appearance' },
  { value: 'light', label: 'Light', detail: 'Alabaster: white, crimson accent' },
  { value: 'dark', label: 'Dark', detail: 'Obsidian: charcoal, soft red' },
  { value: 'glass-light', label: 'Glass light', detail: 'Frosted, over your desktop' },
  { value: 'glass-dark', label: 'Glass dark', detail: 'Frosted, over your desktop' },
];

/** The menu's options: the five built-in choices, then each custom theme by name. */
export function themeMenuOptions(customs: readonly { name: string; isDark?: boolean }[]): SelectOption[] {
  return [
    ...STOCK,
    ...customs.map(t => ({ value: CUSTOM_PREFIX + t.name, label: t.name, detail: `Yours, ${t.isDark ? 'dark' : 'light'}`, group: 'Your themes' })),
  ];
}

/** Which option is in use now. */
export function themeMenuValue(s: { matchSystem: boolean; customName: string | null; glass: boolean; theme: 'light' | 'dark' }): string {
  if (s.customName) return CUSTOM_PREFIX + s.customName;
  if (s.glass) return s.theme === 'dark' ? 'glass-dark' : 'glass-light';
  if (s.matchSystem) return 'system';
  return s.theme;
}

/** What a menu value asks for; null for a value the menu never offered. */
export function parseThemeChoice(value: string): ThemeChoice | null {
  if (value === 'system') return { kind: 'system' };
  if (value === 'light' || value === 'dark') return { kind: 'stock', mode: value };
  if (value === 'glass-light') return { kind: 'glass', base: 'light' };
  if (value === 'glass-dark') return { kind: 'glass', base: 'dark' };
  if (value.startsWith(CUSTOM_PREFIX) && value.length > CUSTOM_PREFIX.length) return { kind: 'custom', name: value.slice(CUSTOM_PREFIX.length) };
  return null;
}
