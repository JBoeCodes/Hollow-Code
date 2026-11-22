import { For, Index, Show, createEffect, createMemo, createSignal, onCleanup, onMount, untrack } from 'solid-js';
import { api } from '../api';
import { Icon } from '../icons';
import { Spinner } from './Spinner';
import { Select } from './Select';
import { BlockedByPicker } from './BlockedByPicker';
import { RefreshNotice } from './RefreshNotice';
import { tasksData, mutateTasks, refetchTasks, flashFooter, projectsData, refetchProjects, openVaultNote, deleteTask, requestOpenTask } from '../store';
import { parseClockTime, fmtClockHHMM, taskErrorText, errMsg } from '../utils';
import {
  applyTaskEditChanges, assistProjectUrl, dueTimePart, linkedLockNotice, lockedTaskFields, taskEditChanges, taskEditValues,
  TASK_PRIORITIES, taskStatusOptions, taskProjectChoices, projectKey, taskFormProblem,
  type LockableField, type TaskEditValues, type TaskProjectChoice,
} from '../taskEdit';
import type { Task, SubTask } from '../types';
import '../styles/task-editor.css';
import { ErrorText } from './ui';
import { TaskOriginText, taskOrigin } from './TaskOrigin';
import { readTaskDraft, sameDraftValue, writeTaskDraft } from '../taskDrafts';
import { TaskDraftReview } from './TaskDraftReview';

function composeBody(prose: string, subs: SubTask[]): string {
  let out = prose.trim();
  if (subs.length) {
    if (out) out += '\n\n';
    out += subs.map(s => `- [${s.done ? 'x' : ' '}] ${s.text}`).join('\n');
  }
  return out;
}

type Draft = TaskEditValues & { labelsText: string; timeText: string } & Record<string, unknown>;

/**
 * The one task form (audit H02). The palette/global sheet (TaskModal) and the
 * Tasks page's side panel both render this, so fields, vocabulary, validation,
 * accessible names and cancel behaviour are identical wherever a task is
 * created or edited. The first screen asks for name, project, due date,
 * priority and notes; repeat, labels, blockers and steps sit under More
 * details. New tasks are local. A tracker-linked task keeps its issue-owned
 * fields locked (lockedTaskFields).
 *
 * Drafts survive closing or switching tasks (sessionStorage, per task), and a
 * stale baseline (TASK_CONFLICT) keeps the form open with the draft.
 */
