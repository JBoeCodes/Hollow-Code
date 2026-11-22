import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import type { MoveHomeOptions, MoveHomePlan, MoveHomeStatus } from '../types';
import { moveHomePlan, moveHomeRollback, moveHomeStart, moveHomeStatus } from '../moveHomeApi';
import { confirmLine, moveSummary, planSections, rollbackState, shouldPoll, siblingsLine, startBlockedReason, statusLine, stepLabel, stepStateWord } from '../moveHome';
import { ErrorText } from '../components/ui';
import { errMsg } from '../utils';

/** How often the progress view reads the journal while a move runs. */
const POLL_MS = 1500;

/**
 * Settings, Sources, Hollow folder, Move Hollow home (2026-11-12): the
 * person's click path for system.move_hollow_home_plan, system.move_hollow_home,
 * system.move_hollow_home_status and system.rollback_move_home.
 *
 * Check the move runs the dry run (changes nothing) and shows its report.
 * "Also move the folders beside the vault" checks again with
 * `includeVaultSiblings`. Quit and move is disabled with its reason while the
 * plan isn't ready (another chat running, a clash), and asks a second time in
 * the panel before it stages the plan the person saw (`moveHomeStart(hash)`);
 * Hollow then quits, a helper moves and Hollow opens again. The progress view
 * reads the journal (polled while staged or running), and Roll back shows
 * whenever the journal says a move can be undone, with the backend's refusal
 * when it would refuse. Lazy: loaded with Settings' Sources section only.
 */
