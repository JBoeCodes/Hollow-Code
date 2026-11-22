// Overnight B8: the keyboard and screen-reader fixes on tonight's UI, pinned
// in the source. Vitest runs in node here (no jsdom), so these read the
// components rather than render them; `focusHold.test.ts` covers the ring.
import { describe, expect, it } from 'vitest';
import PALETTE from './components/CommandPalette.tsx?raw';
import CONFIRM from './components/ConfirmDialog.tsx?raw';
import CONFLICTS from './components/ConflictCopiesDialog.tsx?raw';
import ACTIVITY_VIEW from './components/MissionControl.tsx?raw';
import DIFF from './components/DiffView.tsx?raw';
import SNACKBARS from './components/SnackbarStack.tsx?raw';
import SNACKBAR_STATE from './store/snackbars.ts?raw';
import SETTINGS from './pages/Settings.tsx?raw';

const SOURCES: Record<string, string> = {
  './components/CommandPalette.tsx': PALETTE,
  './components/ConfirmDialog.tsx': CONFIRM,
  './components/ConflictCopiesDialog.tsx': CONFLICTS,
  './components/DiffView.tsx': DIFF,
  './components/SnackbarStack.tsx': SNACKBARS,
  './store/snackbars.ts': SNACKBAR_STATE,
  './pages/Settings.tsx': SETTINGS,
};
const src = (path: string) => SOURCES[path];

describe('keyboard and screen-reader pass (overnight B8)', () => {
  it('the palette is a combobox naming the row Enter will run', () => {
    const p = src('./components/CommandPalette.tsx');
    for (const attr of ['role="combobox"', 'aria-controls="palette-listbox"', 'aria-activedescendant=', "role={filtered().length > 0 ? 'listbox' : undefined}", 'role="option"', 'aria-selected={idx === props.selectedIdx}', 'id={`palette-opt-${idx}`}']) {
      expect(p, attr).toContain(attr);
    }
  });

  it('a confirm reads its body, lets a long body scroll by keyboard, and gives focus back', () => {
    const c = src('./components/ConfirmDialog.tsx');
    // A one-line summary, when given, describes it instead of a long body (P1-38).
    expect(c).toContain("aria-describedby={[dialog().summary ? 'confirm-summary' : dialog().body ? 'confirm-body' : '', dialog().blockedReason ? 'confirm-blocked' : ''].filter(Boolean).join(' ') || undefined}");
    expect(c).toContain('tabIndex={bodyScrolls() ? 0 : undefined}');
    expect(c).toContain('[bodyScrolls() ? bodyRef : undefined, inputRef');
    expect(c, 'Enter on the body chooses nothing').toContain('if (bodyRef && document.activeElement === bodyRef) return;');
    expect(c).toContain('before.focus({ preventScroll: true })');
    expect(c).toContain('d.danger || d.focusCancel ? cancelBtnRef : confirmBtnRef');
  });

  it('the hand-made dialogs hold Tab inside and give focus back on close', () => {
    for (const f of ['./components/ConflictCopiesDialog.tsx']) {
      const s = src(f);
      expect(s, f).toContain("import { holdFocus } from '../focusHold'");
      expect(s, f).toContain('release = holdFocus(card)');
      expect(s, `${f}: a dialog closed before its microtask is not held`).toMatch(/if \(!card \|\| !\w+\(\)\) return;/);
      expect(s, f).toMatch(/aria-label="Close [^"]*\(esc\)">Close</);
    }
  });

  it('Work\'s views are regions in the page, not modal dialogs (consolidation D)', () => {
    for (const [f, s] of Object.entries({ MissionControl: ACTIVITY_VIEW })) {
      expect(s, f).not.toContain('aria-modal="true"');
      expect(s, f).not.toContain('holdFocus');
      expect(s, f).toContain('role="region"');
    }
  });

  it('loading and errors are announced, and a disabled choice says why in words', () => {
    const cc = src('./components/ConflictCopiesDialog.tsx');
    expect(cc).toContain('role="list" aria-label="Conflict copies"');
    expect(cc, 'live roles stay inside list items').not.toMatch(/<li[^>]*role="(status|alert)"/);
    expect(cc).toContain('id="ccp-protected"');
    expect(cc).toContain("aria-describedby={current()?.originalProtected ? 'ccp-protected' : undefined}");
    expect(cc).not.toContain("title={current()?.originalProtected");
    expect(src('./components/DiffView.tsx')).toContain('aria-label="Changed lines" tabIndex={0}');
  });

  it("an Undo snackbar waits while it has focus or the pointer", () => {
    const bar = src('./components/SnackbarStack.tsx');
    expect(bar).toContain("onFocusIn={() => holdSnackbar(bar.id, 'focus')}");
    expect(bar).toContain("onMouseEnter={() => holdSnackbar(bar.id, 'hover')}");
    expect(src('./store/snackbars.ts')).toContain("export function releaseSnackbar(id: string, reason = 'interaction')");
  });

  it('Settings keeps saving diagnostics focusable', () => {
    expect(src('./pages/Settings.tsx')).toContain('aria-disabled={savingDiag()} aria-busy={savingDiag()}');
  });
});
