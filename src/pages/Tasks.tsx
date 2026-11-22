// Tasks, laid out like a tracker's issue list (overhaul B1, board S3,
// design section 12). Team AI's issues from the task tracker's mirror,
// grouped by status in the team's state order, with the vault's local tasks
// in their own group; an issue opens beside the list with every field
// (`tasks/IssueDetail.tsx`), a local task in the local editor
// (`tasks/TaskDetail.tsx`). Keys work only inside this page and never
// overlap macOS shortcuts: j/k or the arrows move, Enter opens, Escape
// closes (from a full-page issue, goes back to the list), c starts a new
// issue, / finds; every one has a visible control. An issue opens beside the
// list or, with Open full page, as the whole page (the list stays mounted,
// hidden, with its scroll and selection; per window).
// Views (My issues, All, Active, Backlog, By project, By assignee), the
// filter bar and Display's grouping and order (B2, `trackerViews.ts`)
// persist per window; "Conflicts (n)" in the view bar lists the issues with
// an open conflict while there are any. Replaced the Notion-era list, board, scopes, saved
// filters, bulk edits and the Unassigned Notion tasks fold (11-10).
import { Show, createEffect, createMemo, createResource, createSignal, on, onCleanup, onMount } from 'solid-js';
import {
  tasksData, enableTasksResource, refetchTasks, taskOpenRequest, setTaskOpenRequest, flashFooter,
  openSettingsFromOutside, trackerSyncPass,
} from '../store';
import { contextMenu } from '../store';
import { refreshWorkspaces } from '../store/workspaces';
import { isTypingTarget } from '../keyboardLeader';
import { modalOpen } from '../modalGuard';
import { applyDrop } from '../dragdrop';
import { isOpenTask } from '../taskStatus';
import { errMsg, taskErrorText } from '../utils';
import { Icon } from '../icons';
import { Button, EmptyState, ErrorText, PageHeader, Skeleton } from '../components/ui';
import { Select } from '../components/Select';
import { TaskEditor } from '../components/TaskEditor';
import { WaitingLink } from '../components/WaitingLink';
import { createTrackerTask, trackerStatus, trackerSync, trackerTeamMeta, trackerView } from '../trackerApi';
import { sourcesStatus } from '../sourcesApi';
import { SourceLabelChip } from '../components/SourceLabelChip';
import type { Task } from '../types';
import type { TrackerState, TrackerTeamMeta, TrackerViewRow } from '../generated/ipc';
import { syncWhen, canSyncNow } from '../trackerSettings';
import { PRIORITY_ORDER, matchesSearch, priorityLabel, step } from '../trackerTasks';
import {
  FILTER_FIELDS, applyView, chooseView, conflictCount, dueMatches, emptyFilters, groupRows, isProjectsView, hasFilters, readSavedView, tracksOnly, writeSavedView,
  type DueFilter, type FilterField, type ViewState,
} from '../trackerViews';
import { DisplayChoices, FilterBar, ViewTabs } from './tasks/ViewBar';
import { TeamFilter, TeamPicker } from './tasks/TeamChoice';
import { createTeamKey, hasTwoTeams, knownTeamKeys, statusTeamKeys, teamName, teamQuery } from '../trackerTeams';
import { IssueList, LOCAL_GROUP } from './tasks/IssueList';
import { IssueDetail } from './tasks/IssueDetail';
import { TaskDetail } from './tasks/TaskDetail';
import { ProjectDetail } from './tasks/ProjectDetail';
import { ProjectList, orderProjects } from './tasks/ProjectList';
import '../styles/workbench.css';
import '../styles/tasks-workspace.css';
import '../styles/tracker-tasks.css';

/** What the pane shows: an issue by its tracker id, a local task by its
 *  url, or a new issue or local task. Module state, so the open item
 *  outlives a visit to another page. */
