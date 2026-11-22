import { beforeEach, expect, it, vi } from 'vitest';

const saved = new Map<string, string>();
let failWrites = false;
beforeEach(() => {
  saved.clear(); failWrites = false; vi.resetModules();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => saved.get(key) ?? null,
    setItem: (key: string, value: string) => { if (failWrites) throw new Error('full'); saved.set(key, value); },
    removeItem: (key: string) => saved.delete(key),
  });
});

it('keeps two conversations separate and resumes only a known matching mode', async () => {
  const p = await import('./permModes');
  p.rememberConversationMode('a', 'plan');
  p.rememberConversationMode('b', 'bypassPermissions');
  expect(p.conversationPermission('a', 'bypassPermissions')).toEqual({ mode: 'plan', resumeNative: false });
  expect(p.conversationPermission('b', 'bypassPermissions')).toEqual({ mode: 'bypassPermissions', resumeNative: true });
  expect(p.conversationPermission('missing', 'unknown')).toEqual({ mode: 'acceptEdits', resumeNative: false });
  expect(p.conversationPermission('missing', null).resumeNative).toBe(false);
  expect(p.conversationPermission('a', null).resumeNative).toBe(false);
});

// 2026-11-04: Main's check-ins run Edits only and Alex's chat is Bypass,
// so every turn after a check-in used to start a fresh native session (and
// the check-in's question card could no longer be sent). Two modes of one
// class share a session, as the backend's mode_class has it.
it('resumes across modes of one class, never across plan or a Grok profile', async () => {
  const p = await import('./permModes');
  p.rememberConversationMode('main', 'bypassPermissions');
  expect(p.conversationPermission('main', 'acceptEdits', 'claude')).toEqual({ mode: 'bypassPermissions', resumeNative: true });
  expect(p.conversationPermission('main', 'auto', 'codex').resumeNative).toBe(true);
  expect(p.conversationPermission('main', 'acceptEdits', 'antigravity').resumeNative).toBe(true);
  expect(p.conversationPermission('main', 'plan', 'claude').resumeNative).toBe(false);
  expect(p.conversationPermission('main', 'acceptEdits', 'grok').resumeNative).toBe(false);
  expect(p.conversationPermission('main', 'auto', 'grok').resumeNative).toBe(true);
  p.rememberConversationMode('reader', 'plan');
  expect(p.conversationPermission('reader', 'acceptEdits', 'claude').resumeNative).toBe(false);
  expect(p.conversationPermission('reader', 'plan', 'grok').resumeNative).toBe(true);
  expect(p.modeClass('claude', 'bypassPermissions')).toBe('act');
  expect(p.modeClass('grok', 'acceptEdits')).toBe('grok:workspace');
  expect(p.modeClass('grok', 'plan')).toBe('plan');
  // agent::grok_sandbox: `default` (a child task can ask for it) reads only.
  expect(p.modeClass('grok', 'default')).toBe('grok:read-only');
  expect(p.modeClass('claude', 'default')).toBe('act');
});

it('retains a safer in-memory choice when persistence fails over an older bypass', async () => {
  const p = await import('./permModes');
  p.rememberConversationMode('a', 'bypassPermissions');
  failWrites = true;
  p.rememberConversationMode('a', 'plan');
  expect(p.conversationPermission('a', 'bypassPermissions')).toEqual({ mode: 'plan', resumeNative: false });
});

it('does not resurrect stale bypass after a failed write and process restart', async () => {
  let p = await import('./permModes');
  p.rememberConversationMode('a', 'bypassPermissions');
  failWrites = true; p.rememberConversationMode('a', 'plan');
  vi.resetModules(); p = await import('./permModes');
  expect(p.conversationPermission('a', 'plan')).toEqual({ mode: 'plan', resumeNative: true });
});

it('does not mark a failed migration as durable', async () => {
  failWrites = true;
  const p = await import('./permModes');
  p.preserveCurrentModeOnUpgrade('migration-plan', 'plan');
  expect(saved.get('hollow.chat.mode.migrated')).toBeUndefined();
  expect(p.conversationPermission('migration-plan').mode).toBe('plan');
});

it('quota exhaustion cannot restore another chat to recorded Bypass after restart', async () => {
  const p = await import('./permModes');
  p.rememberConversationMode('old', 'plan');
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => saved.get(key) ?? null,
    setItem: (key: string, value: string) => { if (value.length > 35) throw new Error('full'); saved.set(key, value); },
    removeItem: (key: string) => saved.delete(key),
  });
  expect(p.rememberConversationMode('new', 'bypassPermissions')).toBe(false);
  vi.resetModules();
  const reloaded = await import('./permModes');
  expect(reloaded.conversationPermission('old', 'bypassPermissions')).toEqual({ mode: 'plan', resumeNative: false });
  expect(reloaded.conversationPermission('new', 'bypassPermissions')).toEqual({ mode: 'plan', resumeNative: false });
});

it('a later successful write retains the conservative fallback for omitted chats', async () => {
  saved.set('hollow.chat.modes.v1', JSON.stringify('plan'));
  let p = await import('./permModes');
  expect(p.rememberConversationMode('explicit', 'auto')).toBe(true);
  vi.resetModules(); p = await import('./permModes');
  expect(p.conversationPermission('explicit', 'auto').resumeNative).toBe(true);
  expect(p.conversationPermission('omitted', 'bypassPermissions')).toEqual({ mode: 'plan', resumeNative: false });
});

