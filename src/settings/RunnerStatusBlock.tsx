import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { capsSentence, checkedSentence, gateSentence, ownerSentence, preflightFacts, routeReason, routeWhere, runnerStateLabel, runnerStatusStale } from '../runnerStatus';
import type { RouteStatus, RunnerStatus } from '../types';
import { errMsg, fmtTime } from '../utils';
import { showConfirm } from '../store/overlays';
import { ErrorText } from '../components/ui';
import '../styles/runs-on.css';

/** The remote runner's state under Settings, Devices, Remote runner
 *  (overnight N1, 2026-11-02; moved from Runs on by remote runners 2): whether
 *  it answers, its preflight line (free disk, memory, toolchain versions),
 *  what runs there now and its last gate, with Check now, and who it belongs
 *  to (its owner record and the other SSH keys that can log in there, with
 *  Record me as owner). Opening Settings checks again when the last answer is
 *  over five minutes old; agents read the same with `build.runner_status`.
 *  Nothing shows while no runner is set up (the Remote runner row says so). */
export function RunnerStatusBlock() {
  const [status, setStatus] = createSignal<RunnerStatus | null>(null);
  const [checking, setChecking] = createSignal(false);
  const [recording, setRecording] = createSignal(false);
  const [error, setError] = createSignal('');
  // "checked 2m ago" and the gate's times move on while Settings stays open.
  const [tick, setTick] = createSignal(0);
  const timer = setInterval(() => setTick(t => t + 1), 30_000);
  let alive = true;
  onCleanup(() => { alive = false; clearInterval(timer); });
  async function check() {
    if (checking()) return;
    setChecking(true);
    setError('');
    try {
      const next = await api.runnerStatus(true);
      if (alive) setStatus(next);
    } catch (e) {
      if (alive) setError(errMsg(e));
    } finally {
      if (alive) setChecking(false);
    }
  }
  async function record() {
    if (recording() || checking()) return;
    if (status()?.owner?.state === 'other' && !(await showConfirm({
      title: 'Record this Hollow as the owner?',
      body: 'Another Hollow recorded itself as this runner\'s owner. Recording replaces that record with this one; it changes nothing else there. Do this only if the runner is yours.',
      confirmLabel: 'Record me as owner',
      cancelLabel: 'Keep the other record',
    }))) return;
    setRecording(true);
    setError('');
    try {
      const next = await api.runnerRecordOwner();
      if (alive) setStatus(next);
    } catch (e) {
      if (alive) setError(errMsg(e));
    } finally {
      if (alive) setRecording(false);
    }
  }
  onMount(() => {
    void api.runnerStatus(false).then(last => {
      if (!alive) return;
      setStatus(last);
      if (runnerStatusStale(last)) void check();
    }, e => { if (alive) setError(errMsg(e)); });
  });
  const facts = () => {
    const s = status();
    if (!s) return [];
    return [...(s.machine ? [s.machine] : []), ...preflightFacts(s.preflight)];
  };
  const answered = () => {
    const s = status();
    return s?.state === 'ready' || s?.state === 'refusing';
  };
  const when = () => { tick(); const s = status(); return s ? checkedSentence(s) : ''; };
  return (
    <>
    <Show when={status()?.host}>
      {host => (
        <div class="runner-status" role="group" aria-label={`Remote runner ${host()}`}>
          <div class="runner-status-head">
            <span class="runner-status-state" aria-live="polite" data-state={checking() && status()?.state === 'unchecked' ? 'checking' : status()!.state}>
              {checking() && status()?.state === 'unchecked' ? 'Checking…' : runnerStateLabel(status()!.state)}
            </span>
            <span class="runner-status-when">{host()}{when() ? ` · ${when()}` : ''}</span>
            {/* aria-disabled, not disabled: WebKit drops the focus of a
                button that becomes disabled; check() ignores repeats. */}
            <button type="button" class="btn small" aria-disabled={checking()} onClick={() => void check()}>
              {checking() ? 'Checking…' : 'Check now'}
            </button>
          </div>
          <Show when={status()!.message}>{msg => <p class="settings-row-warn runner-status-why">{msg()}</p>}</Show>
          {/* S5b-3: closed at rest, so the row keeps its state line; the summary names what is inside. */}
          <Show when={answered()}>
          <details class="settings-fold runner-status-more">
          <summary>Preflight, last gate and owner</summary>
          <Show when={answered() && facts().length}>
            <ul class="runner-status-facts" aria-label="Its preflight">
              <For each={facts()}>{fact => <li>{fact}</li>}</For>
            </ul>
          </Show>
          <Show when={answered()}>
            <p class="runner-status-line">
              {status()!.running ? `Running there now: ${status()!.running} remote ${status()!.running === 1 ? 'session' : 'sessions'}. ` : ''}
              {(tick(), gateSentence(status()!.lastGate, status()!.gateRunning))}
            </p>
          </Show>
          <Show when={answered() && status()!.owner}>
            {owner => (
              <div class="runner-owner" role="group" aria-label="Its owner">
                <p class="runner-status-line">
                  {ownerSentence(owner())}
                  {/* aria-disabled for the same WebKit focus reason as Check now. */}
                  <Show when={owner().state !== 'you' || owner().keyChanged}>
                    {' '}<button type="button" class="btn small" aria-disabled={recording() || checking()} onClick={() => void record()}>
                      {recording() ? 'Recording…' : 'Record me as owner'}
                    </button>
                  </Show>
                </p>
                <For each={owner().warnings}>{w => <p class="settings-row-warn runner-status-why">{w}</p>}</For>
              </div>
            )}
          </Show>
          </details>
          </Show>
        </div>
      )}
    </Show>
    {/* Shown even before any status arrived (the first read failed), with a
        way to try again. */}
    <Show when={error()}>{msg => <ErrorText as="p" class="runner-status-why" text={`Couldn't check the runner: ${msg()}`} />}</Show>
    <Show when={(error() || checking()) && !status()?.host}>
      <button type="button" class="btn small runner-status-retry" aria-disabled={checking()} onClick={() => void check()}>
        {checking() ? 'Checking…' : 'Check now'}
      </button>
    </Show>
    </>
  );
}

