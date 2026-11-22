import { beforeEach, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => {
  const values = new Map<string, string>();
  vi.stubGlobal('localStorage', { getItem: (k: string) => values.get(k) ?? null, setItem: (k: string, v: string) => values.set(k,v), removeItem: (k: string) => values.delete(k) });
  return { listeners: new Map<string, (event: { payload: unknown }) => void>(), navigate: vi.fn(), nativeOpen: vi.fn() };
});
vi.mock('../nativeEvents', () => ({ listen: (name: string, fn: (event: { payload: unknown }) => void) => { h.listeners.set(name,fn); return Promise.resolve(() => {}); } }));
vi.mock('../api', () => ({ api: { setupStatus: async () => ({ vaultPath: '/vault' }), chatPreviewOpen: h.nativeOpen } }));
vi.mock('./runtime', () => ({ activePage: () => 'today', setActivePage: h.navigate, closeChatPanel: vi.fn(), refetchShownSessions: vi.fn(), requestChatFocus: vi.fn(), setChatPanelOpen: vi.fn() }));
vi.mock('./overlays', () => ({ showConfirm: vi.fn() }));
vi.mock('./snackbars', () => ({ flashFooter: vi.fn() }));
vi.mock('../windowLayout', () => ({ windowAtMost: () => false }));
vi.mock('../canvas/askHold', () => ({ answeredRevision: () => null, answersBack: vi.fn(), settleHeldAnswers: vi.fn() }));

import { resetChatState, setChatAgentSessionKeyPersistent, chatAgentSessionKey, chatCwd, chatInput, chatPanel, chatTurns, setChatCwdPersistent, setChatInput, setChatBusy, chatAgent, chatBusy, setChatLiveRunId, pushChatTurn } from './chatRuntime';

const emit = (payload: Record<string, unknown>) => h.listeners.get('chat-event')!({ payload: { conversationKey: 's-quota', ...payload } });

it('only this live run can attach one explicit proposed plan to its reply', () => {
  resetChatState(); setChatAgentSessionKeyPersistent('s-quota'); setChatLiveRunId('plan-run');
  emit({ type: 'assistantText', text: 'The plan' });
  const proposal = (runId: string, text: string, sessionKey = 's-quota') => emit({ type: 'panel', sessionKey, panel: 'hollow:proposedPlan', snapshot: { runId, text } });
  proposal('old-run', 'Stale');
  proposal('plan-run', 'Foreign', 'other-chat');
  expect(chatTurns.some(t => !!t.proposedPlan)).toBe(false);
  proposal('plan-run', 'Explicit proposal');
  proposal('plan-run', 'Updated proposal');
  expect(chatTurns.filter(t => !!t.proposedPlan).map(t => t.proposedPlan)).toEqual(['Updated proposal']);
});

it('handoff progress replaces one divider and rejects foreign or stale runs', () => {
  resetChatState(); setChatAgentSessionKeyPersistent('s-quota'); setChatLiveRunId('run-handoff');
  const progress = (runId: string, state: string, entries = 0, sessionKey = 's-quota') => emit({ type: 'panel', sessionKey, panel: 'hollow:handoff', snapshot: { runId, state, entries } });
  progress('run-old', 'complete', 5);
  progress('run-handoff', 'pending', 0, 's-other');
  expect(chatTurns).toHaveLength(0);
  progress('run-handoff', 'pending');
  progress('run-handoff', 'complete', 4);
  expect(chatTurns).toHaveLength(1);
  expect(chatTurns[0].handoff).toEqual({ state: 'complete', entries: 4 });
  expect(chatTurns[0].text).toContain('prepared');
  progress('run-handoff', 'complete', 0);
  expect(chatTurns[0].handoff?.state).toBe('unavailable');
});

it('reasoning stays on one run row and ignores foreign or stale snapshots', () => {
  setChatLiveRunId('live');
  const reasoning = (runId: string, text: string, sessionKey = 's-quota') => emit({ type: 'panel', sessionKey, panel: 'hollow:reasoning', snapshot: { runId, text } });
  reasoning('live', 'First');
  pushChatTurn({ role: 'assistant', runId: 'live', text: 'Answer', tools: [] });
  reasoning('live', 'First then second');
  reasoning('old', 'Stale');
  reasoning('live', 'Foreign', 's-other');
  expect(chatTurns.filter(t => t.reasoning).map(t => t.reasoning)).toEqual(['First then second']);
  expect(chatTurns.at(-1)?.text).toBe('Answer');
});
beforeEach(() => {
  resetChatState(); setChatAgentSessionKeyPersistent('s-quota');
  h.navigate.mockClear(); h.nativeOpen.mockClear();
});

it('a quota waiting notice preserves live work and never changes providers or chats', () => {
  setChatCwdPersistent('/vault/workspaces/project'); setChatInput('Unsent correction'); setChatBusy(true);
  const provider = chatAgent();
  emit({ type:'loginLow', sessionKey:'s-quota', message:'Claude is keeping its account while background jobs finish.' });
  expect(chatAgent()).toBe(provider); expect(chatAgentSessionKey()).toBe('s-quota');
  expect(chatInput()).toBe('Unsent correction'); expect(chatCwd()).toBe('/vault/workspaces/project');
  expect(chatBusy()).toBe(true);
  expect(chatTurns.at(-1)?.role).toBe('system');
  expect(h.navigate).not.toHaveBeenCalled(); expect(h.nativeOpen).not.toHaveBeenCalled();
  setChatBusy(false);
});

it('another running conversation quota notice never hijacks the selected chat', () => {
  emit({ type:'loginLow', conversationKey:'s-other', sessionKey:'s-other', message:'Another job is waiting.' });
  expect(chatAgentSessionKey()).toBe('s-quota'); expect(chatTurns).toHaveLength(0);
  expect(h.navigate).not.toHaveBeenCalled();
});

it('a context notice shows as a system row only in its own conversation', () => {
  emit({ type:'panel', sessionKey:'s-other', panel:'hollow:context', snapshot:{ message:'Other chat started fresh.' } });
  expect(chatTurns).toHaveLength(0);
  emit({ type:'panel', sessionKey:'s-quota', panel:'hollow:context', snapshot:{ message:'Hollow started this agent fresh.' } });
  expect(chatTurns.at(-1)?.role).toBe('system');
  expect(chatTurns.at(-1)?.text).toContain('started this agent fresh');
});

it('a panel snapshot replaces the last one for its conversation and adds no row', () => {
  const before = chatTurns.length;
  emit({ type:'panel', sessionKey:'s-quota', panel:'hollow:gate', snapshot:{ step:'tests', pass:3, fail:0 } });
  emit({ type:'panel', sessionKey:'s-quota', panel:'hollow:gate', snapshot:{ step:'tests', pass:5, fail:1 } });
  expect(chatPanel('hollow:gate')).toEqual({ step:'tests', pass:5, fail:1 });
  expect(chatTurns).toHaveLength(before);
  emit({ type:'panel', sessionKey:'s-other', panel:'hollow:gate', snapshot:{ step:'x' } });
  expect(chatPanel('hollow:gate')).toEqual({ step:'tests', pass:5, fail:1 });
});
