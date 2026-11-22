import { describe, expect, it } from 'vitest';
import { extractionModelNote } from './extractionModel';

describe('extraction model setting (overnight A11)', () => {
  it('names the agent, what it asks, and where that comes from', () => {
    expect(extractionModelNote({ agent: 'claude', chosen: '', asks: 'claude-haiku-5-5' })).toBe('Now: Claude asks claude-haiku-5-5, the default (about 40× cheaper than Opus 5.5).');
    expect(extractionModelNote({ agent: 'codex', chosen: '', asks: 'gpt-5.5' })).toBe('Now: Codex asks gpt-5.5, its model from Settings → Models.');
    expect(extractionModelNote({ agent: 'codex', chosen: '', asks: null })).toBe('Now: Codex uses its own default model, its model from Settings → Models.');
    expect(extractionModelNote({ agent: 'claude', chosen: 'claude-sonnet-5-5', asks: 'claude-sonnet-5-5' })).toBe('Now: Claude asks claude-sonnet-5-5 (chosen here). A dearer model makes every extraction cost more.');
  });
});
