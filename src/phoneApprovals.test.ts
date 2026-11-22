import { describe, expect, it } from 'vitest';
import { APPROVAL_WAIT_CHOICES, approvalsLine } from './phoneApprovals';

describe('approvalsLine', () => {
  it('says off, with what a card does then', () => {
    expect(approvalsLine(null, () => 'x')).toMatch(/off.*window/);
  });
  it('shows the fingerprint and when it was set up', () => {
    expect(approvalsLine({ fingerprint: 'a1b2 c3d4', enrolledAt: '2026-11-19T12:00:00Z' }, iso => `at ${iso}`)).toBe(
      'Approvals from this iPhone: on · key a1b2 c3d4 · since at 2026-11-19T12:00:00Z',
    );
  });
});

describe('APPROVAL_WAIT_CHOICES', () => {
  it('stay inside the range Hollow accepts (2 to 15 minutes) and include the 5 minute default', () => {
    expect(APPROVAL_WAIT_CHOICES.map(c => c.seconds)).toEqual([120, 300, 600, 900]);
  });
});
