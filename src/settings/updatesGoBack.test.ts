import { describe, expect, it } from 'vitest';
import source from './UpdatesGoBack.tsx?raw';
import updates from './Updates.tsx?raw';

// Raw-source contracts, as UpdateButton.test.ts: no Solid DOM runner here.
describe('Settings, Updates: Go back card (U6)', () => {
  it('loads lazily from Updates and names the goingBack phase', () => {
    expect(updates).toContain("lazy(() => import('./UpdatesGoBack')");
    expect(updates).not.toMatch(/^import .*UpdatesGoBack/m);
    expect(updates).toContain("case 'goingBack': return `Going back to ${phase.version}.`;");
  });
  it('reads the plan without suspending the page', () => {
    expect(source).toContain('await api.updateGoBackPlan()');
    expect(source).not.toContain('createResource');
  });
  it('confirms before going back and offers the restore only for a newer store', () => {
    const confirm = source.indexOf('await showConfirm(');
    expect(confirm).toBeGreaterThan(-1);
    expect(source.indexOf('api.updateGoBack(restoreData, confirmedKeys ?? null)')).toBeGreaterThan(confirm);
    expect(source).toContain("plan()?.schema?.verdict === 'needsRestore'");
    expect(source).toContain('Go back and restore data from before the update');
    expect(source).toContain('(restore() && !p().restoreAvailable)');
    expect(source).toContain('danger: restoreData || !!confirmedKeys');
  });
  it('handles every outcome and the wait', () => {
    for (const outcome of ['started', 'workRunning', 'workChanged', 'refused']) expect(source).toContain(`case '${outcome}'`);
    expect(source).toContain('api.updateGoBackWhenIdle(restore())');
    expect(source).toContain('updateState()?.goBackWaiting');
    expect(source).toContain('act(cancelUpdateWait)');
  });
  it('shows the skip with Undo', () => {
    expect(source).toContain('Skipping {skipped()}.');
    expect(source).toContain('await api.updateUnskip(); await refreshUpdateState();');
  });
});
