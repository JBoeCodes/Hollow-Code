import { describe, expect, it } from 'vitest';
import { RESULT_KIND_LABEL, changedSince, libraryCards, resultKindLabel, resultsCardsModel, vaultChangeUnder, vaultChangeUnderAnyOutputs } from './resultsCards';
import { REVIEW_STATE } from './workState';
import type { WorkResult } from './workspaceTypes';

const WS = 'workspaces/local-q3';
const result = (name: string, over: Partial<WorkResult> = {}): WorkResult => ({
  path: `${WS}/${name}`, name, title: name.split('/').pop()!, kind: 'document', size: 100,
  modified: '2026-10-05T10:00:00Z', sha256: 'a'.repeat(64), review: 'draft', acceptedSha256: null,
  acceptedAt: null, readyAt: null, hasSnapshot: false, acceptance: null, history: [], origin: null,
  checks: { total: null, failed: null, unknown: null }, ...over,
});

describe('Library cards', () => {
  it('keeps workspace identity, filters bookkeeping and shares bounded raster rules', () => {
    const list = [
      { ...result('outputs/photo.png', { kind: 'image' }), workspaceId: 'one', workspaceTitle: 'First' },
      { ...result('outputs/photo.png', { kind: 'image', size: 3 * 1024 * 1024 }), workspaceId: 'two', workspaceTitle: 'Second' },
      { ...result('outputs/canvases/x.canvas.json'), workspaceId: 'one', workspaceTitle: 'First' },
    ];
    const cards = libraryCards(list);
    expect(cards).toHaveLength(2);
    expect(cards.map(r => r.workspaceTitle)).toEqual(['First', 'Second']);
    expect(new Set(cards.map(r => r.key)).size).toBe(2);
    expect(cards.map(r => r.card.thumb)).toEqual(['image', null]);
    expect(cards.map(r => r.card.changedSinceVisit)).toEqual([null, null]);
    expect(list).toHaveLength(3);
  });
});

