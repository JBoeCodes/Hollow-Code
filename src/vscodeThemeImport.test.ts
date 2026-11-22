import { describe, expect, it } from 'vitest';
import { opaqueHex, parseJsonc, vscodeThemeToSource } from './vscodeThemeImport';
import { draftContrast } from './settings/themeContrast';

const THEME = `{
  // a comment
  "name": "Night Owl-ish", /* block */
  "type": "dark",
  "colors": {
    "editor.background": "#011627",
    "editor.foreground": "#d6deebff",
    "focusBorder": "#5f7e97",
    "editor.selectionBackground": "#1d3b53cc",
    "editorCursor.foreground": "#80a4c2",
    "terminal.ansiBlack": "#011627",
    "terminal.ansiRed": "#ef5350",
    "terminal.ansiGreen": "#22da6e",
    "terminal.ansiBlue": "#82aaff",
    "terminal.ansiBrightWhite": "#ffffff",
    "editorError.foreground": "#ef5350",
  },
  "tokenColors": [],
}`;

describe('VS Code theme import', () => {
  it('reads JSON with comments and trailing commas, and keeps // inside strings', () => {
    expect(parseJsonc('{ "a": "http://x", /* c */ "b": [1,2,], // e\n }')).toEqual({ a: 'http://x', b: [1, 2] });
  });
  it('lays alpha colours over the background', () => {
    expect(opaqueHex('#ffffff80', [0, 0, 0])).toBe('#808080');
    expect(opaqueHex('#fff')).toBe('#ffffff');
    expect(opaqueHex('red')).toBeNull();
    expect(opaqueHex(5)).toBeNull();
  });
  it('writes Hollow theme source from the colours Hollow has slots for', () => {
    const r = vscodeThemeToSource(THEME, 'night.json');
    expect(r.name).toBe('Night Owl-ish');
    expect(r.source).toContain('name = "Night Owl-ish"');
    expect(r.source).toContain('is_dark = true');
    expect(r.source).toContain('background = "#011627"');
    expect(r.source).toContain('foreground = "#d6deeb"');
    expect(r.source).toContain('accent     = "#5f7e97"');
    expect(r.source).toContain('color1  = "#ef5350"');
    expect(r.source).toContain('color15 = "#ffffff"');
    expect(r.source).toContain('error   = "#ef5350"');
    expect(r.mapped).toBe(5);
    expect(r.notes.join(' ')).toMatch(/5 of the 16/);
  });
  it('the result is a theme the editor can measure', () => {
    expect(draftContrast(vscodeThemeToSource(THEME).source)).not.toBeNull();
  });
  it('says plainly when it cannot', () => {
    expect(() => vscodeThemeToSource('not json')).toThrow(/not valid JSON/);
    expect(() => vscodeThemeToSource('{"name":"x"}')).toThrow(/no "colors"/);
    expect(() => vscodeThemeToSource('{"colors":{"focusBorder":"#fff"}}')).toThrow(/background and foreground/);
  });
  it('falls back for the accent and the polarity, and names an unnamed theme from its file', () => {
    const r = vscodeThemeToSource('{"colors":{"editor.background":"#ffffff","editor.foreground":"#222222"}}', 'Paper.json');
    expect(r.name).toBe('Paper');
    expect(r.source).toContain('is_dark = false');
    expect(r.source).toContain('accent     = "#222222"');
    expect(r.notes.join(' ')).toMatch(/No accent colour/);
  });
  it('never lets a name break the quoted line', () => {
    expect(vscodeThemeToSource('{"name":"a\\"b\\\\c","colors":{"editor.background":"#000","editor.foreground":"#fff"}}').source).toContain('name = "abc"');
  });
});
