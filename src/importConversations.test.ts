import { describe, expect, it } from 'vitest';
import type { ImportEntry, ImportManifest, ImportSource } from './api';
import {
  broughtIn, countsText, defaultSelection, entryMeta, importLabel, isExportSource, jobFinished, jobListable, pickable, progressText, resultText,
  selectAll, sourceLine, visibleSources, scanNotice,
} from './importConversations';

const entry = (id: string, status: ImportEntry['status'], over: Partial<ImportEntry> = {}): ImportEntry => ({
  id, externalId: `claude-code:${id}`, title: id, createdAt: '2026-11-10T10:00:00Z', lastAt: '2026-11-12T10:00:00Z', messageCount: 4, turnCount: 2,
  size: 100, projectFolder: null, destination: 'quick-chats', status, selected: status === 'new', sidechainLines: 0, malformedLines: 0,
  truncated: false, shortenedTurns: 0, contentHash: id, ...over,
});

const counts = { new: 0, alreadyImported: 0, updated: 0, newVersion: 0, hollowOwn: 0, subagent: 0, unsupported: 0 };

describe('Import conversations (H6 with H5a)', () => {
  it('shows incomplete scan coverage separately from the useful results', () => {
    expect(scanNotice({ capped: false, unreadable: 0, symlinksSkipped: 0 })).toBeNull();
    expect(scanNotice({ capped: true, stopped: 'Stopped after 5,000 files.', unreadable: 2, symlinksSkipped: 1 }))
      .toBe('Scan limit reached. This list is incomplete. Stopped after 5,000 files. 2 files could not be read. 1 symbolic link skipped.');
    expect(scanNotice({ capped: false, stopped: 'Stopped after 120 seconds.', unreadable: 0, symlinksSkipped: 0 })).toContain('120 seconds');
  });
  it('shows a folder source only when its folder exists, and an export source whenever it is listed', () => {
    const sources = [
      { source: 'claude-code', label: 'Claude Code', defaultRoot: '/u/.claude/projects', export: false, exists: true },
      { source: 'codex', label: 'Codex', defaultRoot: '/u/.codex/sessions', export: false, exists: false },
      { source: 'chatgpt', label: 'ChatGPT', defaultRoot: null, export: true, exists: false },
    ] as ImportSource[];
    const shown = visibleSources(sources);
    expect(shown.map(s => s.source)).toEqual(['claude-code', 'chatgpt']);
    expect(isExportSource(shown[1])).toBe(true);
    expect(sourceLine(shown[0])).toBe('Reads /u/.claude/projects. Nothing is read until you click Look.');
    expect(sourceLine(shown[1])).toMatch(/^Choose the ChatGPT export file/);
  });

  it('selects everything not imported by default, and Select all adds the already imported, never Hollow’s own', () => {
    const manifest = { conversations: [entry('a', 'new'), entry('b', 'updated'), entry('c', 'alreadyImported'), entry('d', 'hollowOwn'), entry('e', 'newVersion'), entry('f', 'subagent')] };
    expect([...defaultSelection(manifest)]).toEqual(['a', 'b', 'e']);
    expect([...selectAll(manifest)]).toEqual(['a', 'b', 'c', 'e']);
    expect(pickable(entry('d', 'hollowOwn'))).toBe(false);
  });

  it('tells an inspect pass apart while it reads and once it ends', () => {
    expect(jobFinished({ state: 'inspecting' })).toBe(false);
    expect(jobFinished({ state: 'ready' })).toBe(true);
    expect(jobListable({ state: 'ready' })).toBe(true);
    expect(jobListable({ state: 'failed' })).toBe(false);
    expect(progressText({ state: 'inspecting', done: 2, total: 40 })).toBe('Reading 2 of 40 files');
    expect(progressText({ state: 'inspecting', done: 0, total: 0 })).toBe('Reading');
    expect(progressText({ state: 'failed', done: 0, total: 0, error: 'denied' })).toBe('Could not read it: denied');
    expect(progressText(null)).toBe('Starting');
  });

  it('counts the list and the result in plain words', () => {
    const manifest = { conversations: [entry('a', 'new'), entry('b', 'new'), entry('c', 'alreadyImported')], counts: { ...counts, new: 2, alreadyImported: 1 } } as Pick<ImportManifest, 'conversations' | 'counts'>;
    expect(countsText(manifest)).toBe('3 conversations: 2 new, 1 already imported');
    expect(importLabel(0)).toBe('Import');
    expect(importLabel(1)).toBe('Import 1 conversation');
    const result = { imported: 2, updated: 1, newVersions: 0, alreadyImported: 1, skipped: 0, failed: [{ id: 'x', title: 'X', error: 'bad' }] };
    expect(resultText(result)).toBe('Imported 2, updated 1, 1 already there, 1 failed.');
    expect(broughtIn(result)).toBe(3);
  });

  it('describes an entry by its last date, messages and project folder', () => {
    expect(entryMeta(entry('a', 'new', { projectFolder: '/p/pricing' }))).toBe('2026-11-12 · 4 messages · /p/pricing');
    expect(entryMeta(entry('a', 'new', { lastAt: null, createdAt: null, messageCount: 1 }))).toBe('1 message');
  });
});
