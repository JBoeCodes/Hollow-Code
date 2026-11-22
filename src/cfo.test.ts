import { describe, expect, it } from 'vitest';
import { accountRoomLine, allowRows, allowSummary, cfoFooterLine, effortWord, isAllowed, modelName, setAny, setModel } from './cfo';
import type { CfoAllowEntry, CfoDecision } from './generated/ipc';

const decision = (over: Partial<CfoDecision> = {}): CfoDecision => ({
  agent: 'claude',
  model: 'claude-sonnet-5-5',
  effort: 'medium',
  effortSource: 'cfo',
  kind: 'review',
  reason: 'review',
  ...over,
});

describe('the footer line', () => {
  it('shows the model, the effort and the reason', () => {
    expect(cfoFooterLine(decision({ reason: 'small edit' }))).toBe('Sonnet 5.5 · Medium · small edit');
  });

  it('says (you) when the person chose the effort', () => {
    expect(cfoFooterLine(decision({ effort: 'max', effortSource: 'you', kind: 'code', reason: 'code change' })))
      .toBe('Sonnet 5.5 · Max (you) · code change');
  });

  it('uses the catalog label when known, the agent on its default, and leaves out a missing effort', () => {
    expect(cfoFooterLine(decision(), () => 'Sonnet 5.5 · pinned')).toBe('Sonnet 5.5 · pinned · Medium · review');
    expect(cfoFooterLine(decision({ model: undefined, effort: undefined, effortSource: 'fixed', reason: 'conversation' }), undefined, () => 'Codex'))
      .toBe('Codex · conversation');
    expect(cfoFooterLine(null)).toBeNull();
  });

  it('names models from any provider', () => {
    expect(modelName('claude-opus-5-5')).toBe('Opus 5.5');
    expect(modelName('gpt-5.5')).toBe('GPT 5.5');
    expect(modelName('grok-4.7')).toBe('Grok 4.7');
    expect(modelName('sonnet[1m]')).toBe('Sonnet');
    expect(effortWord('xhigh')).toBe('Extra high');
  });
});

describe('the allowlist editor', () => {
  const claude = ['opus', 'sonnet', 'claude-sonnet-5-5'];

  it('empty means every installed agent', () => {
    expect(isAllowed([], 'codex', 'gpt-6-sol')).toBe(true);
    expect(allowSummary([], a => a, (_, m) => m)).toBe('Any installed agent, on its saved model. Check models to narrow it.');
    expect(allowRows([], ['claude', 'codex'])).toEqual([
      { agent: 'claude', any: false, models: [] },
      { agent: 'codex', any: false, models: [] },
    ]);
  });

  it('takes only real models: a model outside the catalog changes nothing', () => {
    const typed = setModel([], 'claude', 'my-made-up-model', true, claude);
    expect(typed).toEqual([]);
    const one = setModel([], 'claude', 'sonnet', true, claude);
    expect(one).toEqual([{ agent: 'claude', model: 'sonnet' }]);
    expect(isAllowed(one, 'claude', 'sonnet')).toBe(true);
    expect(isAllowed(one, 'claude', 'opus')).toBe(false);
    expect(isAllowed(one, 'codex', null)).toBe(false);
    // Clearing the last one is empty again: every agent.
    expect(setModel(one, 'claude', 'sonnet', false, claude)).toEqual([]);
  });

  it('"any model" covers an agent and a single model narrows it again', () => {
    let allow: CfoAllowEntry[] = setModel([], 'claude', 'sonnet', true, claude);
    allow = setAny(allow, 'claude', true);
    expect(allow).toEqual([{ agent: 'claude' }]);
    expect(allowRows(allow, ['claude'])[0]).toEqual({ agent: 'claude', any: true, models: [] });
    allow = setModel(allow, 'claude', 'opus', true, claude);
    expect(allow).toEqual([{ agent: 'claude', model: 'opus' }]);
    allow = setAny(allow, 'codex', true);
    expect(allowSummary(allow, a => a, (_, m) => m)).toBe('claude opus, any codex model');
  });
});

describe('an account against the reserve', () => {
  it('says whether unattended work has room, and until when it does not', () => {
    expect(accountRoomLine(true, null, 'Room for unattended work')).toBe('Room for unattended work');
    const now = new Date('2026-11-10T12:00:00');
    const at = new Date('2026-11-10T15:00:00');
    const line = accountRoomLine(false, at.toISOString(), 'Past the reserve (80% line) in its 5-hour window', now);
    expect(line.startsWith('Past the reserve until ')).toBe(true);
    expect(line).toContain(at.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }));
  });
});
