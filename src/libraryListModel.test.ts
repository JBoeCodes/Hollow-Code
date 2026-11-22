import { describe, expect, it } from 'vitest';
import type { VaultItem } from './types';
import type { MixedResult } from './libraryResultsModel';
import {
  FILTER_SOURCES, buildLibraryList, filterLibraryList, listEmpty, listsNote, meaningRows, noteKind, noteRows, readState, recordingRows, recordingsState,
  LIBRARY_RESULTS_ASK, LIBRARY_RESULTS_CAP, isDamagedResultsStore, limitShowing, partialNote, resultRows, resultsCapped, shortFolder, stillReading, type LibraryListSource, type SourceState,
} from './libraryListModel';
import libraryListSource from './components/LibraryList.tsx?raw';

const NOW = Date.parse('2026-11-12T12:00:00Z');
const daysAgo = (d: number) => new Date(NOW - d * 86_400_000).toISOString();
const item = (path: string, over: Partial<VaultItem> = {}): VaultItem => ({
  path, title: '', kind: 'note', surface: 'vault', folder: path.split('/')[0], editable: true, hidden: false,
  trust: 'authored', mtime: daysAgo(1), size: 10, excerpt: '', drift: null, ...over,
});
const result = (over: Partial<MixedResult> & { path?: string } = {}): MixedResult => ({
  key: JSON.stringify(['file', 'ws1', over.path ?? 'workspaces/ws1/outputs/report.md']), kind: 'file', workspaceId: 'ws1', workspaceTitle: 'Q3 review',
  title: 'Report', name: 'report.md', modified: daysAgo(2), canvas: null,
  card: { path: over.path ?? 'workspaces/ws1/outputs/report.md' } as MixedResult['card'], ...over,
});

describe('Library → All rules', () => {
  it('sorts Meetings, People and Outputs by where the vault keeps them', () => {
    expect(noteKind('calendar/2026-11-03 standup.md')).toBe('meeting');
    expect(noteKind('library/records/meetings/2026/kickoff.md')).toBe('meeting');
    expect(noteKind('recordings/plaud-1.md')).toBe('meeting');
    expect(noteKind('notes/loose.md', 'meeting')).toBe('meeting');
    for (const p of ['people/ana.md', 'library/people/bo.md', 'library/work/people/cy.md', 'library/records/people/di.md']) expect(noteKind(p)).toBe('person');
    expect(noteKind('outputs/brief.md')).toBe('output');
    expect(noteKind('library/work/decisions.md')).toBe('note');
  });

  it('leaves workspaces’ working files, tasks and hidden mirrors out, but keeps meetings and project records', () => {
    expect(listsNote(item('workspaces/ws1/sources/a.md'))).toBe(false);
    expect(listsNote(item('workspaces/ws1/PROJECT.md'))).toBe(true);
    expect(listsNote(item('tasks/t1.md'))).toBe(false);
    expect(listsNote(item('emails/x.md', { hidden: true, kind: 'email' }))).toBe(false);
    expect(listsNote(item('calendar/standup.md', { hidden: true, kind: 'meeting' }))).toBe(true);
    expect(listsNote(item('library/pic.png'))).toBe(false);
  });

  it('names a note’s folder short', () => {
    expect(shortFolder('library/work/people/cy.md')).toBe('work/people');
    expect(shortFolder('library/records/meetings/2026/kickoff.md')).toBe('…/meetings/2026');
    expect(shortFolder('library/a.md')).toBe('library');
    expect(shortFolder('README.md')).toBe('Vault root');
    expect(shortFolder('calendar/a.md')).toBe('calendar');
  });
});

