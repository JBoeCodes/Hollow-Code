import { describe, expect, it } from 'vitest';
import { contentIndexFilling, contentIndexProgress, contentsIndexedLine, exclusionConsequence, indexedCount, refreshedLabel, summarizeCoverage } from './sourceCoverage';

const base = { projects: [{ id: 'a', title: 'A', path: '/p/A' }], files: [{ path: '/p/A/x.md', relative: 'A/x.md', projectId: 'a', size: 1, readable: true }] };
const cov = (over: object = {}) => ({ projects: [{ id: 'a', title: 'A', indexed: 1, found: 1, capped: false, folders: [] }], scanLimited: false, maxFiles: 20000, maxProjectFiles: 2500, excluded: [], ...over });

describe('source coverage summary (audit H03)', () => {
  it('never calls missing coverage complete', () => {
    expect(summarizeCoverage(undefined).state).toBe('unknown');
    expect(summarizeCoverage({ ...base, warnings: [] }).state).toBe('unknown');
    const legacy = summarizeCoverage({ ...base, warnings: ['Library: only the first 2500 files are indexed; add .hollowignore rules to choose which.'] });
    expect(legacy.state).toBe('partial');
    expect(legacy.other).toHaveLength(1); // raw warning kept visible
  });
  it('summarizes capped projects and an early-stopped scan in one headline', () => {
    const big = { id: 'b', title: 'Big', indexed: 2500, found: 2500, capped: true, folders: [{ relative: 'Big/src', files: 2400 }] };
    const s = summarizeCoverage({ ...base, warnings: ['Big: only the first 2500 files are indexed; add .hollowignore rules to choose which.', 'Scan limit reached; narrow the Projects folder or add .hollowignore rules.', 'permission denied: /p/C'], coverage: cov({ projects: [cov().projects[0], big], scanLimited: true }) });
    expect(s.state).toBe('partial');
    expect(s.headline).toBe('Coverage incomplete: 1 of 2 projects has more than 2,500 files, so only the first 2,500 of each are indexed; the scan stopped at 20,000 files, so projects later in the folder may be missing.');
    expect(s.capped.map(p => p.title)).toEqual(['Big']);
    expect(s.other).toEqual(['permission denied: /p/C']);
  });
  it('reports full coverage and user exclusions plainly', () => {
    expect(summarizeCoverage({ ...base, warnings: [], coverage: cov({ excluded: ['A/gen'] }) }).headline).toBe('1 file across 1 project, all indexed. 1 folder is left out by you.');
    expect(summarizeCoverage({ ...base, warnings: [], coverage: cov() }).state).toBe('complete');
  });
  it('P07: reports listed, omitted and unread counts from the complete inventory', () => {
    const p07 = (over: object = {}) => cov({ indexed: 2804, available: 2804, omitted: 0, tooLarge: 1, needsReader: 2, inventoryLimited: false, changedAt: '2026-10-01T00:00:00Z', maxFiles: 200000, maxProjectFiles: 50000, ...over });
    const full = summarizeCoverage({ ...base, warnings: [], coverage: p07({ projects: [{ id: 'a', title: 'A', indexed: 2804, found: 2804, capped: false, folders: [], omitted: 0, tooLarge: 1, needsReader: 2 }] }) });
    expect(full.state).toBe('complete');
    expect(full.headline).toBe('2,804 files across 1 project, all indexed. 1 is too large to read and 2 need a reader.');
    expect(full.unread).toEqual({ tooLarge: 1, needsReader: 2 });
    expect(indexedCount({ coverage: p07() })).toBe(2804);
    const big = { id: 'b', title: 'Big', indexed: 50000, found: 61000, capped: true, folders: [{ relative: 'Big/gen', files: 40000 }], omitted: 11000, tooLarge: 0, needsReader: 0 };
    const over = summarizeCoverage({ ...base, warnings: ['Big: 11,000 files past the first 50000 are not listed; search still finds them by name. Leave folders out to choose which are listed.'], coverage: p07({ projects: [cov().projects[0], big], indexed: 50001, available: 61001, omitted: 11000, tooLarge: 0, needsReader: 0 }) });
    expect(over.state).toBe('partial');
    expect(over.headline).toBe('Coverage incomplete: 1 of 2 projects has more than 50,000 files, so 11,000 files aren’t listed (search still finds them by name).');
    expect(over.capped.map(p => p.title)).toEqual(['Big']);
    expect(over.other).toEqual([]); // the share warning is a limit, not an "other" note
    const bounded = summarizeCoverage({ ...base, warnings: ['Inventory limit reached: 200000 files are listed and the rest are counted but not listed. Leave folders out to choose which are listed.'], coverage: p07({ omitted: 5, inventoryLimited: true, scanLimited: true }) });
    expect(bounded.headline).toBe('Coverage incomplete: 5 files aren’t listed; the list reached its 200,000-file limit; the scan stopped early, so projects later in the folder may be missing.');
    expect(bounded.other).toEqual([]);
  });
  it('R14a: counts PDF and Office files Hollow reads apart from images that need a reader', () => {
    const r14 = cov({ indexed: 10, available: 10, omitted: 0, tooLarge: 0, needsReader: 1, extracted: 3, inventoryLimited: false, changedAt: '2026-10-18T00:00:00Z', maxFiles: 200000, maxProjectFiles: 50000 });
    const s = summarizeCoverage({ ...base, warnings: [], coverage: r14 });
    expect(s.state).toBe('complete');
    expect(s.extracted).toBe(3);
    expect(s.unread).toEqual({ tooLarge: 0, needsReader: 1 });
    expect(s.headline).toBe('10 files across 1 project, all indexed. 3 PDF and Office files are read by Hollow. 1 needs a reader.');
    // A manifest from before R14a has no count: nothing is claimed.
    expect(summarizeCoverage({ ...base, warnings: [], coverage: cov() }).extracted).toBe(0);
    expect(summarizeCoverage(undefined).extracted).toBe(0);
  });
  it('says when the last successful refresh was', () => {
    const now = Date.parse('2026-10-01T12:00:00Z');
    expect(refreshedLabel('', now)).toBe('Not refreshed yet');
    expect(refreshedLabel('2026-10-01T11:59:30Z', now)).toBe('Refreshed just now');
    expect(refreshedLabel('2026-10-01T11:56:00Z', now)).toBe('Refreshed 4 min ago');
    expect(refreshedLabel('2026-10-01T09:00:00Z', now)).toBe('Refreshed 3 h ago');
    // Noon on Sep 28 by this machine's own clock, so the date holds in any
    // time zone, not only the gate's New York (overnight D2).
    expect(refreshedLabel(new Date(2026, 8, 28, 12).toISOString(), now)).toBe('Refreshed Sep 28');
  });
  it('states the consequence of leaving a folder out before it happens', () => {
    expect(exclusionConsequence('Big/generated', 1200)).toMatch(/stops indexing “Big\/generated” \(1,200 indexed files\).*Nothing inside the folder changes/);
  });
});

