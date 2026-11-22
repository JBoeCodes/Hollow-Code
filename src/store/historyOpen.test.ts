import { beforeEach, describe, expect, it, vi } from 'vitest';
import chatPageSource from '../components/ConversationHistory.tsx?raw';

vi.hoisted(() => {
  const saved = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => saved.get(key) ?? null,
    setItem: (key: string, value: string) => saved.set(key, value),
    removeItem: (key: string) => saved.delete(key),
  });
});

// Reopening a conversation from Conversation history (merged plan slice 4,
// item 1c / review A·D4). A turn left running in a workspace keeps streaming
// into a hidden buffer; opening that conversation from the history list must
// attach to it (snapshot + buffered events, the turn still busy so the
// composer queues), never load a dead stored copy beside the live stream.
const state = vi.hoisted(() => ({
  key: null as string | null, cwd: null as string | null, sessionId: null as string | null,
  agent: 'claude', busy: false, draft: '', mode: 'bypassPermissions', read: vi.fn(),
  turns: [] as { role: string; text: string }[],
  snapshots: new Map<string, { cwd: string | null; busy: boolean; turns: { role: string; text: string }[] }>(),
  resets: 0, queueRefreshes: [] as number[], order: [] as string[],
}));
vi.mock('./runtime', () => ({
  setChatPermissionMode: (mode: string) => { state.mode = mode; },
  chatBusy: () => state.busy,
  chatAgentSessionKey: () => state.key,
  chatSessionId: () => state.sessionId,
  chatTurns: state.turns,
  KNOWN_AGENTS: ['claude', 'codex'],
  resetChatState: () => {
    if (state.busy) return;
    state.resets++;
    state.key = null; state.cwd = null; state.sessionId = null; state.draft = '';
    state.turns.length = 0;
  },
  restoreBackgroundChat: (key: string) => {
    const s = state.snapshots.get(key);
    if (!s || state.busy) return false;
    state.snapshots.delete(key);
    state.key = key; state.cwd = s.cwd; state.busy = s.busy;
    state.turns.splice(0, state.turns.length, ...s.turns);
    return true;
  },
  setChatAgentPersistent: (agent: string) => { state.agent = agent; },
  setChatAgentSessionKeyPersistent: (key: string | null) => { state.key = key; },
  setChatSessionIdPersistent: (id: string | null) => { state.sessionId = id; },
  setChatCwdPersistent: (cwd: string | null) => { state.cwd = cwd; },
  setChatTurns: (turns: { role: string; text: string }[]) => { state.order.push('turns'); state.turns.splice(0, state.turns.length, ...turns); },
  rebuildChatArtifacts: vi.fn(), setChatStoredTranscriptPending: vi.fn(),
}));
vi.mock('./runQueue', () => ({ refreshRunQueue: (delay: number) => { state.order.push('queue'); state.queueRefreshes.push(delay); } }));
vi.mock('../api', () => ({ api: { readAgentConversation: state.read } }));
import { openHistoryConversation, readHistoryConversation, retireLegacyHistorySelection, showHistoryTurns } from './historyOpen';

beforeEach(() => {
  state.mode = 'bypassPermissions'; state.read.mockReset();
  state.key = 's-other'; state.cwd = '/vault/workspaces/beta'; state.sessionId = 'h-other';
  state.agent = 'claude'; state.busy = false; state.draft = 'draft for another chat';
  state.turns.splice(0, state.turns.length, { role: 'assistant', text: 'Other conversation' });
  state.snapshots.clear(); state.resets = 0; state.queueRefreshes.length = 0; state.order.length = 0;
});

// Run 15 P4 removed the Claude history tab. A Claude Code transcript left
// open from it (a native id, no Hollow key, nothing on screen) can't be
// shown any more; its id must not ride along on the next send (review).
describe('a Claude history selection left from before run 15 P4', () => {
  it('is retired: its native id is cleared and the draft stays', () => {
    state.key = null; state.sessionId = 'claude-native-1'; state.turns.length = 0; state.draft = 'kept draft';
    expect(retireLegacyHistorySelection()).toBe(true);
    expect(state.sessionId).toBeNull();
    expect(state.draft).toBe('kept draft');
  });
  it('leaves a Hollow conversation, a conversation on screen and a running turn alone', () => {
    state.key = 's-1'; state.sessionId = 'h-1'; state.turns.length = 0;
    expect(retireLegacyHistorySelection()).toBe(false);
    expect(state.sessionId).toBe('h-1');
    state.key = null; state.turns.splice(0, state.turns.length, { role: 'user', text: 'hi' });
    expect(retireLegacyHistorySelection()).toBe(false);
    state.turns.length = 0; state.busy = true;
    expect(retireLegacyHistorySelection()).toBe(false);
    expect(state.sessionId).toBe('h-1');
  });
  it('runs when Conversation history mounts, before its restore', () => {
    const flat = chatPageSource.replace(/\s+/g, '');
    expect(flat.indexOf('retireLegacyHistorySelection();')).toBeGreaterThan(-1);
    expect(flat.indexOf('retireLegacyHistorySelection();')).toBeLessThan(flat.indexOf('letrestorePending='));
  });
});

