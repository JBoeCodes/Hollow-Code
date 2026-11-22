import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from 'solid-js';
import { Icon } from '../icons';
import { api } from '../api';
import { latestRequest } from '../latestRequest';
import { appendToChatDraft } from '../store/chatRuntime';
import { flashFooter, openInBrowser, showConfirm, showPrompt } from '../store';
import { requestOpenTask } from '../store/runtime';
import { selectWorkspaceOpened, workspaceDetail, workspaceId, workspaces } from '../store/workspaces';
import { errMsg, openExternal } from '../utils';
import { Button, ErrorText, Segmented, StatePanel } from './ui';
import { Select } from './Select';
import { ActionMenu, type ActionMenuItem } from './ActionMenu';
import { checksSummary, mergeWords, reviewStateWords, reviewWords, safeLink } from '../prPanelModel';
import {
  LINEAR_UNTRACKED, buildQuery, confirmCopy, discussionDraft, emptyWords, filtered, firstChoice, itemDraft, labelColor, linearLink, linearWords,
  movesFor, noun, nouns, repoChoices, rowByline, rowChecksWords, rowStateWords, stateChoices, stateFor,
  type GhDetail, type GhKind, type GhList, type GhRow, type GhStateFilter, type Triage,
} from '../githubModel';
import '../styles/pr-panel.css';
import '../styles/github-pages.css';

const SAVED_PROJECT = 'hollow.github.project';
const saved = () => { try { return localStorage.getItem(SAVED_PROJECT); } catch { return null; } };

/** Projects → Pull requests (parity 7.3, D6): a project's GitHub pull requests
 *  and issues through the person's own `gh`. List and filter, open one with its
 *  checks and comments, and triage it (comment, label, assign, close or reopen).
 *  Every change shows a dialog naming the repository first; "Ask about this"
 *  only drafts a message in the project's chat. Linear stays the tracker: a
 *  key a title or branch names links to the mirrored task, nothing more. */
