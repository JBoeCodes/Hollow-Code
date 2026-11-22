import { For, Show, createSignal, onCleanup } from 'solid-js';
import { LAUNCH_IPC_MAX_SAME_CALL, LAUNCH_IPC_WINDOW_MS, launchIpcReport } from '../launchIpc';
import type { CapabilityState } from '../capabilityDoctor';

export function capabilityTone(state: CapabilityState): 'ok' | 'warn' | 'error' | undefined {
  if (state === 'live') return 'ok';
  if (state === 'stale') return 'warn';
  if (state === 'broken') return 'error';
  return undefined;
}

/** The word a capability chip shows (night ST10 follow-up): plain words, not the state's id. */
export function capabilityWord(state: CapabilityState): string {
  if (state === 'live') return 'working';
  if (state === 'stale') return 'not checked lately';
  return state;
}

/** Overnight Y7: this window's IPC calls during launch, against the repeat
 *  budget (launchIpc.ts). Diagnostics only; re-read while the window runs. */
export function LaunchIpcPanel() {
  const [report, setReport] = createSignal(launchIpcReport());
  const timer = window.setInterval(() => {
    const next = launchIpcReport();
    setReport(next);
    if (next.settled) window.clearInterval(timer);
  }, 1000);
  onCleanup(() => window.clearInterval(timer));
  const seconds = LAUNCH_IPC_WINDOW_MS / 1000;
  const over = () => report().repeated.length;
  return (
    <div class="startup-warmup">
      <div class="startup-warmup-head">
        <div>
          <div class="settings-row-v">
            {report().total} requests ({report().commands} kinds) in this window's first {seconds} s
            {report().settled ? '' : ' so far'}. Budget: {LAUNCH_IPC_MAX_SAME_CALL} repeats of one call.
          </div>
        </div>
        <span class="settings-state-chip" data-state={over() ? 'warn' : report().settled ? 'ok' : undefined}>
          {over() ? `${over()} over` : report().settled ? 'within budget' : 'counting'}
        </span>
      </div>
      <Show when={over()}>
        <div class="startup-warmup-list" role="list" aria-label="Calls repeated past the budget">
          <For each={report().repeated.slice(0, 8)}>
            {row => (
              <div class="startup-warmup-row launch-ipc-row" role="listitem" data-status="error">
                <span class="startup-dot" />
                <span class="startup-label">{row.command}</span>
                <span class="launch-ipc-args" title={row.args}>{row.args || 'no details'}</span>
                <span class="launch-ipc-count">×{row.count}</span>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
