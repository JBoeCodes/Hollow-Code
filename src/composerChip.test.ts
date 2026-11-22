import { describe, expect, it } from 'vitest';
import { composerPlaceholder } from './composerChip';

describe('composer placeholder says where the message goes', () => {
  it('names the project in a project chat', () => {
    expect(composerPlaceholder('Claude', { kind: 'project', title: 'Q3 regional operations review' })).toBe('Message Claude in Q3 regional operations review…');
    expect(composerPlaceholder('Codex', { kind: 'project', title: '  hollow ' })).toBe('Message Codex in hollow…');
  });
  it('falls back to the agent alone without a project title', () => {
    expect(composerPlaceholder('Claude', { kind: 'project', title: '' })).toBe('Message Claude…');
    expect(composerPlaceholder('Claude', { kind: 'quick' })).toBe('Message Claude…');
    expect(composerPlaceholder('Claude', { kind: 'vault' })).toBe('Message Claude…');
  });
  it('addresses the COO by its role', () => {
    expect(composerPlaceholder('Claude', { kind: 'main' })).toBe('Message the COO…');
  });
});
