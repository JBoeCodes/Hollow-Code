import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { latestRequest } from '../latestRequest';
import { Icon } from '../icons';
import { flashFooter, openInBrowser, showConfirm, trackTask } from '../store';
import { appendToChatDraft } from '../store/chat';
import { refreshRailGitStates, workspaceWorking } from '../store/workspaces';
import { watchBuildFreshness } from './buildFreshness';
import { askAboutHunk, parseDiff, type DiffFile } from '../diffParse';
import type { AppBuildStatus, BuildChangedFile, BuildStatus, WorkWorkspace } from '../workspaceTypes';
import { errMsg, openExternal } from '../utils';
import { AppBuildPanel } from './AppBuildPanel';
import { DeliveryRecord } from './DeliveryRecord';
import { ServicesPanel } from './ServicesPanel';
import '../styles/build-changes.css';
import { ErrorText } from './ui';
import { RunsOnControl } from './RunsOnControl';

const LETTER: Record<BuildChangedFile['status'], string> = { added: 'A', modified: 'M', deleted: 'D', renamed: 'R', untracked: 'U' };

/** The Changes tab of a build workspace's side panel (Build PR B2): the
 *  worktree against its base, a file's diff with "Ask about this" per hunk,
 *  and Commit / Push / Open PR, each confirmed. Git runs in the backend
 *  (build_changes.rs); nothing here sends a chat turn. */
