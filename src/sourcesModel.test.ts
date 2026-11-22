import { describe, expect, it } from 'vitest';
import { formatBytes, originKind, readSourcesManifest, shortSha, sourceCardsModel, type SourcesManifest } from './sourcesModel';
import type { SourceContext } from './workspaceTypes';
import SOURCES from '../tests/fixtures/workspace-sections/SOURCES.md?raw';

const SHA = '572af837e0933f49978d011f18eefaa15321c52125525eff74d56eca747a4d1e';
const lineOf = (text: string) => text.split('\n').map(l => l.replace(/\r$/, ''));
const src = (path: string, over: Partial<SourceContext> = {}): SourceContext => ({ path, original: null, captured: null, freshness: 'unknown', coverage: 'complete', includedBytes: 10, totalBytes: 10, ...over });

/** No lost text: every non-blank line is the frontmatter, the title, the preamble, an entry's heading, field or note, or another section, exactly once; spans are exact. */
function expectNothingLost(text: string, m: SourcesManifest = readSourcesManifest(text)) {
  const lines = lineOf(text);
  const owners = new Map<number, number>();
  const own = (a: number, b: number, markdown?: string) => {
    for (let n = a; n <= b; n++) owners.set(n, (owners.get(n) ?? 0) + 1);
    if (markdown !== undefined) expect(markdown).toBe(lines.slice(a - 1, b).join('\n'));
  };
  if (m.frontmatter) own(m.frontmatter.line, m.frontmatter.endLine, m.frontmatter.markdown);
  if (m.titleLine) own(m.titleLine, m.titleLine);
  for (const p of m.preamble) own(p.line, p.endLine, p.markdown);
  for (const o of m.other) own(o.line, o.endLine, o.markdown);
  for (const e of m.entries) {
    expect(e.markdown).toBe(lines.slice(e.line - 1, e.endLine).join('\n'));
    if (e.style === 'item') { own(e.line, e.endLine); continue; }
    own(e.line, e.line);
    for (const f of e.fields) own(f.line, f.endLine, f.markdown);
    for (const n of e.notes) own(n.line, n.endLine, n.markdown);
  }
  lines.forEach((l, i) => { if (l.trim()) expect(owners.get(i + 1), `line ${i + 1}: ${l}`).toBe(1); });
}

const SHAPES = ['# Sources', '## a.md', '## `b c.csv`', '## a.md', '# Other', '### Sub', '---', '', '', '', 'Prose line.', '- Origin: Notion page "X"',
  '- **SHA-256:** `' + SHA + '`', '- SHA-256: not-a-hash', '- Snapshot: [a.md](sources/a.md)', '- Original: "/Users/x/a.md"', '- Captured: 2026-10-02T12:00:00Z',
  '- Contents: two parts', '  - nested part', '    wrapped', '- Not in the snapshot: the rest', '- report.csv · finance export', '- `notes.md` — ops', '- plain bullet',
  '```', '~~~', 'Origin: plain line', '**Origin**: bold after', '1. numbered', '\t- tabbed', '***', '- https://example.com/x.html'];
function* fuzzDocs(count: number) {
  let seed = 0x50c5;
  const rnd = (n: number) => { seed = (Math.imul(seed, 1103515245) + 12345) >>> 0; return seed % n; };
  for (let i = 0; i < count; i++) yield Array.from({ length: 1 + rnd(40) }, () => SHAPES[rnd(SHAPES.length)]).join(rnd(5) ? '\n' : '\r\n');
}

