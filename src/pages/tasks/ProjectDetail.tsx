// One project (overhaul B3, board S4; the Projects view opens it too, B4):
// the tracker's fields for a tracked project (status, lead, target date,
// progress and issue counts, "Open in Linear") and the vault's portfolio
// fields from its workspace's PROJECT.md, which edit here and stay in the
// vault. A vault-only project offers "Track in Linear" (an outside write: it
// says what it creates and waits for the click). One component for both
// entry points: the Tasks page's pane and Library's Records.
import { For, Show, createEffect, createMemo, createResource, createSignal, on, type JSX } from 'solid-js';
import { MarkdownView } from '../../components/MarkdownView';
import { Button, ErrorText } from '../../components/ui';
import { chatBusy, flashFooter, openVaultNote, trackerSyncPass } from '../../store';
import { selectWorkspace, workspaces } from '../../store/workspaces';
import { errMsg } from '../../utils';
import { createTrackerProject, projectPortfolio, setProjectPortfolio, trackerStatus, trackerView, writeFridayStatus } from '../../trackerApi';
import type { ProjectPortfolio, TrackerProjectRow } from '../../generated/ipc';
import {
  PORTFOLIO_FIELDS, PROJECT_KIND_LABELS, changedValues, draftOf, draftProblems, isVaultPath, itemLabel, percent,
  projectCounts, projectTitle, type PortfolioDraft,
} from '../../projectPortfolio';
import { dueText } from '../../trackerTasks';
import { Avatar } from './issueIcons';
import { TeamPicker } from './TeamChoice';
import { createTeamKey, hasTwoTeams, statusTeamKeys } from '../../trackerTeams';
import { openLink } from './IssueDetail';
import '../../styles/tracker-tasks.css';

// Unsaved portfolio edits by workspace, so opening another record (or the
// file) and coming back finds them again; Save and Cancel clear them.
const drafts = new Map<string, { base: ProjectPortfolio; draft: PortfolioDraft }>();

