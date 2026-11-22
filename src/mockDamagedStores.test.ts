import { beforeEach, describe, expect, it, vi } from 'vitest';
import { rejectIfDamagedStore } from './mockDamagedStores';

describe('browser-mock damaged stores (B·F08)', () => {
  const storage = new Map<string, string>();
  beforeEach(() => {
    storage.clear();
    vi.stubGlobal('localStorage', { getItem: (k: string) => storage.get(k) ?? null, setItem: (k: string, v: string) => storage.set(k, v) });
  });
  it('lets every listing through while the stores are fine', () => {
    for (const c of ['workspace_results', 'list_all_results', 'set_result_review']) expect(() => rejectIfDamagedStore(c)).not.toThrow();
  });
  it('rejects the Results listings like the backend does for a damaged review store', () => {
    storage.set('mockResultReviewsDamaged', '1');
    expect(() => rejectIfDamagedStore('workspace_results')).toThrow(/Results review state was damaged.*moved to/);
    expect(() => rejectIfDamagedStore('list_all_results')).toThrow(/Results review state was damaged.*moved to/);
  });
  it('the peek listing (Library All) says the store was left as is, never moved', () => {
    storage.set('mockResultReviewsDamaged', '1');
    expect(() => rejectIfDamagedStore('list_all_results_peek')).toThrow(/could not be read.*left as is/);
  });
});
