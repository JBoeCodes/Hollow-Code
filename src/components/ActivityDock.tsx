import { For, Show, createEffect, createMemo, createSignal, onCleanup } from 'solid-js';
import { activityTasks, clearCompletedTasks, type ActivityTask, type ActivityStatus } from '../store';
import { windowAway } from '../store/runtime';

const STATUS: Record<ActivityStatus, string> = {
  pending: 'Running', success: 'Completed', warning: 'Needs attention',
  error: 'Failed', cancelled: 'Stopped',
};

function fmtElapsed(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.floor(ms / 60_000)}m ${Math.floor((ms % 60_000) / 1000)}s`;
}

/** The former dock's operation log now lives in Work > Running. Queue
 *  status and Stop/Cancel stay in MissionControl's single turn list. */
export function ActivityDock() {
  const tasks = activityTasks;
  const pending = createMemo(() => tasks().filter(t => t.status === 'pending').length);
  const [now, setNow] = createSignal(Date.now());
  createEffect(() => {
    if (!pending() || windowAway()) return;
    setNow(Date.now());
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    onCleanup(() => window.clearInterval(id));
  });
  return <Show when={tasks().length}>
    <section class="running-background" aria-label="Background tasks">
      <header class="activity-dock-head">
        <h3 class="activity-dock-title">Background tasks</h3>
        <button type="button" class="activity-dock-clear" onClick={clearCompletedTasks}
          disabled={pending() === tasks().length} title="Drop completed entries">Clear completed</button>
      </header>
      <ol class="activity-dock-list">
        <For each={tasks()}>{task => <ActivityRow task={task} now={now()} />}</For>
      </ol>
    </section>
  </Show>;
}

function ActivityRow(props: { task: ActivityTask; now: number }) {
  const t = () => props.task;
  return <li class={`activity-row ${t().status}`}>
    <span class="activity-icon" title={STATUS[t().status]} aria-label={STATUS[t().status]}>
      {t().status === 'pending' ? '⏵' : t().status === 'success' ? '✓' : t().status === 'cancelled' ? '·' : '!'}
    </span>
    <div class="activity-body">
      <div class="activity-name">
        <span>{t().name}</span>
        <span class="activity-elapsed">{fmtElapsed((t().finishedAt ?? props.now) - t().startedAt)}</span>
      </div>
      <Show when={t().detail}><div class="activity-detail">{t().detail}</div></Show>
      <Show when={t().result}><div class={`activity-result ${t().status}`}>{t().result}</div></Show>
    </div>
  </li>;
}
