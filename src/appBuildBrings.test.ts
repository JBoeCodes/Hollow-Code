import { describe, expect, it } from 'vitest';
import panelSource from './components/AppBuildPanel.tsx?raw';
import { bringsDisplay, noteOnly } from './appBuildBrings';

const pr = { number: 801, title: 'Some fix', merge: 'abcdef12', mergedAt: '', untested: [], entryFound: true };
const commit = { sha: '12345678', subject: 'direct', at: '' };
const unavailable = "The build receipt's installed base commit is unavailable; what it drops could not be checked.";
const dirty = "This build also includes uncommitted changes that aren't listed.";

describe('what this install brings (RF-787-F1/F2)', () => {
  it('a failed comparison shows its note and never the zero counts', () => {
    const shown = bringsDisplay({ note: unavailable, pullRequests: [], commits: [] });
    expect(shown).toEqual({ summary: null, note: unavailable });
    expect(JSON.stringify(shown)).not.toMatch(/0 pull requests|0 direct commits/);
    expect(noteOnly({ note: unavailable, pullRequests: [], commits: [] })).toBe(true);
  });

  it('a dirty note next to listed work stays visible outside the disclosure', () => {
    const shown = bringsDisplay({ note: dirty, pullRequests: [pr], commits: [commit, commit] });
    expect(shown).toEqual({ summary: 'What this install brings · 1 pull request, 2 direct commits', note: dirty });
    expect(noteOnly({ note: dirty, pullRequests: [pr], commits: [] })).toBe(false);
  });

  it('no note: counts only', () => {
    expect(bringsDisplay({ note: null, pullRequests: [], commits: [commit] }))
      .toEqual({ summary: 'What this install brings · 0 pull requests, 1 direct commit', note: null });
  });

  it('the panel renders the note outside the details, and has no install confirm since row U10', () => {
    const start = panelSource.indexOf('<details class="bc-brings"');
    const end = panelSource.indexOf('</details>', start);
    expect(start).toBeGreaterThanOrEqual(0);
    expect(end).toBeGreaterThan(start);
    expect(panelSource.slice(start, end)).not.toContain('.note');
    expect(panelSource).toContain('bringsDisplay(');
    // The install lives in the Update button now (row U10).
    expect(panelSource).not.toContain('Install and restart');
    expect(panelSource).toContain('Show in Update button');
  });
});
