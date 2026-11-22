import { describe, expect, it } from 'vitest';
import { missionTracks, trackPhase, trackCost } from './missionTracks';
import { missionCards, applyFeedEvent, type MissionCard } from './missionControl';
const base = missionCards([], applyFeedEvent({}, { type: 'runStarted', conversationKey: 'x' }, 100), () => null, 100).live[0];
const card = (id: string, over: Partial<MissionCard> = {}): MissionCard => ({ ...base, id, ...over });

describe('mission tracks evidence', () => {
  it('keeps actual workspace identities, chronological segments and live lanes first', () => {
    const groups = missionTracks([card('live', { system: 'a', systemTitle: 'Same', startedAt: 20 })], [
      card('done', { status: 'done', system: 'b', systemTitle: 'Same', startedAt: 1 }),
      card('earlier', { status: 'done', system: 'a', systemTitle: 'Same', startedAt: 10 }),
    ]);
    expect(groups.map(g => g.id)).toEqual(['a', 'b']);
    expect(groups[0].cards.map(c => c.id)).toEqual(['earlier', 'live']);
  });
  it('does not turn a finished claim about a merge into a merged result', () => {
    expect(trackPhase(card('done', { status: 'done', lines: ['Merged PR 123, all gates green'] }), {})).toBe('Finished');
  });
  it('only labels an active matching shell command as gating or building', () => {
    let feed = applyFeedEvent({}, { type: 'runStarted', conversationKey: 'x', queueId: 4 }, 100);
    feed = applyFeedEvent(feed, { type: 'toolUse', conversationKey: 'x', name: 'Bash', input: { command: 'npm run gate' } }, 101);
    expect(trackPhase(card('a', { queueId: 4, tool: 'shell' }), feed)).toBe('Gating');
    expect(trackPhase(card('a', { queueId: 9, tool: 'shell' }), feed)).toBe('Running');
    feed = applyFeedEvent(feed, { type: 'toolResult', conversationKey: 'x' }, 102);
    expect(trackPhase(card('a', { queueId: 4 }), feed)).toBe('Running');
    feed = applyFeedEvent(feed, { type: 'toolUse', conversationKey: 'x', name: 'Bash', input: { command: 'npm run build' } }, 103);
    expect(trackPhase(card('a', { queueId: 4 }), feed)).toBe('Building');
    feed = applyFeedEvent(feed, { type: 'toolUse', conversationKey: 'x', name: 'Read', input: { command: 'npm run gate' } }, 104);
    expect(trackPhase(card('a', { queueId: 4 }), feed)).toBe('Running');
  });
  it('keeps unknown, invalid, running and duplicate costs out of known totals', () => {
    const a = card('a', { status: 'done', costUsd: 0 });
    const b = card('b', { status: 'done', costUsd: 1.23 });
    expect(trackCost([a,b,a,card('c'),card('d', { costUsd: NaN }),card('e', { status: 'done', costUsd: -1 })]))
      .toEqual({ knownUsd: 1.23, known: 2, unknown: 1, pending: 2, total: 5 });
    expect(trackCost([])).toEqual({ knownUsd: null, known: 0, unknown: 0, pending: 0, total: 0 });
  });
});
