// One tracked issue (overhaul B1, design section 12): the title and its
// Markdown description, sub-issues, relations, links and attachments, the
// activity feed (created, comments, updated) and the agent runs on it, beside
// a properties column with status, priority, assignee, labels, project,
// milestone, cycle, estimate and due date. Writable fields edit in place
// through `edit_tracker_item` (compare-and-swap against the values read;
// the tracker wins a conflict, and the conflicts panel says what was lost);
// read-only fields say so on hover. "Open in Linear (ENG-37)" opens the
// issue in the browser; Archive asks first. It shows beside the list (pane)
// or as the whole Tasks page (full page: two columns, properties on the
// right like a tracker's issue page). Sub-issues and relations read as rows
// from the loaded list; the activity merges the tracker's change history
// (`tracker_issue_history`, read when the issue opens) with the comments.
import { For, Match, Show, Switch, createEffect, createResource, createSignal, on, onCleanup, type JSX } from 'solid-js';
import { MarkdownView } from '../../components/MarkdownView';
import { MultiSelect, Select } from '../../components/Select';
import { Button, ErrorText } from '../../components/ui';
import { flashFooter, showConfirm, openInBrowser, tasksData, trackerSyncPass } from '../../store';
import { workspaces } from '../../store/workspaces';
import { errMsg, openExternal } from '../../utils';
import { SourceLabelChip } from '../../components/SourceLabelChip';
import { archiveTrackerTask, commentTrackerIssue, editTrackerItem, trackerIssue, trackerIssueHistory } from '../../trackerApi';
import type { TrackerField, TrackerHistory, TrackerIssue, TrackerIssueDetail, TrackerTeamMeta, TrackerViewRow } from '../../generated/ipc';
import {
  PRIORITY_ORDER, RELATION_LABELS, agentRunsOf, dueText, fieldEdit, fieldLock, priorityLabel, stamp, stateOptions,
} from '../../trackerTasks';
import { Avatar, PriorityIcon, StatusIcon } from './issueIcons';
import { activityFeed, ago } from './issueActivity';
import { openWithTask, workOnBlocked } from './TaskDetail';

export const openLink = (url: string) => void openInBrowser(url).catch(() => openExternal(url));

