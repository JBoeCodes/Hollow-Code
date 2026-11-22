import { describe, expect, it } from 'vitest';
import { planRowId, readPlan, WAITING_HEADING, type PlanModel } from './planModel';
import { sha256Hex } from './sectionMarkdown';
import PLAN from '../tests/fixtures/workspace-sections/PLAN.md?raw';

const sourceOf = (text: string) => text.split('\n').map(l => l.replace(/\r$/, ''));

const SHAPES = ['# Title', '## Outcome', '## Current context', '## Decisions and constraints', '## Next steps', '## Constraints', '## Other', '### Sub',
  '---', '', '', '', '**2026-10-05 09:01: dated.** body', '**2026-10-03 night: x', '**Label:** text', '**unclosed lead', '2026-10-01 late: bare',
  '- item', '  - nested', '    - deeper', '- **Bold (2026-10-01, user):** what', '- 2026-10-02, owner: did', '  - Why: reason', '```', '~~~', 'plain prose',
  '- [ ] open row', '  - [x] done child', '\t- [X] tabbed', '1. [ ] numbered', '***', '  indented continuation', '**Next:**', 'text with [a link](x.md)'];
/** Deterministic random documents made of the shapes above (an LCG, so a failure reproduces). */
function* fuzzDocs(count: number) {
  let seed = 0x5eed;
  const rnd = (n: number) => { seed = (Math.imul(seed, 1103515245) + 12345) >>> 0; return seed % n; };
  for (let i = 0; i < count; i++) yield Array.from({ length: 1 + rnd(40) }, () => SHAPES[rnd(SHAPES.length)]).join(rnd(5) ? '\n' : '\r\n');
}

/** No lost text: every non-blank line is the title, a heading, a row or a note, exactly once; notes are exact. */
function expectNothingLost(text: string, p: PlanModel = readPlan(text)) {
  const lines = sourceOf(text);
  const owners = new Map<number, number>();
  const own = (a: number, b: number) => { for (let n = a; n <= b; n++) owners.set(n, (owners.get(n) ?? 0) + 1); };
  if (p.frontmatter) { own(p.frontmatter.line, p.frontmatter.endLine); expect(p.frontmatter.markdown).toBe(lines.slice(0, p.frontmatter.endLine).join('\n')); }
  if (p.titleLine) own(p.titleLine, p.titleLine);
  for (const s of p.sections) {
    if (s.heading !== null) own(s.line, s.line);
    for (const r of s.rows) {
      own(r.id.line, r.endLine);
      expect(r.id.sha256).toBe(sha256Hex(lines[r.id.line - 1]));
    }
    for (const n of s.notes) { own(n.line, n.endLine); expect(n.markdown).toBe(lines.slice(n.line - 1, n.endLine).join('\n')); }
  }
  lines.forEach((l, i) => { if (l.trim()) expect(owners.get(i + 1), `line ${i + 1}: ${l}`).toBe(1); });
}

describe('plan model (Canvas C5a), on the real workspace plan', () => {
  const p = readPlan(PLAN);

  it('counts every checkbox once, per section and in total', () => {
    expect(p.title).toBe('Plan');
    expect(p.counts).toEqual({ done: 16, open: 34, total: 50 });
    expect(p.sections.map(s => [s.heading, s.line, s.counts.done, s.counts.open])).toEqual([
      [null, 1, 0, 0],
      ['HTML Canvas programme (top feature priority, planned 2026-10-04)', 7, 11, 6],
      ['Merged plan from three reviews (2026-10-02)', 31, 0, 3],
      ['Native checks on the installed app', 43, 3, 10],
      ['Live-service checks', 63, 0, 3],
      ['Waiting on the user', 69, 0, 6],
      ['Other open follow-ups', 78, 2, 6],
    ]);
  });

  it('groups rows open and done, in file order', () => {
    const canvas = p.sections[1];
    expect(canvas.done.map(r => r.id.line)).toEqual([13, 14, 15, 16, 17, 18, 21, 22, 23, 24, 25]);
    expect(canvas.open.map(r => r.id.line)).toEqual([19, 20, 26, 27, 28, 29]);
    expect([...canvas.open, ...canvas.done].every(r => r.depth === 0 && r.parent === null)).toBe(true);
    // A second box inside a row's text is part of that row, not another row.
    expect(canvas.open.at(-1)!.text).toBe('C5 panel canvases (Overview, Plan, Brief, then Results, Sources, Activity) · [ ] C6 curated layer + polish');
  });

  it('flags the waiting-on-you section', () => {
    expect(p.waitingOnYou?.heading).toBe('Waiting on the user');
    expect(p.sections.filter(s => s.waitingOnYou)).toHaveLength(1);
    expect(p.waitingOnYou!.open[1].text).toBe('Connect Notion/M365/Superhuman/Wispr/Zoom on claude.ai as `<email>` (<org-account>), then Settings → Connectors → Check again. Notion and M365 also have Hollow\'s own built-in connectors (#241)');
  });

  it('gives each row its exact line identity', () => {
    // From `sed -n 72p PLAN.md | tr -d '\n' | shasum -a 256`.
    expect(p.waitingOnYou!.open[1].id).toEqual({ line: 72, sha256: '96dffb70f48f31c10dbf8c58074fd8f15e5580a699e74afbb331174dd1126f38' });
    expect(p.sections[1].done[0].id).toEqual({ line: 13, sha256: '1851dc1df2e57e23f3c9e03a169d43f3e177df7f5e4aeabecba7efaf391e0d56' });
    expect(planRowId(13, sourceOf(PLAN)[12])).toEqual(p.sections[1].done[0].id);
  });

  it('keeps everything that is not a row as notes, and loses no line', () => {
    expect(p.sections[0].notes.map(n => [n.line, n.endLine])).toEqual([[3, 5]]);
    expect(p.sections[2].notes.map(n => [n.line, n.endLine])).toEqual([[33, 36], [41, 41]]);
    expect(p.sections[2].notes[1].markdown.startsWith('- Finished rows')).toBe(true);
    expectNothingLost(PLAN, p);
  });

  it('reads CRLF like LF, with the same line hashes', () => {
    const crlf = PLAN.replace(/\n/g, '\r\n');
    expect(readPlan(crlf)).toEqual(p);
    expectNothingLost(crlf);
  });
});