/** Settings, Devices, Builds and tests (smart routing 3, 2026-11-03):
 *  hollow-slot's latest decisions (where each build or test command ran and
 *  why, newest first) and what the runner said it has at its last
 *  preparation. Read from this Mac's slot folder with the runner's status,
 *  again every 30 s while Settings is open. Nothing shows before the first
 *  decision or report. */
export function RouteDecisionsBlock() {
  const [routing, setRouting] = createSignal<RouteStatus | null>(null);
  const [runner, setRunner] = createSignal<string | null>(null);
  let alive = true;
  // With no runner set up nothing is routed, so old decisions stay hidden.
  const load = () => {
    void api.runnerStatus(false).then(s => {
      if (!alive) return;
      setRunner(s.host);
      setRouting(s.host ? s.routing ?? null : null);
    }, () => {});
  };
  const timer = setInterval(load, 30_000);
  onCleanup(() => { alive = false; clearInterval(timer); });
  onMount(load);
  const shown = () => {
    const r = routing();
    return r && (r.decisions.length > 0 || r.caps) ? r : null;
  };
  return (
    <Show when={shown()}>
      {r => (
        <details class="settings-fold route-decisions-fold">
        <summary>Where builds and tests ran</summary>
        <div class="runner-status route-decisions" role="group" aria-label="Where builds and tests ran">
          <Show when={r().caps}>
            {caps => <p class="runner-status-line">{caps().host}: {capsSentence(caps().line)}{caps().at ? ` Reported ${fmtTime(caps().at!)}.` : ''}</p>}
          </Show>
          <Show when={r().decisions.length > 0}>
            <ul class="route-decisions-list" aria-label="Recent decisions">
              <For each={r().decisions}>{d => (
                <li><span class="route-decisions-when">{d.at ? fmtTime(d.at) : ''}</span> <span class="route-decisions-where">{routeWhere(d.where, runner())}</span> · {routeReason(d.label, d.why)}</li>
              )}</For>
            </ul>
          </Show>
        </div>
        </details>
      )}
    </Show>
  );
}