it('bounds durable choices and keeps the newest changes across reload', async () => {
  let p = await import('./permModes');
  for (let i = 0; i < 550; i++) p.rememberConversationMode(`chat-${i}`, 'plan');
  expect(JSON.parse(saved.get('hollow.chat.modes.v1')!).length).toBe(500);
  expect(saved.size).toBe(1);
  vi.resetModules(); p = await import('./permModes');
  expect(p.conversationPermission('chat-549', 'auto').mode).toBe('plan');
  expect(p.conversationPermission('chat-0', null).mode).toBe('acceptEdits');
});

it('migrates the last visible choice once without changing a saved chat choice', async () => {
  let p = await import('./permModes');
  p.rememberConversationMode('a', 'plan');
  p.preserveCurrentModeOnUpgrade('a', 'bypassPermissions');
  expect(p.conversationPermission('a', 'auto').mode).toBe('plan');
  vi.resetModules(); p = await import('./permModes');
  p.preserveCurrentModeOnUpgrade('b', 'bypassPermissions');
  expect(p.conversationPermission('b', null).mode).toBe('acceptEdits');
});

it('migrates only the current pre-upgrade conversation, including drawer choices', async () => {
  saved.set('hollow.mainDrawer.mode.a', 'plan');
  const p = await import('./permModes');
  p.preserveCurrentModeOnUpgrade('a', 'auto');
  expect(p.conversationPermission('a', 'auto').mode).toBe('plan');
  expect(p.conversationPermission('b', null).mode).toBe('acceptEdits');
});

// Review finding 2 (2026-11-04): the phone chat API keeps this window's
// record (`phone_access_chat_modes`), so a phone's send runs in the mode the
// window shows for its chat.
it('gives the whole record oldest first, with drawer choices not moved over and the Plan fallback', async () => {
  saved.set('hollow.mainDrawer.mode.main-1', 'bypassPermissions');
  saved.set('hollow.mainDrawer.mode.odd', 'sideways');
  vi.stubGlobal('localStorage', {
    get length() { return saved.size; },
    key: (i: number) => [...saved.keys()][i] ?? null,
    getItem: (key: string) => saved.get(key) ?? null,
    setItem: (key: string, value: string) => { if (failWrites) throw new Error('full'); saved.set(key, value); },
    removeItem: (key: string) => saved.delete(key),
  });
  const p = await import('./permModes');
  p.rememberConversationMode('a', 'plan');
  p.rememberConversationMode('b', 'auto');
  p.rememberConversationMode('a', 'acceptEdits');
  expect(p.conversationModeChoices()).toEqual({
    choices: [['main-1', 'bypassPermissions'], ['b', 'auto'], ['a', 'acceptEdits']],
    plan: false,
  });
  // Storage full: the disk keeps the Plan fallback alone, so the record is
  // no choice and Plan for every chat, as this window shows once reopened
  // (second review, 2026-11-04), and the drawer's choices no longer count.
  failWrites = true;
  p.rememberConversationMode('c', 'plan');
  expect(p.conversationModeChoices()).toEqual({ choices: [], plan: true });
  // Saved whole again: every choice, with the fallback for the rest.
  failWrites = false;
  p.rememberConversationMode('d', 'auto');
  expect(p.conversationModeChoices()).toEqual({
    choices: [['b', 'auto'], ['a', 'acceptEdits'], ['c', 'plan'], ['d', 'auto']],
    plan: true,
  });
});

// Second review, finding 2 (2026-11-04): the phone chat API kept up to 500
// choices the window had lost. Its copy follows the record on disk: one
// choice at a time while the disk keeps up, the whole record (replacing the
// copy) when the disk changed wholesale, and never a choice the disk lacks.
it('reports one choice, or the whole record when the one on disk changed wholesale', async () => {
  let p = await import('./permModes');
  p.rememberConversationMode('a', 'bypassPermissions');
  expect(p.conversationModeReport('a', 'bypassPermissions')).toEqual({ choices: [['a', 'bypassPermissions']], replace: false, plan: false });
  expect(p.conversationModeReport(null, 'plan')).toBeNull();
  // A write fails: the disk keeps the Plan fallback alone, so the copy is
  // replaced by no choice and Plan for every chat, the new one included.
  failWrites = true;
  expect(p.rememberConversationMode('b', 'auto')).toBe(false);
  expect(p.conversationModeReport('b', 'auto')).toEqual({ choices: [], replace: true, plan: true });
  // Still failing: still nothing the disk lacks.
  p.rememberConversationMode('c', 'bypassPermissions');
  expect(p.conversationModeReport('c', 'bypassPermissions')).toEqual({ choices: [], replace: true, plan: true });
  // Saved whole again: the whole record once, then one choice at a time.
  failWrites = false;
  expect(p.rememberConversationMode('d', 'plan')).toBe(true);
  const whole: [string, string][] = [['a', 'bypassPermissions'], ['b', 'auto'], ['c', 'bypassPermissions'], ['d', 'plan']];
  expect(p.conversationModeReport('d', 'plan')).toEqual({ choices: whole, replace: true, plan: true });
  p.rememberConversationMode('e', 'acceptEdits');
  expect(p.conversationModeReport('e', 'acceptEdits')).toEqual({ choices: [['e', 'acceptEdits']], replace: false, plan: true });
  // Reopened, the window shows what the copy holds.
  vi.resetModules(); p = await import('./permModes');
  expect(p.conversationModeChoices()).toEqual({ choices: [...whole, ['e', 'acceptEdits']], plan: true });
  expect(p.conversationModeRecord()).toEqual({ choices: [...whole, ['e', 'acceptEdits']], replace: true, plan: true });
});
