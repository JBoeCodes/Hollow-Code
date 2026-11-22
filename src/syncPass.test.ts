import { describe, expect, it } from 'vitest';
import { passFromEvent } from './syncPass';

describe('passFromEvent', () => {
  it('reads the {ok} payload', () => {
    expect(passFromEvent({ ok: true }, 5)).toEqual({ at: 5, ok: true });
    expect(passFromEvent({ ok: false }, 6)).toEqual({ at: 6, ok: false });
  });

  it('counts anything else as a failed pass', () => {
    for (const payload of [null, undefined, 1, 'ok', {}, { ok: 'true' }, { ok: 1 }, []]) {
      expect(passFromEvent(payload, 7).ok).toBe(false);
    }
  });
});