export function IssueDetail(props: {
  id: string;
  meta: TrackerTeamMeta | undefined;
  displayName: string;
  /** Opens another issue of the team by its key (sub-issues, relations). */
  onOpenKey: (key: string) => void;
  onClose: () => void;
  /** The list reads the mirror again after an edit. */
  onChanged: () => void;
  /** Opens the issue's project (B3's project detail). */
  onOpenProject?: (projectId: string) => void;
  /** Beside the list (pane, the default) or as the whole page. */
  mode?: 'pane' | 'page';
  /** "Open full page" and "Back to list"; without it neither shows. */
  onMode?: (mode: 'pane' | 'page') => void;
  /** The list's rows, so sub-issues and relations show status, title and assignee. */
  rows?: readonly TrackerViewRow[];
}) {
  const page = () => props.mode === 'page';
  const [detail, { refetch, mutate }] = createResource(() => props.id, id => trackerIssue(id));
  const [busy, setBusy] = createSignal<TrackerField | 'archive' | null>(null);
  const [error, setError] = createSignal('');
  const [note, setNote] = createSignal('');
  const [editingDescription, setEditingDescription] = createSignal(false);
  const [descriptionDraft, setDescriptionDraft] = createSignal('');
  // The issue as it was when the description draft began: its save compares
  // against that, so a change made meanwhile is refused, never overwritten.
  let descriptionBase: TrackerIssue | null = null;
  // A pass (the timer, Sync now, an agent) can send a pending edit, add a
  // conflict or a comment: read the open issue again.
  createEffect(on(trackerSyncPass, pass => { if (pass && !busy()) void refetch(); }, { defer: true }));
  // The tracker's change history, read when the issue opens (a signal, not a
  // resource: a resource under the reply suspends the transcript). null
  // until it arrives or when it can't be read: the synthetic feed shows.
  const [history, setHistory] = createSignal<TrackerHistory | null>(null);
  let historyRead = 0;
  function loadHistory() {
    const id = props.id, read = ++historyRead;
    trackerIssueHistory(id).then(h => { if (read === historyRead && id === props.id) setHistory(h); }, () => { if (read === historyRead) setHistory(null); });
  }
  createEffect(on(() => props.id, () => { setHistory(null); loadHistory(); }));
  // Read the history again only when the open issue itself changed (its
  // updatedAt moved), not on every sync pass: one Linear read per change.
  let seenUpdate = '';
  createEffect(on(() => { const d = ready(); return d ? `${props.id}\n${d.issue.updatedAt}` : ''; }, key => {
    if (!key) return;
    if (seenUpdate && seenUpdate !== key && seenUpdate.split('\n')[0] === key.split('\n')[0]) loadHistory();
    seenUpdate = key;
  }));
  onCleanup(() => { historyRead++; });
  const ready = () => (detail.state === 'ready' || detail.state === 'refreshing' ? detail() : undefined);

  /** One field's edit, compared against `base` (the issue as read). True
   *  when the tracker took it (or queued it). */
  async function save(field: TrackerField, value: unknown, base?: TrackerIssue): Promise<boolean> {
    const d = ready();
    if (!d || busy()) return false;
    setBusy(field);
    setError('');
    setNote('');
    try {
      const outcome = await editTrackerItem(fieldEdit(base ?? d.issue, field, value));
      setNote(outcome.queued ? `Saved here. It sends to ${props.displayName} when Hollow can reach it.` : outcome.note);
      mutate(await trackerIssue(props.id));
      props.onChanged();
      return true;
    } catch (e) {
      setError(errMsg(e));
      void refetch();
      return false;
    } finally {
      setBusy(null);
    }
  }

  async function archive() {
    const d = ready();
    if (!d) return;
    if (!await showConfirm({
      title: `Archive ${d.issue.key} in ${props.displayName}?`,
      body: `${d.issue.title}\n\nIt leaves the team's active lists in ${props.displayName}. Hollow keeps its file with its last values.`,
      confirmLabel: 'Archive',
      danger: true,
    })) return;
    setBusy('archive');
    setError('');
    try {
      const outcome = await archiveTrackerTask(d.issue.id);
      setNote(outcome.note);
      props.onChanged();
      void refetch();
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(null);
    }
  }

  const lock = (field: TrackerField) => { const d = ready(); return d ? fieldLock(d, field) : 'Loading'; };
  const meta = () => props.meta;
  const projectName = (id: string | null) => meta()?.projects.find(p => p.id === id)?.name ?? ready()?.project?.name ?? null;
  const localTask = () => { const path = ready()?.path; return path ? (tasksData()?.items ?? []).find(t => t.vaultPath === path) : undefined; };
  const workspace = () => { const d = ready(); const id = d?.workspaceId ?? d?.project?.workspaceId; return id ? workspaces().find(w => w.id === id) : undefined; };

  return (
    <div class="issue-detail" data-mode={page() ? 'page' : 'pane'} aria-busy={detail.loading ? 'true' : undefined}>
      <Show when={detail.error}>
        <ErrorText as="p" class="issue-detail-error" text={`Couldn’t read this issue: ${errMsg(detail.error)}`} />
        <Button size="sm" variant="ghost" onClick={props.onClose}>Close</Button>
      </Show>
      <Show when={ready()}>{d => (
        <>
          <header class="issue-detail-top">
            <Show when={page() && props.onMode}>
              <Button size="sm" variant="ghost" icon="arrowLeft" class="issue-back" title="Back to the list (Escape)" onClick={() => props.onMode!('pane')}>Back to list</Button>
            </Show>
            <span class="issue-detail-crumb">{meta()?.team.name ?? 'Team'} <span aria-hidden="true">›</span> <strong>{d().issue.key}</strong></span>
            {/* H4b: where the issue lives, in Rust's words ("Tracked in Linear", "Linear unreachable since ..."). */}
            <SourceLabelChip label={props.rows?.find(r => r.id === props.id)?.sourceLabel} class="issue-detail-source" />
            <div class="issue-detail-top-actions">
              <Show when={!page() && props.onMode}>
                <Button size="sm" variant="ghost" icon="appWindow" title="Show this issue as the whole page" onClick={() => props.onMode!('page')}>Open full page</Button>
              </Show>
              <Show when={!page()}>
                <Button size="sm" variant="ghost" icon="externalLink" onClick={() => openLink(d().issue.url)}>Open in {props.displayName} ({d().issue.key})</Button>
              </Show>
              <Button size="sm" variant="ghost" aria-label="Close issue details" onClick={props.onClose}>Close <span aria-hidden="true">×</span></Button>
            </div>
          </header>
          <div class="issue-detail-body">
            <article class="issue-detail-main" aria-labelledby="issue-detail-title">
              <TitleField issue={d().issue} lock={lock('title')} busy={!!busy()} onSave={(v, base) => void save('title', v, base)} />
              <Show when={d().issue.archivedAt}>
                <p class="issue-detail-banner" role="note">Archived in {props.displayName} on {stamp(d().issue.archivedAt)}. Its last values stay here.</p>
              </Show>
              <Show when={d().conflicts.length}>
                <section class="issue-conflicts" aria-label="Conflicts">
                  <h3>{props.displayName} changed fields you edited</h3>
                  <For each={d().conflicts}>{c => (
                    <div class="issue-conflict">
                      <p>Your edit to <strong>{c.field}</strong> was not applied: {props.displayName} has “{conflictText(c.remote)}”, you had “{conflictText(c.local)}” ({stamp(c.at)}).</p>
                      <div class="issue-conflict-actions">
                        <Button size="sm" variant="secondary" disabled={!!busy()} onClick={() => void save(c.field, conflictValue(c.local))}>Use mine</Button>
                        <Button size="sm" variant="ghost" disabled={!!busy()} onClick={() => void save(c.field, conflictValue(c.remote))}>Keep {props.displayName}’s</Button>
                      </div>
                    </div>
                  )}</For>
                </section>
              </Show>
              <section class="issue-description" aria-label="Description">
                <Show when={editingDescription()} fallback={
                  <>
                    <Show when={d().issue.description.trim()} fallback={<p class="issue-muted">No description.</p>}>
                      <MarkdownView class="issue-markdown" text={d().issue.description} forbidMedia />
                    </Show>
                    <Button size="sm" variant="ghost" icon="edit" disabled={!!lock('description')} title={lock('description') ?? undefined}
                      onClick={() => { descriptionBase = d().issue; setDescriptionDraft(d().issue.description); setEditingDescription(true); }}>Edit description</Button>
                  </>
                }>
                  <textarea class="input issue-description-input" aria-label="Description" rows={8} value={descriptionDraft()} onInput={e => setDescriptionDraft(e.currentTarget.value)} />
                  <div class="issue-inline-actions">
                    <Button size="sm" variant="primary" disabled={!!busy()} onClick={() => void save('description', descriptionDraft(), descriptionBase ?? undefined).then(ok => { if (ok) { setEditingDescription(false); descriptionBase = null; } })}>Save</Button>
                    <Button size="sm" variant="ghost" onClick={() => setEditingDescription(false)}>Cancel</Button>
                  </div>
                </Show>
              </section>
              <Show when={d().vaultNotes?.trim()}>
                {notes => <section class="issue-vault-notes" aria-label="Notes in your vault"><h3>Notes in your vault</h3><MarkdownView class="issue-markdown" text={notes()} forbidMedia /></section>}
              </Show>
              <Show when={d().issue.parentId || d().issue.children.length}>
                <section class="issue-section" aria-label="Sub-issues">
                  <Show when={d().issue.parentId}>{pid => <div class="issue-parent"><span class="issue-muted">Sub-issue of</span> <IssueRef idOrKey={pid()} rows={props.rows} onOpen={props.onOpenKey} /></div>}</Show>
                  <Show when={d().issue.children.length}>
                    <h3>Sub-issues <span class="issue-count">{d().issue.children.length}</span></h3>
                    <ul class="issue-plain-list issue-ref-list"><For each={d().issue.children}>{key => <li><IssueRef idOrKey={key} rows={props.rows} onOpen={props.onOpenKey} /></li>}</For></ul>
                  </Show>
                </section>
              </Show>
              <Show when={d().issue.relations.length}>
                <section class="issue-section" aria-label="Relations">
                  <h3>Relations</h3>
                  <ul class="issue-plain-list issue-ref-list"><For each={d().issue.relations}>{r => <li class="issue-relation"><span class="issue-muted issue-relation-kind">{RELATION_LABELS[r.kind]}</span> <IssueRef idOrKey={r.key} rows={props.rows} onOpen={props.onOpenKey} /></li>}</For></ul>
                </section>
              </Show>
              <Show when={d().issue.links.length || d().issue.attachments.length}>
                <section class="issue-section" aria-label="Links and attachments">
                  <h3>Links and attachments</h3>
                  <ul class="issue-plain-list issue-links">
                    <For each={[...d().issue.attachments, ...d().issue.links]}>{l => (
                      <li><button class="btn-link" onClick={() => openLink(l.url)}>{l.title || l.url}</button>
                        <Show when={l.source}>{s => <span class="issue-muted"> · {s()}</span>}</Show></li>
                    )}</For>
                  </ul>
                </section>
              </Show>
              <section class="issue-activity" aria-label="Activity">
                <h3>Activity</h3>
                <ol class="issue-feed">
                  <For each={activityFeed(d().issue, history()?.events ?? null, history()?.more)}>{item => (
                    <Switch>
                      <Match when={item.kind === 'created'}>
                        <li class="issue-feed-event"><Avatar name={d().issue.creator?.name} size={16} /><span><strong>{d().issue.creator?.name ?? 'Someone'}</strong> created the issue</span><time>{stamp(d().issue.createdAt)}</time></li>
                      </Match>
                      <Match when={item.kind === 'comment' && item.comment}>{c => (
                        <li class="issue-feed-comment">
                          <div class="issue-comment-head"><Avatar name={c().author} size={16} /><strong>{c().author ?? 'Someone'}</strong><time>{stamp(c().createdAt)}</time><Show when={c().editedAt}><span class="issue-muted">(edited)</span></Show></div>
                          <MarkdownView class="issue-markdown issue-comment-body" text={c().body} forbidMedia />
                        </li>
                      )}</Match>
                      <Match when={item.kind === 'change' && item}>{change => (
                        <li class="issue-feed-event issue-feed-change" data-kind={change().event.kind}>
                          <Show when={change().event.kind === 'state' && change().event.toKind} fallback={<Avatar name={change().event.actor} size={16} />}>
                            {kind => <StatusIcon kind={kind()} label="" />}
                          </Show>
                          <span><strong>{change().event.actor ?? 'Someone'}</strong> {change().text}</span>
                          <time datetime={change().at} title={stamp(change().at)}>{ago(change().at)}</time>
                        </li>
                      )}</Match>
                      <Match when={item.kind === 'gap'}>
                        <li class="issue-feed-event"><span class="issue-feed-dot" aria-hidden="true" /><span class="issue-muted">Earlier changes aren't shown</span></li>
                      </Match>
                      <Match when={item.kind === 'updated'}>
                        <li class="issue-feed-event"><span class="issue-feed-dot" aria-hidden="true" /><span>Last updated</span><time>{stamp(d().issue.updatedAt)}</time></li>
                      </Match>
                    </Switch>
                  )}</For>
                </ol>
                <CommentBox issueId={d().issue.id} displayName={props.displayName} disabled={!!d().issue.archivedAt || d().lock?.kind === 'outOfScope' || d().lock?.kind === 'remoteMissing'}
                  onPosted={() => { void refetch(); props.onChanged(); }} />
              </section>
              <AgentActivity detail={d()} />
            </article>
            <aside class="issue-properties" aria-label="Properties">
              <Property label="Status" lock={lock('status')}>
                <Select size="sm" quiet label="Status" value={d().issue.state.id} disabled={!!lock('status') || !!busy()} options={stateOptions(meta(), d().issue.state)}
                  trigger={() => <span class="issue-prop-value"><StatusIcon kind={d().issue.state.kind} label="" />{d().issue.state.name}</span>}
                  onChange={v => void save('status', v)} />
              </Property>
              <Property label="Priority" lock={lock('priority')}>
                <Select size="sm" quiet label="Priority" value={String(d().issue.priority)} disabled={!!lock('priority') || !!busy()} searchable={false}
                  options={PRIORITY_ORDER.map(p => ({ value: String(p), label: priorityLabel(p) }))}
                  trigger={() => <span class="issue-prop-value"><PriorityIcon priority={d().issue.priority} />{priorityLabel(d().issue.priority)}</span>}
                  onChange={v => void save('priority', Number(v))} />
              </Property>
              <Property label="Assignee" lock={lock('assignee')}>
                <Select size="sm" quiet label="Assignee" value={d().issue.assignee?.id ?? ''} disabled={!!lock('assignee') || !!busy()}
                  options={[{ value: '', label: 'Unassigned' }, ...(meta()?.members ?? (d().issue.assignee ? [d().issue.assignee!] : [])).map(m => ({ value: m.id, label: m.name }))]}
                  trigger={() => <span class="issue-prop-value"><Avatar name={d().issue.assignee?.name} />{d().issue.assignee?.name ?? 'Unassigned'}</span>}
                  onChange={v => void save('assignee', v)} />
              </Property>
              <Property label="Labels" lock={lock('labels')}>
                <MultiSelect size="sm" quiet label="Labels" value={d().issue.labels} disabled={!!lock('labels') || !!busy()} placeholder="Add labels"
                  options={(meta()?.labels ?? d().issue.labels.map(name => ({ name }))).map(l => ({ value: l.name, label: l.name }))}
                  onChange={v => void save('labels', v)} />
              </Property>
              <Property label="Project" lock={lock('project')}>
                <Select size="sm" quiet label="Project" value={d().issue.projectId ?? ''} disabled={!!lock('project') || !!busy()}
                  options={[{ value: '', label: 'No project' }, ...(meta()?.projects ?? []).map(p => ({ value: p.id, label: p.name }))]}
                  trigger={() => <span class="issue-prop-value">{projectName(d().issue.projectId) ?? 'No project'}</span>}
                  onChange={v => void save('project', v)} />
                <Show when={props.onOpenProject && d().issue.projectId}>{pid => (
                  <button type="button" class="btn-link issue-open-project" onClick={() => props.onOpenProject!(pid())}>Open project</button>
                )}</Show>
              </Property>
              <Property label="Milestone" lock={lock('milestone')}>
                <Select size="sm" quiet label="Milestone" value={d().issue.milestone?.id ?? ''} disabled={!!lock('milestone') || !!busy()}
                  options={[{ value: '', label: 'No milestone' }, ...(meta()?.milestones ?? []).filter(m => !m.projectId || m.projectId === d().issue.projectId).map(m => ({ value: m.id, label: m.name }))]}
                  trigger={() => <span class="issue-prop-value">{d().issue.milestone?.name ?? 'No milestone'}</span>}
                  onChange={v => void save('milestone', v)} />
              </Property>
              <Property label="Cycle" lock={lock('cycle')}>
                <Select size="sm" quiet label="Cycle" value={d().issue.cycle?.id ?? ''} disabled={!!lock('cycle') || !!busy()}
                  options={[{ value: '', label: 'No cycle' }, ...(meta()?.cycles ?? []).map(c => ({ value: c.id, label: c.name ?? `Cycle ${c.number}` }))]}
                  trigger={() => <span class="issue-prop-value">{d().issue.cycle ? (d().issue.cycle!.name ?? `Cycle ${d().issue.cycle!.number}`) : 'No cycle'}</span>}
                  onChange={v => void save('cycle', v)} />
              </Property>
              <Property label="Estimate" lock={lock('estimate')}>
                <input class="input issue-prop-input" type="number" min="0" step="1" aria-label="Estimate" disabled={!!lock('estimate') || !!busy()}
                  value={d().issue.estimate ?? ''} placeholder="None"
                  onChange={e => { const raw = e.currentTarget.value.trim(); void save('estimate', raw === '' ? null : Number(raw)); }} />
              </Property>
              <Property label="Due date" lock={lock('due')}>
                <input class="input issue-prop-input" type="date" aria-label="Due date" disabled={!!lock('due') || !!busy()} value={d().issue.due ?? ''}
                  data-overdue={dueText(d().issue.due)?.overdue && d().issue.state.kind !== 'completed' ? '' : undefined}
                  onChange={e => void save('due', e.currentTarget.value)} />
              </Property>
              <div class="issue-properties-rule" />
              <Property label="Created by" lock="Set by the tracker"><span class="issue-prop-value"><Avatar name={d().issue.creator?.name} />{d().issue.creator?.name ?? 'Unknown'}</span></Property>
              <Property label="Created" lock="Set by the tracker"><span class="issue-prop-value">{stamp(d().issue.createdAt)}</span></Property>
              <Property label="Updated" lock="Set by the tracker"><span class="issue-prop-value">{stamp(d().issue.updatedAt)}</span></Property>
              <Show when={page()}>
                <Button size="sm" variant="ghost" icon="externalLink" onClick={() => openLink(d().issue.url)}>Open in {props.displayName} ({d().issue.key})</Button>
              </Show>
              <div class="issue-properties-rule" />
              <Show when={localTask() && workspace()}>
                <Button size="sm" variant="secondary" disabled={!!workOnBlocked()} title={workOnBlocked() || `Opens ${workspace()!.title} with this issue in the composer. Nothing is sent until you send it.`}
                  onClick={() => void openWithTask(localTask()!, workspace()!.id)}>Work on this in {workspace()!.title}</Button>
              </Show>
              <Button size="sm" variant="ghost" icon="archive" disabled={!!busy() || !!d().issue.archivedAt} onClick={() => void archive()}>Archive in {props.displayName}</Button>
            </aside>
          </div>
          <div class="issue-detail-status" aria-live="polite">
            <Show when={note()}><p class="issue-muted">{note()}</p></Show>
            <Show when={error()}>{msg => <ErrorText as="p" class="issue-detail-error" text={msg()} />}</Show>
          </div>
        </>
      )}</Show>
    </div>
  );
}

