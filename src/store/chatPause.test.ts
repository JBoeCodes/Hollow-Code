import { beforeEach, expect, it, vi } from 'vitest';
import { pausedNotice, RESUME_PROMPT } from '../chatStop';

// P1a PR 2: Pause, the `paused` event and Resume in the chat runtime. The
// backend (PR 1) ends the turn at a safe point and emits `paused` after the
// receipt and before `done`; Resume queues an ordinary turn the composer
// adopts like any send made while busy.
const h = vi.hoisted(() => {
  const values = new Map<string, string>();
  vi.stubGlobal('localStorage', { getItem: (k: string) => values.get(k) ?? null, setItem: (k: string, v: string) => values.set(k, v), removeItem: (k: string) => values.delete(k) });
  return {
    listeners: new Map<string, (e: { payload: unknown }) => void>(),
    calls: [] as Array<[string, unknown]>,
    pauseReached: true as boolean | Error,
    resume: null as null | Error | { sessionKey: string; queueId: number; queuePosition: number; phase: string; waitingFor: null },
    live: null as null | { phase: string; phaseEnteredAt: string },
    flashes: [] as string[],
    // P1b, P1c: what the pause-all, list and resume-time commands answer.
    pauseAll: [] as string[] | Error,
    resumeAll: { resumed: [] as string[], refused: [] as [string, string][], skipped: [] as string[] },
    pausedList: [] as unknown[],
    setResumeAt: null as null | Error | Record<string, unknown>,
  };
});
vi.mock('../nativeEvents', () => ({ listen: (name: string, fn: (e: { payload: unknown }) => void) => { h.listeners.set(name, fn); return Promise.resolve(() => {}); } }));
vi.mock('../api', () => ({
  api: new Proxy({}, {
    get: (_t, name: string) => {
      if (name === 'chatPause') return vi.fn(async (key: string) => { h.calls.push(['chatPause', key]); if (h.pauseReached instanceof Error) throw h.pauseReached; return h.pauseReached; });
      if (name === 'chatResumePaused') return vi.fn(async (key: string) => { h.calls.push(['chatResumePaused', key]); if (h.resume instanceof Error) throw h.resume; return h.resume; });
      if (name === 'agentSessionLive') return vi.fn(async (key?: string) => { h.calls.push(['agentSessionLive', key]); return h.live ? { key, agent: 'claude', runId: '', failureReason: null, lastEventAt: null, progress: null, ...h.live } : null; });
      if (name === 'chatCancel') return vi.fn(async (key: string) => { h.calls.push(['chatCancel', key]); });
      if (name === 'chatPauseAll') return vi.fn(async () => { h.calls.push(['chatPauseAll', null]); if (h.pauseAll instanceof Error) throw h.pauseAll; return h.pauseAll; });
      if (name === 'chatResumeAll') return vi.fn(async () => { h.calls.push(['chatResumeAll', null]); return h.resumeAll; });
      if (name === 'chatPausedList') return vi.fn(async () => { h.calls.push(['chatPausedList', null]); return h.pausedList; });
      if (name === 'chatSetResumeAt') return vi.fn(async (key: string, at: number | null) => { h.calls.push(['chatSetResumeAt', [key, at]]); if (h.setResumeAt instanceof Error) throw h.setResumeAt; return h.setResumeAt; });
      return vi.fn(async () => null);
    },
  }),
}));
vi.mock('./runtime', () => ({ activePage: () => 'today', setActivePage: vi.fn(), closeChatPanel: vi.fn(), refetchShownSessions: vi.fn(), requestChatFocus: vi.fn(), setChatPanelOpen: vi.fn() }));
vi.mock('./overlays', () => ({ showConfirm: vi.fn() }));
vi.mock('./snackbars', () => ({ flashFooter: (text: string) => h.flashes.push(text) }));

const KEY = 's-20261115-pause001';
const emit = (payload: Record<string, unknown>) => h.listeners.get('chat-event')!({ payload: { conversationKey: KEY, sessionKey: KEY, queueId: 7, ...payload } });
const settle = () => new Promise(resolve => setTimeout(resolve, 0));

beforeEach(() => {
  h.calls.length = 0;
  h.flashes.length = 0;
  h.pauseReached = true;
  h.resume = { sessionKey: KEY, queueId: 8, queuePosition: 1, phase: 'queued', waitingFor: null };
  h.live = null;
  h.pauseAll = [];
  h.resumeAll = { resumed: [], refused: [], skipped: [] };
  h.pausedList = [];
  h.setResumeAt = null;
});