describe('one list', () => {
  const notes = noteRows([
    item('library/old.md', { title: 'Old note', mtime: daysAgo(60) }),
    item('people/ana.md', { title: 'Ana Ruiz', mtime: daysAgo(3) }),
    item('workspaces/ws1/outputs/report.md', { title: 'Report copy', mtime: daysAgo(2) }),
    item('library/undated.md', { title: 'Undated', mtime: '' }),
  ]);
  const results = resultRows([result(), result({ key: 'c1', kind: 'canvas', title: 'Chart', modified: daysAgo(0.5), card: null,
    canvas: { workspaceId: 'ws1', workspaceTitle: 'Q3 review', id: 'chart', title: 'Chart', revision: 2, updatedAt: daysAgo(0.5), error: null } })]);
  const recordings = recordingRows([{ id: 'r1', name: 'Weekly sync', date: daysAgo(1), duration: '30m' }, { id: 'r2', name: '', date: 'not a date', duration: null }]);
  const rows = buildLibraryList({ notes, results, recordings });

  it('is newest first, unknown dates last, and a result file is listed once', () => {
    expect(rows.map(r => r.title)).toEqual(['Chart', 'Weekly sync', 'Report', 'Ana Ruiz', 'Old note', 'Undated', 'Untitled recording']);
    expect(rows.find(r => r.title === 'Report')!.source).toBe('results');
  });

  it('Recent keeps the last 30 days until Show older; a search looks through everything', () => {
    const recent = filterLibraryList(rows, { filter: 'recent', query: '', showOlder: false, now: NOW });
    expect(recent.rows.map(r => r.title)).toEqual(['Chart', 'Weekly sync', 'Report', 'Ana Ruiz']);
    expect(recent.older).toBe(3);
    expect(filterLibraryList(rows, { filter: 'recent', query: '', showOlder: true, now: NOW }).rows).toHaveLength(7);
    expect(filterLibraryList(rows, { filter: 'recent', query: 'old', showOlder: false, now: NOW }).rows.map(r => r.title)).toEqual(['Old note']);
  });

  it('filters Meetings, People and Outputs, and searches title, place and path by every word', () => {
    const pick = (filter: 'meetings' | 'people' | 'outputs', query = '') => filterLibraryList(rows, { filter, query, showOlder: false, now: NOW }).rows.map(r => r.title);
    expect(pick('meetings')).toEqual(['Weekly sync', 'Untitled recording']);
    expect(pick('people')).toEqual(['Ana Ruiz']);
    expect(pick('outputs')).toEqual(['Chart', 'Report']);
    expect(pick('outputs', 'q3 chart')).toEqual(['Chart']);
    expect(pick('people', 'people/ana')).toEqual(['Ana Ruiz']);
    expect(pick('people', 'nobody')).toEqual([]);
  });

  it('a failing source leaves the others listed', () => {
    const withoutRecordings = buildLibraryList({ notes, results });
    expect(withoutRecordings.some(r => r.kind === 'recording')).toBe(false);
    expect(withoutRecordings.map(r => r.title)).toContain('Ana Ruiz');
    expect(buildLibraryList({ recordings })).toHaveLength(2);
    expect(FILTER_SOURCES.people).toEqual(['notes', 'results']);
  });

  it('by meaning keeps the index’s order, inside the filter', () => {
    const hits = [{ path: 'people/ana.md' }, { path: 'workspaces/ws1/outputs/report.md' }, { path: 'workspaces/ws2/notes/x.md', basename: 'x' }, { path: 'people/ana.md' }];
    expect(meaningRows(rows, hits, 'recent').map(r => r.title)).toEqual(['Ana Ruiz', 'Report', 'x']);
    expect(meaningRows(rows, hits, 'people').map(r => r.title)).toEqual(['Ana Ruiz']);
  });
});

describe('empty states', () => {
  const base = { total: 0, filter: 'recent' as const, query: '', older: 0, failed: false, loading: false };
  it('says which empty it is', () => {
    expect(listEmpty(base).kind).toBe('empty');
    expect(listEmpty({ ...base, loading: true }).kind).toBe('loading');
    expect(listEmpty({ ...base, failed: true }).kind).toBe('failed');
    expect(listEmpty({ ...base, query: 'x', total: 4 })).toMatchObject({ kind: 'search', title: 'Nothing matches your search' });
    expect(listEmpty({ ...base, total: 4, older: 4 })).toMatchObject({ kind: 'filter', title: 'Nothing from the last 30 days' });
    expect(listEmpty({ ...base, total: 4, filter: 'people' })).toMatchObject({ kind: 'filter', title: 'No people yet' });
    expect(listEmpty({ ...base, total: 4, filter: 'meetings' }).detail).toContain('library/records/meetings/');
  });
});

