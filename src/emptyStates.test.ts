import { describe, expect, it } from 'vitest';
import waitingSource from './components/WaitingOnYouPanel.tsx?raw';
import { reviewEmptyText } from './waitingOnYou';

describe('empty states (overnight G1)', () => {
  it('keeps the empty state Waiting on you already had', () => {
    // Each Review view names itself when empty (P0-09); All keeps its line.
    expect(waitingSource).toContain('reviewEmptyText(props.view');
    expect(reviewEmptyText('all', null, false)).toBe('Nothing is waiting on you.');
  });
});