/** A fresh chat runtime with a started turn of KEY on screen. */
async function runningChat() {
  vi.resetModules();
  const rt = await import('./chatRuntime');
  const pause = await import('./chatPause');
  pause.resetChatPauseState();
  rt.setChatBusy(false); rt.resetChatState();
  rt.setChatAgentPersistent('claude');
  rt.setChatAgentSessionKeyPersistent(KEY);
  rt.setChatQueuedTurn({
    clientTurnId: 'turn-7', queueId: 7, sessionKey: KEY, text: 'hello', attachments: [], requestedCwd: null,
    waitingFor: null, waitingSession: null, queuePosition: null, started: true, intent: null, adopted: false, recovered: false,
  });
  emit({ type: 'runStarted', runId: 'run-7', clientTurnId: 'turn-7', recall: [] });
  await settle();
  h.calls.length = 0;
  return { rt, pause };
}

it('Pause asks the backend by the chat’s key and reads Pausing until the turn ends', async () => {
  const { rt } = await runningChat();
  expect(rt.canPauseChatRun()).toBe(true);
  await rt.pauseChatRun();
  expect(h.calls).toEqual([['chatPause', KEY]]);
  expect(rt.chatPausingHere()).toBe(true);
  // A second click while pausing asks nothing more.
  await rt.pauseChatRun();
  expect(h.calls).toHaveLength(1);
  // Stop still works while pausing, and its end clears Pausing.
  rt.stopChatRun();
  expect(h.calls.at(-1)).toEqual(['chatCancel', KEY]);
  emit({ type: 'cancelled', stop: 'verified' });
  emit({ type: 'done', sessionId: 'native-1', clientTurnId: 'turn-7' });
  expect(rt.chatPausingHere()).toBe(false);
  expect(rt.chatPausedHere()).toBeNull();
  expect(rt.chatBusy()).toBe(false);
});

it('a pause the backend could not deliver, or a failed ask, says so and leaves the turn as it was', async () => {
  const { rt } = await runningChat();
  h.pauseReached = false;
  await rt.pauseChatRun();
  expect(rt.chatPausingHere()).toBe(false);
  expect(h.flashes.at(-1)).toMatch(/Nothing to pause/);
  h.pauseReached = new Error('boom');
  await rt.pauseChatRun();
  expect(rt.chatPausingHere()).toBe(false);
  expect(h.flashes.at(-1)).toBe('× Couldn’t pause: boom');
  expect(rt.chatBusy()).toBe(true);
});

it('the paused event leaves a neutral notice, ends the turn with done and shows Resume', async () => {
  const { rt } = await runningChat();
  await rt.pauseChatRun();
  emit({ type: 'paused', note: 'Reading the plan.', at: 1_700_000_000 });
  emit({ type: 'done', sessionId: 'native-1', clientTurnId: 'turn-7' });
  const last = rt.chatTurns.at(-1)!;
  expect([last.role, last.text]).toEqual(['system', pausedNotice('Reading the plan.')]);
  expect(last.stopNotice).toBeFalsy();
  expect(rt.chatBusy()).toBe(false);
  expect(rt.chatPausingHere()).toBe(false);
  expect(rt.chatPausedHere()).toEqual({ note: 'Reading the plan.', at: 1_700_000_000, reason: 'user', resumeAt: null });
});

it('Resume queues the backend’s turn and adopts it like a busy send: busy, queued, its bubble drawn', async () => {
  const { rt } = await runningChat();
  emit({ type: 'paused', note: 'Reading the plan.', at: 1_700_000_000 });
  emit({ type: 'done', sessionId: 'native-1', clientTurnId: 'turn-7' });
  expect(await rt.resumePausedChat()).toBe(true);
  expect(h.calls).toContainEqual(['chatResumePaused', KEY]);
  expect(rt.chatPausedHere()).toBeNull();
  expect(rt.chatBusy()).toBe(true);
  expect(rt.chatQueuedTurn()).toMatchObject({ sessionKey: KEY, queueId: 8, started: false });
  expect(rt.chatTurns.filter(t => t.role === 'user').at(-1)?.text).toBe(RESUME_PROMPT);
  // Its own events carry it from here (matched by queue id).
  emit({ type: 'runStarted', runId: 'run-8', queueId: 8, clientTurnId: null, recall: [] });
  expect(rt.chatQueuedTurn()?.started).toBe(true);
  emit({ type: 'done', sessionId: 'native-1', queueId: 8, clientTurnId: null });
  expect(rt.chatBusy()).toBe(false);
});

