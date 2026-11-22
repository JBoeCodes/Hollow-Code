import { describe, expect, it } from 'vitest';
import { checkPhrase, checkState, runStage, RUN_STAGE, REVIEW_STATE, DELIVERY_STEP } from './workState';

describe('shared state vocabulary (next-level P01)', () => {
  it('live signals win over the recorded status', () => {
    expect(runStage({ waiting: true, running: true, status: 'success' })).toBe('waiting');
    expect(runStage({ running: true, status: 'error' })).toBe('working');
    expect(runStage({ queued: true })).toBe('queued');
    expect(runStage({ status: 'success' })).toBe('finished');
    expect(runStage({ status: 'cancelled' })).toBe('stopped');
    expect(runStage({ status: 'interrupted' })).toBe('interrupted');
    expect(runStage({ status: 'error' })).toBe('failed');
    expect(runStage({ status: null })).toBeNull();
  });
  it('missing checks are not zero failures', () => {
    expect(checkState(null)).toBe('notChecked');
    expect(checkState({ total: 0, failed: 0 })).toBe('notChecked');
    expect(checkState({ total: 3, failed: 1 })).toBe('failed');
    expect(checkState({ total: 3, failed: 0, unknown: 1 })).toBe('partial');
    expect(checkState({ total: 3, failed: 0, unknown: 0 })).toBe('passed');
  });
  it('says checks in one phrase everywhere (P1-50)', () => {
    expect(checkPhrase(null)).toBe('Not checked');
    expect(checkPhrase({ total: null, failed: null })).toBe('Not checked');
    expect(checkPhrase({ total: 0, failed: 0 })).toBe('Not checked');
    expect(checkPhrase({ total: 3, failed: 1 })).toBe('1 of 3 checks failed');
    expect(checkPhrase({ total: 1, failed: 1 })).toBe('1 of 1 check failed');
    expect(checkPhrase({ total: 3, failed: 1, unknown: 1 })).toBe('1 of 3 checks failed, 1 without a result');
    expect(checkPhrase({ total: 2, failed: 0, unknown: 5 })).toBe('0 of 2 checks passed, 2 without a result');
    expect(checkPhrase({ total: 3, failed: 0, unknown: 1 })).toBe('2 of 3 checks passed, 1 without a result');
    expect(checkPhrase({ total: 1, failed: 0, unknown: 0 })).toBe('1 check passed');
    expect(checkPhrase({ total: 4, failed: 0 })).toBe('4 checks passed');
  });
  it('finished, accepted and installed are different words', () => {
    expect(RUN_STAGE.finished.hint).toMatch(/not the same as checked or accepted/);
    expect(REVIEW_STATE.changed.label).toBe('Changed since accepted');
    expect(DELIVERY_STEP.scheduled.hint).toMatch(/pending until/);
  });
});