export function TaskEditor(props: {
  task: Task | null;
  /** Starting values for a new task (ignored when editing). */
  prefill?: { title?: string; description?: string; project?: string };
  /** Projects offered; defaults to the project list plus names on tasks. */
  projects?: () => TaskProjectChoice[];
  /** `sheet` = inside TaskModal; `panel` = the Tasks page side panel. */
  surface: 'sheet' | 'panel';
  headingId?: string;
  onClose: () => void;
  onSaved?: (url: string, created: boolean) => void | Promise<void>;
}) {
  const t = () => props.task;
  const isEdit = () => t() !== null;
  const cacheKey = `hollow.task-editor.${t()?.url ?? 'new'}`;
  let initial: TaskEditValues = taskEditValues(untrack(t));
  // The file version the form opened on. Fixed at mount: the task prop
  // follows list refetches, and sending a newer hash would let a stale diff
  // overwrite someone else's change instead of failing with TASK_CONFLICT.
  const [baseHash, setBaseHash] = createSignal(untrack(() => t()?.contentHash ?? null));
  const isoTime = dueTimePart((t()?.due ?? '').trim());
  const seed = untrack(() => props.task ? undefined : props.prefill);
  const blank = (): Draft => ({ ...initial, ...(seed?.title ? { title: seed.title } : {}), ...(seed?.description ? { description: seed.description } : {}), ...(seed?.project ? { project: seed.project } : {}), labelsText: initial.labels.join(', '), timeText: fmtClockHHMM(t()?.dueTime || isoTime || ''), subtasks: initial.subtasks.map(s => ({ ...s })), blockedBy: [...initial.blockedBy] });
  const cached = seed ? null : (() => { try {
    const shared = t() ? readTaskDraft<Draft>(t()!.url, 'local') : null;
    if (shared) return shared;
    const raw = sessionStorage.getItem(cacheKey); return raw ? JSON.parse(raw) as { draft: Draft; base?: Draft; hash?: string | null } : null;
  } catch { return null; } })();
  if (cached?.base) initial = cached.base;
  if (cached) setBaseHash(cached.hash ?? null);
  let unknownLegacyBase = !!cached && !cached.base && !!t() && cached.hash !== t()?.contentHash;
  const restored = cached?.draft ?? null;
  const [d, setD] = createSignal<Draft>(restored ?? blank());
  const [draftRestored, setDraftRestored] = createSignal(!!restored);
  const persist = () => {
    try {
      if (t()) { writeTaskDraft(t()!.url, 'local', hasEdits() ? { hash: baseHash(), base: { ...initial, labelsText: initial.labels.join(', '), timeText: fmtClockHHMM(initial.dueTime) }, draft: d() } : null); sessionStorage.removeItem(cacheKey); }
      else sessionStorage.setItem(cacheKey, JSON.stringify({ hash: baseHash(), draft: d() }));
    } catch { setError('Draft storage is full. Keep this editor open until saved.'); }
  };
  const set = <K extends keyof Draft>(k: K, v: Draft[K]) => {
    setD(prev => ({ ...prev, [k]: v }));
    persist();
  };
  const forget = () => { try { if (t()) writeTaskDraft(t()!.url, 'local', null); sessionStorage.removeItem(cacheKey); } catch { /* nothing stored */ } };
  const currentTask = () => tasksData()?.items.find(x => x.url === t()?.url) ?? t();
  const stale = () => !!t() && baseHash() !== currentTask()?.contentHash;
  const [reviewTask, setReviewTask] = createSignal<Task | null>(null);
  const formValues = (task: Task): Draft => { const v = taskEditValues(task); return { ...v, labelsText: v.labels.join(', '), timeText: fmtClockHHMM(v.dueTime) }; };
  async function review() { setBusy(true); try { const snapshot = await refetchTasks(); const task = snapshot?.items.find(x=>x.url===t()?.url); if (!task) throw new Error('This task is unavailable. Your draft is kept; retry when it can be read.'); setReviewTask(structuredClone(task)); } catch (e) { setError(taskErrorText(e)); } finally { setBusy(false); } }
  function discardDraft() { forget(); const task = currentTask(); if (task) { initial = taskEditValues(task); setBaseHash(task.contentHash); setD(formValues(task)); } else setD(blank()); setReviewTask(null); setDraftRestored(false); }

  const [newSub, setNewSub] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [assisting, setAssisting] = createSignal(false);
  const [error, setError] = createSignal('');
  const [attempted, setAttempted] = createSignal(false);
  // Assist output is staged for review, never written straight into fields.
  const [staged, setStaged] = createSignal<string[]>([]);
  const [proposedTitle, setProposedTitle] = createSignal<string | null>(null);
  const [moreOpen, setMoreOpen] = createSignal(!!(initial.dueTime || initial.durationMin || initial.recurrence || initial.section || initial.labels.length || initial.blockedBy.length || initial.subtasks.length));

  const locked = createMemo(() => lockedTaskFields(t()));
  const isLocked = (f: LockableField) => locked().includes(f);
  const reviewValues = (v: Draft) => Object.fromEntries(Object.entries(v).filter(([key]) => key !== 'labels' && key !== 'dueTime' && !locked().includes(key as LockableField) && !(key === 'timeText' && isLocked('dueTime'))));
  const hasEdits = () => { baseHash(); return unknownLegacyBase || !sameDraftValue(reviewValues(d()), reviewValues({...initial,labelsText:initial.labels.join(', '),timeText:fmtClockHHMM(initial.dueTime)})); };
  createEffect(()=>{const task=currentTask();if(!task||baseHash()===task.contentHash||reviewTask()||hasEdits())return;initial=taskEditValues(task);setBaseHash(task.contentHash);setD(formValues(task));setDraftRestored(false);});

  // Free-text time ("4pm" → "16:00"): a native time input desyncs under a
  // controlled binding in WKWebView and hard-blocks submission.
  const timeParsed = createMemo(() => parseClockTime(d().timeText));
  const problem = createMemo(() => taskFormProblem({ title: d().title, dueTime: d().timeText }, timeParsed()));

  const choices = createMemo(() => {
    const own = props.projects?.() ?? [
      ...(projectsData()?.items ?? []).map(p => ({ name: p.name, url: p.url })),
      ...(tasksData()?.items ?? []).flatMap(x => x.projectName ? [{ name: x.projectName, url: x.projectUrl ?? undefined }] : []),
    ];
    return taskProjectChoices(own, t()?.projectName ? { name: t()!.projectName!, url: t()!.projectUrl ?? undefined } : null);
  });
  const projectValue = () => { const name = d().project.trim(); if (!name) return ''; const hit = choices().find(p => p.name.toLowerCase() === name.toLowerCase()); return hit ? projectKey(hit) : `name:${name}`; };
  const knownSections = createMemo(() => [...new Set((tasksData()?.items ?? []).flatMap(x => x.section ? [x.section] : []))].sort());
  const knownLabels = createMemo(() => [...new Set((tasksData()?.items ?? []).flatMap(x => x.labels))].sort());
  const labels = () => d().labelsText.split(',').map(s => s.trim().replace(/^@/, '')).filter(Boolean);

  let titleEl: HTMLTextAreaElement | undefined;
  let formEl: HTMLFormElement | undefined;
  // The title takes focus a frame after mounting, unless focus is already in
  // another field of this form: under load that frame can land after the
  // person (or a test) started typing there (C6 P3, task-blockers flake).
  onMount(() => requestAnimationFrame(() => {
    const active = document.activeElement;
    if (active && active !== titleEl && formEl?.contains(active)) return;
    titleEl?.focus();
    if (!isEdit()) titleEl?.select();
  }));

  function addSub() { const text = newSub().trim(); if (!text) return; set('subtasks', [...d().subtasks, { text, done: false }]); setNewSub(''); }

  async function assist(action: 'breakdown' | 'rephrase') {
    if (!d().title.trim() || assisting()) return;
    setAssisting(true);
    try {
      // A linked task's Break down also sees its project's goals; only the
      // stored link is sent, and only while the Project field is unchanged.
      const projectUrl = action === 'breakdown' ? assistProjectUrl(t(), initial.project, d().project) : undefined;
      const r = await api.taskAssist({ action, title: d().title, description: d().description, ...(projectUrl ? { projectUrl } : {}) });
      if (action === 'rephrase') { if (r.text) setProposedTitle(r.text); }
      else setStaged(prev => [...prev, ...r.suggestions.filter(Boolean)]);
    } catch (e) { flashFooter(`× assist failed: ${errMsg(e)}`, 2800); }
    finally { setAssisting(false); }
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    setAttempted(true);
    if (problem() || busy() || reviewTask()) return;
    if (stale()) { await review(); return; }
    setBusy(true); setError('');
    const time = timeParsed() ?? '';
    const v = d();
    try {
      const task = t();
      if (task) {
        // Only what changed since the form opened; a locked (issue-owned)
        // field is never sent. `expected` makes the write compare-and-swap.
        const changes = taskEditChanges(initial, { ...v, dueTime: time, labels: labels() }, { locked: locked() });
        if (Object.keys(changes).length) {
          const written = await api.updateTask({ url: task.url, expected: baseHash() || undefined, ...changes });
          const snap = tasksData();
          if (snap) mutateTasks({ ...snap, items: snap.items.map(it => it.url !== task.url ? it : applyTaskEditChanges(it, changes, written.contentHash)) });
        }
        forget();
        void refetchTasks();
        await props.onSaved?.(task.url, false);
      } else {
        const project = choices().find(p => projectKey(p) === projectValue());
        const created = await api.createTask({
          title: v.title.trim(),
          status: v.status || undefined,
          priority: v.priority || undefined,
          due: v.due || undefined,
          dueTime: time || undefined,
          durationMin: v.durationMin.trim() ? parseInt(v.durationMin, 10) : undefined,
          recurrence: v.recurrence || undefined,
          section: v.section || undefined,
          project: project?.name ?? (v.project.trim() || undefined),
          projectUrl: project?.url,
          labels: labels(),
          description: composeBody(v.description, v.subtasks) || undefined,
          blockedBy: v.blockedBy.length ? v.blockedBy : undefined,
        });
        forget();
        void refetchTasks();
        await props.onSaved?.(created.url, true);
      }
      props.onClose();
    } catch (err) {
      // Tauri rejects with a plain string; the draft stays in the form.
      setError(taskErrorText(err) || 'Save failed. Your draft is kept.');
      if (String(err).includes('TASK_CONFLICT')) await review();
    } finally { setBusy(false); }
  }

  onMount(() => {
    // ⌘↵ saves from any field, on both surfaces.
    const key = (e: KeyboardEvent) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey) && formEl?.contains(document.activeElement)) { e.preventDefault(); formEl.requestSubmit(); } };
    document.addEventListener('keydown', key);
    onCleanup(() => document.removeEventListener('keydown', key));
  });

  // Where it came from (consolidation G): a plan item, a recording, an agent
  // run or Incoming, with the way back.
  const origin = () => isEdit() ? taskOrigin(t()?.origin, t()?.title) : null;
  const problemId = `task-problem-${Math.random().toString(36).slice(2, 8)}`;
  return <form ref={formEl} class="task-editor-form" classList={{ sheet: props.surface === 'sheet', panel: props.surface === 'panel' }} onSubmit={submit} aria-labelledby={props.headingId}>
    <Show when={!isEdit()}>
      <p class="task-create-note">Saved in Hollow as a local task.</p>
    </Show>
    <Show when={stale()} fallback={<Show when={draftRestored()}><p class="task-draft-note" role="status">Unsaved draft restored. <button type="button" class="btn-link" disabled={busy()} onClick={discardDraft}>Discard draft</button></p></Show>}><p class="task-draft-note" role="status">This task changed since your draft: <button type="button" class="btn-link" disabled={busy()} onClick={()=>void review()}>Review</button> / <button type="button" class="btn-link" disabled={busy()} onClick={discardDraft}>Discard</button></p></Show>
    <Show when={reviewTask()} keyed>{current=><TaskDraftReview base={unknownLegacyBase?{}:reviewValues({...initial,labelsText:initial.labels.join(', '),timeText:fmtClockHHMM(initial.dueTime)})} mine={reviewValues(d())} current={reviewValues(formValues(current))} labels={{labelsText:'Labels',timeText:'Time',title:'Task name',subtasks:'Steps',blockedBy:'Blockers',due:'Due date',description:'Notes',durationMin:'Duration',recurrence:'Repeat',section:'Section',status:'Status',priority:'Priority',project:'Project'}} onApply={values=>{initial=taskEditValues(current);unknownLegacyBase=false;setBaseHash(current.contentHash);setD({...formValues(current),...values});setReviewTask(null);setError('');persist();}} onCancel={()=>setReviewTask(null)}/>}</Show>
    <Show when={origin()}>{o => <p class="task-origin-note"><span class="task-origin-k">From</span> <TaskOriginText origin={o()} before={props.onClose}/></p>}</Show>
    <Show when={locked().length}>
      <div class="task-linked-note" role="note">
        <p>{linkedLockNotice(locked())}</p>
        <Show when={t()?.tracker?.lock.kind === 'linked' && props.surface === 'sheet'}>
          <button type="button" class="btn sm" onClick={() => { const x = t(); if (x) { requestOpenTask(x.url); props.onClose(); } }}>Edit synced fields in Tasks</button>
        </Show>
      </div>
    </Show>

    <label class="task-field">
      <span>Task name</span>
      <textarea ref={titleEl} rows={2} required autocomplete="off" placeholder="What needs to happen next?" disabled={isLocked('title')}
        aria-invalid={attempted() && !d().title.trim() ? 'true' : undefined} aria-describedby={problem() ? problemId : undefined}
        value={d().title} onInput={e => set('title', e.currentTarget.value)}
        onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey && !e.metaKey && !e.ctrlKey) { e.preventDefault(); formEl?.requestSubmit(); } }} />
    </label>
    <Show when={proposedTitle()}>{name =>
      <div class="assist-staged assist-rename"><span class="form-label">Suggested name</span><p class="assist-staged-text">{name()}</p>
        <div class="assist-staged-actions"><button type="button" class="btn sm" onClick={() => { set('title', name()); setProposedTitle(null); }}>Use this name</button><button type="button" class="btn ghost sm" onClick={() => setProposedTitle(null)}>Keep mine</button></div></div>
    }</Show>

    <div class="task-field-grid">
      <div class="task-field"><span id="task-project-label">Project</span>
        <Select label="Project" placeholder="No project" disabled={isLocked('project')} value={projectValue()} emptyText="No project matches"
          options={[{ value: '', label: 'No project' }, ...choices().map(p => ({ value: projectKey(p), label: p.name, detail: p.url ? undefined : 'Name only, not linked' }))]}
          onChange={value => set('project', value ? choices().find(p => projectKey(p) === value)?.name ?? value.replace(/^name:/, '') : '')} />
        {/* X21: the project list is the last one that loaded when a refresh failed. */}
        <Show when={!props.projects}><RefreshNotice resource={projectsData} onRetry={refetchProjects} what="projects" /></Show>
      </div>
      <label class="task-field"><span>Due date</span><input type="date" disabled={isLocked('due')} value={d().due} onInput={e => set('due', e.currentTarget.value)} /></label>
    </div>

    {/* A radio group, not buttons inside a <label>: the label wrapper is what
        produced names like "PRIORITY P2 P3 P4" in the old sheet. */}
    <div class="task-field"><span id="task-priority-label">Priority</span>
      <div class="task-priority" role="radiogroup" aria-labelledby="task-priority-label" onKeyDown={e => {
        // Radio-group keys: arrows move and select; one Tab stop (roving tabindex).
        const order = ['', ...TASK_PRIORITIES.map(p => p.value)];
        const step = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0;
        if (!step || isLocked('priority')) return;
        e.preventDefault();
        const group = e.currentTarget as HTMLElement;
        const next = order[(order.indexOf(d().priority) + step + order.length) % order.length];
        set('priority', next);
        queueMicrotask(() => group.querySelector<HTMLElement>('[aria-checked="true"]')?.focus());
      }}>
        <For each={[{ value: '', label: 'None' }, ...TASK_PRIORITIES]}>{p =>
          <button type="button" role="radio" aria-checked={d().priority === p.value} tabindex={d().priority === p.value ? 0 : -1} class={`task-priority-opt ${p.value.toLowerCase()}`} disabled={isLocked('priority')}
            aria-label={p.value ? `${p.label} priority (${p.value})` : 'No priority'} onClick={() => set('priority', p.value)}>{p.label}</button>
        }</For>
      </div>
    </div>

    <Show when={isEdit()}>
      <div class="task-field"><span>Status</span>
        <Select label="Status" disabled={isLocked('status')} value={d().status} searchable={false}
          options={taskStatusOptions(t()?.status).map(s => ({ value: s, label: s }))} onChange={v => set('status', v)} />
      </div>
    </Show>

    <label class="task-field"><span>Notes <small class="work-muted">optional</small></span>
      <textarea rows={3} disabled={isLocked('description')} value={d().description} onInput={e => set('description', e.currentTarget.value)} /></label>

    <details class="task-more" open={moreOpen()} onToggle={e => setMoreOpen(e.currentTarget.open)}>
      <summary>More details<small>time, repeat, labels, blockers, steps</small></summary>
      <div class="task-field-grid three">
        <label class="task-field"><span>Time</span><input type="text" autocomplete="off" placeholder="e.g. 4pm" disabled={isLocked('dueTime')} aria-invalid={!!d().timeText.trim() && !timeParsed() ? 'true' : undefined} value={d().timeText} onInput={e => set('timeText', e.currentTarget.value)} /></label>
        <label class="task-field"><span>Duration (min)</span><input type="number" min="0" step="5" placeholder="—" value={d().durationMin} onInput={e => set('durationMin', e.currentTarget.value)} /></label>
        <label class="task-field"><span>Repeats</span><input type="text" placeholder="e.g. every weekday" value={d().recurrence} onInput={e => set('recurrence', e.currentTarget.value)} /></label>
      </div>
      <div class="task-field-grid">
        <label class="task-field"><span>Section</span><input type="text" placeholder="Optional" list="task-section-options" value={d().section} onInput={e => set('section', e.currentTarget.value)} />
          <datalist id="task-section-options"><For each={knownSections()}>{s => <option value={s} />}</For></datalist></label>
        <label class="task-field"><span>Labels <small class="work-muted">comma-separated</small></span><input type="text" placeholder="e.g. ui, customer" list="task-label-options" value={d().labelsText} onInput={e => set('labelsText', e.currentTarget.value)} />
          <datalist id="task-label-options"><For each={knownLabels()}>{l => <option value={l} />}</For></datalist></label>
      </div>
      <div class="task-field task-blocked-by"><span>Blocked by</span>
        <BlockedByPicker tasks={() => tasksData()?.items ?? []} selfUrl={t()?.url ?? null} value={() => d().blockedBy} onChange={next => set('blockedBy', next)} disabled={busy()} />
      </div>
      <Show when={staged().length > 0}>
        <div class="assist-staged"><span class="form-label">Suggested steps</span>
          {/* Index, not For: suggestions are bare strings and may repeat. */}
          <Index each={staged()}>{(s, i) =>
            <div class="assist-staged-row"><span class="assist-staged-text">{s()}</span>
              <button type="button" class="btn ghost sm" onClick={() => { set('subtasks', [...d().subtasks, { text: s(), done: false }]); setStaged(prev => prev.filter((_, x) => x !== i)); }}>Add step</button>
              <button type="button" class="row-quick-btn" aria-label={`Dismiss suggestion: ${s()}`} title="Dismiss" onClick={() => setStaged(prev => prev.filter((_, x) => x !== i))}><Icon name="trash" size={12} /></button></div>
          }</Index>
          <div class="assist-staged-actions"><button type="button" class="btn sm" onClick={() => { set('subtasks', [...d().subtasks, ...staged().map(text => ({ text, done: false }))]); setStaged([]); }}>Add all steps</button><button type="button" class="btn ghost sm" onClick={() => setStaged([])}>Dismiss all</button></div>
        </div>
      </Show>
      <div class="subtask-editor"><span class="form-label">Steps</span>
        <For each={d().subtasks}>{(s, i) =>
          <div class="subtask-row">
            <button type="button" class="subtask-tick" classList={{ checked: s.done }} aria-pressed={s.done} aria-label={`${s.done ? 'Reopen' : 'Complete'} step: ${s.text}`}
              onClick={() => set('subtasks', d().subtasks.map((x, idx) => idx === i() ? { ...x, done: !x.done } : x))}><span class="subtask-tick-mark"><Icon name="check" size={12} /></span></button>
            <span class={`subtask-text ${s.done ? 'done' : ''}`}>{s.text}</span>
            <button type="button" class="subtask-del" aria-label={`Remove step: ${s.text}`} title="Remove step" onClick={() => set('subtasks', d().subtasks.filter((_, idx) => idx !== i()))}><Icon name="trash" size={12} /></button>
          </div>
        }</For>
        <div class="subtask-add"><input type="text" aria-label="New step" placeholder="Add a step…" value={newSub()} onInput={e => setNewSub(e.currentTarget.value)} onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); addSub(); } }} />
          <button type="button" class="btn sm" onClick={addSub} disabled={!newSub().trim()}>Add step</button></div>
      </div>
      <div class="assist-row"><span class="form-label">AI assist</span>
        <button type="button" class="btn ghost sm" disabled={assisting() || !d().title.trim()} onClick={() => void assist('breakdown')}>Suggest steps</button>
        <button type="button" class="btn ghost sm" disabled={assisting() || !d().title.trim() || isLocked('title')} onClick={() => void assist('rephrase')}>Suggest a clearer name</button>
        <Show when={assisting()}><Spinner /></Show></div>
    </details>

    <Show when={error()}>{msg => <ErrorText class="task-detail-error" text={msg()}/>}</Show>
    <div class="task-editor-foot">
      <div class="task-editor-foot-left"><Show when={isEdit()}>
        <button type="button" class="btn ghost sm" disabled={!t()?.vaultPath} onClick={() => { openVaultNote(t()?.vaultPath); props.onClose(); }}>Open note</button>
        <button type="button" class="btn ghost sm task-delete" onClick={() => { const x = t(); if (x) { forget(); void deleteTask(x); props.onClose(); } }}>Delete task</button>
      </Show></div>
      <div class="task-editor-foot-right">
        <Show when={problem()}><span id={problemId} class="task-form-problem" classList={{ attempted: attempted() }}>{problem()}</span></Show>
        <button type="button" class="btn ghost sm" disabled={busy()} onClick={props.onClose}>Cancel</button>
        <button type="submit" class="btn primary sm" disabled={busy() || !!problem()} aria-describedby={problem() ? problemId : undefined} title="Save (⌘↵)">{busy() ? (isEdit() ? 'Saving…' : 'Creating…') : isEdit() ? 'Save changes' : 'Create task'}</button>
      </div>
    </div>
  </form>;
}