describe('opening a conversation from Conversation history', () => {
  it('attaches to a turn still running in the background instead of loading a stored copy', () => {
    state.snapshots.set('s-live', { cwd: '/vault/workspaces/alpha', busy: true, turns: [{ role: 'assistant', text: 'Still streaming' }] });
    const how = openHistoryConversation({ key: 's-live', agent: 'codex', sessionId: 'h-live', cwd: '/vault/workspaces/alpha' });
    expect(how).toBe('live');
    expect(state.key).toBe('s-live');
    // Still running: the composer queues behind it rather than starting a
    // second concurrent turn in the same conversation.
    expect(state.busy).toBe(true);
    expect(state.turns).toEqual([{ role: 'assistant', text: 'Still streaming' }]);
    // One source of truth: the hidden buffer is handed to the screen.
    expect(state.snapshots.has('s-live')).toBe(false);
    // The previous conversation's draft does not follow into this one.
    expect(state.draft).toBe('');
  });

  it('brings back a conversation that finished in the background from its buffer', () => {
    state.snapshots.set('s-done', { cwd: null, busy: false, turns: [{ role: 'assistant', text: 'Final reply' }] });
    expect(openHistoryConversation({ key: 's-done', agent: 'claude', sessionId: null, cwd: null })).toBe('live');
    expect(state.turns).toEqual([{ role: 'assistant', text: 'Final reply' }]);
    expect(state.snapshots.size).toBe(0);
  });

  it('points the chat at a stored conversation and asks the caller to load it', () => {
    expect(openHistoryConversation({ key: 's-stored', agent: 'codex', sessionId: 'h-stored', cwd: '/vault/workspaces/alpha' })).toBe('load');
    expect(state).toMatchObject({ key: 's-stored', agent: 'codex', sessionId: null, mode: 'acceptEdits', cwd: '/vault/workspaces/alpha', busy: false });
    expect(state.turns).toEqual([]);
    expect(state.resets).toBe(1);
  });

  it('does not resume a legacy native id whose policy is unknown', () => {
    expect(openHistoryConversation({ key: null, agent: 'claude', sessionId: 'claude-jsonl', cwd: null })).toBe('load');
    expect(state.key).toBeNull();
    expect(state.sessionId).toBeNull();
  });

  it('changes nothing while a turn is on screen', () => {
    state.busy = true;
    state.snapshots.set('s-live', { cwd: null, busy: true, turns: [] });
    expect(openHistoryConversation({ key: 's-live', agent: 'claude', sessionId: null, cwd: null })).toBeNull();
    expect(state.key).toBe('s-other');
    expect(state.snapshots.has('s-live')).toBe(true);
    expect(state.resets).toBe(0);
  });

  it('is what the Conversation history page opens through, including app-tool navigation', () => {
    const flat = chatPageSource.replace(/\s+/g, '');
    expect(flat).toContain('openHistoryConversation(');
    // The page no longer points the chat at a key itself, which is how it
    // showed a stored copy of a conversation still streaming elsewhere.
    expect(flat).not.toContain('setChatAgentSessionKeyPersistent(');
    // navigate(session) used to call loadSessionTurns directly, skipping
    // both the key change and the live attach. A handed-over conversation
    // opens through openSession whether Chat's list has it, the sender
    // handed its summary over, or it is looked up by exact key.
    expect(flat).toContain('if(\'open\'instep){voidopenSession(entryOf(step.open));return;}');
    expect(flat).toContain('elsevoidopenSession(entryOf(target.session));');
  });
});

it('leaves a failed history load conservative and unable to resume native', async () => {
  state.read.mockRejectedValue(new Error('unreadable'));
  await expect(readHistoryConversation('failed', () => true)).rejects.toThrow('unreadable');
  expect(state.mode).toBe('acceptEdits');
  expect(state.sessionId).toBeNull();
});

it('keeps this chat in saved Plan after its history read fails', async () => {
  const { rememberConversationMode } = await import('../permModes');
  rememberConversationMode('failed-plan', 'plan');
  state.read.mockRejectedValue(new Error('unreadable'));
  await expect(readHistoryConversation('failed-plan', () => true)).rejects.toThrow('unreadable');
  expect(state.mode).toBe('plan'); expect(state.sessionId).toBeNull();
});

it('ignores an old history response after another conversation opens', async () => {
  let resolve!: (value: unknown) => void;
  state.read.mockReturnValue(new Promise(r => { resolve = r; }));
  let current = true;
  const pending = readHistoryConversation('stale', () => current);
  expect(state.mode).toBe('acceptEdits'); expect(state.sessionId).toBeNull();
  current = false; state.mode = 'plan'; state.sessionId = 'new-native';
  resolve({ permissionMode: 'bypassPermissions', summary: { resumable: 'native', harnessSessionId: 'stale-native' }, turns: [] });
  expect(await pending).toBeNull();
  expect(state.mode).toBe('plan'); expect(state.sessionId).toBe('new-native');
});

it('restores the loaded history policy before allowing its native resume', async () => {
  state.read.mockResolvedValue({ permissionMode: 'plan', summary: { resumable: 'native', harnessSessionId: 'own-native' }, turns: [] });
  await readHistoryConversation('known', () => true);
  expect(state.mode).toBe('plan'); expect(state.sessionId).toBe('own-native');
});

// Overnight 11-13 c-11: a conversation still running after a reload, opened
// from Library, Conversations, read only the store, so its run was never
// adopted (no Stop, no working mark).
describe('a running conversation opened from Conversation history after a reload', () => {
  it('shows its stored turns, then reads the run queue at once to adopt the run', () => {
    showHistoryTurns([{ role: 'user', text: 'Still working on this' }] as never);
    expect(state.turns).toEqual([{ role: 'user', text: 'Still working on this' }]);
    expect(state.queueRefreshes).toEqual([0]);
    expect(state.order).toEqual(['turns', 'queue']);
  });
  it('is how the page shows a loaded transcript', () => {
    const flat = chatPageSource.replace(/\s+/g, '');
    expect(flat).toContain('if(!current())return;showHistoryTurns(turns);');
  });
});