export function MoveHomePanel() {
  const [plan, setPlan] = createSignal<MoveHomePlan | null>(null);
  const [siblings, setSiblings] = createSignal(false);
  const [status, setStatus] = createSignal<MoveHomeStatus | null>(null);
  const [busy, setBusy] = createSignal<'' | 'check' | 'start' | 'rollback'>('');
  const [confirming, setConfirming] = createSignal<'' | 'move' | 'rollback'>('');
  const [note, setNote] = createSignal('');
  const [error, setError] = createSignal('');

  let timer: ReturnType<typeof setTimeout> | undefined;
  let alive = true;
  onCleanup(() => { alive = false; clearTimeout(timer); });

  async function readStatus() {
    clearTimeout(timer);
    try {
      const next = await moveHomeStatus();
      if (!alive) return;
      setStatus(next);
      if (shouldPoll(next)) timer = setTimeout(() => void readStatus(), POLL_MS);
    }
    catch (e) { if (alive) setError(errMsg(e)); }
  }
  onMount(() => void readStatus());

  const options = (): MoveHomeOptions => ({ includeVaultSiblings: siblings() });

  async function check() {
    if (busy()) return;
    setBusy('check');
    setError('');
    setConfirming('');
    try { setPlan(await moveHomePlan(options())); }
    catch (e) { setError(errMsg(e)); }
    finally { setBusy(''); }
  }

  async function start() {
    const p = plan();
    if (!p || busy() || startBlockedReason(p, status())) return;
    setBusy('start');
    setError('');
    try {
      const started = await moveHomeStart(p.hash, options());
      setNote(started.note);
      setConfirming('');
      await readStatus();
    }
    catch (e) { setError(errMsg(e)); }
    finally { setBusy(''); }
  }

  async function rollBack() {
    const s = status();
    if (!s || busy() || rollbackState(s).disabledReason) return;
    setBusy('rollback');
    setError('');
    try {
      const r = await moveHomeRollback(s.backupDir);
      setNote(r.note);
      setConfirming('');
      setPlan(null);
      await readStatus();
    }
    catch (e) { setError(errMsg(e)); }
    finally { setBusy(''); }
  }

  const blockedReason = () => startBlockedReason(plan(), status());
  const rollback = () => rollbackState(status());
  const line = () => busy() === 'check' ? 'Checking the move. This reads your folders and changes nothing.'
    : busy() === 'start' ? 'Staging the move…'
      : busy() === 'rollback' ? 'Staging the rollback…'
        : plan() ? moveSummary(plan()!) : '';

  return <div class="move-home">
    <div class="settings-actions">
      <button class="btn" aria-disabled={!!busy()} aria-busy={busy() === 'check'} onClick={() => void check()}>{plan() ? 'Check again' : 'Check the move'}</button>
    </div>
    {/* Always mounted, so each result is announced. */}
    <p class="disk-note" role="status">{line()}</p>
    <Show when={note()}>{n => <p class="disk-note" role="status">{n()}</p>}</Show>
    <Show when={error()}>{msg => <ErrorText class="settings-row-warn" text={msg()} />}</Show>

    <Show when={plan()}>{p => <>
      <For each={planSections(p())}>{s =>
        <div data-part={s.id}>
          <p class="disk-note" classList={{ 'settings-row-warn': !!s.warn }}><strong>{s.title}</strong></p>
          <ul class="disk-folders" role="list" aria-label={s.title}>
            <For each={s.lines}>{l => <li><span class="disk-folder-path" title={l}>{l}</span></li>}</For>
          </ul>
        </div>}
      </For>

      <Show when={p().besideVault.length > 0}>
        <label class="cx-check">
          <input type="checkbox" checked={siblings()} disabled={!!busy()}
            onChange={e => { setSiblings(e.currentTarget.checked); void check(); }} />
          Also move the folders beside the vault
        </label>
        <p class="disk-note">{siblingsLine(p())}</p>
      </Show>

      <Show when={confirming() !== 'move'} fallback={
        <div role="group" aria-label="Confirm the move">
          <p class="disk-note settings-row-warn">{confirmLine(p())}</p>
          <div class="settings-actions">
            <button class="btn danger" aria-disabled={!!busy()} aria-busy={busy() === 'start'} onClick={() => void start()}>Yes, quit and move</button>
            <button class="btn ghost" onClick={() => setConfirming('')}>Cancel</button>
          </div>
        </div>
      }>
        <div class="settings-actions">
          <button class="btn" aria-disabled={!!busy() || !!blockedReason()} aria-describedby={blockedReason() ? 'move-home-blocked' : undefined}
            onClick={() => { if (!busy() && !blockedReason()) setConfirming('move'); }}>Quit and move…</button>
        </div>
        <Show when={blockedReason()}>{r => <p id="move-home-blocked" class="disk-note settings-row-warn">{r()}</p>}</Show>
      </Show>
    </>}</Show>

    <Show when={status() && status()!.state !== 'none' ? status() : null}>{s => <>
      <p class="disk-note" role="status" aria-label="Move progress">{statusLine(s())}</p>
      <Show when={s().steps.length > 0}>
        <ul class="disk-folders" role="list" aria-label="Move steps">
          <For each={s().steps}>{x => <li><span class="disk-folder-path">{stepLabel(x.step)}</span> <span class="disk-folder-size">{stepStateWord(x.state)}</span></li>}</For>
        </ul>
      </Show>
      <Show when={s().checks.length > 0}>
        <ul class="disk-folders" role="list" aria-label="Checks after the move">
          <For each={s().checks}>{c => <li classList={{ 'settings-row-warn': !c.ok }}><span class="disk-folder-path" title={c.detail}>{`${c.name}: ${c.detail}`}</span> <span class="disk-folder-size">{c.ok ? 'ok' : 'failed'}</span></li>}</For>
        </ul>
      </Show>
      <Show when={s().backupDir}>{dir => <p class="disk-note">Backup: {dir()}</p>}</Show>
    </>}</Show>

    <Show when={rollback().shown}>
      <Show when={confirming() !== 'rollback'} fallback={
        <div role="group" aria-label="Confirm the rollback">
          <p class="disk-note settings-row-warn">Hollow quits, puts every folder and record back where it was and opens again.</p>
          <div class="settings-actions">
            <button class="btn danger" aria-disabled={!!busy()} aria-busy={busy() === 'rollback'} onClick={() => void rollBack()}>Yes, roll back</button>
            <button class="btn ghost" onClick={() => setConfirming('')}>Cancel</button>
          </div>
        </div>
      }>
        <div class="settings-actions">
          <button class="btn" aria-disabled={!!busy() || !!rollback().disabledReason} aria-describedby={rollback().disabledReason ? 'move-home-rollback-refused' : undefined}
            onClick={() => { if (!busy() && !rollback().disabledReason) setConfirming('rollback'); }}>Roll back…</button>
        </div>
        <Show when={rollback().disabledReason}>{r => <p id="move-home-rollback-refused" class="disk-note settings-row-warn">{r()}</p>}</Show>
      </Show>
    </Show>
  </div>;
}
