import { Show, createSignal, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { PlanWindows } from '../components/PlanWindows';
import type { ProviderAccount } from '../types';

/** Passive snapshots only; this never starts a Codex turn or quota request. */
export function ProviderQuotaCard() {
  const [provider, setProvider] = createSignal<ProviderAccount | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [failed, setFailed] = createSignal(false);
  let alive = true;
  onCleanup(() => { alive = false; });
  async function load() {
    if (busy()) return;
    setBusy(true);
    setFailed(false);
    try {
      const list = await api.providerAccounts();
      if (alive) setProvider(list.find(p => p.id === 'codex') ?? null);
    } catch {
      if (alive) { setFailed(true); setProvider(null); }
    } finally { if (alive) setBusy(false); }
  }
  onMount(() => { void load(); });
  return <section class="usage-block" aria-label="Codex plan">
    <h3 class="usage-block-title">Codex plan</h3>
    <Show when={!failed()} fallback={<p>Couldn't read Codex's last limits. Try again.</p>}>
      <Show when={provider()?.limits?.windows.length}
        fallback={<p>{busy() ? 'Reading…' : "Codex didn't report limits."}</p>}>
        <PlanWindows windows={provider()!.limits!.windows} />
        <p>Last reported {new Date(provider()!.limits!.observedAt).toLocaleString()}.</p>
      </Show>
    </Show>
    <button type="button" class="btn" disabled={busy()} onClick={() => { void load(); }}>
      {busy() ? 'Reading…' : 'Refresh Codex limits'}
    </button>
  </section>;
}
