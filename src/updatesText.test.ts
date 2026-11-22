import { describe, expect, it } from 'vitest';
import { lastCheckedText, updateResultText } from './updatesText';

describe('update result text', () => {
  it('keeps the installed version and plain notes', () => {
    expect(updateResultText({ kind: 'updated', version: '1.2.3', notes: '<b>Fix</b>', channel: 'main', at: '', local: null })).toBe('Updated to 1.2.3\n<b>Fix</b>');
    // Row U10: a Local build names its branch.
    expect(updateResultText({ kind: 'updated', version: '0.5.15', notes: '', channel: 'main', at: '', local: { workspaceId: 'w', branch: 'feat/x', buildId: 'b', dirty: false } })).toBe('Updated to the local build of feat/x (0.5.15)');
  });
  it('explains recovery and an unavailable restored version', () => {
    expect(updateResultText({ kind: 'wentBack', attempted: '2', restored: '1', reason: 'Start failed', at: '', manual: false, dataRestored: false })).toBe('Went back to 1 because Start failed');
    expect(updateResultText({ kind: 'wentBack', attempted: '2', restored: null, reason: 'Start failed', at: '', manual: false, dataRestored: false })).toContain('the previous version');
  });
  it('names a Go back and what it skips (row U6)', () => {
    expect(updateResultText({ kind: 'wentBack', attempted: '0.6.20', restored: '0.6.19', reason: '', at: '', manual: true, dataRestored: false })).toBe('Went back to 0.6.19. Skipping 0.6.20.');
    expect(updateResultText({ kind: 'wentBack', attempted: '0.6.20', restored: '0.6.19', reason: '', at: '', manual: true, dataRestored: true })).toContain('newer ones are kept aside');
    expect(updateResultText({ kind: 'notWentBack', target: '0.6.19', reason: null, at: '' })).toBe("Didn't go back to 0.6.19: No reason was reported.");
  });
  it('explains a failed install even without a reason', () => {
    expect(updateResultText({ kind: 'notInstalled', attempted: '2', reason: 'Signature failed', at: '' })).toBe('Update 2 was not installed: Signature failed');
    expect(updateResultText({ kind: 'notInstalled', attempted: '2', reason: null, at: '' })).toContain('No reason was reported.');
  });
});

describe('last checked text', () => {
  const now = Date.parse('2026-11-15T12:00:00Z');
  it('handles missing, invalid and future timestamps', () => {
    expect(lastCheckedText(null, now)).toBe('Last checked never');
    expect(lastCheckedText('bad', now)).toBe('Last checked never');
    expect(lastCheckedText('2026-11-16T12:00:00Z', now)).toBe('Last checked just now');
  });
  it('uses minutes, hours and days with singular and plural units', () => {
    for (const [minutes, text] of [[1, '1 minute'], [2, '2 minutes'], [60, '1 hour'], [120, '2 hours'], [1440, '1 day'], [2880, '2 days']] as const) {
      expect(lastCheckedText(new Date(now - minutes * 60_000).toISOString(), now)).toBe(`Last checked ${text} ago`);
    }
  });
});
