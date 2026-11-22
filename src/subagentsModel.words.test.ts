import { describe, expect, it } from 'vitest';
import type { SubagentRow } from './generated/ipc';
import { modelEffort, startedWords, tokensLabel } from './subagentsModel';

const AT = '2026-11-12T20:00:00Z';
const CLAUDE = { models: [{ id: 'sonnet', label: 'Sonnet 5.5' }, { id: 'opus', label: 'Opus 5.5' }], newestByFamily: { sonnet: 'claude-sonnet-5-5' } };

function row(over: Partial<SubagentRow> = {}): SubagentRow {
  return {
    key: 's-1', current: 's-1', title: 'Company review', agent: 'claude', model: 'claude-sonnet-5-5', effort: 'medium', account: null,
    status: 'done', queuePlace: null, waitingFor: null, startedAt: AT, endedAt: AT, workingMs: 252_000, wallMs: 300_000,
    inputTokens: 150_000, cachedInputTokens: 90_000, outputTokens: 32_000, tokens: 182_000, costUsd: null,
    earlier: false, conversations: ['s-1'], trackerIssue: null, issue: null, freshFrom: null, ...over,
  };
}

describe('token and model words on a subagent row', () => {
  it('counts tokens with a singular only for exactly one', () => {
    expect(tokensLabel(1)).toBe('1 token');
    expect(tokensLabel(0)).toBe('0 tokens');
    expect(tokensLabel(2)).toBe('2 tokens');
    expect(tokensLabel(182_000)).toBe('182K tokens');
    expect(tokensLabel(1_200_000)).toBe('1.2M tokens');
    // A bad count reads as none, never "NaN tokens".
    expect(tokensLabel(Number.NaN)).toBe('0 tokens');
  });

  it('joins the model label and the effort, with the alias resolved from the catalog', () => {
    expect(modelEffort(row(), CLAUDE)).toBe('Sonnet 5.5 · Medium');
    expect(modelEffort(row({ effort: 'xhigh' }), CLAUDE)).toBe('Sonnet 5.5 · Extra high');
  });

  it('falls back to the agent name when no model was recorded, and keeps an unknown provider as recorded', () => {
    expect(modelEffort(row({ model: null }), CLAUDE)).toBe('Claude · Medium');
    expect(modelEffort(row({ model: '  ', effort: null }), CLAUDE)).toBe('Claude');
    expect(modelEffort(row({ model: null, agent: 'acme-agent', effort: null }))).toBe('acme-agent');
  });

  it('shows the model id as recorded when the catalog does not know it, and drops an empty effort', () => {
    expect(modelEffort(row({ model: 'mystery-1', effort: '' }), CLAUDE)).toBe('mystery-1');
    expect(modelEffort(row({ model: 'mystery-1' }), null)).toBe('mystery-1 · Medium');
  });
});

describe('who started a subagent row', () => {
  it('names the lead, the COO, another chat or the person, and nothing when unknown', () => {
    expect(startedWords(row({ startedBy: 'lead' }))).toBe('by the lead');
    expect(startedWords(row({ startedBy: 'main' }))).toBe('by the COO');
    expect(startedWords(row({ startedBy: 'chat' }))).toBe('by another chat');
    expect(startedWords(row({ startedBy: 'you' }))).toBe('by you');
    expect(startedWords(row())).toBe('');
  });
});