type Pane = { kind: 'issue'; id: string } | { kind: 'local'; url: string } | { kind: 'project'; id: string } | { kind: 'newIssue' } | { kind: 'newLocal' } | null;
const [pane, setPane] = createSignal<Pane>(null);
// Groups the person folded, and done or canceled groups they opened (those
// start folded). By group id, so opening Done never opens another state.
// The team filter (All teams, or one team's key), this page's; shown only with two teams.
const [teamFilter, setTeamFilter] = createSignal<string | null>(null);
const [folded, setFolded] = createSignal<ReadonlySet<string>>(new Set());
const [unfolded, setUnfolded] = createSignal<ReadonlySet<string>>(new Set());
const startsFolded = (kind: string) => kind === 'completed' || kind === 'canceled';
// The view, filters, grouping and order: this window's, else the last one
// chosen in any window (`trackerViews.ts`).
const storage = (pick: () => Storage) => { try { return pick(); } catch { return null; } };
const [viewState, setViewStateRaw] = createSignal<ViewState>(readSavedView(storage(() => sessionStorage), storage(() => localStorage)));
function setViewState(next: ViewState) {
  setViewStateRaw(next);
  writeSavedView(next, storage(() => sessionStorage), storage(() => localStorage));
}
// Filter chips shown: those with values, and any just added from Filter.
const [shownFilters, setShownFilters] = createSignal<readonly FilterField[]>(
  FILTER_FIELDS.map(f => f.field).filter(f => viewState().filters[f].length > 0));
const LOCAL_VIEWS = new Set(['mine', 'all', 'active']);
// An issue as the whole page (Open full page) or beside the list, this window's.
const PAGE_KEY = 'hollow.tasks.issuePage';
const [issuePage, setIssuePageRaw] = createSignal(storage(() => sessionStorage)?.getItem(PAGE_KEY) === '1');
function setIssuePage(on: boolean) {
  setIssuePageRaw(on);
  try { if (on) sessionStorage.setItem(PAGE_KEY, '1'); else sessionStorage.removeItem(PAGE_KEY); } catch { /* private mode */ }
}