describe('plan model (Canvas C5a), other shapes', () => {
  it('counts nested rows once and notes their parent', () => {
    const t = [
      '## Work', '- [x] Parent', '  - [ ] Child', '    - [x] Grandchild', '  - plain bullet', '    - [ ] Under a plain bullet',
      '\t- [X] Tabbed child', '- [ ] Second', '  continued here', '1. [ ] Numbered', '+ [ ] Plus', '- [ ]', '- [x]no space is not a row',
      '', '```', '- [ ] in code', '```', 'Prose resets nesting.', '  - [ ] indented after prose',
    ].join('\n');
    const p = readPlan(t);
    const rows = p.sections[0].rows;
    expect(rows.map(r => [r.id.line, r.text, r.done, r.depth, r.parent?.line ?? null])).toEqual([
      [2, 'Parent', true, 0, null],
      [3, 'Child', false, 1, 2],
      [4, 'Grandchild', true, 2, 3],
      [6, 'Under a plain bullet', false, 2, 2],
      // A tab is four columns, so this sits under the plain bullet, as CommonMark nests it.
      [7, 'Tabbed child', true, 2, 2],
      [8, 'Second continued here', false, 0, null],
      [10, 'Numbered', false, 0, null],
      [11, 'Plus', false, 0, null],
      [12, '', false, 0, null],
      [19, 'indented after prose', false, 0, null],
    ]);
    expect(rows[1].parent).toEqual({ line: 2, text: 'Parent' });
    expect(rows[5].endLine).toBe(9);
    expect(p.counts).toEqual({ done: 3, open: 7, total: 10 });
    expect(p.sections[0].notes.map(n => [n.line, n.endLine])).toEqual([[5, 5], [13, 18]]);
    expectNothingLost(t, p);
  });

  it('handles empty files, no headings and unclosed fences', () => {
    for (const t of ['', '\n', '  \n']) expect(readPlan(t)).toEqual({ frontmatter: null, title: null, titleLine: null, sections: [], counts: { done: 0, open: 0, total: 0 }, waitingOnYou: null });
    const flat = readPlan('- [ ] a\n- [x] b');
    expect(flat.sections).toMatchObject([{ heading: null, line: 1, counts: { done: 1, open: 1, total: 2 } }]);
    const unclosed = '## A\n- [ ] row\n```\n- [ ] code\n## B\n- [ ] still code';
    const u = readPlan(unclosed);
    expect(u.sections.map(s => s.heading)).toEqual(['A']);
    expect(u.counts.total).toBe(1);
    expectNothingLost(unclosed, u);
    const front = readPlan('---\nhollow_plan: 1\n---\n# Plan\n- [ ] a');
    expect(front.frontmatter).toEqual({ line: 1, endLine: 3, markdown: '---\nhollow_plan: 1\n---' });
    expect(front.counts.total).toBe(1);
    const titled = readPlan('# Plan\n\n## Only heading');
    expect(titled.sections).toMatchObject([{ heading: 'Only heading', counts: { total: 0 }, rows: [], notes: [] }]);
  });

  it('detects waiting-on-you headings', () => {
    for (const h of ['Waiting on the user', 'Waiting on you', 'Needs you', 'Blocked on you (decisions)', 'For you to do', 'Waiting for me']) expect(WAITING_HEADING.test(h), h).toBe(true);
    for (const h of ['Waiting', 'Native checks', 'Youth league', 'Live-service checks']) expect(WAITING_HEADING.test(h), h).toBe(false);
  });

  it('stays linear on a very long plan', () => {
    const sections = Array.from({ length: 200 }, (_, s) => [`## S${s}`, ...Array.from({ length: 100 }, (_, i) => `${i % 3 ? '  ' : ''}- [${i % 2 ? 'x' : ' '}] row ${s}.${i}`)].join('\n'));
    const started = Date.now();
    const p = readPlan(sections.join('\n\n'));
    expect(Date.now() - started).toBeLessThan(4000);
    expect(p.counts.total).toBe(20000);
    expect(p.counts.done).toBe(10000);
    expect(p.sections[199].rows[1].parent?.text).toBe('row 199.0');
  });

  it('loses no line on 500 random documents', () => {
    for (const doc of fuzzDocs(500)) expectNothingLost(doc);
  });
});