/** A conflict's side as its value: the ledger keeps each as JSON (the
 *  mirror's own value, a name or a list), which the edit accepts as is. */
function conflictValue(json: string): unknown {
  try { return JSON.parse(json); } catch { return json; }
}

/** A conflict's side in words. */
function conflictText(json: string): string {
  const v = conflictValue(json);
  if (v === null || v === '' || (Array.isArray(v) && !v.length)) return 'empty';
  return Array.isArray(v) ? v.join(', ') : String(v);
}

function Property(props: { label: string; lock: string | null; children: JSX.Element }) {
  return (
    <div class="issue-property" title={props.lock ?? undefined} data-locked={props.lock ? '' : undefined}>
      <span class="issue-property-label">{props.label}</span>
      <div class="issue-property-value">{props.children}</div>
    </div>
  );
}

/** The title edits in place. While it has focus a refresh never replaces
 *  what is typed; the save compares against the issue as it was at focus. */
function TitleField(props: { issue: TrackerIssue; lock: string | null; busy: boolean; onSave: (title: string, base: TrackerIssue) => void }) {
  const [draft, setDraft] = createSignal(props.issue.title);
  let editing = false;
  let cancelled = false;
  let base: TrackerIssue = props.issue;
  createEffect(on(() => props.issue.title, title => { if (!editing) setDraft(title); }));
  const commit = () => {
    editing = false;
    if (cancelled) { cancelled = false; setDraft(props.issue.title); return; }
    const v = draft().trim();
    if (v && v !== base.title) props.onSave(v, base);
    else setDraft(props.issue.title);
  };
  return (
    <h2 class="issue-detail-title" id="issue-detail-title">
      <input class="issue-title-input" aria-label="Title" value={draft()} disabled={!!props.lock || props.busy} title={props.lock ?? undefined}
        onFocus={() => { editing = true; base = props.issue; }} onInput={e => setDraft(e.currentTarget.value)} onBlur={commit}
        onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); e.currentTarget.blur(); } if (e.key === 'Escape') { cancelled = true; e.currentTarget.blur(); } }} />
    </h2>
  );
}

