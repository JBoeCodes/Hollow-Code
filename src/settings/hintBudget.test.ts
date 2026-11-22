import { describe, expect, it } from 'vitest';

/**
 * Settings PR 6 (2026-11-12, plan 3.1 rule 4): one line of help per row, 90
 * characters at most, and no section hints (the section's name says it).
 * Longer help sits behind the row's own `<MoreFold>`, and only on risky rows
 * (permission mode, bypass, deletes). A source scan, because the hints are
 * JSX text: it reads `hint=` props, the `settings-row-v` line under a row's
 * name and the `<small>` under a connector or tool row.
 */
const sources = import.meta.glob(['./*.tsx', '../pages/Settings.tsx'], { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

const HINT_MAX = 90;

const decode = (s: string) => s.replace(/&amp;/g, '&').replace(/&nbsp;/g, ' ').replace(/&middot;/g, '·').replace(/\s+/g, ' ').trim();

/** The body of the closed fold a risky row keeps its longer help in is not a row hint. */
const withoutFolds = (src: string) => src.replace(/<MoreFold>[\s\S]*?<\/MoreFold>/g, '');

/** The text of a JSX expression's string literals, one entry each. */
function literalsIn(expr: string): string[] {
  const out: string[] = [];
  for (const m of expr.matchAll(/'((?:[^'\\\n]|\\.)*)'|"((?:[^"\\\n]|\\.)*)"|`((?:[^`\\]|\\.)*)`/g)) {
    const text = (m[1] ?? m[2] ?? m[3] ?? '').replace(/\$\{[^}]*\}/g, '').replace(/\\(.)/g, '$1');
    // A className, an id or a key is not help text: help is words with a space.
    if (/\s/.test(text.trim())) out.push(decode(text));
  }
  return out;
}

/** The balanced `{...}` starting at `open` (the index of the brace). */
function balanced(src: string, open: number): string {
  let depth = 0;
  for (let i = open; i < src.length; i++) {
    if (src[i] === '{') depth++;
    else if (src[i] === '}' && --depth === 0) return src.slice(open + 1, i);
  }
  return src.slice(open + 1);
}

/** Every `hint="..."` and `hint={...}` prop in a file. */
function hintProps(src: string): string[] {
  const out: string[] = [];
  const body = withoutFolds(src);
  for (const m of body.matchAll(/\bhint=(?:"([^"]*)"|\{)/g)) {
    if (m[1] !== undefined) out.push(decode(m[1]));
    else out.push(...literalsIn(balanced(body, m.index! + m[0].length - 1)));
  }
  return out;
}

/** The text of a `<tag class="...cls...">...</tag>` element, with nested tags of that name balanced. */
function elementTexts(src: string, tag: string, cls: string): string[] {
  const out: string[] = [];
  const open = new RegExp(`<${tag}\\b[^>]*\\bclass=(?:"[^"]*\\b${cls}\\b[^"]*"|\\{[^}]*${cls}[^}]*\\})[^>]*>`, 'g');
  for (const m of src.matchAll(open)) {
    let depth = 1;
    const re = new RegExp(`<${tag}\\b|</${tag}>`, 'g');
    re.lastIndex = m.index! + m[0].length;
    let end = src.length;
    for (let t = re.exec(src); t; t = re.exec(src)) {
      depth += t[0].startsWith('</') ? -1 : 1;
      if (depth === 0) { end = t.index; break; }
    }
    out.push(src.slice(m.index! + m[0].length, end));
  }
  return out;
}

/** The words a row's help line shows: its own text, then each string a `{...}` expression can show. */
function rowHelp(inner: string): string[] {
  const parts: string[] = [];
  let text = '';
  for (let i = 0; i < inner.length; i++) {
    if (inner[i] === '{') {
      const expr = balanced(inner, i);
      parts.push(...literalsIn(expr));
      i += expr.length + 1;
    } else text += inner[i];
  }
  const own = decode(text.replace(/<[^>]*>/g, ''));
  return own ? [own, ...parts] : parts;
}

function hintStrings(file: string, src: string): { where: string; text: string }[] {
  const body = withoutFolds(src);
  const found: { where: string; text: string }[] = [];
  for (const text of hintProps(src)) found.push({ where: `${file} hint`, text });
  for (const inner of elementTexts(body, 'div', 'settings-row-v')) for (const text of rowHelp(inner)) found.push({ where: `${file} row`, text });
  for (const m of body.matchAll(/<(?:LeadText|PageLinkRow)\b[^>]*?\btext="([^"]*)"/g)) found.push({ where: `${file} text`, text: decode(m[1]) });
  for (const inner of elementTexts(body, 'p', 'settings-section-hint')) for (const text of rowHelp(inner)) found.push({ where: `${file} line`, text });
  for (const inner of elementTexts(body, 'div', 'cx-row-text')) {
    for (const m of inner.matchAll(/<small>([\s\S]*?)<\/small>/g)) for (const text of rowHelp(m[1])) found.push({ where: `${file} small`, text });
  }
  return found.filter(f => f.text);
}

describe('Settings hints are one line (plan 3.1 rule 4)', () => {
  it('scans the Settings sources', () => {
    expect(Object.keys(sources).length).toBeGreaterThan(30);
    expect(Object.keys(sources)).toContain('../pages/Settings.tsx');
    const all = Object.entries(sources).flatMap(([file, src]) => hintStrings(file, src));
    expect(all.length).toBeGreaterThan(100);
  });

  it('keeps every hint and row help line to 90 characters, longer help behind the row’s own More', () => {
    const long = Object.entries(sources).flatMap(([file, src]) => hintStrings(file, src))
      .filter(f => f.text.length > HINT_MAX)
      .map(f => `${f.where} (${f.text.length}): ${f.text}`);
    expect(long).toEqual([]);
  });

  it('gives no section a hint: the section name says it', () => {
    const page = sources['../pages/Settings.tsx'];
    const sections = [...page.matchAll(/<SettingsSection\b([\s\S]*?)\n\s{8}>/g)].map(m => m[1]);
    expect(sections.length).toBeGreaterThanOrEqual(12);
    for (const attrs of sections) expect(attrs, 'a SettingsSection with a hint').not.toMatch(/\n\s+hint=/);
    const layout = sources['./settingsLayout.tsx'];
    const props = layout.slice(layout.indexOf('function SettingsSection('), layout.indexOf('}) {', layout.indexOf('function SettingsSection(')));
    expect(props).not.toMatch(/\bhint\b/);
  });

  it('does not count a closed More fold against a risky row', () => {
    const src = '<div class="settings-row-v">Short line.</div><MoreFold><p class="settings-row-v">' + 'long '.repeat(40) + '</p></MoreFold>';
    expect(hintStrings('x', src).map(f => f.text)).toEqual(['Short line.']);
  });

  it('reads JSX text, expressions and hint props', () => {
    const src = `<LeadText text="Lead." /><SettingsSubhead title="T" hint="One two." /><div class="settings-row-v">A &amp; B <Show when={x}>{'Then C'}</Show></div>`;
    expect(hintStrings('x', src).map(f => f.text)).toEqual(['One two.', 'A & B', 'Then C', 'Lead.']);
  });
});
