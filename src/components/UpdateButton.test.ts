import { describe, expect, it } from 'vitest';
import source from './UpdateButton.tsx?raw';
import topBar from './TopBar.tsx?raw';

// Like runsOnHint.test.ts, these use raw source: this repo's unit runner
// has no Solid DOM transform or component rendering environment.
describe('UpdateButton disclosure contracts', () => {
  it('places the update control before notifications and keeps waiting visible', () => {
    expect(topBar.indexOf('<UpdateButton />')).toBeLessThan(topBar.indexOf('<NotificationBell />'));
    expect(source).toContain('<Show when={updateReady() || installing()}>');
    expect(source).toContain("updateWaiting() ? 'Update waiting for chats' : 'Update'");
  });
  it('keeps installation behind store checks and offers no forced restart', () => {
    expect(source).toContain('await restartToUpdate()');
    expect(source).toContain("outcome?.outcome === 'workRunning'");
    expect(source).toContain('updateWork().length > 0 || workRunning()');
    expect(source).toContain('act(hasWork() ? updateWhenChatsFinish : restart)');
    expect(source).not.toMatch(/Restart anyway|confirmedKeys/);
    expect(source).toContain('act(cancelWait)');
    expect(source).toContain('return cancelUpdateWait()');
    expect(source).toContain('if (updateWork().length === 0 ||');
    expect(source).toContain('(wasWaiting && !state.waiting)) setWorkRunning(false)');
    expect(source).toContain('<Show when={!updateWaiting()}>');
    expect(source).toContain('act(dismissUpdate)');
  });
  it('announces each restart result once with an acknowledged Updates action', () => {
    expect(source).toContain('let notifiedUpdateAt: string | undefined;');
    expect(source).toContain('createEffect(() => {');
    expect(source).toContain('const result = lastUpdateResult();');
    expect(source).toContain('if (!result || notifiedUpdateAt === result.at) return;');
    expect(source).toContain('notifiedUpdateAt = result.at;');
    expect(source).toContain('flashWithUndo({');
    expect(source).toContain('message: updateResultText(result)');
    expect(source).toContain("kind: result.kind === 'updated' ? 'status' : 'error'");
    expect(source).toContain('ttlMs: 0');
    expect(source).toContain("actionLabel: 'Open Updates'");
    expect(source).toContain("undo: () => { openSettingsFromOutside('updates'); }");
    expect(source).toContain('onDismiss: () => { void ackLastUpdateResult(); }');
  });
  it('provides dialog semantics, focus return and listener cleanup', () => {
    expect(source).toContain('aria-haspopup="dialog"');
    expect(source).toContain('role="dialog" aria-labelledby={`${id}-heading`}');
    expect(source).toContain('<h2 id={`${id}-heading`}>');
    expect(source).toContain('Hollow update: ');
    expect(source).toContain('Hollow {release().version} {updateState()?.phase.phase');
    expect(source).toContain('panel?.focus()');
    expect(source).toContain('trigger?.focus()');
    for (const event of ['pointerdown', 'focusin', 'keydown']) {
      expect(source).toContain(`document.addEventListener('${event}'`);
      expect(source).toContain(`document.removeEventListener('${event}'`);
    }
    expect(source).toContain("event.key === 'Escape'");
  });
  it('retains the installing disclosure and replaces actions with restart status', () => {
    expect(source).toContain("updateState()?.phase.phase === 'installing' || updateInstalling()");
    expect(source).toContain('<Show when={installing()} fallback={<div class="update-actions">');
    expect(source).toContain('<p role="status">Restarting Hollow to update...</p>');
    expect(source).toContain('if (open() && installing()) queueMicrotask(() => panel?.focus())');
    expect(source).toContain('clearUpdateError();');
  });
  it('explains a channel downgrade', () => {
    expect(source).toContain('<Show when={release().downgrade}>');
    expect(source).toContain('This is an older version than the one you have. You switched channel.');
  });
  it('renders plain scrollable notes and the shared error component', () => {
    expect(source).toContain('{release().notes}');
    expect(source).not.toContain('innerHTML');
    expect(source).toContain('<ErrorText text={error()}');
    expect(source).not.toMatch(/[\u2013\u2014]/);
  });
});
