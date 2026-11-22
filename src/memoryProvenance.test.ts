import { describe, expect, it } from 'vitest';
import { memorySource } from './memoryProvenance';

describe('memory provenance (next-level P18)', () => {
  it('names the run a memory was learned in', () => {
    expect(memorySource('run-chat-0')).toBe('A conversation (run-chat-0).');
    expect(memorySource('run-workflow-1')).toBe('An automation run (run-workflow-1).');
    expect(memorySource('r-123')).toBe('Run r-123.');
    expect(memorySource(null)).toMatch(/No source run recorded/);
    expect(memorySource('  ')).toMatch(/No source run recorded/);
  });
});
