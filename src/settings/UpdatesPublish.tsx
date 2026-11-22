import { For, Show, createSignal, createUniqueId, onCleanup } from 'solid-js';
import { api } from '../api';
import { Select } from '../components/Select';
import { Spinner } from '../components/Spinner';
import { ErrorText } from '../components/ui';
import type { UpdatePublishOutcome, UpdatePublishStatus } from '../generated/ipc';
import { showConfirm } from '../store';
import { errMsg } from '../utils';
import '../styles/updates-cards.css';

export function UpdatesPublish() {
  const id = createUniqueId();
  const [status, setStatus] = createSignal<UpdatePublishStatus | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [pick, setPick] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const [outcome, setOutcome] = createSignal<UpdatePublishOutcome | null>(null);
  let alive = true;
  onCleanup(() => { alive = false; });
  const selected = () => status()?.builds.find(b => b.version === pick()) ?? null;
  const working = () => loading() || busy();

  async function load(nextPick: string | null = pick()) {
    if (loading()) return;
    setLoading(true); setError('');
    try {
      const s = await api.updatePublishStatus(nextPick);
      if (!alive) return;
      setStatus(s); setPick(nextPick ?? s.proposed);
    } catch (e) { if (alive) setError(errMsg(e)); }
    finally { if (alive) setLoading(false); }
  }
  void load(null);

  async function run(fn: () => Promise<UpdatePublishOutcome>) {
    if (working()) return;
    setBusy(true); setError(''); setOutcome(null);
    let failed = '';
    try { const o = await fn(); if (alive) setOutcome(o); }
    catch (e) { failed = errMsg(e); }
    finally { if (alive) setBusy(false); }
    if (alive) await load(null);
    // the status reload clears the error line, so a rejected publish or withdraw is shown again after it
    if (alive && failed) setError(failed);
  }

  async function publish() {
    const s = status(), b = selected();
    if (!s || !b?.commit || working()) return;
    const ok = await showConfirm({
      title: `Publish ${b.version} to Main?`,
      body: `Everyone on Main gets ${b.version} at their next check. Main is ${s.main?.version ?? 'empty'} now. The same signed build is used; nothing is rebuilt.`,
      confirmLabel: 'Publish',
    });
    if (!ok || !alive) return;
    await run(() => api.updatePublishMain(b.version, b.commit!, s.main?.version ?? null));
  }

  async function withdraw() {
    const s = status();
    if (!s?.main || !s.previousMain || working()) return;
    const ok = await showConfirm({
      title: `Withdraw ${s.main.version}?`,
      body: `Main goes back to ${s.previousMain}. New installs stop getting ${s.main.version}. Macs already on it are offered Go back at their next check.`,
      confirmLabel: 'Withdraw', danger: true,
    });
    if (!ok || !alive) return;
    await run(() => api.updateWithdrawMain(s.main!.version));
  }

  return <section class="updates-card" aria-labelledby={id}>
    <h3 class="updates-card-title" id={id}>Publish to Main</h3>
    <Show when={working()}><p class="updates-card-busy" role="status"><Spinner /> Asking the release builder. This can take a minute.</p></Show>
    <Show when={status()}>{s => <>
      <Show when={!s().available}><p class="settings-note">{s().unavailableReason ?? 'Publishing is not available on this Mac.'}</p></Show>
      <p class="settings-note">Main is {s().main?.version ?? 'empty'}. Nightly is {s().nightly?.version ?? 'empty'}.</p>
      <Show when={s().changesAhead != null}><p class="settings-note">Nightly is {s().changesAhead} changes{s().daysAhead != null ? ` and ${s().daysAhead} days` : ''} ahead of Main.</p></Show>
      <Show when={s().builds.length}><div class="settings-row">
        <div class="settings-row-label"><div class="settings-row-k">Build</div><div class="settings-row-v">The newest Nightly that ran on this Mac for an hour is proposed.</div></div>
        <Select label="Build" value={pick() ?? ''} disabled={working()}
          options={s().builds.map(b => ({ value: b.version, label: `${b.version}${b.ranHere ? ', ran on this Mac' : ''}${b.newerThanMain ? '' : ', not newer than Main'}` }))}
          onChange={v => { setOutcome(null); void load(v); }} />
      </div></Show>
      <Show when={!s().proposed && s().proposedWhy}><p class="settings-note">{s().proposedWhy}</p></Show>
      <Show when={s().notes}>{notes => <>
        <p class="settings-row-k">What's new since Main</p>
        <ul class="updates-card-list">
          <For each={notes().pullRequests}>{pr => <li>#{pr.number} {pr.title}</li>}</For>
          <For each={notes().whatsNew}>{w => <li>{w.title}: {w.what}</li>}</For>
          <For each={notes().commits}>{c => <li>{c.sha.slice(0, 7)} {c.subject}</li>}</For>
        </ul>
        <Show when={notes().note}><p class="settings-note">{notes().note}</p></Show>
      </>}</Show>
      <div class="updates-card-actions">
        <button type="button" class="btn" disabled={working()} onClick={() => void load()}>Refresh</button>
        <button type="button" class="btn primary" disabled={working() || !s().available || !selected() || !selected()!.commit || !selected()!.newerThanMain} onClick={() => void publish()}>{selected() ? `Publish ${selected()!.version} to Main` : 'Publish to Main'}</button>
        <Show when={s().canWithdraw && s().main && s().previousMain}><button type="button" class="btn" disabled={working() || !s().available} onClick={() => void withdraw()}>Withdraw {s().main!.version}</button></Show>
      </div>
    </>}</Show>
    <Show when={outcome()}>{o => {
      const text = () => {
        const value = o();
        switch (value.outcome) {
          case 'published': return `Published ${value.version} to Main${value.from ? ` (was ${value.from})` : ''}.`;
          case 'withdrawn': return `Withdrew ${value.from}. Main is ${value.version} again.`;
          case 'refused': return `Nothing changed. ${value.reason}`;
        }
      };
      const warning = () => { const value = o(); return value.outcome === 'refused' ? null : value.historyWarning; };
      return <>
        <p class="settings-note" role="status">{text()}</p>
        <Show when={warning()}><p class="settings-note">{warning()}</p></Show>
      </>;
    }}</Show>
    <Show when={error()}><ErrorText class="settings-error" text={error()} /></Show>
  </section>;
}
