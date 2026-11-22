import { describe, expect, it } from 'vitest';
import { accountSwitchNotice, fillFraction, formatPercent, joinNames, loginOutOfRoom, loginPinNote, loginRowBadge, loginStatus, nextRunBadge, pinnable, resetLabel, routingSummary, failoverSummary, tokenProblem, windowLevel } from './claudeAccounts';
import type { ClaudeAccount, ClaudeAccountsView } from './types';

function account(id: string, fields: Partial<ClaudeAccount> = {}): ClaudeAccount {
  return {
    id, label: id, enabled: true, status: 'ok',
    note: null, usage: null, lastUsedAt: null, createdAt: null, ...fields,
  };
}

function view(fields: Partial<ClaudeAccountsView> = {}): ClaudeAccountsView {
  return {
    routing: 'single', pinned: '', ceilingPercent: 90, failover: true, nextAccount: null,
    accounts: [], error: null, ...fields,
  };
}

describe('claude accounts helpers', () => {
  it('keeps percentages over 100 and clamps only the meter', () => {
    expect(formatPercent(101)).toBe('101%');
    expect(formatPercent(7.5)).toBe('7.5%');
    expect(fillFraction(130)).toBe(1);
    expect(fillFraction(-4)).toBe(0);
  });

  it('levels windows against the ceiling', () => {
    expect(windowLevel({ usedPercent: 40, resetsAt: null }, 90)).toBe('ok');
    expect(windowLevel({ usedPercent: 92, resetsAt: null }, 90)).toBe('high');
    expect(windowLevel({ usedPercent: 100, resetsAt: null }, 90)).toBe('over');
    expect(windowLevel(null, 90)).toBe('ok');
  });

  it('says when a window resets', () => {
    const now = new Date('2026-09-26T12:00:00Z');
    expect(resetLabel('2026-09-26T12:30:00Z', now)).toBe('Resets in 30 min');
    expect(resetLabel('2026-09-26T15:00:00Z', now)).toBe('Resets in 3 h');
    expect(resetLabel('2026-09-26T11:00:00Z', now)).toBe('Resetting now');
    expect(resetLabel(null, now)).toBeNull();
  });

  it('summarizes routing', () => {
    expect(routingSummary(view())).toMatch(/Add accounts/);
    const two = [account('acct-1', { label: 'Work' }), account('acct-2', { label: 'Side', status: 'invalid' }), account('acct-3', { label: 'Off', enabled: false })];
    expect(routingSummary(view({ accounts: two, pinned: 'acct-1', nextAccount: 'acct-1' }))).toContain('Automatically choosing the most room');
    expect(routingSummary(view({ accounts: two, routing: 'spread', nextAccount: 'acct-1' })))
      .toBe('Automatically choosing the most room across 1 account. The next run uses Work.');
    expect(routingSummary(view({ accounts: two, routing: 'spread', nextAccount: null })))
      .toMatch(/Waiting for an eligible Claude account/);
  });

  // 2026-10-21: Spread usage, teammate has room, ExampleCorp is out,
  // and Superhuman Mail (claude.ai) is set to Always. Every chat then runs on
  // Claude Code's login (`agent.rs` → `apply_login`), so the header must not
  // promise teammate for the next run.
  it('says chats run on the login while an Always claude.ai connector pins them (usage spread)', () => {
    const sam = view({
      routing: 'spread',
      nextAccount: 'cogs',
      accounts: [
        account('cogs', { label: 'teammate' }),
        account('pi', { label: 'ExampleCorp', status: 'limited' }),
      ],
    });
    const pin = { connectors: ['Superhuman Mail'], login: 'sam@example.com in ExampleCorp' };
    const summary = routingSummary(sam, pin);
    expect(summary).not.toContain('The next run uses teammate.');
    expect(summary).toContain("Chats run on Claude Code's login (sam@example.com in ExampleCorp)");
    expect(summary).toContain('Superhuman Mail is set to Always');
    expect(summary).toContain('Runs without a claude.ai connector use teammate next.');
    // Nothing pinned: the summary is as before.
    expect(routingSummary(sam, null)).toBe('Automatically choosing the most room across 2 accounts. The next run uses teammate.');
  });

  it('words the login row and the next-run badge while chats are pinned', () => {
    const pin = { connectors: ['Superhuman Mail'], login: null };
    expect(nextRunBadge(view(), pin)).toBe('Next run without a claude.ai connector');
    expect(nextRunBadge(view(), null)).toBe('Next run');
    expect(loginPinNote(pin)).toContain('Superhuman Mail is set to Always, so it rides on every chat while this login has room');
    expect(loginPinNote(pin)).toContain('never moves to an added account');
    expect(loginPinNote(pin, false)).toContain('so it rides on every chat, and');
    expect(loginPinNote({ connectors: ['Notion', 'Superhuman Mail'], login: null })).toContain('Notion and Superhuman Mail are set to Always, so they ride');
    expect(joinNames(['A', 'B', 'C'])).toBe('A, B and C');
    // One account, pinned: the routed account is named without "next".
    const single = view({ accounts: [account('w', { label: 'Work' })], nextAccount: 'w' });
    expect(routingSummary(single, pin)).toBe("Chats run on Claude Code's login, not on your added accounts, because Superhuman Mail is set to Always. Runs without a claude.ai connector use Work.");
    // No added account can run: everything is on the login anyway.
    expect(routingSummary(view({ accounts: [account('w')], nextAccount: null }), pin)).toMatch(/Waiting for an eligible Claude account/);
  });

  it('says when the login is out of room and Always waits for relevance (usage spread)', () => {
    const w = (usedPercent: number) => ({ usedPercent, resetsAt: null });
    const pin = { connectors: ['Superhuman Mail'], login: 'sam@example.com in ExampleCorp' };
    const sam = (fields: Partial<ClaudeAccountsView> = {}) => view({
      routing: 'spread',
      nextAccount: 'cogs',
      loginUsage: { fiveHour: w(105), sevenDay: w(11), observedAt: '' },
      accounts: [account('cogs', { label: 'teammate' }), account('pi', { label: 'ExampleCorp', status: 'limited' })],
      ...fields,
    });
    expect(loginOutOfRoom(sam())).toBe(true);
    expect(routingSummary(sam(), pin)).toBe("Claude Code's login (sam@example.com in ExampleCorp) is out of room, so Superhuman Mail (set to Always) comes along only when a message needs it. Other chats use teammate next.");
    expect(nextRunBadge(sam(), pin)).toBe('Next run');
    expect(loginRowBadge(sam())).toBe('Only messages that need its connectors');
    // Failover off: Hollow doesn't move runs, so Always keeps every chat there.
    expect(loginOutOfRoom(sam({ failover: false }))).toBe(false);
    expect(routingSummary(sam({ failover: false }), pin)).toMatch(/^Chats run on Claude Code's login/);
    expect(loginRowBadge(sam({ failover: false }))).toBe('Chats run here');
    // Nowhere with room, or the login under its line: nothing changes.
    expect(loginOutOfRoom(sam({ accounts: [account('pi', { status: 'limited' })] }))).toBe(false);
    expect(loginOutOfRoom(sam({ loginUsage: { fiveHour: w(50), sevenDay: w(11), observedAt: '' } }))).toBe(false);
    // One account: the line is 100%, not the ceiling.
    expect(loginOutOfRoom(sam({ routing: 'single', loginUsage: { fiveHour: w(95), sevenDay: null, observedAt: '' } }))).toBe(false);
    expect(loginOutOfRoom(sam({ routing: 'single' }))).toBe(true);
  });

  it('levels the login from its windows as last seen', () => {
    const w = (usedPercent: number) => ({ usedPercent, resetsAt: null });
    expect(loginStatus(view())).toBeNull();
    expect(loginStatus(view({ loginUsage: { fiveHour: w(105), sevenDay: w(11), observedAt: '' } }))).toBe('limited');
    expect(loginStatus(view({ loginUsage: { fiveHour: w(20), sevenDay: w(92), observedAt: '' } }))).toBe('near');
    expect(loginStatus(view({ loginUsage: { fiveHour: null, sevenDay: w(10), observedAt: '' } }))).toBe('ok');
  });

  it('pins enabled accounts and keeps the pinned one listed', () => {
    const accounts = [account('a'), account('b', { enabled: false }), account('c', { enabled: false })];
    expect(pinnable(view({ accounts, pinned: 'c' })).map(a => a.id)).toEqual(['a', 'c']);
  });

  it('flags obviously wrong tokens before sending them', () => {
    expect(tokenProblem('')).toBeNull();
    expect(tokenProblem('sk-ant-api03-abc')).toMatch(/API key/);
    expect(tokenProblem('hello')).toMatch(/sk-ant-oat01/);
    expect(tokenProblem('sk-ant-oat01-' + 'a'.repeat(40))).toBeNull();
    expect(tokenProblem('sk-ant-oat01-' + 'a'.repeat(20) + '\n  ' + 'a'.repeat(20))).toBeNull();
  });

  it('words the failover notice quietly (R1)', () => {
    expect(accountSwitchNotice('Admin', 'Tech')).toBe('Switched to Tech (Admin hit its limit)');
    expect(accountSwitchNotice('', 'Tech')).toBe('Switched to Tech');
    expect(accountSwitchNotice('Admin', ' ')).toBe('Switched to another account (Admin hit its limit)');
    expect(accountSwitchNotice('Admin', 'Tech', 'limit')).toBe('Switched to Tech (Admin hit its limit)');
    expect(accountSwitchNotice('Admin', 'Tech', 'ceiling')).toBe('Switched to Tech (Admin passed its ceiling)');
  });
});

describe('failover policy and unavailable fallback', () => {
  it('explains disabled or absent alternatives without changing preferences', () => {
    const off = view({ failover: false }); const before = JSON.stringify(off);
    expect(failoverSummary(off)).not.toContain('failover is off');
    expect(JSON.stringify(off)).toBe(before);
    for (const accounts of [[],[account('a')],[account('a'),account('b',{enabled:false})],[account('a'),account('b',{status:'missingToken'})],[account('a'),account('b',{status:'invalid'})]]) {
      expect(failoverSummary(view({accounts}))).toContain('No second');
    }
  });
  it('does not promise room when all reported alternatives are limited', () => {
    expect(failoverSummary(view({accounts:[account('a',{status:'limited'}),account('b',{status:'limited'})]}))).toContain('wait for a reset');
  });
  it('keeps unknown meters honest and separates safe account failover from provider or paid API switching', () => {
    const text = failoverSummary(view({routing:'spread',accounts:[account('a',{status:'unknown'}),account('b',{status:'unknown'})]}));
    for (const phrase of ['each account once per turn','most room','earliest reset','organization policy','not models or providers','never selects a paid API']) expect(text).toContain(phrase);
  });
});