it('a refused Resume says why and reads the record again; a stale paused event never brings it back', async () => {
  const { rt, pause } = await runningChat();
  emit({ type: 'paused', note: 'x', at: 1_700_000_000 });
  emit({ type: 'done', sessionId: 'native-1', clientTurnId: 'turn-7' });
  h.resume = new Error("This chat isn't paused. Nothing started.");
  expect(await rt.resumePausedChat()).toBe(false);
  expect(h.flashes.at(-1)).toBe("× Couldn’t resume: This chat isn't paused. Nothing started.");
  await settle();
  expect(h.calls).toContainEqual(['agentSessionLive', KEY]);
  expect(rt.chatPausedHere()).toBeNull();
  // The person resumed: a replayed paused event from before is ignored,
  // judged by that event's own `at`, so a new pause a moment later shows
  // even when the clock is still in the same second (review fix 2).
  const now = Math.floor(Date.now() / 1000);
  pause.markChatPaused(KEY, 'y', now - 1);
  pause.clearChatPaused(KEY, true);
  pause.markChatPaused(KEY, 'replayed', now - 1);
  expect(rt.chatPausedHere()).toBeNull();
  pause.markChatPaused(KEY, 'again', now);
  expect(rt.chatPausedHere()).toEqual({ note: 'again', at: now });
});

it('a second Resume while the first is in flight asks nothing; the on-screen running chat says why (review fixes 1, 3)', async () => {
  const { rt, pause } = await runningChat();
  emit({ type: 'paused', note: 'x', at: 1_700_000_000 });
  emit({ type: 'done', sessionId: 'native-1', clientTurnId: 'turn-7' });
  const first = rt.resumePausedChat();
  expect(pause.chatResumingFor(KEY)).toBe(true);
  expect(await rt.resumePausedChat()).toBe(false);
  expect(await first).toBe(true);
  expect(pause.chatResumingFor(KEY)).toBe(false);
  expect(h.calls.filter(c => c[0] === 'chatResumePaused')).toHaveLength(1);
  // Busy again now: Mission Control's Resume on it flashes a note.
  expect(await rt.resumePausedChat(KEY)).toBe(false);
  expect(h.flashes.at(-1)).toBe('This chat is already running.');
});

it('a running phase clears a stale Pausing flag (review fix 4)', async () => {
  const { pause } = await runningChat();
  pause.setChatPausing(KEY, true);
  h.live = { phase: 'running', phaseEnteredAt: '2026-11-15T12:00:00Z' };
  await pause.refreshChatPaused(KEY);
  expect(pause.chatPausingFor(KEY)).toBe(false);
});

it('a turn of the chat starting clears paused (a message sent while paused)', async () => {
  const { rt } = await runningChat();
  emit({ type: 'paused', note: 'x', at: 1_700_000_000 });
  emit({ type: 'done', sessionId: 'native-1', clientTurnId: 'turn-7' });
  expect(rt.chatPausedHere()).not.toBeNull();
  emit({ type: 'runStarted', runId: 'run-9', queueId: 9, recall: [] });
  expect(rt.chatPausedHere()).toBeNull();
});

it('another chat’s paused event marks that chat only, never the transcript on screen', async () => {
  const { rt, pause } = await runningChat();
  const before = rt.chatTurns.length;
  h.listeners.get('chat-event')!({ payload: { type: 'paused', conversationKey: 'other', sessionKey: 'other', note: 'n', at: 1 } });
  expect(rt.chatTurns.length).toBe(before);
  expect(pause.chatPausedFor('other')).toEqual({ note: 'n', at: 1, reason: 'user', resumeAt: null });
  expect(rt.chatPausedHere()).toBeNull();
});

it('opening a conversation reads its paused record (after a reload no event was seen)', async () => {
  const { rt, pause } = await runningChat();
  rt.setChatBusy(false); rt.resetChatState();
  h.live = { phase: 'paused', phaseEnteredAt: '2026-11-15T12:00:00Z' };
  rt.setChatAgentSessionKeyPersistent('s-20261115-pause002');
  await settle();
  expect(h.calls).toContainEqual(['agentSessionLive', 's-20261115-pause002']);
  expect(pause.chatPausedFor('s-20261115-pause002')).toEqual({ note: '', at: Date.parse('2026-11-15T12:00:00Z') / 1000 });
  expect(rt.chatPausedHere()).not.toBeNull();
  // Not paused any more by the next read: it clears.
  h.live = null;
  await pause.refreshChatPaused('s-20261115-pause002');
  expect(rt.chatPausedHere()).toBeNull();
});

// ── P1b to P1d: Pause all, Resume all, resume at a time, a limit pause ──────

const OTHER = 's-20261115-pause002';
const record = (key: string, more: Record<string, unknown> = {}) => ({ sessionKey: key, nativeSessionId: 'n', agent: 'claude', pausedAt: 1_700_000_100, note: 'Writing the report.', reason: 'all', resumeAt: null, ...more });

