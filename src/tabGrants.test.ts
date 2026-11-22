import { describe, expect, it } from 'vitest';
import { chatName, grantOn } from './tabGrants';

describe('a Preview tab shared with a chat (repo ideas 2026-10-27 #5)', () => {
  it('finds the grant on a tab', () => {
    const grants = [{ tabId: 't-1', sessionKey: 's-a', title: 'Vendor portal', grantedAt: '2026-10-27T23:00:00Z' }];
    expect(grantOn(grants, 't-1')?.sessionKey).toBe('s-a');
    expect(grantOn(grants, 't-2')).toBeNull();
    expect(grantOn(undefined, 't-1')).toBeNull();
    expect(grantOn(grants, null)).toBeNull();
  });
  it('names the open chat by its first message, cut short', () => {
    expect(chatName('  Check the  vendor\nportal  ')).toBe('Check the vendor portal');
    expect(chatName('x'.repeat(80))).toHaveLength(60);
    expect(chatName('')).toBe('the open chat');
    expect(chatName(null)).toBe('the open chat');
  });
});
