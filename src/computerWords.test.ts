import { describe, expect, it } from 'vitest';
import { actionWords } from './computerWords';

describe('computer control in a chat (Cua CU8)', () => {
  it('says what an action did, with its tier, and who refused it', () => {
    expect(actionWords({ tool: 'click', tier: 'act', outcome: 'ok', detail: 'ran' })).toBe('click: act tier, ran');
    expect(actionWords({ tool: 'type_text', tier: 'act', outcome: 'failed', detail: 'driver error', app: 'com.apple.notes' }))
      .toBe('type_text in com.apple.notes: act tier, failed');
    expect(actionWords({ tool: 'click', tier: 'act', outcome: 'refused', detail: 'paused' })).toBe('click: act tier, refused (paused)');
    expect(actionWords({ tool: 'x', tier: '', outcome: 'refused', detail: 'plan mode' })).toBe('x: no tier, refused (plan mode)');
  });
});
