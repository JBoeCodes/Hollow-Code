import { describe, expect, it } from 'vitest';
import { resumeChecklist, resumeSummary, type ResumeLabels } from './restartResume';
import { CONTINUE_PROMPT } from './chatContinue';
import type { InterruptedChat } from './generated/ipc';

const chat = (n: number, over: Partial<InterruptedChat> = {}): InterruptedChat => ({
  sessionKey: `s-20261010-aaaa000${n}`, title: `Chat ${n}`, agent: 'claude', workspaceId: null, askedAt: '2026-10-10T01:00:00Z',
  model: null, permissionMode: null, agentStarted: false, ...over,
});
const labels = (ultra: string[] = []): ResumeLabels => ({
  where: id => (id === 'alpha' ? 'Alpha' : ''),
  agent: id => (id === 'codex' ? 'Codex' : 'Claude'),
  mode: m => (m === 'acceptEdits' ? 'Edits only' : m === 'bypassPermissions' ? 'Bypass all (danger)' : m ?? ''),
  ultracode: key => ultra.includes(key),
});

describe('resumeChecklist (P1-40: pick per chat)', () => {
  it('lists one ticked row per chat with its workspace, agent, model, permission mode and Ultracode', () => {
    const c = resumeChecklist([
      chat(1, { workspaceId: 'alpha', model: 'claude-opus-5-5', permissionMode: 'acceptEdits' }),
      chat(2, { agent: 'codex', permissionMode: 'bypassPermissions' }),
    ], labels(['s-20261010-aaaa0002']));
    expect(c.title).toBe('Resume 2 interrupted chats?');
    expect(c.confirmLabel).toBe('Resume ticked');
    expect(c.cancelLabel).toBe('Not now');
    expect(c.focusCancel, 'a stray Enter sends no billed messages').toBe(true);
    expect(c.body).toContain(`“${CONTINUE_PROMPT}”`);
    expect(c.checklist).toEqual([
      { id: 's-20261010-aaaa0001', label: 'Chat 1', detail: 'Alpha · Claude · claude-opus-5-5 · Edits only', checked: true },
      { id: 's-20261010-aaaa0002', label: 'Chat 2', detail: 'Codex · default model · Bypass all (danger) · Ultracode on', checked: true },
    ]);
    expect(c.aside).toBeUndefined();
    // No recorded mode: the resume runs at the agent's default, and says so.
    expect(resumeChecklist([chat(3)], labels()).checklist[0].detail).toBe('Claude · default model · default permissions');
  });

  it('says one chat in the singular and lists every chat, however many', () => {
    expect(resumeChecklist([chat(1)], labels()).title).toBe('Resume 1 interrupted chat?');
    const many = Array.from({ length: 10 }, (_, i) => chat(i));
    expect(resumeChecklist(many, labels()).checklist).toHaveLength(10);
  });

  it('lists agent-started chats apart, to open, with nothing to tick and no bulk resume', () => {
    const c = resumeChecklist([chat(1), chat(2, { agentStarted: true, title: 'Helper' }), chat(3, { agentStarted: true, title: 'Lane B', workspaceId: 'alpha' })], labels());
    expect(c.checklist.map(r => r.id)).toEqual(['s-20261010-aaaa0001']);
    expect(c.aside?.heading).toBe('2 agent-started chats also stopped');
    expect(c.aside?.actionLabel).toBe('Open');
    expect(c.aside?.rows.map(r => [r.id, r.label, r.detail])).toEqual([
      ['s-20261010-aaaa0002', 'Helper', 'Claude'],
      ['s-20261010-aaaa0003', 'Lane B', 'Alpha · Claude'],
    ]);
    expect(resumeChecklist([chat(1), chat(2, { agentStarted: true })], labels()).aside?.heading).toBe('1 agent-started chat also stopped');
  });
});

describe('resumeSummary (P1-40: names every skip)', () => {
  const title = (key: string) => ({ a: 'Alpha chat', b: 'Budget memo', c: 'Copy deck' } as Record<string, string>)[key] ?? key;
  it('counts what was queued', () => {
    expect(resumeSummary([{ sessionKey: 'a', queued: true, reason: null }], title)).toBe('Resumed 1 chat.');
    expect(resumeSummary([], title)).toBe('Nothing to resume.');
  });

  it('names every chat that was not resumed and why', () => {
    expect(resumeSummary([
      { sessionKey: 'a', queued: true, reason: null },
      { sessionKey: 'b', queued: false, reason: 'No longer interrupted.' },
      { sessionKey: 'c', queued: false, reason: 'busy' },
    ], title)).toBe('Resumed 1; not resumed: Budget memo (No longer interrupted.); Copy deck (busy)');
    expect(resumeSummary([{ sessionKey: 'b', queued: false, reason: null }], title)).toBe('× Not resumed: Budget memo (no reason given)');
  });

  it('says how many unticked chats stay offered', () => {
    expect(resumeSummary([{ sessionKey: 'a', queued: true, reason: null }], title, 1)).toBe('Resumed 1 chat. 1 more stays in Needs you to resume later.');
    expect(resumeSummary([{ sessionKey: 'a', queued: true, reason: null }], title, 2)).toBe('Resumed 1 chat. 2 more stay in Needs you to resume later.');
  });
});