describe('review fixes (L3)', () => {
  it('a meeting or person note that is also a result shows in Outputs and in Meetings or People', () => {
    // Real paths (lane S re-review of #1319): list_all returns results under
    // workspaces/<id>/outputs/, which listsNote leaves out as notes, so the
    // kind comes from the raw vault items.
    const items = [item('workspaces/ws1/outputs/Meetings-recap.md', { title: 'Recap', kind: 'meeting', mtime: daysAgo(1) })];
    const notes = noteRows(items);
    expect(notes).toEqual([]);
    const results = resultRows([result({ path: 'workspaces/ws1/outputs/meetings-recap.md', key: 'r', title: 'Recap' })]);
    expect(buildLibraryList({ notes, results })[0].also).toBeUndefined();
    const rows = buildLibraryList({ notes, results, items });
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ source: 'results', also: 'meeting' });
    const pick = (filter: 'meetings' | 'outputs' | 'people') => filterLibraryList(rows, { filter, query: '', showOlder: true, now: NOW }).rows.map(r => r.title);
    expect(pick('outputs')).toEqual(['Recap']);
    expect(pick('meetings')).toEqual(['Recap']);
    expect(pick('people')).toEqual([]);
  });

  it('an unreadable result date is unknown and sorts last, keeping the order total', () => {
    const results = resultRows([result({ key: 'bad', title: 'Bad date', modified: 'garbage' }), result({ key: 'ok', title: 'Good', modified: daysAgo(1), path: 'outputs/good.md' })]);
    expect(results[0].modified).toBeNull();
    const rows = buildLibraryList({ results, notes: noteRows([item('library/a.md', { title: 'A', mtime: daysAgo(3) })]) });
    expect(rows.map(r => r.title)).toEqual(['Good', 'A', 'Bad date']);
  });

  it('folder rules ignore case', () => {
    expect(noteKind('People/ana.md')).toBe('person');
    expect(noteKind('Library/Records/Meetings/x.md')).toBe('meeting');
    expect(listsNote(item('Tasks/t.md'))).toBe(false);
  });
});

