import { describe, expect, it } from 'vitest';
import { ultracodeNote } from './ultracode';

describe('Ultracode switch wording (merged plan Track U)', () => {
  it('says what Claude does and what every other agent gets instead', () => {
    expect(ultracodeNote('claude', true)).toMatch(/^On for this chat\. Claude fans work out to subagents/);
    expect(ultracodeNote('claude', false)).toMatch(/^Off\./);
    for (const agent of ['codex', 'antigravity', 'grok']) {
      expect(ultracodeNote(agent, true)).toMatch(/top effort/);
      expect(ultracodeNote(agent, true)).not.toMatch(/workflows/);
    }
  });
});
