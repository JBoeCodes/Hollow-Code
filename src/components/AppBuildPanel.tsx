import { For, Match, Show, Switch, createEffect, createSignal, on, onCleanup, onMount } from 'solid-js';
import { listen, type UnlistenFn } from '../nativeEvents';
import { api } from '../api';
import { showConfirm, trackTask } from '../store';
import type { AppBuildStatus, InstallBringsReport, WorkWorkspace } from '../workspaceTypes';
import { errMsg } from '../utils';
import { ErrorText } from './ui';
import { bringsDisplay, plural } from '../appBuildBrings';
import { localBuildInButton, offerLocalBuild } from '../store/updates';
import { latestRequest } from '../latestRequest';
import { workspaceWorking } from '../store/workspaces';
import { watchBuildFreshness } from './buildFreshness';

/** How long an app build may run (Rust `self_install::BUILD_DEADLINE`). */
const BUILD_DEADLINE_MIN = 45;

/** "Build app" in the Changes pane of a build workspace whose repository is
 *  Hollow (Build PR B5): build the app from the worktree in the background.
 *  Since row U10 the finished build waits in the top bar's Update button as
 *  "Local build of <branch>" and installs from there (Restart to update);
 *  this panel only builds, shows what the build brings, and puts the build
 *  back in the button (Show in Update button). See self_install.rs and
 *  update_client/local.rs. The build is the Changes pane's one signal
 *  (`build`/`onBuild`, P1-46), which the delivery record reads too. */
