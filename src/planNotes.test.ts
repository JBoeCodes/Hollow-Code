import { describe, expect, it } from 'vitest';
import { readPlan } from './planModel';

describe('Plan notes presentation contract', () => {
  it('retains multiple paragraphs, links and original checklist line identities', () => {
    const text = '# Plan\n\nKeep [archive](outputs/archive.md) intact.\n\nSecond paragraph.\n\n## Build\n- [ ] Ship\n';
    const plan = readPlan(text);
    expect(plan.sections[0].notes.map(n => n.markdown).join('\n\n')).toBe('Keep [archive](outputs/archive.md) intact.\n\nSecond paragraph.');
    expect(plan.sections[1].rows[0].id.line).toBe(8);
    expect(plan.counts).toEqual({ total: 1, open: 1, done: 0 });
  });
});