describe('the project content index (repo features R14b)', () => {
  const status = (over: object = {}) => ({ enabled: true, indexed: 12_340, total: 15_002, failed: 0, imageOnly: 41, skipped: 0, ...over });
  it('shows Settings progress with image-only PDFs, failures and privacy skips', () => {
    expect(contentIndexProgress(status())).toBe('12,340 of 15,002 files · 41 image-only PDFs');
    expect(contentIndexProgress(status({ imageOnly: 1, failed: 2, skipped: 3 }))).toBe('12,340 of 15,002 files · 1 image-only PDF · 2 couldn’t be read · 3 left out (restricted folders, secret names)');
    expect(contentIndexProgress(status({ indexed: 1, total: 1, imageOnly: 0 }))).toBe('1 of 1 file');
    expect(contentIndexProgress(status({ enabled: false, indexed: 0 }))).toMatch(/^Off: agents find project files by name and date only\. No index is kept\.$/);
    expect(contentIndexProgress(status({ total: 0, indexed: 0, imageOnly: 0 }))).toBe('Nothing to index yet.');
    expect(contentIndexProgress(null)).toBe('Checking…');
  });
  it('polls only while the index is on and still filling in', () => {
    expect(contentIndexFilling(status())).toBe(true);
    // Done: every listed file read, whatever came of it.
    expect(contentIndexFilling(status({ indexed: 14_961 }))).toBe(false);
    expect(contentIndexFilling(status({ enabled: false }))).toBe(false);
    expect(contentIndexFilling(null)).toBe(false);
  });
  it('adds Contents indexed to the Library coverage line only while it is on', () => {
    expect(contentsIndexedLine(status())).toBe('Contents indexed: 12,340 of 15,002');
    expect(contentsIndexedLine(status({ enabled: false }))).toBe('');
    expect(contentsIndexedLine(undefined)).toBe('');
  });
});
