import { describe, expect, it } from 'vitest';
import { bufferEvent, routeEvent, type RouteContext } from './chatRouting';

const cx = (over: Partial<RouteContext> = {}): RouteContext => ({
  isBackground: key => key === 's-bg', foregroundKey: 's-fg', queued: null, ...over,
});

describe('chat event routing', () => {
  it('keeps untagged and on-screen events on screen', () => {
    expect(routeEvent({ type: 'assistantText' }, cx())).toBe('foreground');
    expect(routeEvent({ type: 'assistantText', conversationKey: 's-fg' }, cx())).toBe('foreground');
  });
  it('buffers a background conversation and drops a stranger', () => {
    expect(routeEvent({ type: 'assistantDelta', conversationKey: 's-bg' }, cx())).toBe('background');
    expect(routeEvent({ type: 'assistantDelta', conversationKey: 's-other' }, cx())).toBe('orphan');
  });
  it('lets a fresh conversation adopt its key from its own first turn', () => {
    const fresh = cx({ foregroundKey: null, queued: { clientTurnId: 'ct-1', queueId: 9 } });
    expect(routeEvent({ type: 'runStarted', conversationKey: 's-new', clientTurnId: 'ct-1' }, fresh)).toBe('foreground');
    expect(routeEvent({ type: 'runStarted', conversationKey: 's-new', queueId: 9 }, cx({ queued: { queueId: 9 } }))).toBe('foreground');
    // An unsent conversation must not adopt an unrelated agent-started turn.
    expect(routeEvent({ type: 'runStarted', conversationKey: 's-new' }, cx({ foregroundKey: null }))).toBe('orphan');
    // A background key wins even over a matching queued turn.
    expect(routeEvent({ type: 'done', conversationKey: 's-bg', queueId: 9 }, cx({ queued: { queueId: 9 } }))).toBe('background');
  });
  it('joins consecutive deltas and keeps everything else in order', () => {
    const events: Array<{ type: string; text?: string }> = [];
    bufferEvent(events, { type: 'assistantDelta', text: 'Hel' });
    bufferEvent(events, { type: 'assistantDelta', text: 'lo' });
    bufferEvent(events, { type: 'toolUse' });
    bufferEvent(events, { type: 'assistantDelta', text: '!' });
    expect(events).toEqual([{ type: 'assistantDelta', text: 'Hello' }, { type: 'toolUse' }, { type: 'assistantDelta', text: '!' }]);
  });
});
