import { onCleanup, untrack } from 'solid-js';
import { registerNavigationGuard } from '../../store/navigationGuard';
import { showConfirm } from '../../store/overlays';
import { flashFooter } from '../../store/snackbars';

/** Approval covers exactly the draft named by the choice, never later edits. */
export function useAutomationDraftGuard(read: () => { title: string; revision: string } | null, saving: () => boolean) {
  onCleanup(registerNavigationGuard(() => {
    if (untrack(saving)) {
      flashFooter('Wait for the automation to finish saving.');
      return false;
    }
    const draft = untrack(read);
    if (!draft) return true;
    return showConfirm({
      title: `Discard unsaved changes to ${draft.title || 'New automation'}?`,
      body: 'These edits have not been saved.',
      confirmLabel: 'Discard', cancelLabel: 'Keep editing', danger: true,
    }).then(ok => {
      if (!ok) return false;
      if (untrack(saving) || untrack(read)?.revision !== draft.revision) {
        flashFooter('The draft changed. Choose where to go again.');
        return false;
      }
      return true;
    });
  }));
}
