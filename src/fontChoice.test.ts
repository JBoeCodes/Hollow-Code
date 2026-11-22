import { describe, expect, it } from 'vitest';
import { CUSTOM_PREFIX, DEFAULT_STACK, FONT_OPTIONS, cleanFontName, fontStack, normalizeFontChoice } from './fontChoice';
import tokens from './styles/tokens.css?raw';

describe('font choice', () => {
  it('the default stacks are the ones tokens.css writes', () => {
    expect(tokens).toContain(`--ui:      ${DEFAULT_STACK.ui};`);
    expect(tokens).toContain(`--mono:    ${DEFAULT_STACK.code};`);
  });
  it('default means no override; a named option is its stack', () => {
    expect(fontStack('ui', 'default')).toBeNull();
    expect(fontStack('ui', 'serif')).toContain('Georgia');
    expect(fontStack('code', 'menlo')).toContain('Menlo');
  });
  it('a typed family keeps Hollow’s default stack as its fallback', () => {
    expect(fontStack('code', `${CUSTOM_PREFIX}Fira Code`)).toBe(`'Fira Code', ${DEFAULT_STACK.code}`);
  });
  it('refuses anything that could break out of a font-family value', () => {
    for (const bad of ["a'b", 'a;b', 'url(x)', 'a{b}', '', '   ', 'x'.repeat(61), 'a,b', 'a"b']) expect(cleanFontName(bad), bad).toBeNull();
    expect(cleanFontName('  Fira   Code ')).toBe('Fira Code');
    expect(cleanFontName('IBM Plex Sans 2.0')).toBe('IBM Plex Sans 2.0');
  });
  it('a saved value that names nothing falls back to the default', () => {
    expect(normalizeFontChoice('ui', 'comic')).toBe('default');
    expect(normalizeFontChoice('ui', "custom:evil';")).toBe('default');
    expect(normalizeFontChoice('code', null)).toBe('default');
    expect(normalizeFontChoice('code', 'sf-mono')).toBe('sf-mono');
    expect(normalizeFontChoice('ui', 'sf-mono')).toBe('default');
  });
  it('every kind lists the default first and has unique values', () => {
    for (const kind of ['ui', 'code'] as const) {
      expect(FONT_OPTIONS[kind][0].value).toBe('default');
      expect(new Set(FONT_OPTIONS[kind].map(o => o.value)).size).toBe(FONT_OPTIONS[kind].length);
    }
  });
});
