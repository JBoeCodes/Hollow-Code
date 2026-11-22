import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { canvasPins, removeCanvasPin, saveCanvasPin } from '../store/canvasPins';
import { vaultChange } from '../store/vaultChanges';
import { openSessionByKey } from '../store/workspaces';
import { parseCanvasFence } from '../canvas/fence';
import { readPlan } from '../planModel';
import { sha256Hex } from '../sectionMarkdown';
import { CANVAS_FENCE_MODE, innerOf } from '../canvas/chatCanvas';
import { liveDeclarations, selectedValues } from '../canvas/liveBindings';
import { refreshLoop, sharedReads, taskValues, usageValues, type CanvasPin, type LiveValues } from '../canvas/liveModel';
import { HtmlArtifact } from './HtmlArtifact';
import { pinnedVersionLabel } from '../canvas/hostContract';
import '../styles/pinned-canvases.css';
import { ErrorText } from './ui';

/** Lazy host-only view. No pins means no IPC, timers or iframe. */
export function PinnedCanvases(props: { workspaceId?: string }) {
  const [vault, setVault] = createSignal<string | null>(null);
  const [loadError, setLoadError] = createSignal(false);
  let disposed = false;
  const loadVault = () => { setLoadError(false); void api.setupStatus().then(s => {
    if (!disposed) { setVault(s.vaultPath); setLoadError(!s.vaultPath); }
  }).catch(() => { if (!disposed) setLoadError(true); }); };
  onMount(loadVault);
  onCleanup(() => { disposed = true; });
  const pins = createMemo(() => canvasPins().filter(p => p.vault === vault() && (props.workspaceId
    ? p.destination === 'workspace' && p.workspaceId === props.workspaceId : p.destination === 'home')));
  return <><Show when={loadError()}><p role="status">Pinned canvases could not be read. <button type="button" class="btn small" onClick={loadVault}>Retry pins</button></p></Show><Show when={pins().length}><section class="pinned-canvases" aria-label="Pinned canvases">
    <h2>Pinned canvases</h2><For each={pins()}>{pin => <PinnedCanvas pin={pin}/>}</For>
  </section></Show></>;
}

type Snapshot = { body: string; values: LiveValues; failures: string[]; at: number; session: string | null; latest: number; newer: boolean; live: boolean; ask: boolean; project: string | null };
const shared = sharedReads();

async function readPin(pin: CanvasPin): Promise<Snapshot> {
  const scope = { workspaceId: pin.workspaceId, sessionKey: pin.sessionKey };
  const canvas = await api.readCanvas(scope, pin.id);
  if (!canvas) throw Error('The saved canvas is no longer available.');
  let body: string;
  try { body = (await api.readCanvasRevision(scope, pin.id, pin.revision)).html; }
  catch { throw Error('This saved revision is no longer readable. Open its source and pin an available version again.'); }
  if (sha256Hex(body) !== pin.sha256) throw Error('The pinned source changed. Open its chat and pin the saved version again.');
  const parsed = parseCanvasFence(body, true);
  if (parsed.state !== 'ready') throw Error('The pinned canvas is malformed. Open its source chat.');
  const declaration = liveDeclarations(body), values: LiveValues = {}, failures: string[] = [];
  if (declaration.invalid) failures.push('Unsupported data binding');
  // Ask canvases remain a snapshot; answers are submitted only in their source conversation.
  const keys = parsed.root.ask ? [] : declaration.keys;
  const read = async (kind: string, action: () => Promise<LiveValues>) => {
    if (!keys.some(k => k.startsWith(`${kind}.`))) return;
    try { Object.assign(values, await action()); } catch { failures.push(`${kind} unavailable`); }
  };
  await Promise.all([
    read('plan', async () => {
      if (!pin.workspaceId) throw Error('No workspace');
      const detail = await shared(`${pin.vault}/workspace/${pin.workspaceId}`, () => api.readWorkspace(pin.workspaceId!));
      if (!detail.files.some(f => f.name === 'PLAN.md')) throw Error('No plan');
      const plan = readPlan((await shared(`${pin.vault}/plan/${pin.workspaceId}`, () => api.readWorkFile(`${detail.workspace.path}/PLAN.md`))).content).counts;
      return { 'plan.done': plan.done, 'plan.open': plan.open, 'plan.total': plan.total };
    }),
    read('tasks', async () => taskValues((await shared(`${pin.vault}/tasks`, api.tasks)).items, declaration.project)),
    read('usage', async () => {
      const usage = await shared(`${pin.vault}/usage`, () => api.usageSummary('day'));
      if (usage.ledgerError) throw Error('Usage record unavailable');
      return usageValues(usage.total);
    }),
    read('sessions', async () => ({ 'sessions.running': (await shared(`${pin.vault}/sessions`, () => api.listRunQueue(0))).filter(r => r.source === 'chat' && (r.status === 'running' || r.status === 'claimed')).length })),
  ]);
  const selected = selectedValues(keys, values);
  if (Object.values(selected).some(v => v === null) && !failures.length) failures.push('Some data is unavailable');
  return { body, values: selected, failures, at: Date.now(), session: canvas.revisions.find(r => r.revision === pin.revision)?.session ?? pin.sessionKey, latest: canvas.revision, newer: canvas.revision !== pin.revision,
    live: keys.length > 0, ask: parsed.root.ask, project: keys.some(k => k.startsWith('tasks.')) ? declaration.project : null };
}