export function ProjectDetail(props: {
  /** The tracker's project id (the Projects view, an issue's project). */
  trackerId?: string | null;
  /** The workspace whose PROJECT.md holds the record (Library, Records). */
  workspaceId?: string | null;
  onClose?: () => void;
  /** B4's issue list, filtered to this project. */
  issues?: (trackerId: string) => JSX.Element;
  /** Told while the portfolio editor is open (Library keeps the file view
   *  closed until it is saved or canceled). */
  onEditing?: (editing: boolean) => void;
}) {
  const [status] = createResource(() => trackerStatus().catch(() => null));
  const displayName = () => status()?.displayName ?? 'the tracker';
  const trackerReady = () => !!status()?.provider && !status()?.missing && !status()?.blockedBy && status()?.engine !== false;
  // Opened from Library the workspace is known and its record names the
  // tracker project; opened from Tasks the tracker project is known and the
  // mirror names its workspace (once a pass made or adopted one).
  const [linkedWorkspace, setLinkedWorkspace] = createSignal<string | null>(null);
  const workspaceId = () => props.workspaceId ?? linkedWorkspace();
  const [portfolio, { refetch: refetchPortfolio, mutate: setPortfolio }] = createResource(workspaceId, id => projectPortfolio(id));
  const record = () => (portfolio.state === 'ready' || portfolio.state === 'refreshing' ? portfolio() : undefined);
  const trackerId = () => props.trackerId ?? record()?.tracked?.projectId ?? null;
  const [view, { refetch: refetchView }] = createResource(trackerId, id =>
    trackerView({ projectId: id, includeClosed: true, limit: 2000 }).catch(() => undefined));
  const row = createMemo<TrackerProjectRow | null>(() => view()?.projects.find(p => p.id === trackerId()) ?? null);
  createEffect(() => setLinkedWorkspace(row()?.workspaceId ?? null));
  createEffect(on(trackerSyncPass, pass => { if (pass) { void refetchView(); if (workspaceId()) void refetchPortfolio(); } }, { defer: true }));

  const counts = createMemo(() => projectCounts(view()?.rows ?? []));
  const workspace = () => { const id = workspaceId(); return id ? workspaces().find(w => w.id === id) : undefined; };
  const tracked = () => !!trackerId();
  const target = () => row()?.targetDate ?? record()?.targetDate ?? null;
  const description = () => record()?.description ?? '';

  const [editing, setEditing] = createSignal(false);
  const [draft, setDraft] = createSignal<PortfolioDraft>({});
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal('');
  const problems = createMemo(() => draftProblems(draft()));
  // The record as it was when editing began: the save sends only what the
  // person changed from it, against its revision, so a refresh meanwhile is
  // refused (PORTFOLIO_CHANGED), never silently reverted.
  let editBase: ProjectPortfolio | null = null;
  function startEdit() { const r = record(); if (!r) return; editBase = r; setDraft(draftOf(r)); setError(''); setEditing(true); }
  function endEdit() { if (editBase) drafts.delete(editBase.workspaceId); editBase = null; setEditing(false); }
  createEffect(on(editing, e => props.onEditing?.(e)));
  // Keep the open draft; bring back one left unsaved here earlier.
  createEffect(on(draft, d => { if (editing() && editBase) drafts.set(editBase.workspaceId, { base: editBase, draft: d }); }, { defer: true }));
  createEffect(on(() => record()?.workspaceId, id => {
    const kept = id ? drafts.get(id) : undefined;
    if (kept && !editing()) { editBase = kept.base; setDraft(kept.draft); setEditing(true); }
  }));
  async function saveEdit() {
    const r = editBase;
    if (!r || saving() || Object.keys(problems()).length) return;
    const values = changedValues(r, draft());
    if (!Object.keys(values).length) { endEdit(); return; }
    setSaving(true); setError('');
    // The kept draft this save sends: a newer one (another open of this
    // record, edited while this save ran) must survive its answer.
    const sent = drafts.get(r.workspaceId);
    try {
      const next = await setProjectPortfolio({ workspaceId: r.workspaceId, values, expectedRevision: r.revision });
      setPortfolio(next);
      if (drafts.get(r.workspaceId) === sent) drafts.delete(r.workspaceId);
      editBase = null;
      setEditing(false);
      flashFooter('Saved in the project’s PROJECT.md.', 2500);
    } catch (e) {
      const text = errMsg(e);
      setError(/PORTFOLIO_CHANGED/.test(text) ? 'PROJECT.md changed since you opened it, so nothing was saved. Your edits are still here: Reload, then save again.' : text);
    } finally { setSaving(false); }
  }
  async function reload() {
    const id = record()?.workspaceId;
    if (!id) return;
    const next = await projectPortfolio(id).catch(e => { setError(errMsg(e)); return null; });
    if (!next) return;
    setPortfolio(next);
    // Keep only the fields the person changed, over the fresh values, and
    // save against the fresh revision from now on.
    if (editing() && editBase) {
      const mine = Object.keys(changedValues(editBase, draft()));
      const fresh = draftOf(next);
      setDraft({ ...fresh, ...Object.fromEntries(mine.map(k => [k, draft()[k]])) });
      editBase = next;
      drafts.set(next.workspaceId, { base: next, draft: draft() });
    }
    setError('');
  }

  // Track in Linear: an outside write, so it names what it creates first.
  const [confirming, setConfirming] = createSignal(false);
  const [tracking, setTracking] = createSignal(false);
  // The team the project is created in (AI unless the person picks the other); asked only with two teams.
  const [trackTeam, setTrackTeam] = createSignal('ENG');
  async function trackIn() {
    const r = record();
    if (!r || tracking()) return;
    setTracking(true); setError('');
    try {
      const outcome = await createTrackerProject({ name: r.title, description: r.description || null, targetDate: r.targetDate, workspaceId: r.workspaceId, teamKey: createTeamKey(trackTeam(), statusTeamKeys(status())) });
      setConfirming(false);
      flashFooter(outcome.queued ? `Saved; ${r.title} is created in ${displayName()} when Hollow can reach it.` : `${r.title} is tracked in ${displayName()}. Its issues arrive on the next sync.`, 4000);
      await reload();
    } catch (e) { setError(errMsg(e)); } finally { setTracking(false); }
  }

  // A18: this project's Friday status, written into its workspace now.
  const [writingStatus, setWritingStatus] = createSignal(false);
  async function writeStatus() {
    const id = workspaceId();
    if (!id || writingStatus()) return;
    setWritingStatus(true); setError('');
    try {
      const out = await writeFridayStatus(id);
      const note = out.projects[0];
      const copy = out.copies[0];
      flashFooter(`${out.heraldLine}${copy ? (copy.posted ? ` Also posted to ${displayName()}.` : ` Not posted to ${displayName()}: ${copy.note}`) : ''}`, 5000);
      if (note) openVaultNote(note.path);
    } catch (e) { setError(errMsg(e)); } finally { setWritingStatus(false); }
  }

  return (
    <div class="issue-detail project-detail" aria-busy={portfolio.loading || view.loading ? 'true' : undefined}>
      <header class="issue-detail-top">
        <span class="issue-detail-crumb">Projects <span aria-hidden="true">›</span> <strong>{projectTitle(row(), record())}</strong></span>
        <div class="issue-detail-top-actions">
          <Show when={row()}>{r => <Button size="sm" variant="ghost" icon="externalLink" onClick={() => openLink(r().url)}>Open in {displayName()}{r().key ? ` (${r().key})` : ''}</Button>}</Show>
          <Show when={tracked() && workspaceId()}><Button size="sm" variant="ghost" icon="edit" disabled={writingStatus()} title="Writes this week’s status into the project’s outputs/status folder" onClick={() => void writeStatus()}>{writingStatus() ? 'Writing status…' : 'Write status now'}</Button></Show>
          <Show when={workspace()}>{w => <Button size="sm" variant="ghost" icon="message" disabled={chatBusy()} onClick={() => void selectWorkspace(w().id)}>Open the project chat</Button>}</Show>
          <Show when={props.onClose}>{close => <Button size="sm" variant="ghost" aria-label="Close project details" onClick={close()}>Close <span aria-hidden="true">×</span></Button>}</Show>
        </div>
      </header>
      <Show when={portfolio.error && !tracked()}>
        <ErrorText as="p" class="issue-detail-error" text={`Couldn’t read this project: ${errMsg(portfolio.error)}`} />
      </Show>
      <div class="issue-detail-body">
        <article class="issue-detail-main" aria-labelledby="project-detail-title">
          <h2 id="project-detail-title" class="issue-detail-title project-detail-title">{projectTitle(row(), record())}</h2>
          <p class="project-tracked-state" data-tracked={tracked() ? '' : undefined} role="note">
            {tracked() ? `Tracked in ${displayName()}: its name, status, lead, dates and description come from there. The portfolio fields below stay in your vault.`
              : 'Vault only: this project isn’t in the tracker. Its record lives in the workspace’s PROJECT.md.'}
          </p>
          <section class="issue-section" aria-label="Description">
            <Show when={description().trim()} fallback={<p class="issue-muted">No description.</p>}>
              <MarkdownView class="issue-markdown" text={description()} forbidMedia />
            </Show>
          </section>
          <Show when={tracked()}>
            <section class="issue-section project-issues" aria-label="Issues">
              <h3>Issues <span class="issue-count">{counts().total}</span></h3>
              <Show when={props.issues} fallback={
                <p class="issue-muted">{counts().open} open, {counts().done} done{counts().canceled ? `, ${counts().canceled} canceled` : ''}.</p>
              }>{list => list()(trackerId()!)}</Show>
            </section>
          </Show>
          <Show when={!tracked() && record()}>
            <section class="issue-section project-track" aria-label={`Track in ${displayName()}`}>
              <Show when={trackerReady()} fallback={<p class="issue-muted">Set up a task tracker (Settings, Sources, Task tracker) to track this project there.</p>}>
                <Show when={confirming()} fallback={<Button size="sm" variant="secondary" icon="plus" onClick={() => setConfirming(true)}>Track in {displayName()}</Button>}>
                  <p>Creates “{record()!.title}” as a project in {displayName()}, team {hasTwoTeams(statusTeamKeys(status())) ? trackTeam() : status()?.teamKey ?? 'ENG'}. {trackTeam() === 'PER' && hasTwoTeams(statusTeamKeys(status())) ? 'Only you and Hollow see it.' : 'Everyone on the team sees it.'} Its portfolio fields stay here.</p>
                  <Show when={hasTwoTeams(statusTeamKeys(status()))}><TeamPicker teamKeys={statusTeamKeys(status())} value={trackTeam()} onChange={setTrackTeam} /></Show>
                  <div class="issue-inline-actions">
                    <Button size="sm" variant="primary" disabled={tracking()} onClick={() => void trackIn()}>{tracking() ? 'Creating…' : `Create in ${displayName()}`}</Button>
                    <Button size="sm" variant="ghost" onClick={() => setConfirming(false)}>Cancel</Button>
                  </div>
                </Show>
              </Show>
            </section>
          </Show>
        </article>
        <aside class="issue-properties" aria-label="Project properties">
          <Show when={row()} fallback={<Show when={record()?.status}>{s => <Prop label="Status"><span class="issue-prop-value">{s()}</span></Prop>}</Show>}>{r => <>
            <Prop label="Status"><span class="issue-prop-value"><span class="project-status-dot" data-kind={r().statusKind} aria-hidden="true" />{r().statusName}</span></Prop>
            <Prop label="Lead"><span class="issue-prop-value"><Avatar name={r().lead} />{r().lead ?? 'No lead'}</span></Prop>
            <Prop label="Progress"><span class="issue-prop-value project-progress" title={`${counts().done} of ${counts().total} issues done`}>
              <span class="project-progress-bar" aria-hidden="true"><span style={{ width: `${Math.round((counts().progress ?? 0) * 100)}%` }} /></span>
              {percent(counts().progress) ?? 'No issues'}</span></Prop>
            <Prop label="Issues"><span class="issue-prop-value">{counts().open} open of {counts().total}</span></Prop>
            <Show when={r().statusKind && PROJECT_KIND_LABELS[r().statusKind] && PROJECT_KIND_LABELS[r().statusKind] !== r().statusName}>
              <Prop label="Kind"><span class="issue-prop-value issue-muted">{PROJECT_KIND_LABELS[r().statusKind]}</span></Prop>
            </Show>
          </>}</Show>
          <Prop label="Target"><span class="issue-prop-value" data-overdue={dueText(target())?.overdue ? '' : undefined}>{target() ? dueText(target())!.text : 'No target date'}</span></Prop>
          <div class="issue-properties-rule" />
          <div class="project-portfolio-head">
            <h3>Portfolio</h3>
            <Show when={record() && !editing()}><Button size="sm" variant="ghost" icon="edit" onClick={startEdit}>Edit</Button></Show>
          </div>
          <Show when={record()} fallback={<p class="issue-muted">{workspaceId() ? 'Reading PROJECT.md…' : 'No workspace for this project yet; the next sync makes one.'}</p>}>{r => (
            <Show when={editing()} fallback={
              <dl class="project-portfolio">
                <For each={PORTFOLIO_FIELDS}>{f => (
                  <Show when={f.kind === 'list' ? (r().lists[f.key] ?? []).length : (r().text[f.key] ?? '').trim()}>
                    <div class="issue-property"><dt class="issue-property-label">{f.label}</dt><dd class="issue-property-value">
                      <Show when={f.kind === 'list'} fallback={<Show when={f.kind === 'url'} fallback={<span class="issue-prop-value">{r().text[f.key]}</span>}>
                        <button type="button" class="btn-link" onClick={() => openLink(r().text[f.key])}>{r().text[f.key].replace(/^https?:\/\//, '')}</button></Show>}>
                        <ul class="issue-plain-list"><For each={r().lists[f.key]}>{item => <li><Show when={isVaultPath(item)} fallback={item}>
                          <button type="button" class="btn-link" title={item} onClick={() => openVaultNote(item)}>{itemLabel(item)}</button></Show></li>}</For></ul>
                      </Show>
                    </dd></div>
                  </Show>
                )}</For>
                <Show when={PORTFOLIO_FIELDS.every(f => f.kind === 'list' ? !(r().lists[f.key] ?? []).length : !(r().text[f.key] ?? '').trim())}>
                  <p class="issue-muted">No portfolio fields yet. Edit adds department, value, people, domains and links.</p>
                </Show>
              </dl>
            }>
              <form class="project-portfolio-form" aria-label="Edit portfolio" onSubmit={e => { e.preventDefault(); void saveEdit(); }}>
                <fieldset class="project-portfolio-fields" disabled={saving()}>
                <For each={PORTFOLIO_FIELDS}>{f => (
                  <label class="project-portfolio-field"><span class="issue-property-label">{f.label}</span>
                    <Show when={f.kind === 'list'} fallback={
                      <input class="input issue-prop-input" type={f.kind === 'url' ? 'url' : 'text'} inputmode={f.kind === 'number' ? 'decimal' : undefined}
                        aria-invalid={problems()[f.key] ? 'true' : undefined} value={draft()[f.key] ?? ''} placeholder={f.hint}
                        onInput={e => setDraft({ ...draft(), [f.key]: e.currentTarget.value })} />}>
                      <textarea class="input project-portfolio-list" rows={2} value={draft()[f.key] ?? ''} placeholder={f.hint ?? 'One per line'}
                        onInput={e => setDraft({ ...draft(), [f.key]: e.currentTarget.value })} />
                    </Show>
                    <Show when={problems()[f.key]}>{p => <span class="project-portfolio-problem">{p()}</span>}</Show>
                  </label>
                )}</For>
                </fieldset>
                <div class="issue-inline-actions">
                  <Button type="submit" size="sm" variant="primary" disabled={saving() || !!Object.keys(problems()).length}>{saving() ? 'Saving…' : 'Save'}</Button>
                  <Button type="button" size="sm" variant="ghost" disabled={saving()} onClick={() => { endEdit(); setError(''); }}>Cancel</Button>
                  <Show when={/changed since/.test(error())}><Button type="button" size="sm" variant="ghost" onClick={() => void reload()}>Reload</Button></Show>
                </div>
              </form>
            </Show>
          )}</Show>
          <Show when={error()}>{msg => <ErrorText as="p" class="issue-detail-error" text={msg()} />}</Show>
        </aside>
      </div>
    </div>
  );
}

function Prop(props: { label: string; children: JSX.Element }) {
  return <div class="issue-property"><span class="issue-property-label">{props.label}</span><div class="issue-property-value">{props.children}</div></div>;
}
