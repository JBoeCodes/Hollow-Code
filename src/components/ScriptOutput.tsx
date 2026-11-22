import { Show, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { workspaces } from '../store/workspaces';
import { loadProjectScripts, projectScripts, runProjectScript } from '../store/projectScripts';
import { displayChord } from '../shortcuts';
import type { ServiceView } from '../workspaceTypes';
import { errMsg } from '../utils';
import { Button, PlainErrorText, StatePanel } from './ui';
import '../styles/project-scripts.css';

/** A project script's output in the Preview pane (4.5, A10): the run's own service log, polled
 *  while the tab is open, with Run, Stop and the command that ran. The run is a Hollow service
 *  in the project's folder (`services.rs`), so the Changes tab lists it too. */
export function ScriptOutput(props: { workspaceId: string; scriptId: string }) {
  const script = createMemo(() => projectScripts(props.workspaceId).find(s => s.id === props.scriptId) ?? null);
  const cwd = () => workspaces().find(w => w.id === props.workspaceId)?.cwd ?? '';
  const [service, setService] = createSignal<ServiceView | null>(null);
  const [log, setLog] = createSignal('');
  const [error, setError] = createSignal('');
  let body: HTMLPreElement | undefined;
  let alive = true;
  let busy = false;
  async function refresh() {
    const s = script();
    if (busy || !s?.serviceName || !cwd()) return;
    busy = true;
    try {
      const found = (await api.listServices(cwd())).find(v => v.name === s.serviceName) ?? null;
      const text = found ? await api.serviceLog(cwd(), s.serviceName, 400) : '';
      if (!alive) return;
      setService(found); setLog(text); setError('');
      // Follow the end while the reader is already there.
      queueMicrotask(() => { if (body && body.scrollHeight - body.scrollTop - body.clientHeight < 80) body.scrollTop = body.scrollHeight; });
    } catch (e) { if (alive) setError(errMsg(e)); }
    finally { busy = false; }
  }
  onMount(() => {
    void loadProjectScripts(props.workspaceId).then(refresh);
    const timer = setInterval(() => void refresh(), 1000);
    onCleanup(() => clearInterval(timer));
  });
  onCleanup(() => { alive = false; });
  const running = () => service()?.status === 'running';
  async function run() { const s = script(); if (s && await runProjectScript(props.workspaceId, s)) void refresh(); }
  async function stop() {
    const s = script();
    if (!s?.serviceName) return;
    try { await api.stopService(cwd(), s.serviceName); } catch (e) { setError(errMsg(e)); }
    void refresh();
  }
  const state = () => !service() ? 'Not run yet' : running() ? 'Running' : service()!.exitCode === 0 ? 'Finished' : `Exited${service()!.exitCode != null ? ` (${service()!.exitCode})` : ''}`;
  return <div class="script-output">
    <Show when={script()} fallback={<StatePanel compact kind="empty" title="This script is gone">It was removed from the project. Close this tab.</StatePanel>}>
      {s => <>
        <header class="script-output-head">
          <div class="script-output-title"><strong>{s().name}</strong><span class="script-output-state" data-state={running() ? 'running' : service() ? 'done' : 'idle'} role="status">{state()}</span>
            <Show when={s().chord}>{c => <kbd class="script-chord">{displayChord(c())}</kbd>}</Show></div>
          <div class="script-output-actions">
            <Show when={running()} fallback={<Button size="sm" variant="primary" icon="play" onClick={() => void run()}>{service() ? 'Run again' : 'Run'}</Button>}>
              <Button size="sm" icon="stop" onClick={() => void stop()}>Stop</Button>
            </Show>
          </div>
        </header>
        <code class="script-output-command" title="Runs in the project's folder">{s().command}</code>
        <Show when={error()}><p class="script-output-error" role="alert"><PlainErrorText text={error()} /></p></Show>
        <pre ref={body} class="script-output-log" role="log" aria-label={`${s().name} output`} tabIndex={0}>{log() || (service() ? '(no output yet)' : 'Run the script to see its output here.')}</pre>
      </>}
    </Show>
  </div>;
}
