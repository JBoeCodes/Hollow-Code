import { describe, expect, it } from 'vitest';
import { cardMachineName, openInRows, parseCardSections } from './sidePanelModel';
import { cardFileRow, cardFileRows, cardGlyphs, cardOutputs, cardSources, cardSubagentWords, panelTools, recallFeedbackPrompt, recallTraceWords, viewAllSourcesPath } from './sidePanelModel';
import type { SubagentRow, WorkFile } from './generated/ipc';

const row = (key: string, agent: string, status: SubagentRow['status'] = 'done') => ({ key, agent, status }) as SubagentRow;
const view = (over: { running?: number; queued?: number; doneCount?: number; active?: SubagentRow[]; done?: SubagentRow[]; earlier?: unknown[] }) =>
  ({ running: 0, queued: 0, doneCount: 0, active: [], done: [], ...over });
const file = (name: string, kind: WorkFile['kind'], modified: string | null): WorkFile => ({ path: `workspaces/w/${kind === 'output' ? 'outputs/' : 'sources/'}${name}`, name, kind, size: 1, modified });

describe('the side panel card (overnight 11-12 C4)', () => {
  it('defaults closed and accepts only known saved sections', () => {
    for (const raw of [null, '', '{', '{}', 'true']) expect(parseCardSections(raw)).toEqual([]);
    expect(parseCardSections('["recall","outputs","recall","other",3]')).toEqual(['outputs', 'recall']);
  });
  it('drafts the exact recall feedback text for both actions', () => {
    const item = { statement: 'Keep reports concise.', path: 'learnings/reports.md' };
    expect(recallFeedbackPrompt(item, 'notRelevant')).toBe('Teach Hollow: the accepted learning "Keep reports concise." from learnings/reports.md was not relevant to this request and should be recalled more narrowly. Ask me for the corrected durable statement if needed, then create only a review proposal; do not change trusted context directly.');
    expect(recallFeedbackPrompt(item, 'outdated')).toBe('Teach Hollow: the accepted learning "Keep reports concise." from learnings/reports.md is outdated and needs a replacement. Ask me for the corrected durable statement if needed, then create only a review proposal; do not change trusted context directly.');
  });

  it('says why recall was quiet, or how many learnings it used (RC1)', () => {
    expect(recallTraceWords({ silence: 'a weak cue', candidates: [{ kept: false }] })).toBe('Recall stayed quiet: a weak cue.');
    expect(recallTraceWords({ candidates: [] })).toBe('Nothing in memory matched this request.');
    expect(recallTraceWords({ candidates: [{ kept: true }, { kept: true }] })).toBe('2 used.');
    expect(recallTraceWords({ alreadyGiven: true, candidates: [{ kept: true }, { kept: true }] })).toBe('2 used; this chat already holds them, so they are not sent again.');
  });

  it('says "4 done", "2 running, 5 done", "2 earlier chats" or none yet', () => {
    expect(cardSubagentWords(view({ doneCount: 4 }))).toBe('4 done');
    expect(cardSubagentWords(view({ running: 2, doneCount: 5 }))).toBe('2 running, 5 done');
    expect(cardSubagentWords(view({ running: 1, queued: 1, doneCount: 3 }))).toBe('1 running, 1 queued, 3 done');
    expect(cardSubagentWords(view({}))).toBe('None yet');
    // A lead's other chats alone: the button is their way in (L8).
    expect(cardSubagentWords(view({ earlier: [{}, {}] }))).toBe('2 earlier chats');
    expect(cardSubagentWords(view({ earlier: [{}] }))).toBe('1 earlier chat');
    expect(cardSubagentWords(view({ doneCount: 1, earlier: [{}] }))).toBe('1 done');
    expect(cardSubagentWords(null)).toBe('Reading…');
  });

  it('shows one glyph per subagent, up to four, active ones first', () => {
    const done = [row('d1', 'claude'), row('d2', 'codex'), row('d3', 'claude'), row('d4', 'grok')];
    expect(cardGlyphs(view({ done }))).toEqual(['claude', 'codex', 'claude', 'grok']);
    expect(cardGlyphs(view({ active: [row('a1', 'antigravity', 'running')], done }))).toEqual(['antigravity', 'claude', 'codex', 'claude']);
    expect(cardGlyphs(null)).toEqual([]);
  });

  it('lists outputs and sources newest first, three of each', () => {
    const files = [file('a.md', 'output', '2026-11-01'), file('b.md', 'output', '2026-11-03'), file('c.md', 'output', null), file('d.md', 'output', '2026-11-02'),
      file('s.pdf', 'source', '2026-11-01'), { path: 'workspaces/w/WORKSPACE.md', name: 'WORKSPACE.md', kind: 'context', size: 1, modified: '2026-11-09' } as WorkFile];
    expect(cardOutputs(files).map(f => f.name)).toEqual(['b.md', 'd.md', 'a.md']);
    expect(cardSources(files).map(f => f.name)).toEqual(['s.pdf']);
    expect(cardOutputs([])).toEqual([]);
  });

  it('shows a file row as its name and type icon, the full path kept for hover and copy (C8)', () => {
    expect(cardFileRow('workspaces/local-q3-review/outputs/q3-regional-review.md')).toEqual({
      path: 'workspaces/local-q3-review/outputs/q3-regional-review.md', name: 'q3-regional-review.md', folder: '', icon: 'markdown', tone: 'doc',
      label: 'Open q3-regional-review.md, workspaces/local-q3-review/outputs/q3-regional-review.md',
    });
    expect(cardFileRow('workspaces/w/sources/regional-pnl-q3.csv')).toMatchObject({ name: 'regional-pnl-q3.csv', icon: 'fileSpreadsheet', tone: 'sheet' });
    expect(cardFileRow('workspaces/w/sources/pack.pdf')).toMatchObject({ name: 'pack.pdf', icon: 'filePdf', tone: 'pdf' });
    expect(cardFileRow('notes.unknownext')).toMatchObject({ name: 'notes.unknownext', icon: 'file', tone: 'plain', label: 'Open notes.unknownext' });
  });

  it('tells same-name files apart by their folder, and leaves unique names bare', () => {
    const rows = cardFileRows(['outputs/a/report.md', 'outputs/b/report.md', 'outputs/plan.md']);
    expect(rows.map(r => [r.name, r.folder])).toEqual([['report.md', 'a'], ['report.md', 'b'], ['plan.md', '']]);
    expect(rows[0].path).toBe('outputs/a/report.md');
  });

  it('View all opens SOURCES.md, or WORKSPACE.md without one', () => {
    expect(viewAllSourcesPath('workspaces/w', [{ path: 'workspaces/w/SOURCES.md' }])).toBe('workspaces/w/SOURCES.md');
    expect(viewAllSourcesPath('workspaces/w', [])).toBe('workspaces/w/WORKSPACE.md');
  });
});

