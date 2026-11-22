import { describe, expect, it } from 'vitest';
import {
  STEP_IDS, STEP_LINES, STEP_TITLES, agentLine, importedFromBatches, agentRows, agentState, agentsSummary, continueLabel, dotLabel, hasBack, nextOpen, prevIndex,
  shouldOpen, startIndex, startSummary, stateOf, workspaceNameProblem,
  finishWelcome, welcomeAgents, welcomeReady, welcomeCapabilityLines,
} from './firstRun';
import { mockSetupStatus } from './mockFirstRun';
import type { SetupStep, SetupStepId } from './firstRunApi';
import type { AgentDescriptor } from './types';

describe('welcome page', () => {
  it('uses registry membership and capabilities, and never calls unknown sign-in ready', () => {
    const descriptors: AgentDescriptor[] = [{ id: 'future-agent', label: 'Future agent', available: true,
      capabilities: { modelSelection: true, sessionResume: false, toolRestrictions: false, ownedMcp: true, structuredOutput: false } }];
    const unknown = welcomeAgents(descriptors, [], []);
    expect(unknown.map(r => r.agent)).toEqual(['future-agent']);
    expect(welcomeReady(unknown[0])).toBe(false);
    expect(welcomeCapabilityLines(unknown[0])).toEqual(['Choose a model', 'Use shared connectors']);
    const accounts = [{ id: 'future-agent', label: 'Account label', installed: true, signedIn: true },
      { id: 'unregistered', label: 'Other', installed: true, signedIn: true }];
    expect(welcomeAgents(descriptors, [], accounts)).toHaveLength(1);
    expect(welcomeReady(welcomeAgents(descriptors, [], accounts)[0])).toBe(true);
    expect(welcomeReady(welcomeAgents([{ ...descriptors[0], available: false }], [], accounts)[0])).toBe(false);
  });

  it('preserves saved steps and resumes completion after a failed write', async () => {
    let status = mockSetupStatus({ steps: { context: 'done', agents: 'skipped' } });
    const calls: string[] = [];
    const save = async (step: SetupStepId, outcome: 'done' | 'skipped') => {
      calls.push(step);
      if (step === 'sources' && calls.length === 2) throw new Error('write failed');
      return mockSetupStatus({ steps: Object.fromEntries([...status.steps.filter(s => s.state !== 'open').map(s => [s.id, s.state]), [step, outcome]]) });
    };
    await expect(finishWelcome(status, save, next => { status = next; })).rejects.toThrow('write failed');
    expect(stateOf(status.steps, 'find')).toBe('skipped');
    await finishWelcome(status, save, next => { status = next; });
    expect(calls).toEqual(['find', 'sources', 'sources', 'start']);
    expect(status.complete).toBe(true);
    expect(stateOf(status.steps, 'context')).toBe('done');
    expect(stateOf(status.steps, 'start')).toBe('done');
  });
});

const steps = (states: Partial<Record<SetupStepId, 'done' | 'skipped'>>): SetupStep[] =>
  STEP_IDS.map(id => ({ id, state: states[id] ?? 'open' }));

