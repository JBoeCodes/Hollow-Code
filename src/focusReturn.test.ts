import { describe, expect, it } from 'vitest';
import { returnsFocus, returnsFocusAfterRun } from './focusReturn';
import PALETTE from './components/CommandPalette.tsx?raw';

describe('focus back to the opener after a dismissed overlay (H12)', () => {
  it('returns focus when it fell to the body or is still inside the closing overlay', () => {
    expect(returnsFocus(true, 'body')).toBe(true);
    expect(returnsFocus(true, 'overlay')).toBe(true);
  });

  it('leaves focus that went somewhere real, and never focuses a removed opener', () => {
    expect(returnsFocus(true, 'elsewhere')).toBe(false);
    expect(returnsFocus(false, 'body')).toBe(false);
    expect(returnsFocus(false, 'overlay')).toBe(false);
  });

  it('the palette remembers its opener and gives focus back on Escape and on the backdrop', () => {
    // Read once as it opens, before its input takes focus (not on every rerun of the reset effect).
    expect(PALETTE).toMatch(/on\(commandPaletteOpen,\s*open\s*=>\s*\{\s*if\s*\(open\)\s*opener\s*=\s*focusOpener\('\.palette-backdrop'\)/);
    expect(PALETTE).toContain("giveFocusBack(back, '.palette-backdrop')");
    expect(PALETTE).toMatch(/e\.key === 'Escape'\) \{\s*e\.preventDefault\(\);\s*dismiss\(\);/);
    expect(PALETTE).toContain('onClick={dismiss}');
    // Running a row isn't a dismissal: execute() never calls dismiss().
    expect(PALETTE).toMatch(/function execute\(item: PaletteItem\) \{[\s\S]*?closeCommandPalette\(\);[\s\S]*?\n {2}\}/);
    expect(PALETTE.match(/function execute\(item: PaletteItem\) \{[\s\S]*?\n {2}\}/)?.[0]).not.toContain('dismiss');
  });
});

describe('focus back to the opener after a palette row that does not navigate (A05)', () => {
  const stamp = 'today|ws-1|home|all';

  it('returns focus when the location is unchanged and focus fell to the body', () => {
    expect(returnsFocusAfterRun(stamp, stamp, true, 'body')).toBe(true);
  });

  it('leaves focus in an overlay the row opened again (by then the run’s own has closed)', () => {
    expect(returnsFocusAfterRun(stamp, stamp, true, 'overlay')).toBe(false);
  });

  it('leaves focus alone when the row changed the page, workspace or view', () => {
    expect(returnsFocusAfterRun(stamp, 'tasks|ws-1|home|all', true, 'body')).toBe(false);
    expect(returnsFocusAfterRun(stamp, 'today|ws-2|home|all', true, 'body')).toBe(false);
    expect(returnsFocusAfterRun(stamp, 'today|ws-1|map|all', true, 'overlay')).toBe(false);
  });

  it('leaves focus a row put somewhere real (a dialog it opened), and never focuses a removed opener', () => {
    expect(returnsFocusAfterRun(stamp, stamp, true, 'elsewhere')).toBe(false);
    expect(returnsFocusAfterRun(stamp, stamp, false, 'body')).toBe(false);
    expect(returnsFocusAfterRun(stamp, stamp, false, 'overlay')).toBe(false);
  });

  it('the palette reads the location before a non-keepOpen row runs and gives focus back once it settles', () => {
    const run = PALETTE.match(/function execute\(item: PaletteItem\) \{[\s\S]*?\n {2}\}/)?.[0] ?? '';
    // keepOpen rows ("More…") stay in the palette: no focus return there.
    expect(run).toMatch(/if \(item\.keepOpen\) \{[\s\S]*?return;\s*\}\s*const back = opener; opener = null;\s*const stamp = locationStamp\(\);\s*closeCommandPalette\(\);/);
    expect(run).toContain("flashFooter(`× ${err?.message || err}`, 3500);");
    expect(run).toContain(".finally(() => giveFocusBackAfterRun(back, '.palette-backdrop', stamp, locationStamp))");
  });
});
