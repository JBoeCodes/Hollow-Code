import { describe, expect, it } from 'vitest';
import { liveSwitchNotices, switchRow, switchRows } from './accountSwitchRows';
import type { ReceiptSwitchWire } from './executionReceipt';
import CHAT_BODY from './components/ChatBody.tsx?raw';

const account = (from: string, to: string, reason: string): ReceiptSwitchWire => ({ kind: 'account', from, to, reason });
const ceiling = account('Admin', 'Tech', 'Admin passed its ceiling');
const limit = account('Tech', 'Ops', 'Tech hit its limit');
const login: ReceiptSwitchWire = { kind: 'login', from: 'added accounts', to: 'Claude Code login', reason: 'claude.ai connector: Notion, M365' };
const agent: ReceiptSwitchWire = { kind: 'agent', from: 'Claude', to: 'Codex', reason: "Claude's daily budget is spent." };

describe('switchRow', () => {
  it('says which account and why for each kind', () => {
    expect(switchRow(ceiling)).toEqual({ kind: 'account', text: 'Switched to Tech', why: 'Admin passed its ceiling' });
    expect(switchRow(login)).toEqual({ kind: 'login', text: 'Stayed on Claude Code login, not an added account', why: 'claude.ai connector: Notion, M365' });
    expect(switchRow(agent)).toEqual({ kind: 'agent', text: 'Moved to Codex from Claude', why: "Claude's daily budget is spent." });
  });

  it('says a provider outage was retried once, on the same agent (overnight 9 A3)', () => {
    const retry: ReceiptSwitchWire = { kind: 'retry', from: 'Codex', to: 'Codex', reason: 'Retried once after the provider was overloaded (87 s later, same session)' };
    expect(switchRow(retry)).toEqual({ kind: 'retry', text: 'Codex resumed once after an outage', why: retry.reason });
    expect(switchRows([retry], 1).map(r => r.kind)).toEqual(['retry']);
  });

  it('never prints an empty name', () => {
    expect(switchRow(account('Admin', ' ', 'Admin hit its limit')).text).toBe('Switched to another account');
    expect(switchRow({ ...agent, from: '', to: '' }).text).toBe('Moved to another agent');
  });
});

describe('switchRows', () => {
  it('lists every move in order when no live notice shows (a reloaded chat)', () => {
    expect(switchRows([login, ceiling, limit]).map(r => r.text)).toEqual([
      'Stayed on Claude Code login, not an added account',
      'Switched to Tech',
      'Switched to Ops',
    ]);
  });

  it('skips the account moves the live notices already show, never a login stay or agent move', () => {
    expect(switchRows([login, ceiling, limit, agent], 1).map(r => r.text)).toEqual([
      'Stayed on Claude Code login, not an added account',
      'Switched to Ops',
      'Moved to Codex from Claude',
    ]);
    expect(switchRows([ceiling], 3)).toEqual([]);
    expect(switchRows(undefined, 0)).toEqual([]);
    expect(switchRows([ceiling], -1)).toHaveLength(1);
  });
});

describe('liveSwitchNotices', () => {
  const turns = [
    { role: 'user' },
    { role: 'system', notice: 'accountSwitch' },
    { role: 'assistant' },
    { role: 'user' },
    { role: 'assistant' },
    { role: 'system', notice: 'accountSwitch' },
    { role: 'system' },
    { role: 'assistant' },
  ];

  it("counts the whole reply's notices, between its user messages", () => {
    expect(liveSwitchNotices(turns, 7)).toBe(1);
    expect(liveSwitchNotices(turns, 2)).toBe(1);
    // The receipt can sit on a bubble before its notice (a failed resume).
    expect(liveSwitchNotices(turns, 4)).toBe(1);
    expect(liveSwitchNotices([{ role: 'user' }, { role: 'assistant' }, { role: 'system', notice: 'accountSwitch' }], 1)).toBe(1);
    expect(liveSwitchNotices([], 0)).toBe(0);
  });

  it('counts only the notices still rendered (the "Show earlier" window)', () => {
    expect(liveSwitchNotices(turns, 7, 6)).toBe(0);
    expect(liveSwitchNotices(turns, 7, 5)).toBe(1);
    // A window starting past the reply never reaches into the next one.
    expect(liveSwitchNotices(turns, 2, 7)).toBe(0);
  });
});

describe('ChatBody', () => {
  it("shows the rows on a finished reply, from its receipt, less its live notices", () => {
    expect(CHAT_BODY).toContain('switchRows(t.receipt?.switches, liveSwitchNotices(chatTurns, turnIdx(), renderFrom()))');
    // Not beside the full receipt, which lists the same moves.
    expect(CHAT_BODY).toContain('const switched = createMemo(() => (tail() && !showReceipt() ?');
    expect(CHAT_BODY).toContain('<ul class="chat-switch-rows" role="list" aria-label="Which account or agent ran this reply">');
  });
});

describe('a rested start on the receipt (overnight 9 A9)', () => {
  it('reads "Started fresh" with its reason, and never doubles the chat notice as a timeline row', () => {
    const rested: ReceiptSwitchWire = { kind: 'rested', from: 'Claude', to: 'Claude', reason: 'away 3 h at 714K' };
    expect(switchRow(rested)).toEqual({ kind: 'rested', text: 'Started fresh', why: 'away 3 h at 714K' });
    expect(switchRows([rested])).toEqual([]);
  });
});
