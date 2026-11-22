// Automation windows and the run queue, on Automations beside the schedule dial
// (Settings PR 4, overnight 11-12; they were Settings, Background work). A
// closed fold: the windows editor and the queue mount the first time it opens
// and then stay mounted, so closing it never drops an unsaved window edit.
import { Show, Suspense, createEffect, createSignal, lazy } from 'solid-js';
import { Skeleton } from '../../components/ui';
import { automationWindowsOpen, automationWindowsRequest, clearAutomationWindowsRequest, setAutomationWindowsOpen } from '../../store/automationsViews';
import { RunQueuePanel } from './RunQueuePanel';

const AutomationWindowsPanel = lazy(() => import('./AutomationWindowsPanel').then(m => ({ default: m.AutomationWindowsPanel })));

export function WindowsAndQueue() {
  const [seen, setSeen] = createSignal(false);
  let queue: HTMLElement | undefined;
  let windows: HTMLElement | undefined;
  createEffect(() => { if (automationWindowsOpen()) setSeen(true); });
  createEffect(() => {
    const req = automationWindowsRequest();
    if (!req || !seen()) return;
    clearAutomationWindowsRequest();
    // Two frames: the fold's body has to lay out before it can scroll.
    requestAnimationFrame(() => requestAnimationFrame(() => (req.tab === 'queue' ? queue : windows)?.scrollIntoView({ block: 'start' })));
  });
  return (
    <details class="background-work workflow-windows" open={automationWindowsOpen()} onToggle={e => setAutomationWindowsOpen(e.currentTarget.open)}>
      <summary>Automation windows and the run queue</summary>
      <Show when={seen()}>
        <div class="background-work-body" tabIndex={0} role="region" aria-label="Automation windows and the run queue">
          <section ref={el => { windows = el; }} class="workflow-windows-part" aria-labelledby="workflow-windows-h">
            <h4 id="workflow-windows-h">Automation windows</h4>
            <p class="background-work-lead">Named stretches of the week an automation can run in instead of at a fixed time, and the overnight cap. Hollow’s own night queue, Friday update and nightly tidy-up use the built-in ones.</p>
            <Suspense fallback={<Skeleton label="Loading automation windows" rows={4} />}><AutomationWindowsPanel /></Suspense>
          </section>
          <section ref={el => { queue = el; }} class="workflow-windows-part" aria-labelledby="workflow-queue-h">
            <h4 id="workflow-queue-h">Run queue</h4>
            <p class="background-work-lead">Automations wait here, one at a time, while another run holds the vault. Chat and Run now go first; scheduled work waits 30 seconds after each run and a minute after a chat turn.</p>
            <RunQueuePanel />
          </section>
        </div>
      </Show>
    </details>
  );
}