function IssueKeyLink(props: { idOrKey: string; onOpen: (key: string) => void }) {
  return <button class="btn-link issue-key-link" onClick={() => props.onOpen(props.idOrKey)}>{props.idOrKey}</button>;
}

/** A sub-issue, parent or related issue as a row: status, key, title and
 *  assignee from the loaded list. One the list doesn't hold shows its key. */
function IssueRef(props: { idOrKey: string; rows: readonly TrackerViewRow[] | undefined; onOpen: (key: string) => void }) {
  const row = () => props.rows?.find(r => r.id === props.idOrKey || r.key === props.idOrKey);
  return (
    <Show when={row()} fallback={<IssueKeyLink idOrKey={props.idOrKey} onOpen={props.onOpen} />}>{r => (
      <button type="button" class="issue-ref" data-issue-id={r().id} onClick={() => props.onOpen(r().id)}
        aria-label={`${r().key} ${r().title} (${r().statusName}, ${r().assignee ?? 'Unassigned'})`}>
        <StatusIcon kind={r().statusKind} label="" />
        <span class="issue-ref-key">{r().key}</span>
        <span class="issue-ref-title">{r().title}</span>
        <span class="issue-ref-assignee" title={r().assignee ?? 'Unassigned'}><Avatar name={r().assignee} size={16} /></span>
      </button>
    )}</Show>
  );
}

