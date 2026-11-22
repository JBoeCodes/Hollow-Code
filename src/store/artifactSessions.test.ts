import { beforeEach, describe, expect, it, vi } from 'vitest';
import chatRuntimeSource from './chatRuntime.ts?raw';

// Continuing an artifact's session natively (merged plan slice 4, item 1c /
// review A·D4, follow-up). A conversation whose turn finished while it was in
// the background keeps its background entry until it is shown. Adopting its
// key on screen without dropping that entry routes every event of the new
// turn into the hidden buffer (routeEvent checks the background map first),
// so the screen shows a dead turn that stays busy, and the stale entry can
// later be restored over the newer turns: two sources of truth.
const state = vi.hoisted(() => ({
  key: 's-other' as string | null, busy: false, draft: '', mode: 'bypassPermissions', native: null as string | null, holds: 0,
  turns: [] as unknown[],
  continuedFrom: ['s-stale'] as string[],
  background: new Map<string, { done: boolean }>(),
  summary: {
    key: 's-art', agent: 'claude', cwd: '/vault/workspaces/alpha',
    harnessSessionId: 'h-art', resumable: 'native',
  },
}));
vi.mock('../api', () => ({
  api: {
    readArtifactSessionContext: async () => ({ detail: { summary: state.summary, turns: [] }, permissionMode: 'plan', receipts: [], answers: {} }),
    agentAvailable: async () => ['claude', 'codex'],
    chatPreflight: async () => null,
    agentSessionLive: async () => null,
  },
}));
vi.mock('../artifactSession', () => ({
  artifactContinuationPrompt: () => 'Continue the artifact.',
  artifactSessionTurns: () => [],
  canResumeArtifactSession: () => true,
}));
vi.mock('./runtime', () => ({
  chatBusy: () => state.busy, setChatBusy: (b: boolean) => { state.busy = b; },
  chatInput: () => state.draft, setChatInput: (s: string) => { state.draft = s; },
  resetChatState: () => { if (state.busy) return; state.key = null; state.native = null; state.turns.length = 0; state.continuedFrom = []; },
  canLeaveConversation: () => !state.holds,
  holdConversationForSend: () => { state.holds++; return () => { state.holds--; }; },
  setChatPermissionMode: (mode: string) => { state.mode = mode; },
  applyNewChatPermissionMode: () => { state.mode = 'acceptEdits'; },
  setChatContinuedFrom: (keys: string[]) => { state.continuedFrom = keys; },
  setChatAgentPersistent: vi.fn(), setChatSessionIdPersistent: (id: string | null) => { state.native = id; }, setChatCwdPersistent: vi.fn(),
  setChatAgentSessionKeyPersistent: (key: string | null) => { state.key = key; },
  setChatTurns: (t: unknown[]) => { state.turns.splice(0, state.turns.length, ...t); },
  pushChatTurn: (t: unknown) => { state.turns.push(t); },
  setActivePage: vi.fn(), setChatPanelOpen: vi.fn(), requestChatFocus: vi.fn(),
  KNOWN_AGENTS: ['claude', 'codex'],
  isBackgrounded: (key: string) => state.background.has(key),
  isBackgroundChat: (key: string) => state.background.get(key)?.done === false,
  discardBackgroundChat: (key: string) => { state.background.delete(key); },
}));
import { continueArtifactSession } from './artifactSessions';

beforeEach(() => {
  state.mode = 'bypassPermissions'; state.native = null; state.holds = 0;
  state.key = 's-other'; state.busy = false; state.draft = '';
  state.turns.length = 0; state.background.clear();
  state.continuedFrom = ['s-stale'];
});

describe('continuing an artifact session natively', () => {
  it('drops a finished background entry for that conversation before adopting its key', async () => {
    state.background.set('s-art', { done: true });
    await continueArtifactSession('s-art', 'notes/a.md');
    expect(state.key).toBe('s-art');
    // One source of truth: the new on-screen turn's events must reach the
    // screen, not a hidden buffer that would later be restored over it.
    expect(state.background.has('s-art')).toBe(false);
    expect(state.busy).toBe(false);
    expect(state.mode).toBe('plan'); expect(state.native).toBe('h-art');
    expect(state.holds).toBe(0);
  });

  it('refuses while that conversation still runs in the background, keeping the current chat', async () => {
    state.background.set('s-art', { done: false });
    await expect(continueArtifactSession('s-art', 'notes/a.md')).rejects.toThrow(/Open this conversation/);
    expect(state.key).toBe('s-other');
    expect(state.background.has('s-art')).toBe(true);
    expect(state.busy).toBe(false);
  });

  it('leaves other background conversations alone', async () => {
    state.background.set('s-elsewhere', { done: true });
    await continueArtifactSession('s-art', 'notes/a.md');
    expect(state.key).toBe('s-art');
    expect(state.background.has('s-elsewhere')).toBe(true);
  });

  it('discardBackgroundChat deletes the entry and refreshes the rail keys', () => {
    const flat = chatRuntimeSource.replace(/\s+/g, '');
    const start = flat.indexOf('exportfunctiondiscardBackgroundChat(');
    expect(start).toBeGreaterThan(-1);
    const body = flat.slice(start, flat.indexOf('}', flat.indexOf('{', start)) + 1);
    expect(body).toContain('backgroundChats.delete(key)');
    expect(body).toContain('syncBackgroundKeys()');
  });
});

// Agent effectiveness F4 (K1 review): the composer no longer builds the
// conversation bundle, so a Continue from summary must name the conversation
// it continues. The backend's context compiler then bundles that
// conversation's recorded turns on the new conversation's first fresh turn,
// as the old composer did from the turns it showed.
describe('continuing an artifact session from its summary', () => {
  it('names the source conversation for the next send', async () => {
    await continueArtifactSession('s-art', 'notes/a.md', true);
    expect(state.key).toBeNull();
    expect(state.continuedFrom).toEqual(['s-art']);
    expect(state.mode).toBe('acceptEdits'); expect(state.native).toBeNull();
  });

  it('a native continue names none: the resumed session already holds its history', async () => {
    await continueArtifactSession('s-art', 'notes/a.md');
    expect(state.key).toBe('s-art');
    expect(state.continuedFrom).toEqual([]);
  });
});

it('keeps the conversation but clears native resume after a changed choice', async () => {
  const { rememberConversationMode } = await import('../permModes');
  rememberConversationMode('s-art', 'acceptEdits');
  await continueArtifactSession('s-art', 'notes/a.md');
  expect(state.key).toBe('s-art'); expect(state.mode).toBe('acceptEdits');
  expect(state.native).toBeNull(); expect(state.holds).toBe(0);
});