export function GithubPages() {
  const choices = createMemo(() => repoChoices(workspaces()));
  const [projectId, setProjectId] = createSignal<string | null>(null);
  const project = createMemo(() => choices().find(c => c.id === projectId()) ?? null);
  // Pick the first project once the list is there; the open one wins.
  createEffect(() => {
    if (projectId() && project()) return;
    const first = firstChoice(choices(), { open: workspaceDetail()?.workspace.id ?? workspaceId(), saved: saved() });
    setProjectId(first?.id ?? null);
  });
  const chooseProject = (id: string) => { setProjectId(id); try { localStorage.setItem(SAVED_PROJECT, id); } catch { /* the choice just isn't remembered */ } };

  const [kind, setKind] = createSignal<GhKind>('pr');
  const [state, setState] = createSignal<GhStateFilter>('open');
  const [label, setLabel] = createSignal('');
  const [assignee, setAssignee] = createSignal('');
  const [search, setSearch] = createSignal('');
  const [reviewRequested, setReviewRequested] = createSignal(false);
  // Text filters apply on Filter or Enter; the rest apply at once.
  const [applied, setApplied] = createSignal({ label: '', assignee: '', search: '' });
  const query = createMemo(() => buildQuery(kind(), { state: state(), reviewRequested: reviewRequested(), ...applied(), author: '' }));

  const [list, setList] = createSignal<GhList | null>(null);
  const [listError, setListError] = createSignal('');
  const [loading, setLoading] = createSignal(false);
  const reads = latestRequest();
  async function load() {
    const cwd = project()?.cwd;
    const gen = reads.start();
    setListError('');
    if (!cwd) { setList(null); setLoading(false); return; }
    setLoading(true);
    try {
      const next = await api.githubList(cwd, query());
      if (reads.isLatest(gen)) setList(next);
    } catch (e) { if (reads.isLatest(gen)) { setList(null); setListError(errMsg(e)); } }
    finally { if (reads.isLatest(gen)) setLoading(false); }
  }
  // Another project, kind or filter: drop the old list at once and read again.
  createEffect(on([() => project()?.cwd, query], () => { setSelected(null); setList(null); void load(); }));
  onCleanup(() => { reads.cancel(); details.cancel(); });

  const [selected, setSelected] = createSignal<number | null>(null);
  const [detail, setDetail] = createSignal<GhDetail | null>(null);
  const [detailNote, setDetailNote] = createSignal('');
  const [detailLoading, setDetailLoading] = createSignal(false);
  const details = latestRequest();
  async function loadDetail(n: number | null) {
    const cwd = project()?.cwd;
    const gen = details.start();
    setDetailNote('');
    if (n === null || !cwd) { setDetail(null); setDetailLoading(false); return; }
    setDetailLoading(true);
    try {
      const v = await api.githubDetail(cwd, kind(), n);
      if (!details.isLatest(gen)) return;
      setDetail(v.detail); setDetailNote(v.note ?? '');
    } catch (e) { if (details.isLatest(gen)) { setDetail(null); setDetailNote(errMsg(e)); } }
    finally { if (details.isLatest(gen)) setDetailLoading(false); }
  }
  createEffect(on(selected, n => { setDetail(null); void loadDetail(n); }));
  const open = (url: string | null | undefined) => { const safe = safeLink(url); if (safe) openInBrowser(safe).catch(() => openExternal(safe)); };

  const applyText = (e: Event) => { e.preventDefault(); setApplied({ label: label(), assignee: assignee(), search: search() }); };
  const clearFilters = () => { setState('open'); setLabel(''); setAssignee(''); setSearch(''); setApplied({ label: '', assignee: '', search: '' }); setReviewRequested(false); };
  const changeKind = (k: GhKind) => { setKind(k); setState(s => stateFor(k, s)); };

  // Triage: a dialog naming the repository and the change comes first, always.
  const [busy, setBusy] = createSignal(false);
  async function run(t: Triage, confirmed: boolean) {
    const cwd = project()?.cwd; const d = detail();
    if (!cwd || !d || busy()) return;
    if (!confirmed) {
      const copy = confirmCopy(d.repo, d.kind, t, d.item.title);
      if (!await showConfirm({ title: copy.title, body: copy.body, confirmLabel: copy.confirmLabel, danger: copy.danger, focusCancel: true })) return;
    }
    setBusy(true);
    try {
      const done = await api.githubTriage(cwd, t);
      flashFooter(done.message);
      if (t.action === 'comment') setComment('');
      if (project()?.cwd === cwd) await Promise.all([loadDetail(t.number), load().then(() => { if (project()?.cwd === cwd && kind() === t.kind) setSelected(t.number); })]);
    } catch (e) { flashFooter(`× ${errMsg(e)}`, 7000); }
    finally { setBusy(false); }
  }
  async function typed(action: 'label' | 'assign', d: GhDetail, initial = '') {
    const copy = confirmCopy(d.repo, d.kind, { kind: d.kind, number: d.item.number, action }, d.item.title);
    const value = await showPrompt({
      title: copy.title, body: copy.body, confirmLabel: copy.confirmLabel,
      input: action === 'label' ? { value: initial, label: 'Label', placeholder: 'bug', maxLength: 100 } : { value: initial, label: 'GitHub login', placeholder: 'octocat', maxLength: 40 },
    });
    if (value) await run({ kind: d.kind, number: d.item.number, action, value }, true);
  }
  const [comment, setComment] = createSignal('');
  const triageItems = (d: GhDetail): ActionMenuItem[] => {
    const n = d.item.number; const k = d.kind; const moves = movesFor(d.item.state);
    return [
      { label: 'Add label…', action: () => void typed('label', d) },
      ...d.labels.map(l => ({ label: `Remove label ${l.name}`, action: () => void run({ kind: k, number: n, action: 'unlabel', value: l.name }, false) })),
      { label: 'Assign to me', action: () => void run({ kind: k, number: n, action: 'assign', value: '@me' }, false), separatorBefore: true },
      { label: 'Assign someone…', action: () => void typed('assign', d) },
      ...d.assignees.map(a => ({ label: `Unassign ${a}`, action: () => void run({ kind: k, number: n, action: 'unassign', value: a }, false) })),
      ...(moves.close ? [{ label: `Close ${noun(k)}`, action: () => void run({ kind: k, number: n, action: 'close' }, false), danger: true, separatorBefore: true }] : []),
      ...(moves.reopen ? [{ label: `Reopen ${noun(k)}`, action: () => void run({ kind: k, number: n, action: 'reopen' }, false), separatorBefore: true }] : []),
    ];
  };

  // "Ask about this" opens the project's chat and drafts there; nothing is sent.
  async function ask(text: string) {
    const id = projectId();
    if (!id) return;
    if (workspaceId() !== id && !await selectWorkspaceOpened(id)) { flashFooter('× The project didn’t open, so nothing was added. Try Ask about this again.', 6000); return; }
    appendToChatDraft(text);
  }
  const askLabel = 'Open this project’s chat with it in the message box (nothing is sent)';

  const repoLine = () => list()?.repo ?? detail()?.repo ?? null;

  return (
    <section class="gh-page" aria-label="Pull requests and issues">
      <div class="gh-bar">
        <Show when={choices().length} fallback={<span class="bc-muted">No project with a folder yet.</span>}>
          <Select size="sm" label="Project" value={projectId() ?? ''} onChange={chooseProject} options={choices().map(c => ({ value: c.id, label: c.title }))} />
        </Show>
        <Segmented label="Show" items={[{ value: 'pr', label: 'Pull requests' }, { value: 'issue', label: 'Issues' }]} value={kind()} onChange={changeKind} />
        <Segmented label="State" items={stateChoices(kind())} value={state()} onChange={v => setState(v)} />
        <Show when={repoLine()}>{r => <span class="gh-repo bc-muted">{r()}</span>}</Show>
        <Button size="sm" variant="ghost" icon="refresh" disabled={loading() || !project()} aria-label="Refresh the list" title="Refresh the list" onClick={() => void load()}>Refresh</Button>
      </div>
      <form class="gh-filters" role="search" aria-label="Filter the list" onSubmit={applyText}>
        <input type="search" aria-label="Search text" placeholder="Search" value={search()} onInput={e => setSearch(e.currentTarget.value)} />
        <input type="text" aria-label="Label" placeholder="Label" value={label()} onInput={e => setLabel(e.currentTarget.value)} />
        <input type="text" aria-label="Assignee" placeholder="Assignee or @me" value={assignee()} onInput={e => setAssignee(e.currentTarget.value)} />
        <Show when={kind() === 'pr'}>
          <label class="gh-check"><input type="checkbox" checked={reviewRequested()} onChange={e => setReviewRequested(e.currentTarget.checked)} /> Waiting for my review</label>
        </Show>
        <Button size="sm" type="submit">Filter</Button>
        <Show when={filtered(query()) || label() || assignee() || search()}><Button size="sm" variant="ghost" onClick={clearFilters}>Clear filters</Button></Show>
      </form>

      <div class="gh-split">
        <div class="gh-list-col">
          <Show when={listError()}>{msg => <ErrorText class="bc-error" as="div" text={msg()} />}</Show>
          <Show when={loading() && !list()}><p class="bc-muted" role="status">Asking GitHub…</p></Show>
          <Show when={list()?.note}>{note => <StatePanel compact kind="empty" title={`No ${nouns(kind())} to show`}>{note()}</StatePanel>}</Show>
          <Show when={list() && !list()!.note && !list()!.rows.length}><p class="bc-muted" role="status">{emptyWords(kind(), query())}</p></Show>
          <Show when={list()?.rows.length}>
            <ul class="gh-rows" aria-label={nouns(kind())}>
              <For each={list()!.rows}>{row => <Row row={row} kind={kind()} selected={selected() === row.number} onOpen={() => setSelected(row.number)} />}</For>
            </ul>
            <Show when={list()?.more}><p class="bc-muted">Showing the first 50. Narrow the filters to see others.</p></Show>
          </Show>
        </div>

        <div class="gh-detail-col" aria-label={`${noun(kind())} details`} role="region">
          <Show when={!selected()}><p class="bc-muted">Choose a {noun(kind())} to see its checks and comments.</p></Show>
          <Show when={selected() && detailLoading() && !detail()}><p class="bc-muted" role="status">Asking GitHub…</p></Show>
          <Show when={detailNote()}>{note => <StatePanel compact kind="empty" title={`No ${noun(kind())} to show`}>{note()}</StatePanel>}</Show>
          <Show when={detail()}>{d => (
            <div class="pr-panel gh-detail">
              <header class="pr-head">
                <h2><button type="button" class="pr-link" onClick={() => open(d().item.url)} title={d().item.url}>#{d().item.number} {d().item.title}</button></h2>
                <p class="pr-meta">
                  <span class="pr-chip" data-state={d().item.draft ? 'draft' : d().item.state}>{rowStateWords({ state: d().item.state, draft: d().item.draft })}</span>
                  <span>{d().author}</span>
                  <Show when={d().kind === 'pr' && d().item.head}><span>{d().item.head} → {d().item.base}</span></Show>
                  <Show when={reviewWords(d().item.reviewDecision)}>{w => <span class="pr-chip" data-review={d().item.reviewDecision}>{w()}</span>}</Show>
                  <Show when={d().item.state === 'open' && mergeWords(d().item.mergeState)}>{w => <span class="bc-muted">{w()}</span>}</Show>
                </p>
                <Show when={d().labels.length || d().assignees.length}>
                  <p class="gh-tags">
                    <For each={d().labels}>{l => <Label name={l.name} color={l.color} />}</For>
                    <Show when={d().assignees.length}><span class="bc-muted">Assigned to {d().assignees.join(', ')}</span></Show>
                  </p>
                </Show>
                <div class="gh-actions">
                  <ActionMenu label="Triage" items={triageItems(d())} icon={<><Icon name="edit" size={13} /> Triage</>} />
                  <Button size="sm" variant="ghost" title={askLabel} onClick={() => void ask(itemDraft(d()))}>Ask about this</Button>
                  <Button size="sm" variant="ghost" onClick={() => open(d().item.url)}>Open on GitHub</Button>
                </div>
              </header>

              <Show when={d().linear.length}>
                <section class="pr-section" aria-label="Linear">
                  <h3>Linear</h3>
                  <ul class="gh-linear">
                    <For each={d().linear}>{ref => (
                      <li>
                        <span class="gh-linear-key">{linearWords(ref)}</span>
                        <Show when={ref.tracked && linearLink(ref)} fallback={<span class="bc-muted">{LINEAR_UNTRACKED}</span>}>{u => <>
                          <Button size="sm" variant="ghost" onClick={() => requestOpenTask(u())}>Open in Tasks</Button>
                          <Button size="sm" variant="ghost" onClick={() => open(u())}>Open in Linear</Button>
                        </>}</Show>
                      </li>
                    )}</For>
                  </ul>
                </section>
              </Show>

              <Show when={d().body}>
                <section class="pr-section" aria-label="Description"><h3>Description</h3><p class="pr-comment-body">{d().body}</p></section>
              </Show>

              <Show when={d().kind === 'pr'}>
                <section class="pr-section" aria-label="Checks">
                  <h3>Checks <span class="pr-chip" data-checks={checksSummary(d().item.checks).worst}>{checksSummary(d().item.checks).words}</span></h3>
                  <ul class="pr-checks">
                    <For each={d().item.checks}>{ch => (
                      <li data-state={ch.state}>
                        <span class="pr-dot" data-state={ch.state} role="img" aria-label={ch.state} />
                        <Show when={safeLink(ch.url)} fallback={<span>{ch.name}</span>}>{u => <button type="button" class="pr-link" onClick={() => open(u())} title={u()}>{ch.name}</button>}</Show>
                      </li>
                    )}</For>
                  </ul>
                  <Show when={d().item.moreChecks}><p class="bc-muted">{d().item.moreChecks} more checks not listed.</p></Show>
                </section>
              </Show>

              <section class="pr-section" aria-label="Comments">
                <h3>Comments <span class="bc-muted">{d().item.comments.length + d().item.moreComments}</span></h3>
                <Show when={d().item.moreComments}><p class="bc-muted">The {d().item.moreComments} oldest are on GitHub.</p></Show>
                <Show when={d().item.comments.length} fallback={<p class="bc-muted">No comments yet.</p>}>
                  <ul class="pr-comments">
                    <For each={d().item.comments}>{c => (
                      <li>
                        <div class="pr-comment-head">
                          <strong>{c.author}</strong>
                          <Show when={c.kind === 'review'}><span class="pr-chip" data-review={c.state ?? ''}>{reviewStateWords(c.state)}</span></Show>
                          <time class="bc-muted" datetime={c.createdAt}>{c.createdAt ? new Date(c.createdAt).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' }) : ''}</time>
                        </div>
                        <Show when={c.body}><p class="pr-comment-body">{c.body}</p></Show>
                        <div class="pr-comment-actions">
                          <Show when={c.body}><Button size="sm" variant="ghost" title={askLabel} onClick={() => void ask(discussionDraft(d(), c))}>Ask about this</Button></Show>
                          <Show when={safeLink(c.url)}>{u => <Button size="sm" variant="ghost" onClick={() => open(u())}>Open on GitHub</Button>}</Show>
                        </div>
                      </li>
                    )}</For>
                  </ul>
                </Show>
                <form class="gh-comment" aria-label="Write a comment" onSubmit={e => { e.preventDefault(); void run({ kind: d().kind, number: d().item.number, action: 'comment', value: comment() }, false); }}>
                  <textarea aria-label="Comment" rows="3" placeholder="Write a comment" value={comment()} onInput={e => setComment(e.currentTarget.value)} />
                  <Button size="sm" type="submit" disabled={busy() || !comment().trim()}>Comment…</Button>
                </form>
              </section>
            </div>
          )}</Show>
        </div>
      </div>
    </section>
  );
}

function Label(props: { name: string; color: string }) {
  const c = () => labelColor(props.color);
  return <span class="gh-label" style={c() ? { '--gh-label': c()! } : undefined} data-colored={c() ? 'true' : 'false'}>{props.name}</span>;
}

function Row(props: { row: GhRow; kind: GhKind; selected: boolean; onOpen: () => void }) {
  const r = () => props.row;
  const checks = () => rowChecksWords(r().checks);
  return (
    <li>
      <button type="button" class="gh-row" classList={{ selected: props.selected }} aria-current={props.selected ? 'true' : undefined} onClick={props.onOpen}>
        <span class="gh-row-title"><span class="gh-num">#{r().number}</span> {r().title}</span>
        <span class="gh-row-meta">
          <span class="pr-chip" data-state={r().draft ? 'draft' : r().state}>{rowStateWords(r())}</span>
          <Show when={checks()}>{w => <span class="pr-chip" data-checks={r().checks}>{w()}</span>}</Show>
          <Show when={reviewWords(r().reviewDecision)}>{w => <span class="pr-chip" data-review={r().reviewDecision}>{w()}</span>}</Show>
          <For each={r().labels}>{l => <Label name={l.name} color={l.color} />}</For>
          <For each={r().linear}>{ref => <span class="gh-linear-chip" title={ref.tracked ? linearWords(ref) : LINEAR_UNTRACKED}>{ref.key}</span>}</For>
        </span>
        <span class="gh-row-by bc-muted">
          {rowByline(r())}
          <Show when={r().updatedAt}> · <time datetime={r().updatedAt}>{new Date(r().updatedAt).toLocaleDateString([], { month: 'short', day: 'numeric' })}</time></Show>
        </span>
      </button>
    </li>
  );
}