describe('SOURCES.md model (Canvas C5b), on the real workspace file', () => {
  const m = readSourcesManifest(SOURCES);

  it('reads the title, the preamble and the one entry with its fields', () => {
    expect(m.title).toBe('Sources');
    expect(m.titleLine).toBe(1);
    expect(m.preamble.map(p => p.markdown)).toEqual(['Sources attached through Hollow are local snapshots. Originals stay untouched. Each entry records its origin and checksum.']);
    expect(m.entries).toHaveLength(1);
    const e = m.entries[0];
    expect(e).toMatchObject({ name: 'notion-hollow-next-steps-2026-10-02.md', style: 'heading', line: 5, endLine: 10, sha256: SHA, snapshot: null, original: null, captured: null });
    expect(e.origin).toBe('Notion page "Hollow next steps" (`3edb2372-5296-8124-9488-f3a8061ff913`), fetched through Hollow\'s built-in Notion connector on 2026-10-02.');
    expect(e.contents).toMatch(/^review A \(six-lane review, S\/D\/R findings\) and review B .*The user's chat paste has the full text\.$/);
    expect(e.fields.map(f => [f.key, f.label, f.line])).toEqual([['origin', 'Origin', 7], ['contents', 'Contents', 8], ['sha256', 'SHA-256', 10]]);
  });

  it('keeps the unrecognised "Not in the snapshot" line as a note, exactly', () => {
    expect(m.entries[0].notes).toEqual([{ line: 9, endLine: 9, markdown: '- Not in the snapshot: the third pasted review ("Grok plan"), which matches `outputs/adversarial-review-2026-10-02.md`.' }]);
    expectNothingLost(SOURCES, m);
  });

  it('joins with the listing: one card, origin a connector in plain text, excerpt coverage, untracked freshness, the recorded checksum', () => {
    const v = sourceCardsModel(m, [src('sources/notion-hollow-next-steps-2026-10-02.md', { coverage: 'excerpt', includedBytes: 8000, totalBytes: 60229 })]);
    expect(v.unmatched).toEqual([]);
    const c = v.cards[0];
    expect(c).toMatchObject({ name: 'notion-hollow-next-steps-2026-10-02.md', folder: null, sizeBytes: 60229, coverage: 'excerpt', coverageLabel: 'Sent when relevant', freshness: 'unknown', freshnessLabel: 'Origin not tracked', freshnessTone: 'unknown', sha256: SHA, captured: null, originalPath: null, earlier: [] });
    expect(c.coverageDetail).toBe('The passages that best match each request are sent (24 KB of source text per turn); agents can read the rest');
    expect(c.origin).toEqual({ kind: 'connector', text: 'Notion page "Hollow next steps" (3edb2372-5296-8124-9488-f3a8061ff913), fetched through Hollow\'s built-in Notion connector on 2026-10-02.' });
    expect(c.entry).toBe(m.entries[0]);
    expect(v.counts).toEqual({ all: 1, inContext: 1, excerpt: 1, notRead: 0, changed: 0, untracked: 1 });
  });

  it('says a small attached file is sent whole when it matches the request, and nothing about a first slice', () => {
    const v = sourceCardsModel(null, [src('sources/note.md', { coverage: 'excerpt', includedBytes: 900, totalBytes: 900 }), src('sources/big.md', { coverage: 'budget_exceeded', includedBytes: 0, totalBytes: 90_000 })]);
    expect(v.cards[0].coverageDetail).toBe('Sent whole when it matches the request (24 KB of source text per turn)');
    expect(v.cards[1].coverageDetail).toMatch(/^Not sent: its matching passages didn’t fit/);
    expect(v.cards.map(c => c.coverageDetail).join(' ')).not.toMatch(/The first|8 KB/);
  });
});

describe('SOURCES.md model: what Hollow\'s attach writes', () => {
  // workspaces.rs attach_source's record, twice (a re-attach after the first file was removed appends a second).
  const record = (name: string, enc: string, original: string, at: string, sha: string) => `\n## ${name}\n\n- Snapshot: [${name}](sources/${enc})\n- Original: ${JSON.stringify(original)}\n- Captured: ${at}\n- SHA-256: \`${sha}\`\n`;
  const text = '# Sources\n\nSources attached through Hollow are local snapshots. Originals stay untouched. Each entry records its origin and checksum.\n'
    + record('Q3 plan  v2 .xlsx', 'Q3%20plan%20%5Bv2%5D.xlsx', '/Users/x/Q3 plan [v2].xlsx', '2026-10-01T09:00:00+00:00', 'a'.repeat(64))
    + record('notes.md', 'notes.md', '/Users/x/old/notes.md', '2026-09-01T09:00:00+00:00', 'b'.repeat(64))
    + record('notes.md', 'notes.md', '/Users/x/notes.md', '2026-10-02T09:00:00+00:00', 'C'.repeat(64));
  const m = readSourcesManifest(text);

  it('reads every field and decodes the snapshot link and the quoted original', () => {
    expect(m.entries.map(e => e.name)).toEqual(['Q3 plan v2 .xlsx', 'notes.md', 'notes.md']);
    expect(m.entries[0]).toMatchObject({ snapshot: 'sources/Q3 plan [v2].xlsx', original: '/Users/x/Q3 plan [v2].xlsx', captured: '2026-10-01T09:00:00+00:00', sha256: 'a'.repeat(64), notes: [] });
    expect(m.entries[2].sha256).toBe('c'.repeat(64));
    expectNothingLost(text, m);
  });

  it('claims by the snapshot link (a bracketed name), shows the newest of duplicate entries and keeps the older', () => {
    const v = sourceCardsModel(m, [
      src('sources/Q3 plan [v2].xlsx', { coverage: 'requires_reader', includedBytes: 0, totalBytes: 2_400_000, original: '/Users/x/Q3 plan [v2].xlsx', captured: '2026-10-01T09:00:01+00:00', freshness: 'changed' }),
      src('sources/notes.md', { original: '/Users/x/notes.md', captured: '2026-10-02T09:00:00+00:00', freshness: 'unchanged_metadata' }),
    ]);
    const [xlsx, notes] = v.cards;
    expect(xlsx.entry).toBe(m.entries[0]);
    expect(xlsx).toMatchObject({ coverageLabel: 'Needs a file reader', freshnessLabel: 'Original changed since capture', freshnessTone: 'warn', origin: { kind: 'local', text: '/Users/x/Q3 plan [v2].xlsx' }, originalPath: null, captured: { value: '2026-10-01T09:00:01+00:00', from: 'provenance' } });
    expect(xlsx.coverageDetail).toMatch(/^Not read as text/);
    expect(notes.entry).toBe(m.entries[2]);
    expect(notes.earlier).toEqual([m.entries[1]]);
    expect(notes).toMatchObject({ sha256: 'c'.repeat(64), freshnessLabel: 'Original size and date unchanged', freshnessDetail: 'Compared by size and modified time, not content', freshnessTone: 'ok' });
    expect(v.unmatched).toEqual([]);
    expect(v.counts).toMatchObject({ all: 2, inContext: 1, notRead: 1, changed: 1, untracked: 0 });
  });

  it('never claims fresh without evidence: no provenance reads "Origin not tracked", a missing original "Original unavailable"', () => {
    const v = sourceCardsModel(m, [src('sources/notes.md'), src('sources/gone.md', { freshness: 'unavailable', original: '/x/gone.md' })]);
    expect(v.cards.map(c => [c.freshnessLabel, c.freshnessTone])).toEqual([['Origin not tracked', 'unknown'], ['Original unavailable', 'warn']]);
    for (const c of v.cards) expect(`${c.freshnessLabel} ${c.freshnessDetail}`).not.toMatch(/\bfresh\b|up to date|current/i);
  });

  it('takes the captured date from the entry only when the listing has none', () => {
    const v = sourceCardsModel(m, [src('sources/notes.md')]);
    expect(v.cards[0].captured).toEqual({ value: '2026-10-02T09:00:00+00:00', from: 'manifest' });
    expect(v.cards[0].origin).toEqual({ kind: 'local', text: '/Users/x/notes.md' });
  });
});

describe('SOURCES.md model: tolerance and edge cases', () => {
  it('reads missing fields as null and an empty file as nothing', () => {
    const m = readSourcesManifest('## lonely.md\n');
    expect(m.entries[0]).toMatchObject({ name: 'lonely.md', origin: null, captured: null, sha256: null, contents: null, fields: [], notes: [] });
    expect(readSourcesManifest('')).toEqual({ frontmatter: null, title: null, titleLine: null, preamble: [], entries: [], other: [] });
    const v = sourceCardsModel(null, [src('sources/a.md')]);
    expect(v.cards[0]).toMatchObject({ entry: null, origin: null, sha256: null, captured: null });
  });

  it('reads CRLF exactly as LF, and a BOM', () => {
    const lf = readSourcesManifest(SOURCES);
    const crlf = '\uFEFF' + SOURCES.replace(/\n/g, '\r\n');
    expect(readSourcesManifest(crlf)).toEqual(lf);
    expectNothingLost(SOURCES.replace(/\n/g, '\r\n'));
  });

  it('accepts bold labels, plain lines, other label names and backticked headings', () => {
    const text = [
      '## `report.csv`', '', '**Origin**: https://example.com/export?id=1', '**Fetched:** 2026-09-29', 'Checksum: ' + SHA.toUpperCase(),
      '- Summary: monthly numbers', '  - by region', '  - by month', '', '  with a second paragraph', '', 'Some prose about it.',
    ].join('\n');
    const m = readSourcesManifest(text);
    const e = m.entries[0];
    expect(e.name).toBe('report.csv');
    expect(e.origin).toBe('https://example.com/export?id=1');
    expect(e.captured).toBe('2026-09-29');
    expect(e.sha256).toBe(SHA);
    expect(e.contents).toBe('monthly numbers\n- by region\n- by month\nwith a second paragraph');
    expect(e.notes.map(n => n.markdown)).toEqual(['Some prose about it.']);
    expectNothingLost(text, m);
    const c = sourceCardsModel(m, [src('sources/report.csv')]).cards[0];
    expect(c.origin).toEqual({ kind: 'url', text: 'https://example.com/export?id=1' });
  });

  it('keeps a bad checksum, a second field of the same kind and fenced lookalikes as notes', () => {
    const text = '## a.md\n\n- SHA-256: abc123\n- Origin: one\n- Origin: two\n\n```\nSHA-256: ' + SHA + '\n```\n';
    const e = readSourcesManifest(text).entries[0];
    expect(e.sha256).toBeNull();
    expect(e.origin).toBe('one');
    expect(e.notes.map(n => n.markdown)).toEqual(['- SHA-256: abc123', '- Origin: two\n\n```\nSHA-256: ' + SHA + '\n```']);
    expectNothingLost(text);
  });

  it('reads file-name bullets before any entry as entries, and leaves other bullets as preamble', () => {
    // The demo workspace's SOURCES.md shape.
    const text = '# Sources\n\n- regional-pnl-q3.csv · finance export, captured 2026-09-29\n- `staffing-overtime.md` — operations notes\n- [Board pack](sources/sub/pack.pdf)\n- v0.5.10 shipped\n- see https://example.com/a.html\n- plain bullet\n';
    const m = readSourcesManifest(text);
    expect(m.entries.map(e => [e.name, e.style, e.detail, e.snapshot])).toEqual([
      ['regional-pnl-q3.csv', 'item', 'finance export, captured 2026-09-29', null],
      ['staffing-overtime.md', 'item', 'operations notes', null],
      ['Board pack', 'item', null, 'sources/sub/pack.pdf'],
    ]);
    expect(m.preamble.map(p => p.markdown)).toEqual(['- v0.5.10 shipped\n- see https://example.com/a.html\n- plain bullet']);
    expectNothingLost(text, m);
    const v = sourceCardsModel(m, [src('sources/regional-pnl-q3.csv'), src('sources/staffing-overtime.md'), src('sources/sub/pack.pdf')]);
    expect(v.cards.map(c => c.entry?.name)).toEqual(['regional-pnl-q3.csv', 'staffing-overtime.md', 'Board pack']);
    expect(v.cards[2]).toMatchObject({ name: 'pack.pdf', folder: 'sub' });
  });

  it('matches a nested file by its path under sources/, and by its bare name only when that is unique', () => {
    const m = readSourcesManifest('## sub/a.md\n\n## b.md\n\n## c.md\n');
    const v = sourceCardsModel(m, [src('sources/sub/a.md'), src('sources/x/b.md'), src('sources/y/b.md'), src('sources/z/c.md')]);
    expect(v.cards.map(c => c.entry?.name ?? null)).toEqual(['sub/a.md', null, null, 'c.md']);
    expect(v.unmatched.map(e => e.name)).toEqual(['b.md']);
  });

  it('keeps entries no listed file claims, in file order', () => {
    const m = readSourcesManifest('## gone.md\n\n- Origin: deleted later\n\n## kept.md\n');
    const v = sourceCardsModel(m, [src('sources/kept.md')], 3);
    expect(v.unmatched.map(e => e.name)).toEqual(['gone.md']);
    expect(v.omitted).toBe(3);
  });

  it('keeps frontmatter, a later # section and ### headings', () => {
    const text = '---\nkind: sources\n---\n# Sources\n\nIntro.\n\n## a.md\n\n### Detail\n\n- Origin: x\n\n# Archive\n\nOld text.\n';
    const m = readSourcesManifest(text);
    expect(m.frontmatter?.markdown).toBe('---\nkind: sources\n---');
    expect(m.entries[0].notes.map(n => n.markdown)).toEqual(['### Detail']);
    expect(m.other.map(o => o.markdown)).toEqual(['# Archive\n\nOld text.']);
    expectNothingLost(text, m);
  });

  it('reads a huge SOURCES.md (5,000 entries) quickly and joins it with 100 files', () => {
    const text = '# Sources\n' + Array.from({ length: 5000 }, (_, i) => `\n## file-${i}.md\n\n- Snapshot: [file-${i}.md](sources/file-${i}.md)\n- Original: "/x/file-${i}.md"\n- Captured: 2026-10-0${1 + (i % 9)}T00:00:00Z\n- SHA-256: \`${(i % 16).toString(16).repeat(64)}\`\n- Note: entry ${i}\n`).join('');
    const t0 = performance.now();
    const m = readSourcesManifest(text);
    const v = sourceCardsModel(m, Array.from({ length: 100 }, (_, i) => src(`sources/file-${i}.md`)), 4900);
    expect(performance.now() - t0).toBeLessThan(2000);
    expect(m.entries).toHaveLength(5000);
    expect(v.cards.every(c => c.entry !== null)).toBe(true);
    expect(v.unmatched).toHaveLength(4900);
    expectNothingLost(text, m);
  });

  it('drops no line in 500 random documents', () => {
    for (const doc of fuzzDocs(500)) expectNothingLost(doc);
  });
});

describe('sources helpers', () => {
  it('names an origin from its text alone', () => {
    expect(originKind('https://example.com/a')).toBe('url');
    expect(originKind('Google Drive folder "Q3"')).toBe('connector');
    expect(originKind('fetched through the Slack connector')).toBe('connector');
    expect(originKind('/Users/x/a.md')).toBe('local');
    expect(originKind('~/Downloads/a.md')).toBe('local');
    expect(originKind('C:\\data\\a.md')).toBe('local');
    expect(originKind('a colleague, by email')).toBe('written');
  });

  it('formats sizes and short checksums', () => {
    expect([0, 1, 1023, 1024, 60229, 2_400_000, 3 * 1024 ** 3].map(formatBytes)).toEqual(['0 bytes', '1 byte', '1,023 bytes', '1 KB', '59 KB', '2.3 MB', '3 GB']);
    expect(shortSha(SHA)).toBe('572af837…4d1e');
  });
});

/** Plan L3 X3/X5 (2026-10-14): each card leads with what the file is, and a
 *  refresh (a new entry in attach's shape plus a Refreshed note) becomes the
 *  current entry while the capture before it shows as earlier. */
describe('snapshot state and refresh entries', () => {
  const NOW = new Date(2026, 9, 14, 12, 0);
  const entry = (at: string, sha: string, extra = '') => `\n## notes.md\n\n- Snapshot: [notes.md](sources/notes.md)\n- Original: "/Users/x/notes.md"\n- Captured: ${at}\n- SHA-256: \`${sha}\`\n${extra}`;

  it('says snapshot first, with the captured day, and never "current"', () => {
    const v = sourceCardsModel(null, [
      src('sources/a.md', { original: '/Users/x/a.md', captured: '2026-10-13T09:00:00Z', freshness: 'unchanged_metadata' }),
      src('sources/b.md', { original: '/Users/x/b.md', captured: '2026-10-02T09:00:00Z', freshness: 'changed' }),
      src('sources/c.md', { original: '/Users/x/c.md', captured: '2026-10-02T09:00:00Z', freshness: 'unavailable' }),
      src('sources/d.md'),
    ], 0, NOW);
    expect(v.cards.map(c => c.state.summary)).toEqual([
      'Snapshot captured yesterday; doesn’t update on its own',
      'Snapshot captured 2 Oct; the original has changed since',
      'Snapshot captured 2 Oct; its original can’t be found',
      'Snapshot; nothing records where it came from',
    ]);
    expect(v.cards.map(c => c.refreshable)).toEqual([true, true, true, false]);
    expect(v.cards[1].original).toBe('/Users/x/b.md');
    for (const c of v.cards) expect(`${c.state.summary} ${c.state.detail}`).not.toMatch(/\bcurrent\b|up to date|\bfresh\b/i);
    expect(v.cards[1].freshnessDetail).not.toMatch(/new name/);
  });

  it('reads a refresh as the current entry and keeps the capture before it as earlier', () => {
    const text = '# Sources\n' + entry('2026-10-02T09:00:00Z', 'a'.repeat(64))
      + entry('2026-10-14T09:00:00Z', 'b'.repeat(64), `- Refreshed: replaced the snapshot captured 2026-10-02T09:00:00Z (SHA-256 \`${'a'.repeat(64)}\`); the earlier copy is kept as sources/.previous/aaaaaaaaaaaa-notes.md\n`);
    const m = readSourcesManifest(text);
    const [card] = sourceCardsModel(m, [src('sources/notes.md', { original: '/Users/x/notes.md', captured: '2026-10-14T09:00:00Z', freshness: 'unchanged_metadata' })], 0, NOW).cards;
    expect(card.sha256).toBe('b'.repeat(64));
    expect(card.earlier).toEqual([m.entries[0]]);
    expect(card.entry!.notes.map(n => n.markdown).join('\n')).toMatch(/^- Refreshed: replaced the snapshot captured 2026-10-02/);
    expect(card.state.summary).toBe('Snapshot captured today; doesn’t update on its own');
  });
});
