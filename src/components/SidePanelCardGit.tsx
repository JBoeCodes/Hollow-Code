import { Show, createEffect, createResource, createSignal, on, onCleanup, onMount } from 'solid-js';
import { Icon } from '../icons';
import { api } from '../api';
import { latestRequest } from '../latestRequest';
import { ActionMenu, type ActionMenuItem } from './ActionMenu';
import { chatBusy, chatCwd, chatLiveRunId, setChatCwdPersistent } from '../store/chatRuntime';
import { flashFooter, showPrompt, trackTask } from '../store';
import { errMsg } from '../utils';
import { openPreview } from '../store/preview';
import '../styles/card-git.css';
import { branchLabel, branchParts, changesCount, finalMessage, primaryAction, splitMessage, type CardGitStatus } from '../cardGitModel';

/** The card's git rows (parity 5.2, D1 D4 D5): the branch picker with
 *  worktree mode, Changes +N -N and Commit and push with a generated message,
 *  for any git folder. Every row is a visible button or menu item; the
 *  backend (`git_card.rs`) runs the person's own git. Hidden outside git. */
export function SidePanelCardGit() {
  const [status, setStatus] = createSignal<CardGitStatus | null>(null);
  const reads = latestRequest();
  async function refresh() {
    const cwd = chatCwd();
    const gen = reads.start();
    if (!cwd) { setStatus(null); return; }
    try {
      const next = await api.gitCardStatus(cwd);
      if (reads.isLatest(gen)) setStatus(next);
    } catch { if (reads.isLatest(gen)) setStatus(null); }
  }
  // Another folder: drop the old folder's facts at once, so no click mixes them with the new one.
  createEffect(on(chatCwd, () => { setStatus(null); void refresh(); }));
  // A turn that just ended may have changed files or branches.
  createEffect(on(chatLiveRunId, (now, before) => { if (before && !now) void refresh(); }, { defer: true }));
  const onFocus = () => void refresh();
  onMount(() => window.addEventListener('focus', onFocus));
  onCleanup(() => { window.removeEventListener('focus', onFocus); reads.cancel(); });

  // The branch list is read when the menu's rows are built, not on every refresh.
  const [branches, { refetch }] = createResource(() => (status()?.repo ? chatCwd() : null), async cwd => {
    try { return await api.gitCardBranches(cwd); } catch { return null; }
  });
  const wait = () => status()?.blocked ?? (chatBusy() ? 'Wait for this chat’s turn to finish: git would race it.' : null);

  async function run(label: string, job: () => Promise<CardGitStatus | void>, done: string) {
    const cwd = chatCwd();
    // A read still in flight must not put its older answer back over this action's.
    const gen = reads.start();
    try {
      const next = await trackTask(label, job);
      if (next && chatCwd() === cwd && reads.isLatest(gen)) setStatus(next);
      void refetch();
      flashFooter(done);
    } catch (e) { flashFooter(`× ${label} failed: ${errMsg(e)}`, 6000); void refresh(); }
  }
  const switchTo = (name: string) => { const cwd = chatCwd(); if (cwd) void run('Switch branch', () => api.gitCardSwitch(cwd, name, false), `On ${name}`); };
  async function newBranch() {
    const cwd = chatCwd(); if (!cwd) return;
    const name = await showPrompt({ title: 'New branch', body: 'Creates the branch from where this folder is now and switches to it. Uncommitted changes come with you.', confirmLabel: 'Create branch', input: { label: 'Branch name', value: '', placeholder: 'feature/my-change', maxLength: 200 } });
    if (name && chatCwd() === cwd) await run('New branch', () => api.gitCardSwitch(cwd, name, true), `On ${name}`);
  }
  async function newWorktree() {
    const cwd = chatCwd(); if (!cwd) return;
    const name = await showPrompt({ title: 'New worktree', body: 'Makes a separate folder on a new branch, so this chat can work without touching the current checkout. Hollow keeps it with its other worktrees.', confirmLabel: 'Create worktree', input: { label: 'Branch name', value: '', placeholder: 'feature/my-change', maxLength: 200 } });
    if (!name || chatCwd() !== cwd) return;
    try {
      const path = await trackTask('New worktree', () => api.gitCardNewWorktree(cwd, name));
      if (!chatBusy()) { setChatCwdPersistent(path); flashFooter(`This chat now works in the new worktree on ${name}`); }
      else flashFooter(`Worktree created at ${path}; switch this chat's folder to it when the turn ends`, 6000);
      void refetch();
    } catch (e) { flashFooter(`× New worktree failed: ${errMsg(e)}`, 6000); }
  }
  async function commit(push: boolean) {
    const cwd = chatCwd(); const s = status(); if (!cwd || !s) return;
    if (s.files.length + s.moreFiles === 0) {
      if (push) await run('Push', () => api.gitCardCommitPush(cwd, '', true), `Pushed ${s.branch}`);
      return;
    }
    let generated = { subject: '', body: '' };
    try { generated = splitMessage(await api.gitCardSuggestMessage(cwd)); } catch { /* the person writes one */ }
    const text = await showPrompt({
      title: push ? 'Commit and push' : 'Commit',
      body: `${changesCount(s)}, +${s.additions} -${s.deletions}. Every change in the folder is staged and committed to ${s.branch}${push ? ', then pushed to origin' : ''}. The message below is generated from the file names: edit it freely.`,
      confirmLabel: push ? 'Commit and push' : 'Commit',
      input: { label: 'Commit message', value: generated.subject, maxLength: 300 },
    });
    if (!text || chatCwd() !== cwd) return;
    await run(push ? 'Commit and push' : 'Commit', () => api.gitCardCommitPush(cwd, finalMessage(text, generated), push), push ? `Committed and pushed ${s.branch}` : `Committed to ${s.branch}`);
  }

  const branchItems = (s: CardGitStatus): ActionMenuItem[] => {
    const off = wait();
    const rows: ActionMenuItem[] = (branches()?.branches ?? [s.branch].filter(Boolean)).map(name => ({
      label: name === s.branch ? `${name} (current)` : name, disabled: name === s.branch || !!off, title: off ?? `Switch this folder to ${name}`, action: () => switchTo(name),
    }));
    rows.push({ label: 'New branch…', disabled: !!off, title: off ?? 'Create a branch here and switch to it', separatorBefore: true, action: () => void newBranch() });
    rows.push({ label: 'New worktree…', disabled: !!off, title: off ?? 'Work on a new branch in a separate folder', action: () => void newWorktree() });
    return rows;
  };
  const moreItems = (s: CardGitStatus): ActionMenuItem[] => {
    const off = wait(); const dirty = s.files.length + s.moreFiles > 0;
    return [
      { label: 'Commit only', disabled: !dirty || !!off, title: off ?? (dirty ? 'Commit every change without pushing' : 'Nothing to commit'), action: () => void commit(false) },
      { label: 'Push', disabled: s.ahead === 0 || !s.branch || !!off, title: off ?? (s.ahead ? 'Push the branch to origin' : 'Nothing to push'), action: () => void commit(true) },
      { label: 'Pull request', title: 'Open this branch’s pull request, checks and comments', separatorBefore: true, action: () => { const cwd = chatCwd(); if (cwd) openPreview({ kind: 'pr', cwd }); } },
      { label: 'Refresh', action: () => void refresh() },
    ];
  };

  return (
    <Show when={status()?.repo ? status() : null}>{s => {
      const primary = () => primaryAction(s(), chatBusy());
      return (
        <div class="side-card-git" aria-label="Git">
          <ActionMenu label={branchLabel(s())} align="start" icon={<><Icon name="gitBranch" size={14} /><span class="side-card-branch">{branchParts(s()).name}</span><Show when={branchParts(s()).marks}>{m => <span class="side-card-marks-chip">{m()}</span>}</Show></>} items={branchItems(s())} />
          <button type="button" class="side-card-changes" aria-label={`Open the Diff tab: ${changesCount(s())}, ${s().additions} added, ${s().deletions} removed`}
            title={`Open the Diff tab\n${s().files.slice(0, 12).map(f => `${f.status}: ${f.path}`).join('\n')}`} onClick={() => { const cwd = chatCwd(); if (cwd) openPreview({ kind: 'diff', cwd }); }}>
            <span>Changes</span><span class="side-card-diff-add">+{s().additions}</span><span class="side-card-diff-del">-{s().deletions}</span>
          </button>
          <div class="side-card-git-actions">
            <button type="button" class="side-card-commit" disabled={primary().kind === 'none'} title={primary().why ?? (primary().kind === 'push' ? 'Push the branch to origin' : 'Commit every change with a generated message you can edit, then push')}
              onClick={() => void commit(true)}><Icon name="upload" size={14} /><span>{primary().label}</span></button>
            <ActionMenu label="More git actions" align="start" items={moreItems(s())} />
          </div>
        </div>
      );
    }}</Show>
  );
}
