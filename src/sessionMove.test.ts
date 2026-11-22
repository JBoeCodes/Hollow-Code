import { describe, expect, it } from 'vitest';
import { sessionWorkspaceId, workspaceChoices, workspaceFromChat } from './sessionMove';
import type { WorkWorkspace } from './workspaceTypes';

const w = (id: string, title: string, extra: Partial<WorkWorkspace> = {}): WorkWorkspace =>
  ({ id, title, status: 'active', path: `workspaces/${id}`, cwd: `/v/workspaces/${id}`, modified: '', ...extra });

describe('workspace choices for New conversation… and Move to…', () => {
  const alpha = w('alpha', 'Alpha'), beta = w('beta', 'Beta', { status: 'planning' }), gamma = w('gamma', 'Gamma');
  const quick = w('quick-chats', 'Quick chats', { kind: 'quick' });

  it('puts Quick chats first, then rail order, then the rest, each once', () => {
    const choices = workspaceChoices([gamma, quick, alpha, beta], [beta, alpha]);
    expect(choices.map(c => c.id)).toEqual(['quick-chats', 'beta', 'alpha', 'gamma']);
    expect(choices[0]).toEqual({ id: 'quick-chats', title: 'Quick chats', hint: 'No project', quick: true });
    expect(choices[1]).toEqual({ id: 'beta', title: 'Beta', hint: 'planning', quick: false });
  });

  it('offers Quick chats before it exists', () => {
    expect(workspaceChoices([alpha], []).map(c => c.id)).toEqual(['quick-chats', 'alpha']);
  });

  it('leaves out the conversation’s own workspace', () => {
    expect(workspaceChoices([quick, alpha, beta], [alpha], 'alpha').map(c => c.id)).toEqual(['quick-chats', 'beta']);
    expect(workspaceChoices([quick, alpha], [], 'quick-chats').map(c => c.id)).toEqual(['alpha']);
  });
});

describe('which workspace a conversation is in', () => {
  const all = [w('alpha', 'Alpha'), w('beta', 'Beta')];
  it('reads the rail groups first, then the cwd, else none', () => {
    const groups = { beta: { sessions: [{ key: 's-1' }] } };
    expect(sessionWorkspaceId('s-1', '/v/workspaces/alpha', groups, all)).toBe('beta');
    expect(sessionWorkspaceId('s-2', '/v/workspaces/alpha', groups, all)).toBe('alpha');
    expect(sessionWorkspaceId('s-3', '/v', groups, all)).toBeNull();
    expect(sessionWorkspaceId('s-3', null, {}, all)).toBeNull();
  });
});

describe('New workspace from this chat', () => {
  it('titles it from the conversation and takes the first ask as the outcome', () => {
    expect(workspaceFromChat('Draft the launch memo', 'Help me draft the Q4 launch memo\nwith pricing')).toEqual({
      title: 'Draft the launch memo', goal: 'Help me draft the Q4 launch memo\nwith pricing',
    });
  });
  it('falls back to the first ask’s first line, then a placeholder', () => {
    expect(workspaceFromChat(null, '\n  Plan the offsite  \nsecond line').title).toBe('Plan the offsite');
    expect(workspaceFromChat('  ', '')).toEqual({ title: 'Conversation', goal: 'Continue the conversation “Conversation”.' });
  });
  it('fits the backend limits', () => {
    const made = workspaceFromChat('x'.repeat(300), 'é'.repeat(20_000));
    expect(made.title.length).toBe(100);
    expect(new TextEncoder().encode(made.goal).length).toBeLessThanOrEqual(12_000);
    expect(made.goal.length).toBeGreaterThan(1000);
  });
});
