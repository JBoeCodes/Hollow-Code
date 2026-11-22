// The single action-detail home, under Automations. The existing Main store
// owns loading, retained rows and refreshes; opening this view adds no read.
import { createEffect, For, Show } from 'solid-js';
import { actLabel } from '../mainAgent';
import { recentActionsFact } from '../mainAgentSpace';
import { mainAgentActs, mainAgentActsRead, mainAgentRowsLoading, refreshMainAgentChildren } from '../store/mainAgent';
import { consumeMainActionsReveal, mainActionsOpen, mainActionsRevealRequest, setMainActionsOpen } from '../store/automationsViews';
import '../styles/main-actions.css';

export default function MainAgentActions() {
  let summary!: HTMLElement;
  createEffect(() => {
    const request = mainActionsRevealRequest();
    if (!request) return;
    queueMicrotask(() => {
      if (!summary.isConnected || !consumeMainActionsReveal(request)) return;
      // A late lazy load must not take focus from a control the person reached.
      const focused = document.activeElement;
      if (focused && focused !== document.body && focused !== document.documentElement
        && focused !== summary && focused.isConnected) return;
      summary.focus({ preventScroll: true });
      summary.scrollIntoView({ block: 'nearest', behavior: 'instant' });
    });
  });
  return <details class="main-action-history" open={mainActionsOpen()} onToggle={e => setMainActionsOpen(e.currentTarget.open)}>
    <summary ref={summary}>COO actions · {recentActionsFact(mainAgentActs().length, mainAgentActsRead())}</summary>
    <Show when={mainActionsOpen()}>
      <div class="main-action-history-body" tabIndex={0} role="region" aria-label="COO actions">
        <Show when={mainAgentActsRead() === 'error'}><p role="status">{mainAgentActs().length ? 'Could not refresh its actions. Showing the last read.' : 'Actions are unavailable.'}</p></Show>
        <button type="button" class="btn-link" disabled={mainAgentRowsLoading()} onClick={() => refreshMainAgentChildren(0)}>{mainAgentActsRead() === 'error' ? 'Retry actions' : 'Refresh actions'}</button>
        <Show when={mainAgentActs().length} fallback={<Show when={mainAgentActsRead() !== 'error'}><p>{mainAgentActsRead() === 'ready' ? 'No recent actions recorded.' : 'Reading its actions…'}</p></Show>}>
          <Show when={mainAgentActs().length > 20}><p>Showing the latest 20 of {mainAgentActs().length} recorded actions.</p></Show>
          <ul><For each={mainAgentActs().slice(0, 20)}>{act => <li>
            <time dateTime={act.at}>{new Date(act.at).toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}</time>
            <div>{actLabel(act.action)}<Show when={act.summary}><p>{act.summary}</p></Show></div>
          </li>}</For></ul>
        </Show>
      </div>
    </Show>
  </details>;
}
