// Settings, Usage, Limits: the Claude accounts state the Claude meters and
// Spread usage share (Settings PR 8). `run` is the one door every account
// change goes through, so its two failure paths (a footer flash for a named
// action, an inline error for the plain read) and its busy flag matter.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ClaudeAccountsView } from '../types';

const h = vi.hoisted(() => ({ claudeAccounts: vi.fn(), listConnectors: vi.fn(), flash: vi.fn() }));
vi.mock('../api', () => ({ api: { claudeAccounts: h.claudeAccounts, listConnectors: h.listConnectors } }));
vi.mock('../store', () => ({ flashFooter: h.flash }));

// The accounts view, error and busy count are module state now (S3, review of #1369), so each
// test loads a fresh copy of the module.
let createClaudeLimitsState: typeof import('./claudeLimitsState').createClaudeLimitsState;

const accounts = (tag: string) => ({ tag }) as unknown as ClaudeAccountsView;
/** A claude.ai connector that rides on the login (turned on, shared with Claude, chat). */
const forcing = (name: string, mode: 'always' | 'relevant') => ({
  id: name, name, transport: 'claudeAi', enabled: true, agents: ['claude'], chat: true, automations: false, mode, url: '',
});
const flush = () => new Promise(resolve => setTimeout(resolve, 0));

beforeEach(async () => {
  vi.clearAllMocks();
  vi.resetModules();
  ({ createClaudeLimitsState } = await import('./claudeLimitsState'));
});

describe('run: one door for every account change', () => {
  it('stores the returned view, clears an old error and reports success', async () => {
    const s = createClaudeLimitsState();
    expect(await s.run(() => Promise.reject(new Error('offline')))).toBe(false);
    expect(s.error()).toBe('offline');
    expect(await s.run(async () => accounts('fresh'))).toBe(true);
    expect(s.view()).toEqual(accounts('fresh'));
    expect(s.error()).toBe('');
  });

  it('shows a plain failure inline, keeps the last view and never flashes the footer', async () => {
    const s = createClaudeLimitsState();
    await s.run(async () => accounts('first'));
    expect(await s.run(() => Promise.reject('plain string from Tauri'))).toBe(false);
    expect(s.error()).toBe('plain string from Tauri');
    expect(s.view()).toEqual(accounts('first'));
    expect(h.flash).not.toHaveBeenCalled();
  });

  it('flashes a named failure in the footer for 5 s and leaves the inline error alone', async () => {
    const s = createClaudeLimitsState();
    expect(await s.run(() => Promise.reject(new Error('no such account')), 'Could not save routing')).toBe(false);
    expect(h.flash).toHaveBeenCalledWith('× Could not save routing: no such account', 5000);
    expect(s.error()).toBe('');
  });

  it('is busy while the call is in flight and not after, success or failure', async () => {
    const s = createClaudeLimitsState();
    let finish!: (v: ClaudeAccountsView) => void;
    const pending = s.run(() => new Promise<ClaudeAccountsView>(resolve => { finish = resolve; }));
    expect(s.busy()).toBe(true);
    finish(accounts('x'));
    await pending;
    expect(s.busy()).toBe(false);
    await s.run(() => Promise.reject(new Error('x')));
    expect(s.busy()).toBe(false);
  });
});

describe('load: the accounts read plus the login-override lines', () => {
  it('reads the accounts and names the connectors that pin runs to the login', async () => {
    h.claudeAccounts.mockResolvedValue(accounts('loaded'));
    h.listConnectors.mockResolvedValue({ connectors: [forcing('Gmail', 'always'), forcing('Docs', 'relevant')], claudeaiLogin: null });
    const s = createClaudeLimitsState();
    s.load();
    await flush();
    expect(h.listConnectors).toHaveBeenCalledWith(false);
    expect(s.view()).toEqual(accounts('loaded'));
    expect(s.loginLine()).toContain('Gmail (always), Docs');
    expect(s.loginLine()).toContain('One set to Always rides on every run.');
    // Only the Always connector pins every chat to the login.
    expect(s.pin()).toEqual({ connectors: ['Gmail'], login: null });
  });

  it('shows no line and no pin when no connector forces the login', async () => {
    h.claudeAccounts.mockResolvedValue(accounts('loaded'));
    h.listConnectors.mockResolvedValue({ connectors: [forcing('Docs', 'relevant')] });
    const s = createClaudeLimitsState();
    s.load();
    await flush();
    expect(s.loginLine()).toContain('Docs');
    expect(s.loginLine()).not.toContain('Always');
    expect(s.pin()).toBeNull();
  });

  it('still shows the accounts when the connector library cannot load', async () => {
    h.claudeAccounts.mockResolvedValue(accounts('loaded'));
    h.listConnectors.mockRejectedValue(new Error('library down'));
    const s = createClaudeLimitsState();
    s.load();
    await flush();
    expect(s.view()).toEqual(accounts('loaded'));
    expect(s.loginLine()).toBeNull();
    expect(s.pin()).toBeNull();
    expect(s.error()).toBe('');
  });

  it('reports a failed accounts read inline while the connector lines still load', async () => {
    h.claudeAccounts.mockRejectedValue(new Error('claude_accounts failed'));
    h.listConnectors.mockResolvedValue({ connectors: [forcing('Gmail', 'always')] });
    const s = createClaudeLimitsState();
    s.load();
    await flush();
    expect(s.view()).toBeNull();
    expect(s.error()).toBe('claude_accounts failed');
    expect(s.pin()).toEqual({ connectors: ['Gmail'], login: null });
  });
});
