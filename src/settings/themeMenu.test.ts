import { describe, expect, it } from 'vitest';
import { CUSTOM_PREFIX, parseThemeChoice, themeMenuOptions, themeMenuValue } from './themeMenu';

describe('the Theme menu', () => {
  it('lists the five built-in choices in order, then the custom themes', () => {
    const options = themeMenuOptions([{ name: 'Sepia', isDark: false }, { name: 'Night', isDark: true }]);
    expect(options.map(o => o.label)).toEqual(['Match macOS', 'Light', 'Dark', 'Glass light', 'Glass dark', 'Sepia', 'Night']);
    expect(options[5]).toMatchObject({ value: `${CUSTOM_PREFIX}Sepia`, group: 'Your themes' });
    expect(themeMenuOptions([])).toHaveLength(5);
  });

  it('names the option in use, custom first, then glass, then match macOS', () => {
    const base = { matchSystem: false, customName: null, glass: false, theme: 'light' as const };
    expect(themeMenuValue(base)).toBe('light');
    expect(themeMenuValue({ ...base, theme: 'dark' })).toBe('dark');
    expect(themeMenuValue({ ...base, matchSystem: true })).toBe('system');
    expect(themeMenuValue({ ...base, glass: true, theme: 'dark' })).toBe('glass-dark');
    expect(themeMenuValue({ ...base, glass: true })).toBe('glass-light');
    expect(themeMenuValue({ ...base, customName: 'Sepia', matchSystem: true })).toBe('custom:Sepia');
  });

  it('parses every value the menu offers and refuses others', () => {
    for (const o of themeMenuOptions([{ name: 'a:b' }])) expect(parseThemeChoice(o.value)).not.toBeNull();
    expect(parseThemeChoice('glass-dark')).toEqual({ kind: 'glass', base: 'dark' });
    expect(parseThemeChoice('custom:a:b')).toEqual({ kind: 'custom', name: 'a:b' });
    expect(parseThemeChoice('custom:')).toBeNull();
    expect(parseThemeChoice('sepia')).toBeNull();
  });
});