describe('a New tab’s Tools (overnight 11-12 C5)', () => {
  it('offers Files, Browser, Subagents and Terminal, and Changes in a build workspace', () => {
    expect(panelTools({ project: true, chat: true, build: false }).map(t => [t.id, t.unavailable])).toEqual([['files', null], ['browser', null], ['subagents', null], ['terminal', null]]);
    expect(panelTools({ project: true, chat: true, build: true }).map(t => t.id)).toEqual(['files', 'browser', 'subagents', 'terminal', 'changes']);
  });
  it('says why a tool can’t open here', () => {
    const tools = panelTools({ project: false, chat: false, build: false });
    expect(tools.find(t => t.id === 'files')?.unavailable).toMatch(/Chats has no project files/);
    expect(tools.find(t => t.id === 'subagents')?.unavailable).toMatch(/Send a message first/);
    expect(tools.find(t => t.id === 'browser')?.unavailable).toBeNull();
    // A terminal needs no project and no sent message: a draft chat can start one.
    expect(tools.find(t => t.id === 'terminal')?.unavailable).toBeNull();
  });
});

describe('the card header (parity 5.1)', () => {
  it('lists Finder first, once, then the installed editors in the backend order', () => {
    expect(openInRows(null)).toEqual([{ id: 'finder', label: 'Finder' }]);
    expect(openInRows([{ id: 'zed', label: 'Zed' }, { id: 'finder', label: 'Finder' }, { id: 'cursor', label: 'Cursor' }, { id: 'zed', label: 'Zed' }]).map(r => r.id)).toEqual(['finder', 'zed', 'cursor']);
    expect(openInRows([{ id: '', label: 'x' }, { id: 'vscode', label: '' }])).toEqual([{ id: 'finder', label: 'Finder' }]);
  });
  it('names this Mac, with a plain fallback', () => {
    expect(cardMachineName('  Alex MacBook Pro ')).toBe('Alex MacBook Pro');
    expect(cardMachineName('')).toBe('This Mac');
    expect(cardMachineName(null)).toBe('This Mac');
  });
});