describe('first-run setup steps (H6)', () => {
  it('has the five steps in order, each with a title and one line', () => {
    expect(STEP_IDS).toEqual(['context', 'find', 'sources', 'agents', 'start']);
    for (const id of STEP_IDS) {
      expect(STEP_TITLES[id]).toBeTruthy();
      expect(STEP_LINES[id].length).toBeLessThanOrEqual(90);
      expect(STEP_LINES[id]).not.toMatch(/[\u2013\u2014]/);
    }
  });

  it('opens a fresh setup at its first step and never a complete one', () => {
    const fresh = mockSetupStatus({ steps: {} });
    expect(shouldOpen(fresh)).toBe(true);
    expect(startIndex(fresh)).toBe(0);
    const complete = mockSetupStatus(null);
    expect(complete.complete).toBe(true);
    expect(shouldOpen(complete)).toBe(false);
    expect(shouldOpen(null)).toBe(false);
  });

  it('resumes at the first step neither done nor skipped', () => {
    const status = mockSetupStatus({ steps: { context: 'skipped', find: 'done' } });
    expect(status.current).toBe('sources');
    expect(startIndex(status)).toBe(2);
    expect(stateOf(status.steps, 'context')).toBe('skipped');
  });

  it('moves to the next open step, wraps to an earlier open one, and ends when none is left', () => {
    expect(nextOpen(steps({ context: 'done' }), 0)).toBe(1);
    // Back to Find, then Continue: Sources is next even though Agents is done.
    expect(nextOpen(steps({ context: 'done', find: 'done', agents: 'done' }), 1)).toBe(2);
    // The last step marked while an earlier one is still open: back to it.
    expect(nextOpen(steps({ find: 'done', sources: 'done', agents: 'skipped', start: 'done' }), 4)).toBe(0);
    expect(nextOpen(steps({ context: 'done', find: 'done', sources: 'skipped', agents: 'done', start: 'done' }), 4)).toBeNull();
  });

  it('shows Back after the first step and Finish on the last', () => {
    expect(hasBack(0)).toBe(false);
    expect(hasBack(1)).toBe(true);
    expect(prevIndex(0)).toBe(0);
    expect(prevIndex(3)).toBe(2);
    expect(continueLabel(0)).toBe('Continue');
    expect(continueLabel(4)).toBe('Finish');
  });

  it('names each dot with its place, title and state', () => {
    expect(dotLabel(1, 'skipped')).toBe('Step 2 of 5: Find what’s already here, skipped');
    expect(dotLabel(0, 'open')).toBe('Step 1 of 5: Connect your context');
  });

  it('reads agents from the CLI rows and the sign-in rows, without guessing Claude Code’s login', () => {
    const rows = agentRows(
      [{ agent: 'claude', label: 'Claude Code', installed: true, version: '2.1.3' }, { agent: 'codex', label: 'Codex', installed: true, version: '0.164.0' }, { agent: 'grok', label: 'Grok', installed: false, version: null }],
      [{ id: 'codex', label: 'ChatGPT (Codex)', installed: true, signedIn: false }, { id: 'antigravity', label: 'Antigravity', installed: true, signedIn: null }],
    );
    expect(rows.map(r => r.agent)).toEqual(['claude', 'codex', 'grok', 'antigravity']);
    expect(agentLine(rows[0])).toBe('Installed (2.1.3)');
    expect(agentLine(rows[1])).toBe('Installed (0.164.0), signed out');
    expect(agentLine(rows[2])).toBe('Not installed');
    expect(agentState(rows[0])).toBe('unknown');
    expect(agentState(rows[1])).toBe('missingToken');
    expect(agentsSummary(rows)).toBe('2 of 4 agents ready');
    expect(agentsSummary([])).toBe('No agents found.');
  });

  it('allows an empty workspace name and refuses a slash or a long one', () => {
    expect(workspaceNameProblem('')).toBeNull();
    expect(workspaceNameProblem('Pricing study')).toBeNull();
    expect(workspaceNameProblem('a/b')).toMatch(/slash/);
    expect(workspaceNameProblem('x'.repeat(81))).toMatch(/80/);
  });

  it('sums up the import and the suggestions waiting, and says nothing about an import it can’t count', () => {
    expect(startSummary(3, 2)).toEqual([
      'Imported 3 conversations. They are in Library, Conversations.',
      '2 suggestions waiting in Library, Learnings.',
    ]);
    expect(startSummary(0, 0)).toEqual(['No conversations imported. Settings, Sources, Where your work lives has Run setup again any time.']);
    // Unknown (the batches couldn't be read): no import line at all, never a false "No conversations".
    expect(startSummary(null, null)).toEqual([]);
    expect(startSummary(null, 1)).toEqual(['1 suggestion waiting in Library, Learnings.']);
  });

  it('counts an import from the committed batches after a quit and resume', () => {
    expect(importedFromBatches([])).toBe(0);
    expect(importedFromBatches([{ conversations: 4 }, { conversations: 1 }])).toBe(5);
    expect(startSummary(importedFromBatches([{ conversations: 4 }]), null)).toEqual(['Imported 4 conversations. They are in Library, Conversations.']);
  });
});
