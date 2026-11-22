import { For, Show, createEffect, createMemo, createResource, createSignal, on, onCleanup, type JSX } from 'solid-js';
import { Icon } from '../icons';
import { api } from '../api';
import { latestRequest } from '../latestRequest';
import { askAboutHunk, parseDiff, type DiffFile } from '../diffParse';
import { appendToChatDraft, chatAgentSessionKey, chatCwd } from '../store/chatRuntime';
import { errMsg } from '../utils';
import { ErrorText } from './ui';
import {
  VIEWED_KEY, buildTree, countLineMatches, filterFiles, isViewed, parseViewed, scopeChoices, scopeKey, setViewed, splitMatches, viewedCount,
  type DiffFileRow, type DiffScope, type DiffTreeNode, type ViewedStore,
} from '../diffPanelModel';
import '../styles/build-changes.css';
import '../styles/diff-panel.css';

const LETTER: Record<string, string> = { added: 'A', modified: 'M', deleted: 'D', renamed: 'R', untracked: 'U' };

type Loaded = { files: DiffFile[]; truncated: boolean; cut: string[] };

/** The Diff tab (parity 5.3, D3) for any git folder: the folder's uncommitted
 *  changes or one agent turn's, a file tree, Find (file names and diff text)
 *  and a Viewed checkbox per file that collapses it. Reads only: nothing is
 *  staged or sent. Every control is a visible button or field. */