describe('a slow source holds back only its own rows (lane S re-review of #1319)', () => {
  it('reads each source’s state without a value as loading, its error first', () => {
    expect(readState(null, undefined)).toEqual({ status: 'loading' });
    expect(readState(null, [])).toEqual({ status: 'ready' });
    expect(readState('gone', [])).toEqual({ status: 'failed', error: 'gone' });
    expect(recordingsState(null, undefined)).toEqual({ status: 'loading' });
    expect(recordingsState(null, { state: 'notConnected', error: null }).status).toBe('off');
    expect(recordingsState(null, { state: 'unavailable', error: null })).toEqual({ status: 'failed', error: 'Plaud didn’t answer.' });
    expect(recordingsState(null, { state: 'ok', error: null })).toEqual({ status: 'ready' });
  });

  it('a pending recordings read leaves notes and results listed, and says it is still reading', () => {
    const states: Record<LibraryListSource, SourceState> = {
      notes: readState(null, [item('people/ana.md')]), results: readState(null, []), canvases: readState(null, []), recordings: recordingsState(null, undefined),
    };
    const reading = stillReading(FILTER_SOURCES.recent, s => states[s]);
    expect(reading).toEqual(['recordings']);
    const rows = buildLibraryList({
      notes: noteRows([item('people/ana.md', { title: 'Ana Ruiz' })]), results: resultRows([result()]), recordings: [],
    });
    const view = filterLibraryList(rows, { filter: 'recent', query: '', showOlder: false, now: NOW });
    expect(view.rows.map(r => r.title)).toEqual(['Ana Ruiz', 'Report']);
    // Rows on screen: no list-wide loading state, only "Still reading recordings…".
    expect(listEmpty({ total: rows.length, filter: 'recent', query: '', older: view.older, failed: false, loading: reading.length > 0 }).kind).not.toBe('loading');
    expect(stillReading(FILTER_SOURCES.people, s => states[s])).toEqual([]);
  });

  it('Meetings and People wait on and report result files, which can be meetings or people (also)', () => {
    expect(FILTER_SOURCES.meetings).toEqual(['notes', 'results', 'recordings']);
    const slowResults: Record<LibraryListSource, SourceState> = {
      notes: readState(null, []), results: readState(null, undefined), canvases: readState(null, []), recordings: recordingsState(null, { state: 'ok', error: null }),
    };
    expect(stillReading(FILTER_SOURCES.people, s => slowResults[s])).toEqual(['results']);
    expect(stillReading(FILTER_SOURCES.meetings, s => slowResults[s])).toEqual(['results']);
    // Nothing listed yet while results are read: loading, not "No people yet".
    expect(listEmpty({ total: 0, filter: 'people', query: '', older: 0, failed: false, loading: true }).kind).toBe('loading');
  });

  it('Show older paints enough pages for the first older row to take focus', () => {
    expect(limitShowing(0, 200)).toBe(200);
    expect(limitShowing(199, 200)).toBe(200);
    expect(limitShowing(200, 200)).toBe(400);
    expect(limitShowing(450, 200)).toBe(600);
    // Never fewer rows than are already painted.
    expect(limitShowing(10, 200, 600)).toBe(600);
  });

  it('marks the count partial when the results listing stopped at its cap', () => {
    expect(resultsCapped(LIBRARY_RESULTS_CAP - 1)).toBe(false);
    expect(resultsCapped(undefined)).toBe(false);
    // Exactly the cap is a whole listing; only the extra row asked for says more exist.
    expect(resultsCapped(LIBRARY_RESULTS_CAP)).toBe(false);
    expect(LIBRARY_RESULTS_ASK).toBe(LIBRARY_RESULTS_CAP + 1);
    expect(resultsCapped(LIBRARY_RESULTS_ASK)).toBe(true);
    expect(partialNote({ loading: false, failed: false, capped: false })).toBe('');
    expect(partialNote({ loading: false, failed: false, capped: true })).toBe(' · partial');
    expect(partialNote({ loading: true, failed: false, capped: false })).toBe(' · partial');
    expect(partialNote({ loading: false, failed: true, capped: false })).toBe(' · partial');
  });

  it('every row click takes the open ticket first, so the newest click wins', () => {
    const flat = libraryListSource.replace(/\s+/g, '');
    const open = flat.slice(flat.indexOf('constopen=async('), flat.indexOf('constopenLabel'));
    const ticket = open.indexOf('constticket=opening.start();');
    expect(ticket).toBeGreaterThan(-1);
    expect(ticket).toBeLessThan(open.indexOf("row.kind==='recording'"));
    expect(ticket).toBeLessThan(open.indexOf("row.kind==='canvas'"));
    expect(open.match(/opening\.start\(\)/g)).toHaveLength(1);
  });

  it('All reads the peek listing and tells a damaged review store from a plain failure', () => {
    const flat = libraryListSource.replace(/\s+/g, '');
    expect(flat).toContain('api.listAllResultsPeek(null,LIBRARY_RESULTS_ASK)');
    expect(flat).not.toContain('api.listAllResults(');
    expect(isDamagedResultsStore('Results review state could not be read (/v/w/.hollow/results.json): bad. It was left as is; open Results to see it.')).toBe(true);
    expect(isDamagedResultsStore('Results review state was damaged (x). It was moved to /y.')).toBe(true);
    expect(isDamagedResultsStore('network down')).toBe(false);
    expect(isDamagedResultsStore(null)).toBe(false);
  });

  it('the view reads recordings and notes without suspending into Library’s fallback', () => {
    const flat = libraryListSource.replace(/\s+/g, '');
    expect(flat).toContain('constrecordingsShown=unsuspended(recordings);');
    expect(flat).toContain('constnotesShown=unsuspended(vaultItemsData);');
    expect(flat).not.toContain('latestUnlessErrored(');
    expect(flat).not.toMatch(/vaultItemsData\(\)/);
    expect(flat).not.toMatch(/[^.]recordings\(\)/);
  });
});