it('a paused event keeps why and when it resumes (a usage limit with its reset)', async () => {
  const { rt } = await runningChat();
  emit({ type: 'paused', note: 'Claude hit its usage limit. It resets at 16:40; the chat resumes then.', at: 1_700_000_000, reason: 'usage_limit', resumeAt: 1_700_003_660 });
  emit({ type: 'done', sessionId: 'native-1', clientTurnId: 'turn-7' });
  expect(rt.chatPausedHere()).toMatchObject({ reason: 'usage_limit', resumeAt: 1_700_003_660 });
  // Stop, Resume and a new message still work on it: Resume clears it.
  expect(await rt.resumePausedChat()).toBe(true);
  expect(rt.chatPausedHere()).toBeNull();
});

it('Pause all marks every asked chat Pausing until its paused event, and says how many', async () => {
  const { pause } = await runningChat();
  const actions = await import('./chatPauseActions');
  h.pauseAll = [KEY, OTHER];
  expect(await actions.pauseAllChats()).toBe(2);
  expect(h.calls).toEqual([['chatPauseAll', null]]);
  expect(pause.chatPausingFor(KEY)).toBe(true);
  expect(pause.chatPausingFor(OTHER)).toBe(true);
  emit({ type: 'paused', note: 'Reading the plan.', at: 1_700_000_000, reason: 'all', resumeAt: null });
  expect(pause.chatPausingFor(KEY)).toBe(false);
  expect(pause.chatPausedFor(KEY)).toMatchObject({ reason: 'all' });
  // Nothing running: nothing asked.
  h.pauseAll = [];
  expect(await actions.pauseAllChats()).toBe(0);
  h.pauseAll = new Error('nope');
  await expect(actions.pauseAllChats()).rejects.toThrow('nope');
});

it('Resume all clears what it resumed and reads the list again for the rest', async () => {
  const { pause } = await runningChat();
  const actions = await import('./chatPauseActions');
  pause.markChatPaused(KEY, 'a', 10);
  pause.markChatPaused(OTHER, 'b', 11);
  h.resumeAll = { resumed: [KEY], refused: [[OTHER, 'This chat already has a turn running or queued. Nothing started.']], skipped: ['s-waiting'] };
  h.pausedList = [record(OTHER)];
  const r = await actions.resumeAllChats();
  // Review fix: a chat waiting for its own time (a limit's reset) is skipped.
  expect(r).toEqual({ resumed: 1, refused: 1, skipped: 1, firstError: 'This chat already has a turn running or queued. Nothing started.' });
  expect(pause.chatPausedFor(KEY)).toBeNull();
  expect(pause.chatPausedFor(OTHER)).toMatchObject({ note: 'Writing the report.', reason: 'all' });
  expect(actions.pausedCount()).toBe(1);
});

it('the paused list is the backend’s: listed chats are paused with their time, others are not', async () => {
  const { pause } = await runningChat();
  const actions = await import('./chatPauseActions');
  pause.markChatPaused(KEY, 'stale', 10);
  h.pausedList = [record(OTHER, { resumeAt: 1_800_000_000 })];
  await actions.refreshPausedChats();
  expect(pause.chatPausedFor(KEY)).toBeNull();
  expect(pause.chatPausedFor(OTHER)).toEqual({ note: 'Writing the report.', at: 1_700_000_100, reason: 'all', resumeAt: 1_800_000_000 });
});

it('Resume at sets the time, Clear time clears it, a refusal says why and changes nothing', async () => {
  const { pause } = await runningChat();
  const actions = await import('./chatPauseActions');
  pause.markChatPaused(OTHER, 'b', 11);
  h.setResumeAt = record(OTHER, { resumeAt: 1_800_000_000 });
  expect(await actions.setChatResumeAt(OTHER, 1_800_000_000)).toBeNull();
  expect(h.calls.at(-1)).toEqual(['chatSetResumeAt', [OTHER, 1_800_000_000]]);
  expect(pause.chatPausedFor(OTHER)?.resumeAt).toBe(1_800_000_000);
  h.setResumeAt = record(OTHER, { resumeAt: null });
  expect(await actions.setChatResumeAt(OTHER, null)).toBeNull();
  expect(pause.chatPausedFor(OTHER)?.resumeAt).toBeNull();
  // A time that passed in flight: the backend resumed it and answers null.
  h.setResumeAt = null;
  expect(await actions.setChatResumeAt(OTHER, 1_800_000_000)).toBeNull();
  expect(pause.chatPausedFor(OTHER)).toBeNull();
  pause.markChatPaused(OTHER, 'b', 12);
  h.setResumeAt = new Error('This chat isn’t paused. Nothing changed.');
  expect(await actions.setChatResumeAt(OTHER, 1_800_000_000)).toBe('This chat isn’t paused. Nothing changed.');
});