export function Tasks() {
  const [query, setQuery] = createSignal('');
  const [error, setError] = createSignal('');
  const [syncing, setSyncing] = createSignal(false);
  const [busyLocal, setBusyLocal] = createSignal<ReadonlySet<string>>(new Set());
  const [status, { refetch: refetchStatus }] = createResource(() => trackerStatus().catch(() => null));
  // A failed read keeps the page (and its local tasks) up and says why.
  const [viewError, setViewError] = createSignal('');
  const [view, { refetch: refetchView }] = createResource(() => ({ team: teamFilter() }), ({ team }) => trackerView({ includeClosed: true, limit: 10_000, ...(team ? { teamKey: team } : {}) })
    .then(v => { setViewError(''); return v; }, e => { setViewError(errMsg(e)); return undefined; }));
  const [meta, { refetch: refetchMeta }] = createResource<TrackerTeamMeta | undefined>(() => trackerTeamMeta().catch(() => undefined));
  // H4b: where tasks live, in Rust's words ("Tracked in Linear", "Linear unreachable since 9:40 AM").
  const [sources] = createResource(() => sourcesStatus().catch(() => undefined));
  const tasksLabel = () => sources()?.rows.find(r => r.role === 'tasks')?.label;
  let search: HTMLInputElement | undefined;
  let collection: HTMLElement | undefined;
  // The list's scroll when an issue went full page, put back on the way out
  // (a hidden list loses its scroll position).
  let listScroll = 0;
  const fullPage = () => issuePage() && pane()?.kind === 'issue';
  function setMode(mode: 'pane' | 'page') {
    if (mode === 'page') { listScroll = collection?.scrollTop ?? 0; setIssuePage(true); return; }
    backToList();
  }
  function backToList() {
    setIssuePage(false);
    // A narrow window shows the pane instead of the list: close it there.
    if (collection && getComputedStyle(collection).display === 'none') setPane(null);
    if (collection) collection.scrollTop = listScroll;
    const id = selectedKey();
    if (id) document.querySelector<HTMLElement>(`.issue-row[data-issue-id="${CSS.escape(id)}"] .issue-row-main`)?.focus({ preventScroll: true });
  }

  onMount(() => { enableTasksResource(); void refreshWorkspaces(); });
  // A pass (the timer, Sync now here or in Settings, an agent) refreshes
  // the list, the header and the team's states.
  createEffect(on(trackerSyncPass, pass => { if (pass) refreshAll(); }, { defer: true }));
  function refreshAll() { void refetchView(); void refetchStatus(); void refetchMeta(); }

  // The teams in the mirror (shared first). The filter, the row chips and the
  // create choice show only with two; a filtered view still lists both.
  const teamKeys = () => knownTeamKeys(view()?.teamKeys, statusTeamKeys(status()));
  const twoTeams = () => hasTwoTeams(teamKeys());
  const chosenTeam = () => teamQuery(teamFilter(), teamKeys());
  const displayName = () => view()?.displayName ?? status()?.displayName ?? 'the tracker';
  const tracked = () => !!view()?.provider && !view()?.note;
  const viewer = () => meta()?.viewer?.id ?? null;
  // My issues needs to know whom the key belongs to; until a pass reads it
  // the view says so instead of showing nobody's issues as yours.
  const needsViewer = () => tracked() && viewState().view === 'mine' && !meta.loading && !viewer();
  const rows = createMemo(() => applyView((view()?.rows ?? []).filter(r => matchesSearch(r, query())), viewState(), viewer()));
  const groups = createMemo(() => groupRows(rows(), viewState(), view()?.states ?? [], viewer()));
  // Local tasks: the vault's own tasks the tracker doesn't hold. Open ones,
  // and done ones only when a search asks for them. They sit in My issues,
  // All and Active, under the label and due filters; a filter only an
  // issue has (status, priority, assignee, cycle, project) hides them. A
  // ticked task stays, its check disabled, until its write lands (P0-22).
  const localShown = () => LOCAL_VIEWS.has(viewState().view) && !tracksOnly(viewState().filters) && !viewState().filters.project.length;
  const local = createMemo(() => !localShown() ? [] : (tasksData()?.items ?? []).filter(t => !t.tracker
    && (isOpenTask(t.status) || query().trim() || busyLocal().has(t.url)) && localMatches(t, query()) && localFiltered(t, viewState())));
  const isFolded = (id: string, kind: string) => folded().has(id) || (startsFolded(kind) && !unfolded().has(id));
  const collapsedIds = createMemo(() => new Set([...groups().filter(g => isFolded(g.id, g.kind)).map(g => g.id), ...(folded().has(LOCAL_GROUP) ? [LOCAL_GROUP] : [])]));
  function toggleGroup(id: string) {
    const kind = groups().find(x => x.id === id)?.kind ?? 'local';
    const f = new Set(folded()), u = new Set(unfolded());
    if (isFolded(id, kind)) { f.delete(id); u.add(id); } else { f.add(id); u.delete(id); }
    setFolded(f); setUnfolded(u);
  }

  const projectsShown = () => tracked() && isProjectsView(viewState());
  const projects = createMemo(() => orderProjects((view()?.projects ?? []).filter(p => !query().trim() || p.name.toLowerCase().includes(query().trim().toLowerCase()))));
  /** Every row the keys move through, in screen order. */
  const order = createMemo((): { kind: 'issue' | 'local' | 'project'; id: string }[] => projectsShown() ? projects().map(p => ({ kind: 'project' as const, id: p.id })) : [
    ...groups().filter(g => !collapsedIds().has(g.id)).flatMap(g => g.rows.map(r => ({ kind: 'issue' as const, id: r.id }))),
    ...(collapsedIds().has(LOCAL_GROUP) ? [] : local().map(t => ({ kind: 'local' as const, id: t.url }))),
  ]);
  const selectedKey = () => { const p = pane(); return p?.kind === 'issue' || p?.kind === 'project' ? p.id : p?.kind === 'local' ? p.url : null; };
  function open(target: { kind: 'issue' | 'local' | 'project'; id: string }) {
    setPane(target.kind === 'local' ? { kind: 'local', url: target.id } : { kind: target.kind, id: target.id });
    const attr = target.kind === 'issue' ? 'issue-id' : target.kind === 'project' ? 'project-id' : 'task-url';
    queueMicrotask(() => document.querySelector<HTMLElement>(`.issue-row[data-${attr}="${CSS.escape(target.id)}"] .issue-row-main`)?.focus({ preventScroll: false }));
  }
  function openKey(idOrKey: string) {
    const row = (view()?.rows ?? []).find(r => r.id === idOrKey || r.key === idOrKey);
    if (row) open({ kind: 'issue', id: row.id });
    else flashFooter(`${idOrKey} isn’t in your mirror of the team yet. Sync, or open it in ${displayName()}.`, 4000);
  }

  // Another page asked for a task (a notification, Waiting on you, the
  // palette): by url or by file, local or tracked.
  createEffect(() => {
    const req = taskOpenRequest();
    if (!req) return;
    const items = tasksData()?.items;
    if (!items || view.loading) return;
    const task = req.path != null ? items.find(t => t.vaultPath === req.path) : items.find(t => t.url === req.url);
    const row = (view()?.rows ?? []).find(r => (req.path != null && r.path === req.path) || (req.path == null && r.url === req.url) || (task?.tracker && r.id === task.tracker.id));
    if (row) setPane({ kind: 'issue', id: row.id });
    else if (task) setPane({ kind: 'local', url: task.url });
    else setError(req.path != null ? `No task has the file ${req.path} any more.` : 'That task isn’t in your list any more.');
    setTaskOpenRequest(null);
  });

  async function syncNow() {
    if (syncing()) return;
    setSyncing(true);
    setError('');
    try {
      const outcome = await trackerSync();
      if (outcome.errors.length) setError(`Sync finished with errors: ${outcome.errors.join(' ')}`);
      else flashFooter(`Synced with ${displayName()}: ${outcome.pulled} pulled, ${outcome.pushed} sent${outcome.conflicts ? `, ${outcome.conflicts} conflicts` : ''}.`, 3000);
      refreshAll();
      void refetchTasks();
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setSyncing(false);
    }
  }

  async function toggleLocal(t: Task) {
    if (busyLocal().has(t.url)) return;
    setBusyLocal(new Set([...busyLocal(), t.url]));
    setError('');
    try {
      const value = isOpenTask(t.status) ? 'Done' : 'Not started';
      await applyDrop({ kind: 'task', url: t.url, dimension: 'status', value, originValue: t.status });
    } catch (e) {
      setError(taskErrorText(e));
    } finally {
      const next = new Set(busyLocal()); next.delete(t.url); setBusyLocal(next);
    }
  }

  const openSetup = () => { openSettingsFromOutside('sources', 'Task tracker'); };

  // Page-local keys (no modifiers, never while typing, a dialog or a menu is up).
  onMount(() => {
    const key = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || modalOpen() || contextMenu()) return;
      const target = e.target as HTMLElement | null;
      if (isTypingTarget(target)) {
        if (e.key === 'Escape' && target === search) { search!.blur(); e.preventDefault(); }
        return;
      }
      if (!target?.closest?.('.tracker-tasks') && target !== document.body) return;
      const inDetail = !!target?.closest?.('.issue-pane');
      const list = order();
      // From the focused row when focus is in the list, else the open one.
      const focused = target?.closest?.<HTMLElement>('.issue-row');
      const focusedId = focused?.dataset.issueId ?? focused?.dataset.projectId ?? focused?.dataset.taskUrl;
      const at = list.findIndex(r => r.id === (focusedId ?? selectedKey()));
      if (!inDetail && (e.key === 'j' || e.key === 'ArrowDown')) { e.preventDefault(); const i = step(at, 1, list.length); if (i >= 0) open(list[i]); }
      else if (!inDetail && (e.key === 'k' || e.key === 'ArrowUp')) { e.preventDefault(); const i = step(at, -1, list.length); if (i >= 0) open(list[i]); }
      else if (e.key === 'Escape' && fullPage()) { e.preventDefault(); backToList(); }
      else if (e.key === 'Escape' && pane()) { e.preventDefault(); setPane(null); }
      else if (!inDetail && e.key === 'c' && tracked()) { e.preventDefault(); setPane({ kind: 'newIssue' }); }
      else if (!inDetail && e.key === '/') { e.preventDefault(); search?.focus(); }
    };
    window.addEventListener('keydown', key);
    onCleanup(() => window.removeEventListener('keydown', key));
  });

  const issueCount = () => view()?.rows.filter(r => r.statusKind !== 'completed' && r.statusKind !== 'canceled').length ?? 0;

  return (
    <div class="work-page tasks-workspace tracker-tasks" classList={{ 'issue-page-open': fullPage() }}>
      <PageHeader title="Tasks" actions={<>
        <Show when={tracked()}>
          <span class="tracker-sync-line" title={status()?.lastError ?? undefined}>
            {status()?.lastError ? 'Last sync failed' : `Synced ${syncWhen(status()?.lastSync ?? view()?.lastSync, Date.now(), 'not yet')}`}
          </span>
          <Button variant="ghost" size="sm" icon="refresh" disabled={syncing() || !canSyncNow(status() ?? null)} onClick={() => void syncNow()}
            title={`Read ${displayName()} now and send waiting edits`}>{syncing() ? 'Syncing…' : 'Sync now'}</Button>
          <Button variant="primary" size="sm" icon="plus" onClick={() => setPane({ kind: 'newIssue' })} title="New issue (c)">New issue</Button>
        </Show>
        <Button variant={tracked() ? 'ghost' : 'primary'} size="sm" icon="plus" onClick={() => setPane({ kind: 'newLocal' })}>New local task</Button>
      </>} />
      <Show when={tracked()}>
        <div class="tracker-viewbar">
          <ViewTabs state={viewState()} conflicts={conflictCount(view()?.rows ?? [])} onChange={setViewState} />
        </div>
      </Show>
      <div class="tracker-toolbar">
        <label class="task-search"><Icon name="search" size={15} />
          <input ref={search} aria-label="Find a task" placeholder="Find a task…  /" value={query()} onInput={e => setQuery(e.currentTarget.value)} /></label>
        <Show when={tracked()}>
          <Show when={twoTeams()} fallback={
            <span class="tracker-team-chip" title={`Hollow reads and writes this team only`}>{meta()?.team.name ?? 'Team'} <span class="issue-muted">{view()?.teamKey}</span></span>
          }>
            <TeamFilter teamKeys={teamKeys()} value={chosenTeam()} onChange={setTeamFilter} />
          </Show>
          <span class="issue-muted tracker-count">{issueCount()} open</span>
          <SourceLabelChip label={tasksLabel()} class="tracker-source" />
          <Show when={!projectsShown()}><DisplayChoices state={viewState()} onChange={setViewState} /></Show>
        </Show>
      </div>
      <Show when={tracked() && !projectsShown()}>
        <FilterBar state={viewState()} meta={meta()} shown={shownFilters()} onChange={setViewState}
          onShow={f => setShownFilters([...shownFilters().filter(x => x !== f), f])}
          onHide={f => setShownFilters(shownFilters().filter(x => x !== f))} />
      </Show>
      <Show when={needsViewer()}>
        <div class="tracker-setup-note" role="note">
          <p>Hollow hasn’t read whom your {displayName()} key belongs to yet, so My issues can’t tell which issues are yours. Sync now reads it.</p>
          <Button size="sm" variant="secondary" disabled={syncing() || !canSyncNow(status() ?? null)} onClick={() => void syncNow()}>Sync now</Button>
        </div>
      </Show>
      <Show when={!view.loading && view()?.note}>
        {note => (
          <div class="tracker-setup-note" role="note">
            <p>{view()?.provider ? note() : 'Tasks are local. Set up a task tracker to see the team’s issues here.'}</p>
            <Button size="sm" variant="secondary" onClick={openSetup}>Open Task tracker settings</Button>
          </div>
        )}
      </Show>
      {/* Suggested tasks and other waiting items about tasks (Waiting on you). */}
      <div class="tracker-waiting"><WaitingLink context="tasks" hideWhenEmpty /></div>
      <Show when={tasksData.error}>
        <div class="task-error" role="alert"><ErrorText as="p" class="task-error-text" text={`Couldn’t read the tasks folder: ${errMsg(tasksData.error)}`} />
          <Button size="sm" variant="secondary" onClick={() => void refetchTasks()}>Retry</Button></div>
      </Show>
      <Show when={viewError()}>{msg => <ErrorText as="p" class="task-error" text={`Couldn’t read the tracker’s issues: ${msg()}`} />}</Show>
      <Show when={(view()?.total ?? 0) > (view()?.rows.length ?? 0)}>
        <p class="issue-muted tracker-truncated" role="note">Showing {view()!.rows.length} of {view()!.total} issues. Find a task searches these; open {displayName()} for the rest.</p>
      </Show>
      <Show when={error()}>{msg => <ErrorText as="p" class="task-error" text={msg()} />}</Show>
      <div class="task-desk tracker-desk" classList={{ "has-selection": !!pane(), 'issue-page': fullPage() }}>
        <section class="task-collection" aria-label="Task list" ref={collection}>
          <Show when={!view.loading || view()} fallback={<Skeleton label="Loading tasks" rows={8} />}>
            <Show when={!projectsShown()} fallback={
              <Show when={projects().length} fallback={<EmptyState class="task-empty" icon="folder" title={query().trim() ? 'No project matches' : 'No projects yet'}>
                {query().trim() ? <Button size="sm" variant="ghost" onClick={() => setQuery('')}>Clear the search</Button> : `The team has no projects in ${displayName()} yet.`}</EmptyState>}>
                <ProjectList projects={projects()} rows={view()?.rows ?? []} selected={pane()?.kind === 'project' ? selectedKey() : null} onOpen={id => open({ kind: 'project', id })} showTeam={twoTeams()} />
              </Show>
            }>
            <Show when={groups().length || local().length} fallback={
              <Show when={viewState().view !== 'conflicts' || query().trim() || hasFilters(viewState().filters)} fallback={
                <EmptyState class="task-empty" icon="checkSquare" title="No conflicts">
                  Every edit you made reached {displayName()}.{' '}
                  <Button size="sm" variant="ghost" onClick={() => setViewState(chooseView(viewState(), 'all'))}>Show all issues</Button>
                </EmptyState>
              }>
              <EmptyState class="task-empty" icon="checkSquare" title={query().trim() || hasFilters(viewState().filters) ? 'No task matches' : 'Nothing here'}>
                {query().trim() ? <Button size="sm" variant="ghost" onClick={() => setQuery('')}>Clear the search</Button>
                  : hasFilters(viewState().filters) ? <Button size="sm" variant="ghost" onClick={() => { setViewState({ ...viewState(), filters: emptyFilters() }); setShownFilters([]); }}>Clear filters</Button>
                  : tracked() ? 'No issue in this view. Another view or New issue.' : 'New local task adds one.'}
              </EmptyState>
              </Show>
            }>
              <IssueList groups={groups()} local={local()} showLocal={localShown()} selected={selectedKey()} collapsed={collapsedIds()} onToggleGroup={toggleGroup}
                onOpen={id => open({ kind: 'issue', id })} onOpenLocal={url => open({ kind: 'local', id: url })} onToggleLocal={t => void toggleLocal(t)}
                localBusy={url => busyLocal().has(url) ? 'Saving this task' : ''} showTeam={twoTeams()} />
            </Show>
            </Show>
          </Show>
        </section>
        <aside class="task-detail issue-pane" aria-label="Task details">
          <Show when={pane()} fallback={<EmptyState class="task-detail-empty" icon="checkSquare" title="No task selected">Select a task to see its details. j and k move, Enter opens.</EmptyState>}>
            {p => <>
              <Show when={p().kind === 'issue' && (p() as { id: string }).id} keyed>
                {id => <IssueDetail id={id} meta={meta()} displayName={displayName()} onOpenKey={openKey}
                  onClose={() => { if (fullPage()) backToList(); setPane(null); }} onChanged={() => { void refetchView(); void refetchTasks(); }}
                  onOpenProject={pid => setPane({ kind: 'project', id: pid })}
                  mode={fullPage() ? 'page' : 'pane'} onMode={setMode} rows={view()?.rows ?? []} />}
              </Show>
              <Show when={p().kind === 'project' && (p() as { id: string }).id} keyed>
                {id => <ProjectDetail trackerId={id} onClose={() => setPane(null)}
                  issues={pid => <ProjectIssues projectId={pid} rows={view()?.rows ?? []} states={view()?.states ?? []} viewer={viewer()}
                    onOpen={iid => setPane({ kind: 'issue', id: iid })} />} />}
              </Show>
              <Show when={p().kind === 'local' && (p() as { url: string }).url} keyed>
                {url => <TaskDetail url={url} task={() => (tasksData()?.items ?? []).find(t => t.url === url)}
                  onClose={() => setPane(null)} refresh={async () => { await refetchTasks(); }}
                  workspace={() => undefined} choices={() => localProjectChoices()} />}
              </Show>
              <Show when={p().kind === 'newLocal'}>
                <div class="task-detail-top"><span>New local task</span><button class="btn small ghost" aria-label="Close new task" onClick={() => setPane(null)}>Close <span aria-hidden="true">×</span></button></div>
                <h2 class="task-editor-heading" id="task-create-heading">What’s the next step?</h2>
                {/* The editor closes itself after onSaved; by then the pane shows the new task, so only a pane still on the form closes. */}
                <TaskEditor surface="panel" task={null} headingId="task-create-heading" onClose={() => { if (pane()?.kind === 'newLocal') setPane(null); }}
                  onSaved={async url => { await refetchTasks(); setPane({ kind: 'local', url }); }} />
              </Show>
              <Show when={p().kind === 'newIssue'}>
                <NewIssue meta={meta()} displayName={displayName()} teamKeys={teamKeys()} defaultTeam={chosenTeam()} onClose={() => setPane(null)}
                  onCreated={id => { void refetchView(); void refetchTasks(); if (id) setPane({ kind: 'issue', id }); else setPane(null); }} />
              </Show>
            </>}
          </Show>
        </aside>
      </div>
    </div>
  );
}