export function BuildChanges(props: { workspace: WorkWorkspace }) {
  const id = () => props.workspace.id;
  const [status, setStatus] = createSignal<BuildStatus | null>(null);
  const [error, setError] = createSignal('');
  const [readError, setReadError] = createSignal('');
  const [loading, setLoading] = createSignal(false);
  const [selected, setSelected] = createSignal<string | null>(null);
  const [diff, setDiff] = createSignal<{ path: string; files: DiffFile[]; truncated: boolean; cut: string[] } | null>(null);
  const [diffLoading, setDiffLoading] = createSignal(false);
  const [message, setMessage] = createSignal('');
  const [prForm, setPrForm] = createSignal(false);
  const [prTitle, setPrTitle] = createSignal('');
  const [prBody, setPrBody] = createSignal('');
  const [acting, setActing] = createSignal<string | null>(null);
  // The app build: one signal, fed by the Hollow app section's reads and
  // self-install-event, for that section and the delivery record (P1-46).
  const [appBuild, setAppBuild] = createSignal<AppBuildStatus | null>(null);
  createEffect(on(id, () => setAppBuild(null), { defer: true }));
  const statusReads = latestRequest();
  const [readAt, setReadAt] = createSignal<Date>();
  let root: HTMLDivElement | undefined;
  let disposed = false;


  // An agent turn in this worktree is running: git would race it.
  const agentHere = () => workspaceWorking(props.workspace.cwd);
  const blocked = () => status()?.blocked || (agentHere() ? 'Wait for this workspace’s running conversations to finish.' : null);
  const uncommitted = createMemo(() => status()?.files.filter(f => f.uncommitted).length ?? 0);
  const canPush = () => { const s = status(); return !!s && !!s.branch && (s.upstream ? s.unpushed > 0 : s.ahead > 0); };

  async function refresh(forAction = false) {
    // A timer/event must not supersede the read a click is waiting for.
    if (acting() && !forAction) return;
    const gen = statusReads.start();
    const workspaceId = id();
    setLoading(true);
    try {
      const next = await api.buildStatus(workspaceId);
      if (!statusReads.isLatest(gen) || id() !== workspaceId) return;
      setStatus(next); setReadAt(new Date()); setReadError('');
      const sel = selected();
      if (sel && !next.files.some(f => f.path === sel)) { setSelected(null); setDiff(null); }
      else if (sel) void loadDiff(sel);
      return next;
    } catch (e) { if (statusReads.isLatest(gen)) setReadError(errMsg(e)); }
    finally { if (statusReads.isLatest(gen)) setLoading(false); }
  }

  const diffReads = latestRequest();
  async function loadDiff(path: string) {
    const gen = diffReads.start();
    setDiffLoading(true);
    try {
      const d = await api.buildDiff(id(), path);
      if (!diffReads.isLatest(gen)) return;
      setDiff({ path, files: parseDiff(d.diff), truncated: d.truncated, cut: d.truncatedFiles });
    } catch (e) { if (diffReads.isLatest(gen)) { setDiff(null); setError(errMsg(e)); } }
    finally { if (diffReads.isLatest(gen)) setDiffLoading(false); }
  }

  function selectFile(path: string) {
    if (selected() === path) { setSelected(null); setDiff(null); return; }
    setSelected(path); setDiff(null); void loadDiff(path);
  }

  onMount(() => void refresh());
  watchBuildFreshness(id, () => root, refresh);
  onCleanup(() => { disposed = true; statusReads.cancel(); diffReads.cancel(); });
  // A chat turn in this workspace finished: its changes are new.
  createEffect(on(agentHere, () => void refresh(), { defer: true }));

  async function ready() {
    const fresh = await refresh(true);
    return fresh && !blocked() ? fresh : null;
  }

  async function act(name: string, run: () => Promise<void>) {
    if (acting()) return;
    setActing(name); setError('');
    try { await run(); } catch (e) { setError(errMsg(e)); } finally { setActing(null); if (!disposed) void refresh(); }
  }

  function commit() {
    const msg = message().trim(), workspaceId = id();
    if (!msg) return;
    void act('commit', async () => {
      const s = await ready(); if (!s || !s.files.some(f => f.uncommitted)) return;
      const count = s.files.filter(f => f.uncommitted).length;
      const ok = await showConfirm({
        title: `Commit ${count} change${count === 1 ? '' : 's'}?`,
        body: `Every change in the worktree is staged (git add -A) and committed to ${s.branch || 'the current branch'} with your message. Your hooks run as usual.`,
        confirmLabel: 'Commit',
      });
      if (!ok || disposed || id() !== workspaceId) return;
      const next = await trackTask('Commit', () => api.buildCommit(workspaceId, msg), { detail: s.branch });
      void refreshRailGitStates(0, [workspaceId]);
      setStatus(next); setMessage('');
      const sel = selected(); if (sel) void loadDiff(sel);
      flashFooter(`Committed to ${next.branch}`);
    });
  }

  function push() {
    const workspaceId = id();
    void act('push', async () => {
      const s = await ready(); if (!s || !canPush()) return;
      const ok = await showConfirm({
        title: `Push ${s.branch} to origin?`,
        body: s.upstream ? `Sends ${s.unpushed} new commit${s.unpushed === 1 ? '' : 's'} to ${s.upstream}.` : `Creates origin/${s.branch} with ${s.ahead} commit${s.ahead === 1 ? '' : 's'} and tracks it.`,
        confirmLabel: 'Push',
      });
      if (!ok || disposed || id() !== workspaceId) return;
      setStatus(await trackTask('Push', () => api.buildPush(workspaceId), { detail: s.branch }));
      void refreshRailGitStates(0, [workspaceId]);
      flashFooter(`Pushed ${s.branch}`);
    });
  }

  function startPr() {
    setPrTitle(props.workspace.title);
    setPrBody(status()?.goal ?? '');
    setPrForm(true);
  }

  function openPr() {
    const title = prTitle().trim(), workspaceId = id();
    if (!title) return;
    void act('pr', async () => {
      const s = await ready(); if (!s) return;
      const base = s.base.replace(/^origin\//, '');
      const ok = await showConfirm({
        title: 'Open a pull request?',
        body: `Runs gh pr create for ${s.branch} into ${base}${s.upstream ? '' : ', pushing the branch to origin first'}. The PR is public to everyone who can see the repository.`,
        confirmLabel: 'Open pull request',
      });
      if (!ok || disposed || id() !== workspaceId) return;
      const url = await trackTask('Open pull request', () => api.buildOpenPr(workspaceId, title, prBody()), { detail: s.branch });
      void refreshRailGitStates(0, [workspaceId]);
      setPrForm(false);
      flashFooter(`Opened ${url}`);
      await refresh(true);
    });
  }

  function updateFromBase() {
    const workspaceId = id();
    void act('update', async () => {
      const s = await ready(); if (!s || !s.behind) return;
      const ok = await showConfirm({
        title: `Update ${s.branch} from ${s.base}?`,
        body: s.upstream
          ? `Fetches ${s.base} and merges it into this branch (git merge), so it is 0 behind. The branch is already pushed, so its commits aren't rewritten and Push still works. If anything conflicts, the merge is undone and nothing changes.`
          : `Fetches ${s.base} and replays this branch’s ${s.ahead} commit${s.ahead === 1 ? '' : 's'} on top of it (git rebase), so it is 0 behind. If a commit conflicts, the rebase is undone and nothing changes.`,
        confirmLabel: 'Update',
      });
      if (!ok || disposed || id() !== workspaceId) return;
      setStatus(await trackTask(`Update from ${s.base}`, () => api.buildUpdateFromBase(workspaceId), { detail: s.branch }));
      void refreshRailGitStates(0, [workspaceId]);
      flashFooter(`Updated ${s.branch} from ${s.base}`);
    });
  }

  function openLink(url: string) { openInBrowser(url).catch(() => openExternal(url)); }

  return (
    <div class="build-changes" ref={root}>
      <DeliveryRecord workspace={props.workspace} status={status()} build={appBuild()} />
      <div class="bc-head">
        <Show when={status()} fallback={<span class="bc-muted">{loading() ? 'Reading the worktree…' : ' '}</span>}>{s => <>
          <div class="bc-branch">
            <Icon name="terminal" size={13} />
            <strong title={s().worktree}>{s().branch || `detached at ${s().head}`}</strong>
            <span class="bc-muted">→ {s().base}</span>
          </div>
          <div class="bc-counts">
            <span title="Commits on this branch that the base doesn't have">{s().ahead} ahead</span>
            <Show when={s().behind}><span class="bc-behind" title="Commits on the base since this branch started">{s().behind} behind</span></Show>
            <Show when={s().upstream} fallback={<span class="bc-muted">not pushed</span>}><span class="bc-muted" title={s().upstream!}>{s().unpushed ? `${s().unpushed} to push` : 'pushed'}</span></Show>
            <Show when={s().pr}>{pr => <button class="bc-pr" data-state={pr().state} onClick={() => openLink(pr().url)} title={pr().url}>PR #{pr().number} · {pr().state}</button>}</Show>
          </div>
        </>}</Show>
        <Show when={readAt()}>{time => <span class="bc-muted" title={time().toLocaleString()}>Read {time().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</span>}</Show>
        <button class="btn icon" aria-label="Refresh changes" title="Refresh changes" disabled={loading()} onClick={() => void refresh()}><Icon name="refresh" size={14} /></button>
      </div>

      {/* The Mac mini runner (2026-11-01): where this workspace's Claude and
          Codex turns run. Default follows Settings → Models. */}
      <RunsOnControl class="bc-runs-on" workspaceId={id()} saved={props.workspace.build?.runsOn} />

      <Show when={error()}>{msg => <ErrorText class="bc-error" as="div" text={msg()}/>}</Show>
      <Show when={readError()}>{msg => <ErrorText class="bc-error" as="div" text={msg()}/>}</Show>

      <div class="bc-scroll">
        <Show when={status()}>{s => <>
          <section class="bc-files" aria-label="Changed files">
            <For each={s().files} fallback={<p class="bc-muted bc-empty">No changes against {s().base} yet. Ask the agent to make one.</p>}>{f =>
              <div class="bc-file-block">
                <button class="bc-file" aria-pressed={selected() === f.path} onClick={() => selectFile(f.path)} title={f.oldPath ? `${f.oldPath} → ${f.path}` : f.path}>
                  <span class="bc-letter" data-status={f.status} aria-label={f.status}>{LETTER[f.status]}</span>
                  <span class="bc-path">{f.path}</span>
                  <Show when={f.uncommitted}><span class="bc-dot" title="Not committed yet" aria-label="uncommitted" /></Show>
                  <span class="bc-stat">
                    <Show when={f.additions != null} fallback={<span class="bc-muted">binary</span>}>
                      <span class="bc-add">+{f.additions}</span> <span class="bc-del">−{f.deletions}</span>
                    </Show>
                  </span>
                </button>
                <Show when={selected() === f.path}>
                  <FileDiff loading={diffLoading()} diff={diff()?.path === f.path ? diff() : null} />
                </Show>
              </div>
            }</For>
            <Show when={s().moreFiles}><p class="bc-muted">{s().moreFiles} more files not listed.</p></Show>
          </section>

          <Show when={s().commits.length}>
            <details class="bc-commits"><summary>{s().commits.length} commit{s().commits.length === 1 ? '' : 's'} on this branch</summary>
              <For each={s().commits}>{c => <div class="bc-commit"><code>{c.sha}</code><span>{c.subject}</span></div>}</For>
            </details>
          </Show>

          <section class="bc-actions" aria-label="Commit and publish">
            <textarea class="bc-message" aria-label="Commit message" rows={2} placeholder={uncommitted() ? 'Describe the change…' : 'Nothing to commit'} value={message()} onInput={e => setMessage(e.currentTarget.value)} disabled={!uncommitted()} />
            <div class="bc-buttons">
              <button class="btn small primary" disabled={!uncommitted() || !message().trim() || !!acting() || !!blocked()} onClick={commit}>{acting() === 'commit' ? 'Committing…' : 'Commit'}</button>
              <button class="btn small" disabled={!canPush() || !!acting() || !!blocked()} onClick={push}>{acting() === 'push' ? 'Pushing…' : 'Push'}</button>
              {/* An open or merged PR is the branch's PR; only a closed one offers another (P1-46). */}
              <Show when={!s().pr || s().pr!.state === 'closed'}>
                <button class="btn small" disabled={!s().ahead || !s().branch || !!acting() || !!blocked()} onClick={startPr}>Open PR…</button>
              </Show>
              <Show when={s().behind > 0}>
                <button class="btn small" disabled={!!uncommitted() || !!acting() || !!blocked()} title={uncommitted() ? 'Commit or discard the uncommitted changes first' : undefined} onClick={updateFromBase}>{acting() === 'update' ? 'Updating…' : `Update from ${s().base}…`}</button>
              </Show>
            </div>
            <Show when={blocked()}>{reason => <p class="bc-muted" role="status">{reason()}</p>}</Show>
            <Show when={prForm()}>
              <div class="bc-pr-form">
                <label>Title<input aria-label="Pull request title" value={prTitle()} onInput={e => setPrTitle(e.currentTarget.value)} /></label>
                <label>Description<textarea aria-label="Pull request description" rows={4} value={prBody()} onInput={e => setPrBody(e.currentTarget.value)} /></label>
                <div class="bc-buttons">
                  <button class="btn small primary" disabled={!prTitle().trim() || !!acting() || !!blocked()} onClick={openPr}>{acting() === 'pr' ? 'Opening…' : 'Open pull request'}</button>
                  <button class="btn small ghost" onClick={() => setPrForm(false)}>Cancel</button>
                </div>
              </div>
            </Show>
          </section>
          <ServicesPanel workspace={props.workspace} />
          <AppBuildPanel workspace={props.workspace} build={appBuild()} onBuild={setAppBuild} behind={s().behind} base={s().base} />
        </>}</Show>
      </div>
    </div>
  );
}

function FileDiff(props: { loading: boolean; diff: { files: DiffFile[]; truncated: boolean; cut: string[] } | null }) {
  return (
    <div class="bc-diff">
      <Show when={props.diff} fallback={<p class="bc-muted">{props.loading ? 'Loading the diff…' : 'No diff.'}</p>}>{d => <>
        <For each={d().files}>{file => <>
          <Show when={file.binary}><p class="bc-muted">Binary file; no text diff.</p></Show>
          <Show when={!file.binary && !file.hunks.length}><p class="bc-muted">{file.status === 'renamed' ? `Renamed from ${file.oldPath} with no content change.` : 'No content change (mode or empty file).'}</p></Show>
          <For each={file.hunks}>{h =>
            <div class="bc-hunk">
              <div class="bc-hunk-head">
                <code>{h.header.replace(/ @@.*$/, ' @@')}</code><span class="bc-section">{h.section}</span>
                <button class="btn small ghost" onClick={() => appendToChatDraft(askAboutHunk(file, h))} title="Add this hunk to your message (nothing is sent)">Ask about this</button>
              </div>
              <table class="bc-lines"><tbody>
                <For each={h.lines}>{l =>
                  <tr class={`bc-line ${l.kind}`}>
                    <td class="bc-no">{l.oldNo ?? ''}</td><td class="bc-no">{l.newNo ?? ''}</td>
                    <td class="bc-code"><span class="bc-sign">{l.kind === 'add' ? '+' : l.kind === 'del' ? '−' : ' '}</span>{l.text}</td>
                  </tr>
                }</For>
              </tbody></table>
            </div>
          }</For>
        </>}</For>
        <Show when={d().cut.length}><p class="bc-muted">The diff was cut at 256 KB for {d().cut.join(', ')}.</p></Show>
        <Show when={d().truncated}><p class="bc-muted">The diff is over 1 MB; the rest is not shown.</p></Show>
      </>}</Show>
    </div>
  );
}