/** Who worked on this issue: the COO, project leads and their subagents,
 *  with the model, when, the outcome and a link to the run's session. Lane
 *  4 of the COO rebuild fills it (`TrackerIssueDetail.agentRuns`). */
function AgentActivity(props: { detail: TrackerIssueDetail }) {
  const runs = () => agentRunsOf(props.detail);
  return (
    <section class="issue-agent-activity" aria-label="Agent activity">
      <h3>Agent activity</h3>
      <Show when={runs().length} fallback={<p class="issue-muted">No agent has worked on this issue yet.</p>}>
        <ol class="issue-feed">
          <For each={runs()}>{run => (
            <li class="issue-feed-event issue-agent-run" data-outcome={run.outcome}>
              <span class="issue-feed-dot" aria-hidden="true" />
              <span><strong>{run.who}</strong><Show when={run.model}>{m => <span class="issue-muted"> · {m()}</span>}</Show> · {run.outcome}
                <Show when={run.summary}>{s => <span class="issue-agent-summary">{s()}</span>}</Show></span>
              <time>{stamp(run.startedAt)}</time>
            </li>
          )}</For>
        </ol>
      </Show>
    </section>
  );
}

// Comment drafts by issue, shared by every open box (a reactive map), so
// moving to another issue and back keeps them and a late answer clears what
// it posted everywhere. A draft's request key stays with that exact text: a
// retry of the same text never posts twice, and changed text after an
// attempt is a new comment with a new key. `sent` is the text last
// submitted with the key.
type CommentDraft = { text: string; key: string; sent?: string };
const [commentDrafts, setCommentDrafts] = createSignal<Readonly<Record<string, CommentDraft>>>({});
function putDraft(issue: string, draft: CommentDraft | null) {
  const next = { ...commentDrafts() };
  if (draft) next[issue] = draft; else delete next[issue];
  setCommentDrafts(next);
}
const newCommentKey = () => `comment-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
// Issues with a comment on its way, across remounts of the box.
const [postingTo, setPostingTo] = createSignal<ReadonlySet<string>>(new Set());

/** Write a comment (overhaul B5): it posts to the tracker through the
 *  adapter, team AI only, and joins the thread above once it is confirmed. */
function CommentBox(props: { issueId: string; displayName: string; disabled: boolean; onPosted: () => void }) {
  const draft = () => commentDrafts()[props.issueId];
  const text = () => draft()?.text ?? '';
  const [error, setError] = createSignal('');
  const [note, setNote] = createSignal('');
  const busy = () => postingTo().has(props.issueId);
  function remember(value: string) {
    const was = draft();
    const keep = !!was && (was.sent === undefined || was.sent === value.trim());
    if (!keep) setNote('');
    if (value.trim() || keep) putDraft(props.issueId, { text: value, key: keep ? was!.key : newCommentKey(), sent: keep ? was!.sent : undefined });
    else putDraft(props.issueId, null);
  }
  // The tracker's limit is 2,000 bytes; an accented letter or emoji takes more than one.
  const bytes = () => new TextEncoder().encode(text().trim()).length;
  const tooLong = () => bytes() > 2000;
  async function post() {
    const body = text().trim();
    if (!body || busy() || tooLong() || props.disabled) return;
    const entry: CommentDraft = { ...(draft() ?? { key: newCommentKey() }), text: text(), sent: body };
    putDraft(props.issueId, entry);
    const issue = props.issueId;
    setPostingTo(new Set([...postingTo(), issue])); setError(''); setNote('');
    try {
      const outcome = await commentTrackerIssue(issue, body, entry.key);
      if (outcome.id) {
        // Clear only the draft this sent, in every open box; a newer one
        // (edited after, so with a new key) stays.
        if (commentDrafts()[issue]?.key === entry.key) putDraft(issue, null);
        flashFooter(`Comment posted in ${props.displayName}.`, 2500);
        props.onPosted();
      } else {
        // Unconfirmed: it may have posted; this text with this key is never sent again.
        setNote(outcome.note);
      }
    } catch (e) {
      setError(errMsg(e));
    } finally {
      const next = new Set(postingTo()); next.delete(issue); setPostingTo(next);
    }
  }
  return (
    <form class="issue-comment-box" aria-label="Write a comment" onSubmit={e => { e.preventDefault(); void post(); }}>
      <textarea class="input issue-comment-input" aria-label="Comment" rows={3} placeholder={`Leave a comment in ${props.displayName}…`}
        value={text()} disabled={busy() || props.disabled} aria-invalid={tooLong() ? 'true' : undefined} onInput={e => remember(e.currentTarget.value)} />
      <div class="issue-comment-actions">
        <span class="issue-muted">{props.disabled ? 'This issue can’t take comments from Hollow.' : tooLong() ? `Too long: ${bytes().toLocaleString('en-US')} of 2,000 bytes` : `Posts to ${props.displayName}; the team sees it.`}</span>
        <Button type="submit" size="sm" variant="primary" disabled={busy() || !text().trim() || tooLong() || props.disabled}>{busy() ? 'Posting…' : 'Comment'}</Button>
      </div>
      <Show when={note()}>{n => <p class="issue-detail-banner" role="status">{n()}</p>}</Show>
      <Show when={error()}>{msg => <ErrorText as="p" class="issue-detail-error" text={msg()} />}</Show>
    </form>
  );
}
