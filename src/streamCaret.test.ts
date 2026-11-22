import { describe, expect, it } from 'vitest';
import { showsCaret } from './streamCaret';

describe('showsCaret', () => {
  const live = { role: 'assistant', partial: true, tools: [] };
  it('shows only on the last partial assistant turn while text is arriving', () => {
    expect(showsCaret(live, true, true)).toBe(true);
  });
  it('never on earlier partial turns of the same run', () => {
    expect(showsCaret(live, false, true)).toBe(false);
  });
  it('not once a tool call follows the text', () => {
    expect(showsCaret({ ...live, tools: [{}] }, true, true)).toBe(false);
  });
  it('not while the run waits and no text is arriving', () => {
    expect(showsCaret(live, true, false)).toBe(false);
  });
  it('not on finished or user turns', () => {
    expect(showsCaret({ ...live, partial: false }, true, true)).toBe(false);
    expect(showsCaret({ role: 'user', partial: true, tools: [] }, true, true)).toBe(false);
  });
});