function PinnedCanvas(props: { pin: CanvasPin }) {
  let root!: HTMLDivElement;
  const [snapshot, setSnapshot] = createSignal<Snapshot | null>(null);
  const [error, setError] = createSignal('');
  const [visible, setVisible] = createSignal(false);
  const [windowVisible, setWindowVisible] = createSignal(!document.hidden);
  const [refreshing, setRefreshing] = createSignal(false);
  const [stale, setStale] = createSignal(true);
  const [updating, setUpdating] = createSignal(false);
  let disposed = false;
  onCleanup(() => { disposed = true; });
  const update = async () => {
    const pin = props.pin, target = snapshot()?.latest;
    if (updating() || !target || target <= pin.revision) return;
    setUpdating(true); setError('');
    try {
      const scope = { workspaceId: pin.workspaceId, sessionKey: pin.sessionKey };
      const current = await api.readCanvas(scope, pin.id);
      if (!current || current.htmlChanged) throw Error('The current canvas cannot be verified. The existing pin is kept.');
      if (current.revision !== target) throw Error('The current version changed. Refresh before updating this pin.');
      const revision = await api.readCanvasRevision(scope, pin.id, target);
      const hash = sha256Hex(revision.html);
      if (revision.revision !== target || hash !== revision.fenceSha256 || hash !== current.fenceSha256 || parseCanvasFence(revision.html, true).state !== 'ready') {
        throw Error('The new revision could not be verified. The existing pin is kept.');
      }
      if (disposed || props.pin !== pin) return;
      saveCanvasPin({ ...pin, revision: target, sha256: hash, title: revision.title }, pin);
    } catch (e) { if (!disposed) setError(e instanceof Error ? e.message : 'Could not update this pin.'); }
    finally { if (!disposed) setUpdating(false); }
  };
  // Scalar refreshes must not invalidate HtmlArtifact's document resource.
  const body = createMemo(() => snapshot()?.body ?? '');
  const inner = createMemo(() => innerOf(body(), ''));
  const loop = refreshLoop(async () => { setRefreshing(true); return readPin(props.pin); }, value => {
    setSnapshot(value); setError(''); setStale(false); setRefreshing(false);
  }, e => { setError(e instanceof Error ? e.message : 'Could not refresh this canvas.'); setStale(true); setRefreshing(false); });
  const active = () => visible() && windowVisible();
  const request = () => { loop.request(); };
  createEffect(() => { loop.visible(active()); if (!active()) { setStale(true); setRefreshing(false); } });
  createEffect(on(vaultChange, () => request(), { defer: true }));
  onMount(() => {
    const observer = new IntersectionObserver(entries => setVisible(entries.some(e => e.isIntersecting)), { threshold: 0 });
    observer.observe(root);
    const visibility = () => setWindowVisible(!document.hidden);
    document.addEventListener('visibilitychange', visibility);
    // No repeating timer while hidden/offscreen. Restarting it never schedules a paid turn.
    let timer: ReturnType<typeof setInterval> | undefined;
    createEffect(() => { clearInterval(timer); timer = active() ? setInterval(request, 30_000) : undefined; });
    onCleanup(() => { observer.disconnect(); clearInterval(timer); document.removeEventListener('visibilitychange', visibility); });
  });
  onCleanup(() => loop.dispose());
  const unpin = () => {
    try {
      // Its page: a workspace's Overview, or Your work → Today (overnight
      // 11-03: unpinning Today's last pin left focus on the page body).
      const owner = root.closest<HTMLElement>('.work-overview, .sec-view, .today-card');
      const neighbor = (root.nextElementSibling ?? root.previousElementSibling)?.querySelector<HTMLElement>('button:not([disabled])');
      removeCanvasPin(props.pin);
      queueMicrotask(() => {
        if (neighbor?.isConnected) neighbor.focus();
        else if (owner?.isConnected) { owner.tabIndex = -1; owner.focus({ preventScroll: true }); }
      });
    } catch { setError('Could not save this change. Try unpinning again.'); }
  };
  // Plan L3 X7 (2026-10-14; review U canvases): the head says which version
  // this pin keeps, and whether a newer one exists. Made once, as the head's
  // badge is (HtmlArtifact renders it as given).
  const version = <span class="chat-artifact-state" data-pinned-version="">{pinnedVersionLabel(props.pin.revision, snapshot() ? snapshot()!.newer : null)}</span>;
  const status = () => error() ? 'Needs refresh' : refreshing() ? 'Refreshing…' : snapshot()?.failures.length ? 'Data unavailable' : stale() ? 'Waiting to refresh' : snapshot()?.live ? 'Live · updated' : 'Saved snapshot';
  return <div class="pinned-canvas" ref={root} data-pin={props.pin.id}>
    <header><div><strong>{props.pin.title}</strong><span class="pinned-canvas-status" role="status">{status()}<Show when={snapshot()}>{s => <> · {new Date(s().at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</>}</Show></span></div>
      <div class="pinned-canvas-actions">
        <button class="btn small ghost" type="button" onClick={request} disabled={refreshing()}>Refresh</button>
        <button class="btn small ghost" type="button" disabled={!snapshot()?.session} title={snapshot()?.session ? 'Open the conversation that saved this canvas' : 'The saved canvas has no source conversation recorded'} onClick={() => { const key = snapshot()?.session; if (key) void openSessionByKey(key).then(ok => { if (!ok) setError('The source conversation could not be opened.'); }).catch(() => setError('The source conversation could not be opened.')); }}>Open source chat</button>
        <button class="btn small ghost" type="button" onClick={unpin}>Unpin</button>
      </div>
    </header>
    <Show when={error()}><ErrorText class="pinned-canvas-note" text={error()}> Last shown values may be out of date.</ErrorText></Show>
    <Show when={snapshot()?.failures.length}><p class="pinned-canvas-note">{snapshot()!.failures.join(' · ')}. Missing data is never counted as zero.</p></Show>
    <Show when={snapshot()?.newer}><p class="pinned-canvas-note">A newer version exists. This pin keeps v{props.pin.revision}.{' '}
      <button type="button" class="btn small" disabled={updating()} onClick={() => void update()}>{updating() ? 'Updating…' : `Update to v${snapshot()!.latest}`}</button>
    </p></Show>
    <Show when={snapshot()?.ask}><p class="pinned-canvas-note">Answer this canvas in its source conversation.</p></Show>
    <Show when={active() && snapshot()} fallback={<div class="pinned-canvas-paused">{snapshot() ? 'Paused while out of view' : error() ? 'The pin is kept so you can retry or unpin it.' : 'Reading the saved canvas…'}</div>}>
      <HtmlArtifact lookWorkspace={props.pin.workspaceId} code={inner()} defaultMode={CANVAS_FENCE_MODE} localOnly
        canvas={{ title: props.pin.title, source: body(), badge: version }} host="pinned"
        liveData={() => snapshot()?.values ?? {}} bridge={{ canvasId: props.pin.id, state: () => null }}
        fileBase={() => props.pin.workspaceId ? `${props.pin.vault}/workspaces/${props.pin.workspaceId}` : null} />
    </Show>
    <Show when={snapshot()?.live}><p class="pinned-canvas-note">Read-only local data · usage and running chats cover this Mac · refreshes while visible</p></Show>
    <Show when={snapshot()?.project !== null && snapshot()?.project !== undefined}><p class="pinned-canvas-note">Tasks: {snapshot()!.project || 'all local tasks'}</p></Show>
  </div>;
}