export function AppBuildPanel(props: { workspace: WorkWorkspace; build: AppBuildStatus | null; onBuild: (b: AppBuildStatus | null) => void; behind: number; base: string }) {
  const build = () => props.build;
  const setBuild = (b: AppBuildStatus | null) => props.onBuild(b);
  const [now, setNow] = createSignal(Date.now());
  const [error, setError] = createSignal('');
  const [readError, setReadError] = createSignal('');
  const [brings, setBrings] = createSignal<InstallBringsReport | null>(null);
  const [bringsError, setBringsError] = createSignal('');
  const [bringsLoading, setBringsLoading] = createSignal(false);
  const id = () => props.workspace.id;
  const reads = latestRequest();
  const [offering, setOffering] = createSignal(false);
  let root: HTMLElement | undefined;
  /** This build is the Local build in the Update button. */
  const inButton = () => { const b = build(); return !!b?.receipt && localBuildInButton() === b.receipt.buildId; };
  async function refresh() {
    const workspaceId = id(), ticket = reads.start();
    try {
      const b = await api.appBuildStatus(workspaceId);
      if (disposed || id() !== workspaceId || !reads.isLatest(ticket)) return;
      setBuild(b); setReadError(''); return b;
    } catch (e) { if (!disposed && id() === workspaceId && reads.isLatest(ticket)) setReadError(errMsg(e)); }
  }
  watchBuildFreshness(id, () => root, refresh);
  createEffect(on(() => workspaceWorking(props.workspace.cwd), () => void refresh(), { defer: true }));

  let unlisten: UnlistenFn | undefined;
  let disposed = false;
  let requested = '';
  createEffect(() => {
    const b = build(), workspaceId = id(), buildId = b?.state === 'built' && b.workspaceId === workspaceId ? b.receipt?.buildId : null;
    const key = buildId ? `${workspaceId}:${buildId}` : '';
    if (key === requested) return;
    requested = key; setBrings(null); setBringsError(''); setBringsLoading(!!key);
    if (!key) return;
    void api.appBuildBrings(workspaceId).then(report => {
      if (disposed || requested !== key) return;
      if (report.buildId !== buildId) throw Error('The finished build changed. Reopen Changes to refresh its comparison.');
      setBrings(report);
    }).catch(e => { if (!disposed && requested === key) setBringsError(errMsg(e)); })
      .finally(() => { if (!disposed && requested === key) setBringsLoading(false); });
  });
  const count = plural;
  const missingWarning = () => brings()?.missing ? `This build drops ${count(brings()!.missing, 'commit')} from the installed base recorded when it finished.` : '';
  onMount(() => {
    void refresh();
    listen<AppBuildStatus>('self-install-event', e => { if (e.payload.workspaceId === id()) void refresh(); })
      .then(fn => { if (disposed) fn(); else unlisten = fn; })
      .catch(() => { /* browser mock: no events */ });
    // Events can be missed while the window sleeps; poll while a build runs.
    const timer = setInterval(() => { setNow(Date.now()); if (document.visibilityState !== 'hidden' && root?.getClientRects().length && build()?.state === 'building') void refresh(); }, 5000);
    onCleanup(() => clearInterval(timer));
  });
  onCleanup(() => { disposed = true; reads.cancel(); unlisten?.(); });

  const elapsed = () => {
    const at = Date.parse(build()?.startedAt ?? '');
    return Number.isNaN(at) ? null : Math.max(0, Math.floor((now() - at) / 60_000));
  };
  async function start() {
    const behind = props.behind ? `\n\nThis branch is ${count(props.behind, 'commit')} behind ${props.base}, so the build won't have them. Use Update from ${props.base}… first to include them.` : '';
    const ok = await showConfirm({
      title: 'Build Hollow from this worktree?',
      body: `Runs npm run tauri:bundle here in the background, with its own build folder. It takes several minutes and uses one of this Mac’s two build slots. The finished build waits in the Update button in the top bar; nothing is installed until you choose Restart to update there.${behind}`,
      confirmLabel: 'Build app',
    });
    if (!ok) return;
    setError('');
    try { await trackTask('Build Hollow', () => api.startAppBuild(id()), { detail: build()?.branch ?? undefined }); await refresh(); }
    catch (e) { setError(errMsg(e)); }
  }
  async function stop() {
    try { await api.cancelAppBuild(id()); } catch (e) { setError(errMsg(e)); }
    // The job's own event follows once its processes are gone; read now too.
    setTimeout(() => void refresh(), 300);
  }
  async function showInButton() {
    if (offering()) return;
    setOffering(true); setError('');
    try { await offerLocalBuild(id()); } catch (e) { if (!disposed) setError(errMsg(e)); } finally { setOffering(false); }
  }

  return (
    <Show when={build()?.isHollow}>
      <section class="bc-app" aria-label="Build app" ref={root}>
        <h3>Hollow app</h3>
        <Show when={error()}>{msg => <ErrorText class="bc-error" as="div" text={msg()}/>}</Show>
        <Show when={readError()}>{msg => <ErrorText class="bc-error" as="div" text={msg()}/>}</Show>
        <Switch>
          <Match when={build()?.state === 'building'}>
            <p class="bc-muted">Building the app from {build()!.branch} ({build()!.commit})…</p>
            <Show when={elapsed() != null}><p class="bc-muted">Building · {elapsed()} min (stops at {BUILD_DEADLINE_MIN} min)</p></Show>
            <div class="bc-buttons"><button class="btn small" onClick={() => void stop()}>Stop build</button></div>
          </Match>
          <Match when={build()?.state === 'built'}>
            <p class="bc-muted">Built from {build()!.branch} ({build()!.commit}).</p>
            <Show when={bringsLoading()}><p class="bc-muted" role="status">Listing what this install brings…</p></Show>
            <Show when={brings()}>{r => <>
              <Show when={bringsDisplay(r()).summary}>{summary => <details class="bc-brings"><summary>{summary()}</summary>
                <For each={r().pullRequests}>{pr => <div><p>#{pr.number} {pr.title}</p><Show when={!pr.entryFound}><small>no build-log entry</small></Show><For each={pr.untested}>{note => <p class="bc-muted">{note}</p>}</For></div>}</For>
                <For each={r().commits}>{commit => <p>{commit.sha} {commit.subject}</p>}</For>
              </details>}</Show>
              <Show when={bringsDisplay(r()).note}>{note => <p class="bc-muted">{note()}</p>}</Show>
              <Show when={missingWarning()}>{warning => <ErrorText class="bc-error" as="div" text={warning()} plain={false}/>}</Show>
            </>}</Show>
            <Show when={bringsError()}>{error => <ErrorText class="bc-error" as="div" text={`What it brings couldn't be listed: ${error()}`}/>}</Show>
            <Show when={inButton()} fallback={<p class="bc-muted">It isn’t in the Update button now (Not now, or it couldn’t be offered).</p>}>
              <p class="bc-muted" role="status">It waits in the Update button in the top bar as Local build of {build()!.branch}: Restart to update installs it.</p>
            </Show>
            <div class="bc-buttons">
              <Show when={!inButton()}><button class="btn small" disabled={offering()} onClick={() => void showInButton()}>Show in Update button</button></Show>
              <button class="btn small ghost" onClick={() => void start()}>Build again</button>
            </div>
          </Match>
          <Match when={true}>
            <Show when={build()?.state === 'failed'}><p class="bc-error" role="alert">The build failed: {build()!.error}</p></Show>
            <Show when={build()?.otherJob}><p class="bc-muted">Another workspace’s app build is running.</p></Show>
            <p class="bc-muted">Build this worktree as an app to test it. The finished build waits in the Update button in the top bar.</p>
            <div class="bc-buttons"><button class="btn small" disabled={!!build()?.otherJob} onClick={() => void start()}>{build()?.state === 'failed' ? 'Build again' : 'Build app'}</button></div>
          </Match>
        </Switch>
        <Show when={build()?.logTail}>
          <details class="bc-log" open={build()?.state !== 'built'}><summary>Log{build()?.log ? ` · ${build()!.log}` : ''}</summary><pre>{build()!.logTail}</pre></details>
        </Show>
      </section>
    </Show>
  );
}
