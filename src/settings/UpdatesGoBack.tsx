import { For, Show, createSignal, createUniqueId, onCleanup } from 'solid-js';
import { api } from '../api';
import { ErrorText } from '../components/ui';
import type { UpdateGoBackPlan, UpdateWork } from '../generated/ipc';
import { showConfirm } from '../store';
import { cancelUpdateWait, refreshUpdateState, updateState } from '../store/updates';
import { errMsg } from '../utils';
import '../styles/updates-cards.css';

export function UpdatesGoBack() {
  const id = createUniqueId();
  // A plain signal, not a resource: a re-read must not suspend the Updates page.
  const [plan, setPlan] = createSignal<UpdateGoBackPlan | null>(null);
  const [planError, setPlanError] = createSignal(false);
  const [work, setWork] = createSignal<UpdateWork[]>([]);
  const [notice, setNotice] = createSignal('');
  const [error, setError] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  let alive = true;
  onCleanup(() => { alive = false; });
  async function refetch() {
    try { const next = await api.updateGoBackPlan(); if (alive) { setPlan(next); setPlanError(false); } }
    catch { if (alive) setPlanError(true); }
  }
  void refetch();
  const goingBack = () => updateState()?.phase.phase === 'goingBack';
  const waiting = () => !!(updateState()?.goBackWaiting || plan()?.waiting);
  const restore = () => plan()?.schema?.verdict === 'needsRestore';
  const skipped = () => updateState()?.skippedVersion ?? plan()?.skippedVersion ?? null;
  const when = (at: string | null | undefined) => at ? new Date(at).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' }) : '';
  const goingBackVersion = () => {
    const phase = updateState()?.phase;
    return phase?.phase === 'goingBack' ? phase.version : '';
  };

  async function act(fn: () => Promise<unknown>) {
    if (busy()) return;
    setBusy(true); setError(''); setNotice('');
    try { await fn(); }
    catch (e) { if (alive) setError(errMsg(e)); }
    finally { if (alive) { setBusy(false); void refetch(); } }
  }

  async function goBack(confirmedKeys?: string[]) {
    const p = plan();
    if (!p?.previous || busy()) return;
    const restoreData = restore();
    const lostSummary = p.schema?.verdict === 'needsRestore' ? p.schema.lost?.summary : undefined;
    const confirmed = await showConfirm({
      title: `Go back to ${p.previous.version}?`,
      body: [
        restoreData ? `${lostSummary ?? 'Chats from after the update are not in the restored data.'} They are kept aside, not deleted.` : '',
        confirmedKeys ? `These stop: ${work().map(w => w.name).join(', ')}.` : '',
        `Hollow quits and opens ${p.previous.version}. It skips ${p.willSkip} until a newer version is published.`,
      ].filter(Boolean).join('\n\n'),
      confirmLabel: restoreData ? 'Go back and restore data' : 'Go back',
      danger: restoreData || !!confirmedKeys,
    });
    if (!confirmed || !alive) return;
    await act(async () => {
      const outcome = await api.updateGoBack(restoreData, confirmedKeys ?? null);
      if (!alive) return;
      switch (outcome.outcome) {
        case 'started':
          setWork([]); setNotice(`Going back to ${outcome.version}. Hollow quits and opens it.`); break;
        case 'workRunning': setWork(outcome.work); break;
        case 'workChanged':
          setWork(outcome.work); setNotice('Something else started. Check the list and choose again.'); break;
        case 'refused': setWork([]); setError(outcome.reason); break;
      }
      await refreshUpdateState();
    });
  }

  return <section class="updates-card" aria-labelledby={id}>
    <h3 class="updates-card-title" id={id}>Go back to the previous version</h3>
    <Show when={!plan() && !planError()}><p class="settings-note">Reading the previous version.</p></Show>
    <Show when={planError()}><ErrorText class="settings-error" text="Could not read the previous version. Reopen Updates to try again." /></Show>
    <Show when={plan()}>{p => {
      const schema = () => {
        const value = p().schema;
        return value?.verdict === 'needsRestore' ? value : undefined;
      };
      return <>
        <Show when={p().previous}>{previous => <p class="settings-note">Hollow {p().runningVersion}{p().runningInstalledAt ? `, installed ${when(p().runningInstalledAt)}` : ''}. The previous version is {previous().version}{previous().installedAt ? `, installed ${when(previous().installedAt)}` : ''}.</p>}</Show>
        <Show when={!p().available}><p class="settings-note">{p().disabledReason ?? 'Going back is not available on this Mac.'}</p></Show>
        <Show when={schema()}>{s => <>
          <p class="settings-note">{s().summary}</p>
          <Show when={s().lost}>{lost => <p class="settings-note">{lost().summary}</p>}</Show>
          <Show when={!p().restoreAvailable}><p class="settings-note">{p().snapshot?.problem ?? "There's no intact copy of your chats from before the update."}</p></Show>
        </>}</Show>
        <Show when={p().indexesOff.length}><ul class="updates-card-list"><For each={p().indexesOff}>{line => <li>{line}</li>}</For></ul></Show>
        <Show when={goingBack()}><p class="settings-note" role="status">Going back to {goingBackVersion()}. Hollow quits and opens it.</p></Show>
        <Show when={waiting()}>
          <p class="settings-note" role="status">Hollow goes back when chats finish.</p>
          <button type="button" class="btn" disabled={busy()} onClick={() => void act(cancelUpdateWait)}>Cancel going back</button>
        </Show>
        <Show when={work().length && !waiting()}>
          <p class="settings-note" role="status">Chats are running: {work().map(w => w.name).join(', ')}.</p>
          <div class="updates-card-actions">
            <button type="button" class="btn" disabled={busy()} onClick={() => void act(async () => {
              await api.updateGoBackWhenIdle(restore());
              if (alive) setWork([]);
              await refreshUpdateState();
            })}>Go back when chats finish</button>
            <button type="button" class="btn" disabled={busy()} onClick={() => void goBack(work().map(w => w.key))}>Stop these and go back</button>
          </div>
        </Show>
        <Show when={!waiting() && !goingBack()}><div class="updates-card-actions">
          <button type="button" class="btn" disabled={busy() || !p().available || !p().previous || (restore() && !p().restoreAvailable)} onClick={() => void goBack()}>{restore() ? 'Go back and restore data from before the update' : `Go back to ${p().previous?.version ?? 'the previous version'}`}</button>
        </div></Show>
      </>;
    }}</Show>
    <Show when={skipped()}><div class="settings-row">
      <p class="settings-note">Skipping {skipped()}. Hollow offers it again when a newer version is published.</p>
      <button type="button" class="btn" disabled={busy() || goingBack()} onClick={() => void act(async () => { await api.updateUnskip(); await refreshUpdateState(); })}>Undo</button>
    </div></Show>
    <Show when={notice()}><p class="settings-note" role="status">{notice()}</p></Show>
    <Show when={error()}><ErrorText class="settings-error" text={error()} /></Show>
  </section>;
}
