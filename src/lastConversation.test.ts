import { describe, expect, it } from 'vitest';
import { launchLanding, launchTarget, parseLastConversation, workspaceConversationKeys } from './lastConversation';

describe('last conversation restore (frontier F4)', () => {
  it('parses only a well-formed record', () => {
    expect(parseLastConversation(null)).toBeNull();
    expect(parseLastConversation('not json')).toBeNull();
    expect(parseLastConversation('{"key":"s-1"}')).toBeNull();
    expect(parseLastConversation('{"workspaceId":"a","key":"s-1"}')).toEqual({ workspaceId: 'a', key: 's-1' });
    expect(parseLastConversation('{"workspaceId":"a","key":null}')).toEqual({ workspaceId: 'a', key: null });
    expect(parseLastConversation('{"workspaceId":"a","key":7}')).toEqual({ workspaceId: 'a', key: null });
  });

  it('reopens the saved conversation, else the workspace (a new unsent one, or a legacy record)', () => {
    expect(launchTarget({ workspaceId: 'a', key: 's-1' }, 'b')).toEqual({ workspaceId: 'a', key: 's-1' });
    expect(launchTarget({ workspaceId: 'a', key: null }, null)).toEqual({ workspaceId: 'a' });
    // A build before F4 saved only the workspace: its newest conversation opens.
    expect(launchTarget(null, 'b')).toEqual({ workspaceId: 'b' });
    expect(launchTarget(null, null)).toBeNull();
  });

  it('opens the workspace when the conversation is gone, and Home when the workspace is', () => {
    expect(launchLanding({ workspaceId: 'a', key: 's-1' }, ['s-1', 's-2'])).toBe('open');
    expect(launchLanding({ workspaceId: 'a', key: 's-9' }, ['s-1'])).toBe('workspace');
    expect(launchLanding({ workspaceId: 'a' }, [])).toBe('open');
    expect(launchLanding({ workspaceId: 'a', key: 's-1' }, null)).toBe('home');
  });
});

describe('a project COO before its first turn (overnight 11-03 R4)', () => {
  it('counts as one of the workspace conversations, so launch reopens it', () => {
    const detail = { sessions: [{ key: 's-1' }], projectAgentKey: 's-main' };
    expect(workspaceConversationKeys(detail)).toEqual(['s-1', 's-main']);
    expect(launchLanding({ workspaceId: 'a', key: 's-main' }, workspaceConversationKeys(detail))).toBe('open');
    expect(workspaceConversationKeys({ sessions: [{ key: 's-main' }], projectAgentKey: 's-main' })).toEqual(['s-main']);
    expect(workspaceConversationKeys({ sessions: [], projectAgentKey: null })).toEqual([]);
  });
});
