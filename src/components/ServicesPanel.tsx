import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import { listen, type UnlistenFn } from '../nativeEvents';
import { api } from '../api';
import { showConfirm, trackTask } from '../store';
import { openWebLink } from '../store/browser';
import type { ServiceView, WorkWorkspace } from '../workspaceTypes';
import { errMsg } from '../utils';
import { ErrorText } from './ui';
import '../styles/build-changes.css';

/** Services an agent started in this worktree that keep running after its
 *  turn (Build PR B6; services.rs). Listed in the Changes tab. */
export function ServicesPanel(props: { workspace?: WorkWorkspace; /** A folder instead of a workspace (the chat card, parity 5.5). */ cwd?: string; /** The card's section names it: no heading. */ compact?: boolean }) {
  const [services, setServices] = createSignal<ServiceView[]>([]);
  const [error, setError] = createSignal('');
  const [logs, setLogs] = createSignal<Record<string, string>>({});
  const cwd = () => props.cwd ?? props.workspace?.cwd ?? '';
  const refresh = () => api.listServices(cwd()).then(list => { setServices(list); setError(''); }).catch(e => setError(errMsg(e)));

  let unlisten: UnlistenFn | undefined;
  let disposed = false;
  onMount(() => {
    void refresh();
    listen('services-changed', () => void refresh()).then(fn => { if (disposed) fn(); else unlisten = fn; }).catch(() => { /* browser mock */ });
    const timer = setInterval(() => void refresh(), 10_000);
    onCleanup(() => clearInterval(timer));
  });
  onCleanup(() => { disposed = true; unlisten?.(); });

  async function stop(s: ServiceView) {
    if (s.status === 'running') {
      const ok = await showConfirm({ title: `Stop ${s.name}?`, body: `Ends ${s.command} and everything it started.`, confirmLabel: 'Stop' });
      if (!ok) return;
    }
    try { await trackTask(`Stop ${s.name}`, () => api.stopService(cwd(), s.name)); } catch (e) { setError(errMsg(e)); }
    await refresh();
  }
  async function toggleLog(s: ServiceView) {
    if (logs()[s.name] != null) { const { [s.name]: _, ...rest } = logs(); setLogs(rest); return; }
    try { const text = await api.serviceLog(cwd(), s.name, 80); setLogs({ ...logs(), [s.name]: text }); } catch (e) { setError(errMsg(e)); }
  }
  const open = (port: number, e: MouseEvent) => openWebLink(`http://localhost:${port}`, e);

  return (
    <section class="bc-services" aria-label="Services">
      <Show when={!props.compact}><h3>Services</h3></Show>
      <Show when={error()}>{msg => <ErrorText class="bc-error" as="div" text={msg()}/>}</Show>
      <For each={services()} fallback={<p class="bc-muted">Nothing running. Ask the agent to start a dev server here. It keeps running after the turn ends, until you stop it here or quit Hollow; archiving the chat that started it stops it too, unless that chat runs in Bypass all.</p>}>{s =>
        <div class="bc-service" data-status={s.status}>
          <div class="bc-service-row">
            <span class="bc-service-dot" aria-label={s.status} title={s.status === 'exited' ? `Exited${s.exitCode != null ? ` (${s.exitCode})` : ''}` : 'Running'} />
            <strong>{s.name}</strong>
            <code class="bc-service-cmd" title={s.command}>{s.command}</code>
            <Show when={s.port && s.status === 'running'}><button class="btn small ghost" onClick={e => open(s.port!, e)}>Open :{s.port}</button></Show>
            <button class="btn small ghost" aria-expanded={logs()[s.name] != null} onClick={() => void toggleLog(s)}>Log</button>
            <button class="btn small ghost" onClick={() => void stop(s)}>{s.status === 'running' ? 'Stop' : 'Remove'}</button>
          </div>
          <Show when={logs()[s.name] != null}><pre class="bc-service-log">{logs()[s.name] || '(no output yet)'}</pre></Show>
        </div>
      }</For>
    </section>
  );
}