describe('Results cards (Canvas C5b)', () => {
  it('names every kind the Results store lists, and a canvas with its revision', () => {
    for (const kind of ['document', 'table', 'image', 'video', 'page', 'canvas', 'review', 'pdf', 'data', 'file']) {
      expect(RESULT_KIND_LABEL[kind]).toBeTruthy();
    }
    expect(resultKindLabel({ kind: 'video' })).toBe('Video');
    expect(resultsCardsModel({ results: [result('outputs/demo.mp4', { kind: 'video' })], lastVisited: null }).cards[0])
      .toMatchObject({ kindLabel: 'Video', icon: 'play' });
    expect(resultKindLabel({ kind: 'canvas', canvas: { id: 'release', revision: 3 } })).toBe('Canvas v3');
    expect(resultKindLabel({ kind: 'canvas', canvas: { id: 'release', revision: null } })).toBe('Canvas');
    // Whiteboards were removed (2026-10-07): a leftover board file is data.
    expect(resultKindLabel({ kind: 'data' })).toBe('Data');
    // A kind from a newer Hollow reads as a file, never blank.
    expect(resultKindLabel({ kind: 'hologram' })).toBe('File');
    const m = resultsCardsModel({ results: [result('outputs/x.weird', { kind: 'hologram' })], lastVisited: null });
    expect(m.cards[0]).toMatchObject({ kindLabel: 'File', icon: 'fileText', thumb: null });
  });

  it('says each review state in words, the same words as the full review', () => {
    const states = ['draft', 'ready', 'accepted', 'changed'] as const;
    const m = resultsCardsModel({ results: states.map(review => result(`outputs/${review}.md`, { review })), lastVisited: null });
    expect(m.cards.map(c => c.reviewLabel)).toEqual(['Draft', 'Ready for review', 'Accepted', 'Changed since accepted']);
    for (const c of m.cards) {
      expect(c.reviewLabel).toBe(REVIEW_STATE[c.review].label);
      expect(c.reviewHint).toBe(REVIEW_STATE[c.review].hint);
    }
    // Everything not accepted still needs the person.
    expect(m.counts).toMatchObject({ all: 4, toReview: 3, accepted: 1 });
  });

  it('carries its run\'s checks, so a card can mark a failure (P1-50)', () => {
    const m = resultsCardsModel({ results: [
      result('outputs/failed.md', { checks: { total: 3, failed: 1, unknown: 0 } }),
      result('outputs/passed.md', { checks: { total: 2, failed: 0, unknown: 0 } }),
      result('outputs/none.md'),
    ], lastVisited: null });
    expect(m.cards.map(c => [c.checks, c.checkText])).toEqual([
      ['failed', '1 of 3 checks failed'], ['passed', '2 checks passed'], ['notChecked', 'Not checked'],
    ]);
  });

  it('marks results changed since the last visit, and says unknown rather than no', () => {
    const results = [
      result('outputs/old.md', { modified: '2026-10-04T09:00:00Z' }),
      result('outputs/new.md', { modified: '2026-10-05T12:00:00Z' }),
    ];
    const m = resultsCardsModel({ results, lastVisited: '2026-10-05T08:00:00Z' });
    expect(m.cards.map(c => c.changedSinceVisit)).toEqual([false, true]);
    expect(m.counts.sinceVisit).toBe(1);
    expect(m.newest).toBe('2026-10-05T12:00:00Z');
    // No visit recorded, or a result without a time: unknown, never zero.
    expect(resultsCardsModel({ results, lastVisited: null }).counts.sinceVisit).toBeNull();
    const undated = resultsCardsModel({ results: [...results, result('outputs/undated.md', { modified: null })], lastVisited: '2026-10-05T08:00:00Z' });
    expect(undated.cards.map(c => c.changedSinceVisit)).toEqual([false, true, null]);
    expect(undated.counts.sinceVisit).toBeNull();
    expect(changedSince('2026-10-05T08:00:00Z', '2026-10-05T08:00:00Z')).toBe(false);
    expect(changedSince('not a date', '2026-10-05T08:00:00Z')).toBeNull();
  });

  it('never makes a card of a canvas record or the hidden revision store', () => {
    const results = [
      result('outputs/canvases/release.canvas.html', { kind: 'canvas', canvas: { id: 'release', revision: 3 }, title: 'Ship readiness' }),
      result('outputs/canvases/release.canvas.json', { kind: 'data', modified: '2026-10-05T23:00:00Z' }),
      result('outputs/canvases/.revisions/release/abc.canvas.html', { kind: 'page', modified: '2026-10-05T23:00:00Z' }),
      result('outputs/.hidden/notes.md', { modified: '2026-10-05T23:00:00Z' }),
    ];
    const m = resultsCardsModel({ results, lastVisited: '2026-10-05T11:00:00Z' });
    expect(m.cards.map(c => c.name)).toEqual(['outputs/canvases/release.canvas.html']);
    expect(m.cards[0]).toMatchObject({ kind: 'canvas', kindLabel: 'Canvas v3', thumb: 'canvas', canvas: { id: 'release', revision: 3 }, title: 'Ship readiness' });
    // A state save rewrote only the record: not a change since the visit.
    expect(m.counts).toMatchObject({ all: 1, sinceVisit: 0 });
    expect(m.newest).toBe('2026-10-05T10:00:00Z');
  });

  it('draws bounded raster images, saved canvases and video posters', () => {
    const m = resultsCardsModel({
      results: [
        result('outputs/boards/flow.board.json', { kind: 'data' }),
        result('outputs/canvases/c.canvas.html', { kind: 'canvas', canvas: { id: 'c', revision: 1 } }),
        // A canvas kind without its store id (an older listing) gets the icon.
        result('outputs/canvases/d.canvas.html', { kind: 'canvas' }),
        result('outputs/report.md'),
        result('outputs/photo.png', { kind: 'image' }),
        // A video draws its poster frame (inline video PR 3).
        result('outputs/walkthrough.mp4', { kind: 'video' }),
      ],
      lastVisited: null,
    });
    expect(m.cards.map(c => c.thumb)).toEqual([null, 'canvas', null, null, 'image', 'poster']);
  });

  it('rejects oversized, unknown-sized and non-raster images before reading', () => {
    const rows = [
      result('outputs/photo.JPG', {kind:'image', size:2*1024*1024}),
      result('outputs/large.png', {kind:'image', size:2*1024*1024+1}),
      result('outputs/unknown.png', {kind:'image', size:NaN}),
      result('outputs/negative.png', {kind:'image', size:-1}),
      result('outputs/diagram.svg', {kind:'image'}),
      result('outputs/notpng', {kind:'image'}),
      result('outputs/fake.png', {kind:'document'}),
    ];
    expect(resultsCardsModel({results:rows,lastVisited:null}).cards.map(c=>c.thumb))
      .toEqual(['image',null,null,null,null,null,null]);
  });

  it('names the conversation a result came from when it is in the workspace', () => {
    const origin = (sessionKey: string | null) => ({ sessionKey, runId: 'run-1', finishedAt: '2026-10-05T09:00:00Z', status: 'success', verification: 'unverified' });
    const m = resultsCardsModel({
      results: [result('outputs/a.md', { origin: origin('s-1') }), result('outputs/b.md', { origin: origin('s-gone') }), result('outputs/c.md', { origin: origin(null) }), result('outputs/d.md')],
      lastVisited: null,
      sessions: [{ key: 's-1', title: 'Revise the labour section' }],
    });
    expect(m.cards.map(c => c.origin)).toEqual([
      { sessionKey: 's-1', title: 'Revise the labour section', finishedAt: '2026-10-05T09:00:00Z' },
      { sessionKey: 's-gone', title: null, finishedAt: '2026-10-05T09:00:00Z' },
      { sessionKey: null, title: null, finishedAt: '2026-10-05T09:00:00Z' },
      null,
    ]);
  });

  it('keeps the listing order and handles an empty workspace', () => {
    const m = resultsCardsModel({ results: [result('outputs/b.md'), result('outputs/a.md')], lastVisited: null });
    expect(m.cards.map(c => c.name)).toEqual(['outputs/b.md', 'outputs/a.md']);
    expect(resultsCardsModel({ results: [], lastVisited: '2026-10-05T08:00:00Z' })).toEqual({ cards: [], counts: { all: 0, toReview: 0, accepted: 0, sinceVisit: 0 }, newest: null });
  });

  it('re-reads on a vault change under outputs/, or a capped burst in the workspaces folder', () => {
    expect(vaultChangeUnder({ folders: ['workspaces'], paths: [`${WS}/outputs/report.md`] }, WS)).toBe(true);
    expect(vaultChangeUnder({ folders: ['workspaces'], paths: [`${WS}/outputs/canvases/.revisions/x/a.canvas.html`] }, WS)).toBe(true);
    expect(vaultChangeUnder({ folders: ['workspaces'], paths: [`${WS}/WORKSPACE.md`] }, WS)).toBe(false);
    expect(vaultChangeUnder({ folders: ['workspaces'], paths: [`${WS}-other/outputs/report.md`] }, WS)).toBe(false);
    const burst = Array.from({ length: 200 }, (_, i) => `notes/n${i}.md`);
    expect(vaultChangeUnder({ folders: ['notes', 'workspaces'], paths: burst }, WS)).toBe(true);
    expect(vaultChangeUnder({ folders: ['notes'], paths: burst }, WS)).toBe(false);
    expect(vaultChangeUnder(null, WS)).toBe(false);
  });
  it('tells Library → Results when any workspace’s outputs/ changed (P1-19)', () => {
    expect(vaultChangeUnderAnyOutputs({ folders: ['workspaces'], paths: ['workspaces/a/outputs/report.md'] })).toBe(true);
    expect(vaultChangeUnderAnyOutputs({ folders: ['workspaces'], paths: ['workspaces/b/outputs/canvases/x.canvas.html'] })).toBe(true);
    expect(vaultChangeUnderAnyOutputs({ folders: ['workspaces'], paths: ['workspaces/a/WORKSPACE.md', 'workspaces/a/sources/outputs/x.md'] })).toBe(false);
    expect(vaultChangeUnderAnyOutputs({ folders: ['library'], paths: ['library/outputs/x.md'] })).toBe(false);
    const burst = Array.from({ length: 200 }, (_, i) => `notes/n${i}.md`);
    expect(vaultChangeUnderAnyOutputs({ folders: ['notes', 'workspaces'], paths: burst })).toBe(true);
    expect(vaultChangeUnderAnyOutputs({ folders: ['notes'], paths: burst })).toBe(false);
    expect(vaultChangeUnderAnyOutputs(null)).toBe(false);
  });
});
