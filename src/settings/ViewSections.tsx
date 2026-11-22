/**
 * Settings sections added by t3code parity 4.7 (A17): Diffs, Browser,
 * Worktrees and Archived (About waits on #1671, which creates that section). Each is small and reads what Hollow already
 * has: the two diff view switches and the link target are per-machine
 * preferences (store/viewPrefs.ts); Worktrees reports `worktree_hygiene`;
 * Archived lists settled conversations with Restore.
 */
import { For, Show, createResource, createSignal } from 'solid-js';
import { api } from '../api';
import { Switch } from '../components/Switch';
import { Select } from '../components/Select';
import { ErrorText } from '../components/ui';
import { flashFooter, showConfirm } from '../store';
import { errMsg } from '../utils';
import { diffNumbers, diffWrap, linkTarget, setDiffNumbers, setDiffWrap, setLinkTarget, type LinkTarget } from '../store/viewPrefs';
import { restoreConversation } from '../conversationActions';

export function DiffsPanel() {
  return (
    <>
      <div class="settings-row">
        <div class="settings-row-label">
          <div class="settings-row-k">Wrap long lines</div>
          <div class="settings-row-v">Off: a long line scrolls sideways inside its hunk.</div>
        </div>
        <Switch checked={diffWrap()} label="Wrap long lines" onChange={on => setDiffWrap(on)} />
      </div>
      <div class="settings-row">
        <div class="settings-row-label">
          <div class="settings-row-k">Show line numbers</div>
          <div class="settings-row-v">The old and new line number beside each changed line.</div>
        </div>
        <Switch checked={diffNumbers()} label="Show line numbers" onChange={on => setDiffNumbers(on)} />
      </div>
    </>
  );
}

const LINK_OPTIONS: { value: LinkTarget; label: string }[] = [
  { value: 'preview', label: 'The Preview pane' },
  { value: 'browser', label: 'My default browser' },
];

export function BrowserPanel() {
  return (
    <div class="settings-row">
      <div class="settings-row-label">
        <div class="settings-row-k">Open web links in</div>
        <div class="settings-row-v">Replies, services and pull requests. Hold ⌘ or Shift to use the other.</div>
      </div>
      <Select label="Open web links in" value={linkTarget()} options={LINK_OPTIONS} searchable={false} align="end"
        onChange={v => setLinkTarget(v === 'browser' ? 'browser' : 'preview')} />
    </div>
  );
}

export function WorktreesPanel() {
  const [report, { refetch }] = createResource(() => api.worktreeHygiene(null));
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const worktrees = () => (report()?.repos ?? []).flatMap(r => r.worktrees.map(w => ({ ...w, repo: r.repo })));
  async function clean() {
    const r = report();
    if (!r || !r.remove.length || busy()) return;
    const ok = await showConfirm({
      title: `Remove ${r.remove.length} finished ${r.remove.length === 1 ? 'item' : 'items'}?`,
      body: `${r.remove.join('\n')}\n\nWorktrees are removed with git; their branches stay. Nothing with unsaved work, unpushed commits or a running chat is on this list.`,
      confirmLabel: 'Remove', danger: true, focusCancel: true,
    });
    if (!ok) return;
    setBusy(true); setError('');
    try {
      const done = await api.cleanWorktrees(null, r.hash);
      flashFooter(`Removed ${done.removed.length}${done.failed.length ? `, ${done.failed.length} could not be removed` : ''}`, 3000);
      if (done.failed.length) setError(done.failed.map(([p, e]) => `${p}: ${e}`).join('\n'));
      await refetch();
    } catch (e) { setError(errMsg(e)); } finally { setBusy(false); }
  }
  return (
    <>
      <div class="settings-row">
        <div class="settings-row-label">
          <div class="settings-row-k">Build workspace worktrees</div>
          <div class="settings-row-v">
            <Show when={!report.loading} fallback="Reading worktrees…">
              {worktrees().length} {worktrees().length === 1 ? 'worktree' : 'worktrees'} across {report()?.repos.length ?? 0} {(report()?.repos.length ?? 0) === 1 ? 'repo' : 'repos'};
              {' '}{report()?.remove.length ?? 0} can go.
            </Show>
          </div>
          <Show when={error()}>{msg => <ErrorText class="settings-row-warn" as="div" text={msg()} />}</Show>
        </div>
        <div class="settings-actions">
          <button type="button" class="btn" disabled={report.loading || busy()} onClick={() => void refetch()}>Refresh</button>
          <button type="button" class="btn" disabled={busy() || !(report()?.remove.length)} onClick={() => void clean()}>Clean up…</button>
        </div>
      </div>
      <For each={worktrees()}>{w => (
        <div class="settings-row" data-worktree={w.path}>
          <div class="settings-row-label">
            <div class="settings-row-k">{w.branch ?? 'Detached'}</div>
            <div class="settings-row-v" style="font-family:var(--mono);font-size:var(--fs-caption);">{w.path}</div>
          </div>
          <span class="settings-state-chip" data-state={w.inUse ? 'warn' : w.removable ? 'ok' : undefined}>
            {w.inUse ? 'In use' : w.removable ? 'Can go' : w.reason ?? 'Kept'}
          </span>
        </div>
      )}</For>
    </>
  );
}

export function ArchivedPanel() {
  const [rows, { refetch }] = createResource(() => api.listAgentSessions(200, true));
  const [busy, setBusy] = createSignal('');
  async function restore(key: string) {
    setBusy(key);
    try { await restoreConversation(key, () => refetch()); } finally { setBusy(''); }
  }
  return (
    <>
      <div class="settings-row">
        <div class="settings-row-label">
          <div class="settings-row-k">Archived conversations</div>
          <div class="settings-row-v">Settled conversations. Restore one to bring it back to its project.</div>
        </div>
        <div class="settings-actions"><button type="button" class="btn" disabled={rows.loading} onClick={() => void refetch()}>Refresh</button></div>
      </div>
      <Show when={!rows.loading && !(rows() ?? []).length}>
        <p class="settings-note">Nothing is archived.</p>
      </Show>
      <For each={rows() ?? []}>{s => (
        <div class="settings-row" data-archived={s.key}>
          <div class="settings-row-label">
            <div class="settings-row-k">{s.title?.trim() || 'Untitled conversation'}</div>
            <div class="settings-row-v">{s.agent}{s.lastActivityAt ? ` · ${new Date(s.lastActivityAt).toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' })}` : ''}</div>
          </div>
          <div class="settings-actions">
            <button type="button" class="btn" disabled={busy() === s.key} aria-label={`Restore ${s.title?.trim() || 'this conversation'}`} onClick={() => void restore(s.key)}>Restore</button>
          </div>
        </div>
      )}</For>
    </>
  );
}