export function DiffPanel(props: { cwd: string }) {
  const [scope, setScope] = createSignal<DiffScope>({ kind: 'all' });
  const [files, setFiles] = createSignal<DiffFileRow[]>([]);
  const [error, setError] = createSignal('');
  const [loading, setLoading] = createSignal(false);
  const [query, setQuery] = createSignal('');
  const [viewed, setViewedStore] = createSignal<ViewedStore>((() => { try { return parseViewed(localStorage.getItem(VIEWED_KEY)); } catch { return {}; } })());
  const [open, setOpen] = createSignal<Record<string, boolean>>({});
  const [diffs, setDiffs] = createSignal<Record<string, Loaded | 'loading' | string>>({});
  const [matchAt, setMatchAt] = createSignal(0);
  let body: HTMLDivElement | undefined;
  const reads = latestRequest();
  const diffReads = latestRequest();

  // Turn checkpoints exist only for a chat that owns an isolated worktree.
  // The tab is keyed by folder, so a chat's turns are offered only while that
  // chat works in this very folder (its own worktree); else the ids belong elsewhere.
  const [turns, { refetch: refetchTurns }] = createResource(() => chatCwd() === props.cwd ? chatAgentSessionKey() : null, async key => {
    if (!key) return null;
    try { return await api.turnCheckpoints(key); } catch { return null; }
  });
  const choices = createMemo(() => scopeChoices(turns()?.turns ?? []));
  const why = () => { const t = turns(); return chatCwd() !== props.cwd ? 'Per-turn diffs show while this chat works in this folder.' : t && !t.isolated ? 'Per-turn diffs need this chat in its own worktree.' : !chatAgentSessionKey() ? 'Send a message first: turns belong to a chat.' : null; };
  // A turn that is no longer offered (another chat, an undone turn) goes back to all changes.
  createEffect(() => { const s = scope(); if (s.kind === 'turn' && !choices().some(c => scopeKey(c.scope) === scopeKey(s))) setScope({ kind: 'all' }); });

  async function load() {
    const gen = reads.start();
    // A new read supersedes every per-file diff still in flight for the old one.
    diffReads.start();
    const sc = scope();
    setLoading(true); setError('');
    try {
      let rows: DiffFileRow[];
      if (sc.kind === 'all') {
        const s = await api.gitCardStatus(props.cwd);
        if (!s.repo) throw new Error('This folder is not a git repository.');
        rows = s.files.map(x => ({ path: x.path, status: x.status, additions: x.additions, deletions: x.deletions }));
      } else {
        rows = (await api.gitCardRangeFiles(props.cwd, sc.before, sc.after)).map(x => ({ path: x.path, status: x.status, additions: x.additions, deletions: x.deletions }));
      }
      if (!reads.isLatest(gen)) return;
      setFiles(rows);
      // The first few files start open; the rest open when asked.
      const first = rows.slice(0, 5).filter(r => !isViewed(viewed(), props.cwd, sc, r));
      setOpen(Object.fromEntries(first.map(r => [r.path, true])));
      setDiffs({});
      for (const r of first) void loadDiff(r.path);
    } catch (e) { if (reads.isLatest(gen)) { setError(errMsg(e)); setFiles([]); } }
    finally { if (reads.isLatest(gen)) setLoading(false); }
  }
  async function loadDiff(path: string) {
    if (diffs()[path]) return;
    const sc = scope();
    setDiffs(d => ({ ...d, [path]: 'loading' }));
    const gen = diffReads.peek();
    try {
      const d = sc.kind === 'all' ? await api.gitCardDiff(props.cwd, path, null, null) : await api.gitCardDiff(props.cwd, path, sc.before, sc.after);
      if (scope() !== sc || diffReads.peek() !== gen) return;
      setDiffs(m => ({ ...m, [path]: { files: parseDiff(d.diff), truncated: d.truncated, cut: d.truncatedFiles } }));
    } catch (e) { if (scope() === sc && diffReads.peek() === gen) setDiffs(m => ({ ...m, [path]: errMsg(e) })); }
  }
  createEffect(on(scope, () => void load()));
  onCleanup(() => { reads.cancel(); diffReads.cancel(); });

  const linesOf = (d: Loaded | 'loading' | string | undefined): string[] => typeof d === 'object' ? d.files.flatMap(f => f.hunks.flatMap(h => h.lines.map(l => l.text))) : [];
  // Find reads every file's diff (up to 30, newest first as listed) so a text hit in a
  // file whose name doesn't match still shows it; the file list then follows the hits.
  createEffect(on(query, q => { if (q.trim()) for (const f of files().slice(0, 30)) void loadDiff(f.path); }, { defer: true }));
  const shown = createMemo(() => {
    const q = query();
    if (!q.trim()) return files();
    const byPath = new Set(filterFiles(files(), q).map(f => f.path));
    return files().filter(f => byPath.has(f.path) || countLineMatches(linesOf(diffs()[f.path]), q) > 0);
  });
  const tree = createMemo(() => buildTree(shown()));
  const seen = () => viewedCount(viewed(), props.cwd, scope(), files());
  const isSeen = (f: DiffFileRow) => isViewed(viewed(), props.cwd, scope(), f);
  function mark(f: DiffFileRow, on: boolean) {
    const next = setViewed(viewed(), props.cwd, scope(), f, on);
    setViewedStore(next);
    try { localStorage.setItem(VIEWED_KEY, JSON.stringify(next)); } catch { /* Viewed stays for this session. */ }
    setOpen(o => ({ ...o, [f.path]: !on }));
    if (!on) void loadDiff(f.path);
  }
  function toggle(f: DiffFileRow) {
    const next = !(open()[f.path] ?? false);
    setOpen(o => ({ ...o, [f.path]: next }));
    if (next) void loadDiff(f.path);
  }
  function reveal(path: string) {
    setOpen(o => ({ ...o, [path]: true }));
    void loadDiff(path);
    queueMicrotask(() => body?.querySelector<HTMLElement>(`[data-diff-file="${CSS.escape(path)}"]`)?.scrollIntoView({ block: 'start', behavior: 'smooth' }));
  }

  // Find: text matches across the diffs loaded so far; Enter or the arrows step through them.
  const total = createMemo(() => shown().reduce((n, f) => n + countLineMatches(linesOf(diffs()[f.path]), query()), 0));
  function step(by: 1 | -1) {
    const marks = [...(body?.querySelectorAll<HTMLElement>('mark.dp-hit') ?? [])];
    if (!marks.length) return;
    const next = (matchAt() + by + marks.length) % marks.length;
    setMatchAt(next);
    marks.forEach((m, i) => m.classList.toggle('current', i === next));
    marks[next].scrollIntoView({ block: 'center', behavior: 'smooth' });
  }
  createEffect(on(query, () => setMatchAt(-1), { defer: true }));

  const Row = (p: { node: DiffTreeNode; depth: number }): JSX.Element => {
    const n = p.node;
    if (n.kind === 'dir') return (
      <li style={{ '--depth': p.depth }}>
        <span class="dp-dir"><Icon name="folder" size={13} />{n.name}</span>
        <ul><For each={n.children}>{c => <Row node={c} depth={p.depth + 1} />}</For></ul>
      </li>
    );
    const f = n.file;
    return (
      <li style={{ '--depth': p.depth }}>
        <button type="button" class="dp-file-link" classList={{ seen: isSeen(f) }} title={f.path} onClick={() => reveal(f.path)}>
          <span class="bc-letter" data-status={f.status} aria-label={f.status}>{LETTER[f.status] ?? 'M'}</span>
          <span class="dp-name">{n.name}</span>
          <span class="dp-stat"><Show when={f.additions != null} fallback={<span class="bc-muted">binary</span>}><span class="bc-add">+{f.additions}</span> <span class="bc-del">−{f.deletions}</span></Show></span>
        </button>
      </li>
    );
  };

  return (
    <div class="diff-panel" ref={body}>
      <div class="dp-bar">
        <label class="dp-scope"><span>Compare</span>
          <select aria-label="What to compare" value={scopeKey(scope())} onChange={e => { const c = choices().find(x => scopeKey(x.scope) === e.currentTarget.value); if (c) setScope(c.scope); }}>
            <For each={choices()}>{c => <option value={scopeKey(c.scope)}>{c.label}</option>}</For>
          </select>
        </label>
        <button type="button" class="btn icon" aria-label="Refresh the diff" title="Refresh the diff" disabled={loading()} onClick={() => { void refetchTurns(); void load(); }}><Icon name="refresh" size={14} /></button>
      </div>
      <Show when={why()}>{w => <p class="bc-muted dp-note">{w()}</p>}</Show>
      <div class="dp-find">
        <input type="search" aria-label="Find in the diff" placeholder="Find a file or text" value={query()} onInput={e => setQuery(e.currentTarget.value)}
          onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); step(e.shiftKey ? -1 : 1); } }} />
        <Show when={query().trim()}>
          <span class="dp-count" aria-live="polite">{total()} line{total() === 1 ? '' : 's'}, {shown().length} file{shown().length === 1 ? '' : 's'}</span>
          <button type="button" class="btn icon" aria-label="Previous match" title="Previous match" disabled={!total()} onClick={() => step(-1)}><Icon name="arrowUp" size={14} /></button>
          <button type="button" class="btn icon" aria-label="Next match" title="Next match" disabled={!total()} onClick={() => step(1)}><Icon name="arrowDown" size={14} /></button>
        </Show>
        <span class="dp-viewed" title="Files you marked viewed">{seen()}/{files().length} viewed</span>
      </div>
      <Show when={error()}>{msg => <ErrorText class="bc-error" as="div" text={msg()} />}</Show>
      <Show when={!error() && !loading() && files().length === 0}><p class="bc-muted bc-empty">{scope().kind === 'all' ? 'No uncommitted changes in this folder.' : 'This turn changed no files.'}</p></Show>
      <div class="dp-main">
        <Show when={tree().length}>
          <ul class="dp-tree" aria-label="Changed files"><For each={tree()}>{n => <Row node={n} depth={0} />}</For></ul>
        </Show>
        <div class="dp-files">
          <For each={shown()}>{f => (
            <section class="dp-file" data-diff-file={f.path} classList={{ seen: isSeen(f) }}>
              <div class="dp-file-head">
                <button type="button" class="dp-file-toggle" aria-expanded={open()[f.path] ?? false} onClick={() => toggle(f)} title={f.path}>
                  <Icon name={open()[f.path] ? 'chevronDown' : 'chevronRight'} size={14} />
                  <span class="bc-letter" data-status={f.status} aria-label={f.status}>{LETTER[f.status] ?? 'M'}</span>
                  <span class="dp-path">{f.path}</span>
                </button>
                <label class="dp-check"><input type="checkbox" aria-label={`Viewed ${f.path}`} checked={isSeen(f)} onChange={e => mark(f, e.currentTarget.checked)} /><span>Viewed</span></label>
              </div>
              <Show when={open()[f.path]}>
                {(() => {
                  const d = () => diffs()[f.path];
                  return <>
                    <Show when={d() === 'loading' || d() === undefined}><p class="bc-muted">Loading the diff…</p></Show>
                    <Show when={typeof d() === 'string' && d() !== 'loading'}><ErrorText class="bc-error" as="div" text={String(d())} /></Show>
                    <Show when={typeof d() === 'object' ? (d() as Loaded) : null}>{loaded => <>
                      <For each={loaded().files}>{file => <>
                        <Show when={file.binary}><p class="bc-muted">Binary file; no text diff.</p></Show>
                        <Show when={!file.binary && !file.hunks.length}><p class="bc-muted">{file.status === 'renamed' ? `Renamed from ${file.oldPath} with no content change.` : 'No content change (mode or empty file).'}</p></Show>
                        <For each={file.hunks}>{h => (
                          <div class="bc-hunk">
                            <div class="bc-hunk-head">
                              <code>{h.header.replace(/ @@.*$/, ' @@')}</code><span class="bc-section">{h.section}</span>
                              <button class="btn small ghost" onClick={() => appendToChatDraft(askAboutHunk(file, h))} title="Add this hunk to your message (nothing is sent)">Ask about this</button>
                            </div>
                            <table class="bc-lines"><tbody>
                              <For each={h.lines}>{l => (
                                <tr class={`bc-line ${l.kind}`}>
                                  <td class="bc-no">{l.oldNo ?? ''}</td><td class="bc-no">{l.newNo ?? ''}</td>
                                  <td class="bc-code"><span class="bc-sign">{l.kind === 'add' ? '+' : l.kind === 'del' ? '−' : ' '}</span>
                                    <For each={splitMatches(l.text, query())}>{part => part.hit ? <mark class="dp-hit">{part.text}</mark> : <>{part.text}</>}</For></td>
                                </tr>
                              )}</For>
                            </tbody></table>
                          </div>
                        )}</For>
                      </>}</For>
                      <Show when={loaded().cut.length}><p class="bc-muted">The diff was cut at 256 KB for {loaded().cut.join(', ')}.</p></Show>
                      <Show when={loaded().truncated}><p class="bc-muted">The diff is over 1 MB; the rest is not shown.</p></Show>
                    </>}</Show>
                  </>;
                })()}
              </Show>
            </section>
          )}</For>
        </div>
      </div>
    </div>
  );
}
