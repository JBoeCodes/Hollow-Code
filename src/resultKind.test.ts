import { describe, expect, it } from 'vitest';
import { REFUSAL_NOTICE, RESULT_KIND_LABELS, resultErrorText, resultEventTurn, resultKindLabel } from './resultKind';

describe('result kinds (Paperclip PR 14a)', () => {
  it('labels every kind the backend sends, note-safe', () => {
    expect(Object.keys(RESULT_KIND_LABELS)).toEqual([
      'success', 'refusal', 'maxTurns', 'providerQuota', 'transient', 'auth', 'resumeFailed',
      'permissionDenied', 'stalled', 'budgetStop', 'cancelled', 'timedOut', 'error',
    ]);
    for (const label of Object.values(RESULT_KIND_LABELS)) {
      expect(label).not.toMatch(/[—<>]/);
    }
    expect(resultKindLabel('providerQuota')).toBe('usage limit reached');
    expect(resultKindLabel('future-kind')).toBeNull();
    expect(resultKindLabel(null)).toBeNull();
    expect(REFUSAL_NOTICE).toBe('The model declined the request (refusal).');
  });

  it('labels an error result once, and never for success or a plain failure', () => {
    const raw = 'Claude AI usage limit reached|1790374200';
    expect(resultErrorText(raw, 'providerQuota')).toBe(`(error: usage limit reached: ${raw})`);
    expect(resultErrorText('usage limit reached: again', 'providerQuota')).toBe('(error: usage limit reached: again)');
    expect(resultErrorText('Account unavailable', 'error')).toBe('(error: Account unavailable)');
    expect(resultErrorText('done', 'success')).toBe('(error: done)');
    expect(resultErrorText('no kind')).toBe('(error: no kind)');
    expect(resultErrorText('API Error: 529', 'transient')).toBe('(error: rate limited, overloaded or network error: API Error: 529)');
  });

  it('turns a result event into a refusal notice, a labelled error, or nothing', () => {
    expect(resultEventTurn({ result: 'I cannot help with that.', isError: false, resultKind: 'refusal' }))
      .toEqual({ notice: REFUSAL_NOTICE });
    expect(resultEventTurn({ result: null, isError: true, resultKind: 'refusal' })).toEqual({ notice: REFUSAL_NOTICE });
    expect(resultEventTurn({ result: 'Claude AI usage limit reached', isError: true, resultKind: 'providerQuota' }))
      .toEqual({ append: '(error: usage limit reached: Claude AI usage limit reached)' });
    expect(resultEventTurn({ result: 'boom', isError: true })).toEqual({ append: '(error: boom)' });
    expect(resultEventTurn({ result: 'done', isError: false, resultKind: 'success' })).toBeNull();
    expect(resultEventTurn({ result: null, isError: true, resultKind: 'error' })).toBeNull();
  });
});
