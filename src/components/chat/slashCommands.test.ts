import { beforeEach, expect, it, vi } from 'vitest';
const state = vi.hoisted(() => ({ agent: 'claude', model: '', cwd: '/vault/workspaces/main', main: true, text: [] as string[], mode: 'acceptEdits', unavailable: {} as Record<string, string> }));
vi.mock('../../store', () => ({
  chatAgent: () => state.agent, chatModel: () => state.model, chatCwd: () => state.cwd,
  chatAgentSessionKey: () => null, chatConversationVersion: () => 0,
  chatBusy: () => false, chatSessionLoading: () => false,
  agentsAvailable: () => ['claude', 'codex', 'grok', 'antigravity'],
  AGENT_LABELS: { claude: 'Claude', codex: 'Codex' },
  switchChatAgent: (agent: string) => { state.agent = agent; return ''; },
  chatPermissionMode: () => state.mode,
  setChatPermissionModePersistent: (mode: string) => { state.mode = mode; return ''; },
  confirmPermissionModeChange: async () => true,
  currentAgentCapabilities: () => ({ unavailableModes: state.unavailable }),
  sameChatAsNow: () => () => true,
  setChatModelPersistent: (model: string) => { state.model = model; },
  pushChatTurn: (turn: { text: string }) => state.text.push(turn.text),
  resetChatState: () => { state.agent = 'codex'; state.cwd = ''; },
  setChatInput: vi.fn(),
  applyNewChatPermissionMode: () => {},
  setChatCwdPersistent: (cwd: string) => { state.cwd = cwd; },
  stopChatRun: vi.fn(), pauseChatRun: vi.fn(), canPauseChatRun: () => false, chatPausedHere: () => null, resumePausedChat: vi.fn(), flashFooter: vi.fn(),
}));
vi.mock('../../store/workspaces', () => ({
  workspaceDetail: () => state.main ? { workspace: { id: 'main', cwd: state.cwd } } : null,
  newWorkspaceSessionOpened: vi.fn(async () => true),
}));
vi.mock('../../api', () => ({ api: {} }));
import { busyCommandRefusal, busyLocalCommand, createSlashCommands } from './slashCommands';
import chatSendSource from './chatSendController.ts?raw';
import { newWorkspaceSessionOpened } from '../../store/workspaces';
import { setChatInput } from '../../store';
beforeEach(() => { Object.assign(state, { agent: 'claude', model: '', cwd: '/vault/workspaces/main', main: true, text: [], mode: 'acceptEdits', unavailable: {} }); });
// Per-agent menus (2026-11-12): /permissions speaks the chat's agent's words.
it.each([
  ['claude', 'acceptEdits', 'Accept edits', 'default | accept-edits | plan | auto | bypass-permissions'],
  ['codex', 'acceptEdits', 'Ask for approval', 'read-only | ask-for-approval | approve-for-me | full-access'],
  ['grok', 'default', 'Default', 'default | accept-edits | plan | auto | bypass-permissions'],
  ['antigravity', 'bypassPermissions', 'Skip permissions', 'accept-edits | plan | auto | skip-permissions'],
])('/permissions on %s names the mode and its choices in that agent’s words', (agent, mode, label, words) => {
  Object.assign(state, { agent, mode });
  createSlashCommands({ followSend() {} }).handleLocalCommand('/permissions');
  expect(state.text.join()).toContain(`Permission mode: **${label}**`);
  expect(state.text.join()).toContain(`/permissions ${words}`);
});
it('/permissions takes an agent’s own name and maps it to the canonical mode', async () => {
  Object.assign(state, { agent: 'codex', mode: 'plan' });
  createSlashCommands({ followSend() {} }).handleLocalCommand('/permissions Approve for me');
  await vi.waitFor(() => expect(state.mode).toBe('auto'));
  // The old short word for Edits only lands on Codex's Ask for approval.
  createSlashCommands({ followSend() {} }).handleLocalCommand('/permissions edits');
  await vi.waitFor(() => expect(state.mode).toBe('default'));
  expect(state.text.at(-1)).toBe('Permission mode → **Ask for approval** (default)');
});
it('/permissions refuses a mode the agent can’t run, with its reason', () => {
  Object.assign(state, { agent: 'antigravity', mode: 'acceptEdits', unavailable: { auto: 'No Auto here.' } });
  createSlashCommands({ followSend() {} }).handleLocalCommand('/permissions auto');
  expect(state.text.at(-1)).toBe('No Auto here.');
  expect(state.mode).toBe('acceptEdits');
});
it.each(['codex', 'grok', 'antigravity'])('/help filters Claude-only commands for %s just like the popup', agent => {
  state.agent = agent;
  const commands = createSlashCommands({ followSend() {} });
  commands.handleLocalCommand('/help');
  expect(state.text.join()).not.toMatch(/\/mcp|\/plugin/);
  expect(state.text.join()).toContain('/model');
});
// L6 (11-10): any installed agent may run the COO.
it('the COO switches to another installed agent like any chat', () => {
  createSlashCommands({ followSend() {} }).handleLocalCommand('/agent codex');
  expect(state.agent).toBe('codex');
  expect(state.text.join()).not.toMatch(/Claude only/);
});
it('ordinary conversations can change provider', () => {
  state.main = false;
  createSlashCommands({ followSend() {} }).handleLocalCommand('/agent codex');
  expect(state.agent).toBe('codex');
});
it.each(['/new', '/clear'])('%s starts a fresh COO conversation on the same agent in its own folder', command => {
  vi.mocked(newWorkspaceSessionOpened).mockClear(); vi.mocked(setChatInput).mockClear();
  createSlashCommands({ followSend() {} }).handleLocalCommand(command);
  expect(newWorkspaceSessionOpened).toHaveBeenCalledWith('main');
  expect(setChatInput).toHaveBeenCalledWith('');
  expect(state.agent).toBe('claude');
  expect(state.cwd).toBe('/vault/workspaces/main');
});
it('/model in the COO takes any model at once, another provider\'s included', () => {
  const commands = createSlashCommands({ followSend() {} });
  commands.handleLocalCommand('/model gpt-6-sol');
  expect(state.model).toBe('gpt-6-sol');
  expect(state.text.join()).not.toMatch(/Claude only/);
  commands.handleLocalCommand('/model default');
  expect(state.model).toBe('');
});
// P1-27: /clear is an alias of /new, so the menus list /new once.
it('lists /new once in the menus and /help, and still takes /clear', () => {
  state.main = false;
  const commands = createSlashCommands({ followSend() {} });
  const names = commands.allCommands().map(c => c.name);
  expect(names).toContain('/new');
  expect(names).not.toContain('/clear');
  commands.handleLocalCommand('/help');
  expect(state.text.join()).toContain('/new — ');
  expect(state.text.join()).not.toContain('/clear — ');
  expect(state.text.join()).toContain('/clear does the same as /new');
  expect(commands.handleLocalCommand('/clear')).toBe(true);
  expect(state.agent).toBe('codex');
});
// P1-30: while a turn runs, a local command never steers into the agent.
// One that changes the next turn waits (its text stays); a read runs.
it('sorts local commands while a turn runs: refuse what changes the next turn, run reads, leave the rest', () => {
  for (const cmd of ['/model opus', '/agent codex', '/cwd ~/x', '/permissions plan', '/new', '/clear', '/MODEL default', '/mcp', '/plugins list'])
    expect(busyLocalCommand(cmd), cmd).toBe('refuse');
  for (const cmd of ['/model', '/agent', '/cwd', '/help', '/permissions', '/stop', '/pause'])
    expect(busyLocalCommand(cmd), cmd).toBe('run');
  // Review: /mcp holds the busy flag while it runs, so it never runs mid-turn.
  expect(busyCommandRefusal('/mcp')).toContain("can't run during a turn");
  expect(busyCommandRefusal('/new')).toContain('starts a new chat');
  expect(busyCommandRefusal('/model opus')).toMatch(/^\/model changes the next turn: send it again once this turn ends/);
  for (const cmd of ['/compact', 'Also check /model', 'plain words', ''])
    expect(busyLocalCommand(cmd), cmd).toBeNull();
});
it('checks local commands before the busy branch, so /model while busy neither steers nor sends', () => {
  const flat = (text: string) => text.replace(/\s+/g, '');
  const source = flat(chatSendSource);
  const local = source.indexOf('busyLocalCommand(prompt)');
  expect(local).toBeGreaterThan(-1);
  expect(local).toBeLessThan(source.indexOf('returnsendWhileBusy(busyChoice())'));
});
