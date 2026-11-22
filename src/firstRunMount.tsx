// Mounts the first-run setup sheet (H6) outside the App tree, so App.tsx and
// the App chunk gain nothing. main.tsx dynamic-imports this module once the
// app has rendered and calls `checkFirstRun`: one `first_run_status` read,
// and only when a step is open does the lazy sheet load, into its own root
// element. Settings, Sources' Run setup again calls `openFirstRun` after
// `first_run_reopen`. In the browser mock `?firstRun=1` starts a fresh setup
// (mockFirstRun.ts).
import { Suspense, lazy } from 'solid-js';
import { render } from 'solid-js/web';
import { firstRunStatus, type SetupStatus } from './firstRunApi';
import { shouldOpen } from './firstRun';

const FirstRunSheet = lazy(() => import('./components/FirstRunSheet'));

const ROOT_ID = 'first-run-root';
let dispose: (() => void) | null = null;

/** Show the sheet at `status`'s first open step (once; a second call is ignored while it shows). */
export function openFirstRun(status: SetupStatus) {
  if (dispose || !shouldOpen(status)) return;
  const el = document.createElement('div');
  el.id = ROOT_ID;
  document.body.appendChild(el);
  const close = () => {
    const stop = dispose;
    dispose = null;
    stop?.();
    el.remove();
  };
  dispose = render(() => <Suspense><FirstRunSheet status={status} onClose={close} /></Suspense>, el);
}

/** After boot: open the sheet when setup has an open step. Never throws. */
export async function checkFirstRun() {
  try {
    const status = await firstRunStatus();
    openFirstRun(status);
  } catch { /* an older backend or a failed read: no sheet */ }
}