/** A local task under the view's label and due filters. */
function localFiltered(t: Task, s: ViewState): boolean {
  const labels = s.filters.label;
  if (labels.length && !(t.labels.some(l => labels.includes(l)) || (labels.includes('') && !t.labels.length))) return false;
  return !s.filters.due.length || s.filters.due.some(d => dueMatches(t.due, d as DueFilter));
}

function localMatches(t: Task, query: string): boolean {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = [t.title, t.projectName ?? '', ...t.labels, t.status].join(' ').toLowerCase();
  return words.every(w => hay.includes(w));
}

/** The local editor's project choices: the projects local tasks already use. */
const localProjectChoices = () => [...new Map((tasksData()?.items ?? []).filter(t => t.projectName)
  .map(t => [t.projectUrl ?? `name:${t.projectName}`, t.projectName!] as const)).entries()]
  .sort((a, b) => a[1].localeCompare(b[1])).map(([url, name]) => ({ name, url: url.startsWith('name:') ? undefined : url }));

/** "Create in Linear": a new team issue with its main fields. */
function NewIssue(props: { meta: TrackerTeamMeta | undefined; displayName: string; teamKeys: readonly string[]; defaultTeam: string | null; onClose: () => void; onCreated: (id: string | null) => void }) {
  const [title, setTitle] = createSignal('');
  const [description, setDescription] = createSignal('');
  const [stateId, setStateId] = createSignal('');
  const [priority, setPriority] = createSignal('0');
  const [assignee, setAssignee] = createSignal('');
  const [project, setProject] = createSignal('');
  const [due, setDue] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  // Which team the issue goes to (AI unless the person picks the other); asked only with two teams.
  const [team, setTeam] = createSignal(props.defaultTeam ?? 'ENG');
  // The status, assignee and project lists belong to the shared team; another team uses its own defaults.
  const sharedTeam = () => team() === 'ENG';
  const states = () => (props.meta?.states ?? []).filter(s => s.kind !== 'completed' && s.kind !== 'canceled').sort((a, b) => a.position - b.position);
  async function create() {
    if (busy() || !title().trim()) return;
    setBusy(true);
    setError('');
    try {
      const outcome = await createTrackerTask({
        title: title().trim(), description: description().trim() || null, stateId: sharedTeam() ? stateId() || null : null, priority: Number(priority()),
        assigneeId: sharedTeam() ? assignee() || null : null, projectId: sharedTeam() ? project() || null : null, due: due() || null,
        teamKey: createTeamKey(team(), props.teamKeys),
      });
      flashFooter(outcome.queued ? `Saved; it is created in ${props.displayName} when Hollow can reach it.` : `${outcome.key ?? 'Issue'} created in ${props.displayName}.`, 3500);
      props.onCreated(outcome.id);
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <form class="issue-new" aria-labelledby="issue-new-heading" onSubmit={e => { e.preventDefault(); void create(); }}>
      <div class="task-detail-top"><span>New issue in {hasTwoTeams(props.teamKeys) ? teamName(team()) : props.meta?.team.name ?? 'the team'}</span><button type="button" class="btn small ghost" aria-label="Close new issue" onClick={props.onClose}>Close <span aria-hidden="true">×</span></button></div>
      <h2 id="issue-new-heading" class="task-editor-heading">New issue</h2>
      <Show when={hasTwoTeams(props.teamKeys)}><TeamPicker teamKeys={props.teamKeys} value={team()} onChange={setTeam} /></Show>
      <input class="input issue-title-input" aria-label="Issue title" placeholder="Issue title" value={title()} onInput={e => setTitle(e.currentTarget.value)} autofocus />
      <textarea class="input issue-description-input" aria-label="Description" placeholder="Add a description (Markdown)" rows={5} value={description()} onInput={e => setDescription(e.currentTarget.value)} />
      <div class="issue-new-props">
        <Show when={sharedTeam()}><Select size="sm" label="Status" value={stateId()} placeholder="Status" onChange={setStateId} options={[{ value: '', label: 'Team default' }, ...states().map(s => ({ value: s.id, label: s.name }))]} /></Show>
        <Select size="sm" label="Priority" value={priority()} onChange={setPriority} searchable={false} options={PRIORITY_ORDER.map(p => ({ value: String(p), label: priorityLabel(p) }))} />
        <Show when={sharedTeam()}>
          <Select size="sm" label="Assignee" value={assignee()} onChange={setAssignee} options={[{ value: '', label: 'Unassigned' }, ...(props.meta?.members ?? []).map(m => ({ value: m.id, label: m.name }))]} />
          <Select size="sm" label="Project" value={project()} onChange={setProject} options={[{ value: '', label: 'No project' }, ...(props.meta?.projects ?? []).map(p => ({ value: p.id, label: p.name }))]} />
        </Show>
        <input class="input issue-prop-input" type="date" aria-label="Due date" value={due()} onInput={e => setDue(e.currentTarget.value)} />
      </div>
      <div class="issue-inline-actions">
        <Button type="submit" size="sm" variant="primary" disabled={busy() || !title().trim()}>{busy() ? 'Creating…' : `Create in ${props.displayName}`}</Button>
        <Button type="button" size="sm" variant="ghost" onClick={props.onClose}>Cancel</Button>
      </div>
      <Show when={error()}>{msg => <ErrorText as="p" class="issue-detail-error" text={msg()} />}</Show>
    </form>
  );
}

/** A project's issues inside its detail (B4): the same list, filtered to the
 *  project and grouped by status; done and canceled start folded. An issue
 *  opens in the pane (its Open project comes back). */
function ProjectIssues(props: { projectId: string; rows: readonly TrackerViewRow[]; states: readonly TrackerState[]; viewer: string | null; onOpen: (id: string) => void }) {
  const [opened, setOpened] = createSignal<ReadonlySet<string>>(new Set());
  const [closed, setClosed] = createSignal<ReadonlySet<string>>(new Set());
  const groups = createMemo(() => groupRows(props.rows.filter(r => r.projectId === props.projectId), { grouping: 'status', ordering: 'priority' }, props.states, props.viewer));
  const collapsed = createMemo(() => new Set(groups().filter(g => closed().has(g.id) || (startsFolded(g.kind) && !opened().has(g.id))).map(g => g.id)));
  const toggle = (id: string) => {
    const shut = collapsed().has(id);
    const o = new Set(opened()), c = new Set(closed());
    if (shut) { o.add(id); c.delete(id); } else { c.add(id); o.delete(id); }
    setOpened(o); setClosed(c);
  };
  return (
    <Show when={groups().length} fallback={<p class="issue-muted">No issues in this project yet.</p>}>
      <IssueList groups={groups()} local={[]} showLocal={false} selected={null} collapsed={collapsed()} onToggleGroup={toggle}
        onOpen={props.onOpen} onOpenLocal={() => {}} onToggleLocal={() => {}} localBusy={() => ''} />
    </Show>
  );
}
