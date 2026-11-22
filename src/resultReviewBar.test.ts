import { describe, expect, it } from 'vitest';
import { reviewBarModel, reviewedMessage } from './resultReviewBar';
import BAR from './components/ResultReviewBar.tsx?raw';

// COO rebuild L4: a Result opens in the Files tab with its review bar.
describe('the review bar on a Result (COO rebuild L4)', () => {
  const r = (review: 'draft' | 'ready' | 'accepted' | 'changed', over: { sha256?: string | null; hasSnapshot?: boolean } = {}) =>
    ({ review, sha256: 'sha256' in over ? over.sha256! : 'abc', hasSnapshot: over.hasSnapshot ?? false });

  it('a draft offers Mark ready and a secondary Accept', () => {
    const m = reviewBarModel(r('draft'));
    expect(m.markReady).toMatchObject({ label: 'Mark ready for review', disabled: false, primary: true });
    expect(m.accept).toMatchObject({ label: 'Accept', primary: false });
    expect(m.backToDraft).toBe(false);
    expect(m.compare).toBe(false);
  });

  it('ready and changed lead with Accept; a change can be compared with the accepted snapshot', () => {
    expect(reviewBarModel(r('ready')).accept).toMatchObject({ label: 'Accept', primary: true, title: 'Accept exactly this version' });
    const changed = reviewBarModel(r('changed', { hasSnapshot: true }));
    expect(changed.accept?.label).toBe('Accept this new version');
    expect(changed.compare).toBe(true);
    expect(reviewBarModel(r('changed')).compare).toBe(false);
    expect(changed.backToDraft).toBe(true);
  });

  it('an accepted Result offers no Accept; one too large to hash can’t be recorded; busy disables', () => {
    expect(reviewBarModel(r('accepted')).accept).toBeNull();
    expect(reviewBarModel(r('accepted')).markReady).toBeNull();
    const big = reviewBarModel(r('ready', { sha256: null }));
    expect(big.accept).toMatchObject({ disabled: true, title: 'Too large to record an exact version' });
    expect(reviewBarModel(r('ready'), true).accept?.disabled).toBe(true);
  });

  it('says what was recorded, and that nothing was published', () => {
    expect(reviewedMessage('accepted', 'Q3')).toBe('Accepted “Q3”. Nothing was sent or published.');
    expect(reviewedMessage('ready', 'Q3')).toBe('Marked “Q3” ready for review.');
    expect(reviewedMessage('draft', 'Q3')).toBe('“Q3” is a draft again.');
  });

  it('accepting stays the person’s: the bar records it through set_result_review, never an agent action', () => {
    expect(BAR).toContain('await api.setResultReview(props.workspaceId, r.path, state, r.sha256)');
    expect(BAR).not.toMatch(/runAction|app_actions/);
  });
});
