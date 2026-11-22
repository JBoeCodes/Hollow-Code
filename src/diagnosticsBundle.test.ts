import { describe, expect, it } from 'vitest';
import { DIAGNOSTICS_HINT, savedDiagnosticsText } from './diagnosticsBundle';

describe('diagnostics bundle words', () => {
  it('says where the zip went, its size, and that nothing was sent', () => {
    expect(savedDiagnosticsText({ path: '/Users/a/Desktop/hollow-diagnostics-20261010-030405.zip', bytes: 18_432, entries: 6 }))
      .toBe('Saved diagnostics (6 files, 18 KB) to /Users/a/Desktop/hollow-diagnostics-20261010-030405.zip. Nothing was sent.');
    expect(savedDiagnosticsText({ path: '/x.zip', bytes: 10, entries: 5 })).toContain('(5 files, 1 KB)');
  });

  it('promises only what the bundle keeps: no note text, never sent', () => {
    expect(DIAGNOSTICS_HINT).toContain('note text is never included');
    expect(DIAGNOSTICS_HINT).toContain('never sends it');
  });
});
