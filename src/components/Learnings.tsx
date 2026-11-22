/** The one review home for learned context, shared with its reply line.
 * Loaded only by Library's Learnings tab or a finished reply. */
import { For, Show, Suspense, createEffect, createMemo, createSignal, lazy, on, onCleanup, onMount } from 'solid-js';
import { createStore } from 'solid-js/store';
import { api } from '../api';
import type { LearningContext, LearningProposal } from '../types';
import type { LearningBatchOutcome } from '../generated/ipc';
import { reviewInboxRevision, refreshReviewInbox } from '../reviewInbox';
import { refreshWaitingOnYou } from '../store/waitingOnYou';
import { openLibraryLearnings } from '../store/libraryMode';
import { activeChatRecall, flashFooter, flashWithUndo, openVaultNote } from '../store';
import { workspaceId } from '../store/workspaces';
import { dismissSnackbar } from '../store/snackbars';
import { listen, type UnlistenFn } from '../nativeEvents';
import { scopedListen } from '../scopedListen';
import {
  autoSaveUndoBatch, batchAfterUndo, batchMessage, editedWords, failureDetail, failures, nothingLeftToUndo, pinEditVersion, undoLastLabel, undoMessage, versionForSave, BATCH_MAX, REJECT_REASON_MAX, type LastBatch,
} from '../learningBulk';
import { memorySource } from '../memoryProvenance';
import { errMsg } from '../utils';
import { shortcutsEnabled } from '../shortcuts';
import { Tabs, ErrorText, Skeleton, type TabItem } from './ui';
import { Select } from './Select';
import { ProfileSuggestions } from './ProfileSuggestions';
import { profileWaitingCount } from '../store/profileBuilder';
import '../styles/learnings.css';

/** The found-context review card (CX20): its own lazy chunk. */
const ContextReviewCard = lazy(() => import('./ContextReviewCard').then(m => ({ default: m.ContextReviewCard })));

/** `editedAt` is the version the words were first edited at (ME1): a Save
 *  sends it, not the version of a proposal refreshed under the edit. */
type Draft = { reason: string; edited: string | null; editedAt?: string; rejecting: boolean; open: boolean };
const NO_DRAFT: Draft = { reason: '', edited: null, rejecting: false, open: false };

const [proposals, setProposals] = createSignal<LearningProposal[]>([]);
const [skillLessons,setSkillLessons] = createSignal<Awaited<ReturnType<typeof api.listSkillLessons>>>([]);
const [contexts, setContexts] = createSignal<Awaited<ReturnType<typeof api.listLearningContext>>>([]);
const [maintenance, setMaintenance] = createSignal<Awaited<ReturnType<typeof api.memoryMaintenance>> | null>(null);
const [loaded, setLoaded] = createSignal(false);
const [loadError, setLoadError] = createSignal('');
/** A failed action's message by learning id, so it shows under that row
 *  only; `undo:<id>` holds a reply line's Undo (`undoErrorKey`),
 *  `skill:<id>:<status>` a skill lesson's actions (`skillErrorKey`), and ''
 *  the page's own actions (bulk, Undo, Review again). */
const [actionErrors, setActionErrors] = createStore<Record<string, string | undefined>>({});
const [busy, setBusy] = createSignal(false);
let inflight: Promise<void> | null = null;
let again = false;
let readers = 0;
let stop: UnlistenFn | undefined;
const [picked, setPicked] = createSignal<Set<string>>(new Set());
const [bulkReason, setBulkReason] = createSignal('');
const [actionDrafts, setActionDrafts] = createStore<Record<string, Draft | undefined>>({});
/** The last bulk Save or Reject, while something in it can be undone: its
 *  snackbar's Undo and "Undo last batch" both take it (overnight A06). A
 *  clean Undo, or one that finds nothing left, lets it go. */
const [lastBatch, setLastBatch] = createSignal<LastBatch | null>(null);
const forgetBatch = (id: string) => setLastBatch(b => (b?.id === id ? null : b));

/** Where a reply line keeps a failed Undo of saved learning `id`, shown
 *  beside that line's actions while the learning stays saved. */
const undoErrorKey = (id: string) => `undo:${id}`;

/** Drafts and errors of learnings no longer waiting go with them; a failed
 *  reply-line Undo goes once its learning is no longer saved. Skill lesson
 *  errors are `forgetSkillErrors`'. */
function forgetDecided(items: readonly LearningProposal[]) {
  const open = new Set(items.filter(p => p.status === 'pending' || p.status === 'applying').map(p => p.id));
  const saved = new Set(items.filter(p => p.status === 'accepted').map(p => undoErrorKey(p.id)));
  for (const id of Object.keys(actionDrafts)) if (!open.has(id)) setActionDrafts(id, undefined);
  for (const id of Object.keys(actionErrors)) if (id && !id.startsWith('skill:') && !open.has(id) && !saved.has(id)) setActionErrors(id, undefined);
}

/** Where a skill lesson keeps a failed Save, Reject or Undo: under the lesson
 *  and the status it was tried in, so the message shows on that lesson's card
 *  in Library and in the reply line alike, until the lesson moves on. */
const skillErrorKey = (p: { id: string; status: string }) => `skill:${p.id}:${p.status}`;

function forgetSkillErrors(lessons: readonly { id: string; status: string }[]) {
  const live = new Set(lessons.map(skillErrorKey));
  for (const key of Object.keys(actionErrors)) if (key.startsWith('skill:') && !live.has(key)) setActionErrors(key, undefined);
}

function reload(force = false): Promise<void> {
  if (inflight) { if (force) again = true; return inflight; }
  if (loaded() && !force) return Promise.resolve();
  inflight = (async () => {
    do {
      again = false;
      const [p, c, m, s] = await Promise.allSettled([api.listLearningProposals(), api.listLearningContext(true), api.memoryMaintenance(),api.listSkillLessons()]);
      if (p.status === 'fulfilled') { setProposals(p.value); forgetDecided(p.value); }
      if (c.status === 'fulfilled') setContexts(c.value);
      if (m.status === 'fulfilled') setMaintenance(m.value);
      if (s.status === 'fulfilled') { setSkillLessons(s.value); forgetSkillErrors(s.value); }
      setLoadError([p.status === 'rejected' ? `Proposals: ${errMsg(p.reason)}` : '', c.status === 'rejected' ? `Saved memories: ${errMsg(c.reason)}` : '', m.status === 'rejected' ? `Memory maintenance: ${errMsg(m.reason)}` : '',s.status === 'rejected' ? `Skill lessons: ${errMsg(s.reason)}` : ''].filter(Boolean).join(' '));
      if (p.status === 'fulfilled' && c.status === 'fulfilled') setLoaded(true);
    } while (again);
  })().finally(() => { inflight = null; });
  return inflight;
}

function useLearnings() {
  let mounted = false;
  createEffect(on(reviewInboxRevision, () => void reload(true)));
  onMount(() => {
    mounted = true;
    readers++;
    void reload();
    if (readers === 1) {
      stop = scopedListen(() => listen('learning-proposals-changed', () => void reload(true)));
    }
  });
  onCleanup(() => { if (mounted && --readers === 0) { stop?.(); stop = undefined; } });
}

export const openLearnings = openLibraryLearnings;

function skillState(status:string) { return status === 'accepted' ? 'saved' : status === 'applying' ? 'pending' : status; }

function SkillLessons(props:{runId?:string|null;state?:string}) {
  const items = () => skillLessons().filter(p => props.runId ? p.payload.sourceRunId === props.runId : !props.state || skillState(p.status) === props.state);
  return <Show when={items().length}><section aria-label="Skill lessons"><h3>Skill lessons</h3>
    <For each={items()}>{p => <article class="review-card"><strong>{p.status === 'accepted' ? 'Saved skill lesson' : p.status === 'rejected' && p.error?.startsWith('Superseded:') ? 'Stale skill lesson' : p.status === 'rejected' ? 'Rejected skill lesson' : p.payload.expectedHash ? 'Proposed skill edit' : 'Proposed new skill'}: {p.payload.name}</strong>
      <p>{p.payload.lesson}</p><blockquote>{p.payload.quote}</blockquote><p>{p.status}. {memorySource(p.payload.sourceRunId)}</p>
      <Show when={p.error}><ErrorText class="work-error" text={p.error!}/></Show>
      <Show when={p.status === 'accepted'}><div class="work-actions"><button class="btn small ghost" disabled={busy()} onClick={() => void change(() => api.undoSkillLesson(p.id), skillErrorKey(p))}>Undo skill lesson</button></div><p>Undo puts back the skill as it was before Save, or removes a new skill, while it is still exactly what Save wrote.</p></Show>
      <Show when={p.status === 'pending'}><div class="work-actions"><button class="btn small" disabled={busy()} onClick={() => void change(() => api.resolveSkillLesson(p.id,'accept'), skillErrorKey(p))}>Save skill lesson</button><button class="btn small ghost" disabled={busy()} onClick={() => void change(() => api.resolveSkillLesson(p.id,'reject'), skillErrorKey(p))}>Reject skill lesson</button></div><p>Save checks that the skill still matches its fresh-read snapshot. Existing instructions stay in place.</p></Show>
      <Show when={actionErrors[skillErrorKey(p)]}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
    </article>}</For>
  </section></Show>;
}

function PrivateMemories() {
  const [items,setItems] = createSignal<Awaited<ReturnType<typeof api.listPrivateMemories>>>([]);
  const [didRead,setDidRead] = createSignal(false);
  const [reading,setReading] = createSignal(false);
  const [failure,setFailure] = createSignal('');
  let generation = 0;
  createEffect(on(workspaceId, () => { generation++; setItems([]); setDidRead(false); setReading(false); setFailure(''); }));
  async function read() {
    const project = workspaceId(); if (!project) return;
    const token = ++generation; setReading(true); setFailure('');
    try { const result = await api.listPrivateMemories(project); if (token === generation) { setItems(result); setDidRead(true); } }
    catch(e) { if (token === generation) setFailure(errMsg(e)); }
    finally { if (token === generation) setReading(false); }
  }
  return <details class="private-memories"><summary>Agents' private memories</summary>
    <p>Read-only scratch notes for the selected Hollow project. They stay outside shared memory until you promote a passage and Save it. Provider files are never edited.</p>
    <button class="btn small" disabled={!workspaceId() || reading()} onClick={() => void read()}>{reading() ? 'Reading private memories' : 'Read this project’s private memories'}</button>
    <Show when={!workspaceId()}><p>Open a project first.</p></Show>
    <Show when={failure()}><ErrorText class="work-error" text={failure()}/></Show>
    <Show when={didRead() && !items().length}><p>No supported private memory was found for this folder. This reads Claude’s project memory Markdown and matching Codex registry groups; it does not scan other folders.</p></Show>
    <For each={items()}>{item => {
      const [statement,setStatement] = createSignal('');
      const [scope,setScope] = createSignal('knowhow');
      return <article class="review-card"><strong>{item.provider} scratch memory</strong><small>{item.path}</small><pre class="private-memory-text">{item.text}</pre>
        <label>Exact passage to promote<textarea maxlength={2000} value={statement()} onInput={e => setStatement(e.currentTarget.value)}/></label>
        <div class="work-actions"><Select label="Promote to a home" value={scope()} onChange={setScope} options={[{value:'knowhow',label:'Know-how'},{value:'you',label:'You'},{value:'project',label:'Project'}]}/>
        <button class="btn small" disabled={busy() || statement().trim().length < 12} onClick={() => {
          const project = workspaceId(); if (project) void change(() => api.promotePrivateMemory(project,item.id,item.expectedHash,scope(),statement()));
        }}>Propose promotion</button></div><p>Creates a pending learning. Save is still required.</p>
      </article>;
    }}</For>
  </details>;
}

/** Run one decision; true once it worked and the lists were read again. A
 *  failure is kept under `key`: the learning's id, or '' for the page. */
async function change(action: () => Promise<unknown>, key = ''): Promise<boolean> {
  if (busy()) return false;
  setBusy(true); setActionErrors(key, undefined);
  try {
    await action();
    refreshReviewInbox(); refreshWaitingOnYou(true);
    await reload(true);
    return true;
  } catch (e) { setActionErrors(key, errMsg(e)); return false; }
  finally { setBusy(false); }
}

/** Where focus goes once learning `id` is decided and its controls leave:
 *  in Learnings the next row's actions, else the previous row's, else the
 *  Learning state tabs; in a reply line, that learning's own result link.
 *  Read before the decision; moves focus only when it fell to the page. */
function focusAfterDecision(from: HTMLElement | undefined, id: string): () => void {
  const home = from?.closest<HTMLElement>('.learning-review, .learning-line');
  if (!home) return () => {};
  const ids = [...home.querySelectorAll<HTMLElement>('[data-learning-actions]')].map(el => el.dataset.learningId ?? '');
  const at = ids.indexOf(id);
  const near = at < 0 ? [] : [ids[at + 1], ids[at - 1]].filter(Boolean);
  return () => {
    const active = document.activeElement;
    if (!home.isConnected || (active && active !== document.body && active.isConnected)) return;
    const find = (selector: string) => home.querySelector<HTMLElement>(selector);
    const target = find(`[data-learning-result="${CSS.escape(id)}"]`)
      ?? near.map(other => find(`[data-learning-actions][data-learning-id="${CSS.escape(other)}"]`)).find(Boolean)
      ?? find('nav[aria-label="Learning state"] button[aria-pressed="true"]');
    target?.focus();
  };
}

/** Undo bulk review `batchId`, from its snackbar (which says a thrown error)
 *  or "Undo last batch". After a partial Undo the button stays for the
 *  learnings that stayed; once nothing is left to undo it goes. */
async function undoBatch(batchId: string) {
  try {
    const outcome = await api.undoLearningBatch(batchId);
    flashFooter(undoMessage(outcome).text, 5000);
    const batch = lastBatch();
    if (batch?.id === batchId) {
      const after = batchAfterUndo(batch, outcome);
      if (!after && batch.snackbar) dismissSnackbar(batch.snackbar);
      setLastBatch(after);
    }
  } catch (error) {
    if (nothingLeftToUndo(error)) forgetBatch(batchId);
    throw error;
  } finally {
    refreshReviewInbox(); refreshWaitingOnYou(true);
    await reload(true);
  }
}

async function undoLast() {
  const batch = lastBatch();
  if (!batch || busy()) return;
  setBusy(true); setActionErrors('', undefined);
  try { await undoBatch(batch.id); }
  catch (error) {
    if (nothingLeftToUndo(error)) flashFooter('Nothing left to undo: each learning in that batch was undone or decided again since.', 5000);
    else setActionErrors('', errMsg(error));
  } finally { setBusy(false); }
}

/** A Save or Reject that went through gets its batch's 10 s Undo snackbar
 *  and "Undo last batch". A row's Save is a one-item batch, edited words or
 *  not, so it gets them too (a reworded saved line gets its words back). */
function offerUndo(outcome: LearningBatchOutcome, action: 'accept' | 'reject', detail?: string) {
  const message = batchMessage(outcome).text;
  const batchId = outcome.batchId;
  if (!batchId) { flashFooter(message, 6000); return; }
  const snackbar = flashWithUndo({ message, detail, kind: 'learning-batch', ttlMs: 10000, undo: () => undoBatch(batchId) });
  setLastBatch({ id: batchId, action, count: outcome.items.filter(i => i.ok).length, snackbar });
}

function LearningActions(props: { item: LearningProposal }) {
  const id = props.item.id;
  if (!actionDrafts[id]) setActionDrafts(id, { ...NO_DRAFT });
  const draft = () => actionDrafts[id] ?? NO_DRAFT;
  const rejecting = () => draft().rejecting;
  const reason = () => draft().reason;
  const edited = () => draft().edited;
  const setRejecting = (value: boolean | ((old: boolean) => boolean)) => setActionDrafts(id, 'rejecting', value);
  const setReason = (value: string) => setActionDrafts(id, 'reason', value);
  const setEdited = (value: string) => setActionDrafts(id, old => ({
    ...(old ?? NO_DRAFT), edited: value, editedAt: pinEditVersion(old?.editedAt, props.item.version),
  }));
  let detail: HTMLDetailsElement | undefined;
  let editor: HTMLTextAreaElement | undefined;
  let group: HTMLDivElement | undefined;
  createEffect(() => { if (detail) detail.open = draft().open; });
  /** One decision on this learning; its controls leave once it worked, so
   *  focus moves on (`focusAfterDecision`) instead of falling to the page. */
  const decide = async (action: () => Promise<unknown>) => {
    const after = focusAfterDecision(group, id);
    if (await change(action, id)) after();
  };
  const save = () => decide(async () => {
    const result = await api.resolveLearningProposalsBatch([props.item.id], 'accept', undefined, editedWords(edited(), props.item.payload.statement), versionForSave(draft().editedAt, props.item.version));
    const failed = result.items.find(i => !i.ok);
    if (failed) throw new Error(failed.error ?? 'Save failed');
    offerUndo(result, 'accept');
  });
  const reject = () => decide(async () => {
    const result = await api.resolveLearningProposalsBatch([props.item.id], 'reject', reason().trim());
    if (result.items.some(i => !i.ok)) throw new Error(result.items.find(i => !i.ok)?.error ?? 'Reject failed');
  });
  const skip = () => decide(() => api.snoozeReviewItem(props.item.id, '9999-12-31T00:00:00Z'));
  function key(e: KeyboardEvent) {
    if (!shortcutsEnabled() || e.metaKey || e.ctrlKey || e.altKey || busy()
      || (e.target instanceof HTMLElement && e.target.closest('input,textarea,select,[contenteditable="true"]'))) return;
    if (e.key === 'e') { setActionDrafts(id, 'open', true); editor?.focus(); }
    else if (e.key === 'a') void save();
    else if (e.key === 'R' && e.shiftKey) setRejecting(true);
    else if (e.key === 'z') void skip();
    else return;
    e.preventDefault(); e.stopPropagation();
  }
  return <>
    <div ref={el => { group = el; }} class="work-actions" role="group" aria-label="Learning actions" tabindex={0} data-learning-actions data-learning-id={id} onKeyDown={key}>
      <button class="btn small" disabled={busy() || (edited() !== null && !edited()!.trim())} onClick={() => void save()}>Save</button>
      <button class="btn small ghost" disabled={busy()} onClick={() => setRejecting(v => !v)} aria-expanded={rejecting()}>Reject</button>
      <button class="btn small ghost" disabled={busy()} onClick={() => void skip()}>Skip</button>
      <details ref={el => { detail = el; el.open = draft().open; }} onToggle={() => setActionDrafts(id, 'open', detail?.open ?? false)}><summary>Edit and source</summary>
        <label>Learning<textarea ref={el => { editor = el; }} maxlength={2000} value={edited() ?? props.item.payload.statement} onInput={e => setEdited(e.currentTarget.value)} /></label>
        <Show when={props.item.payload.evidenceExcerpt}><blockquote>{props.item.payload.evidenceExcerpt}</blockquote></Show>
        <p>{props.item.payload.rationale}</p><small>{memorySource(props.item.payload.sourceRunId)}</small>
        <Show when={props.item.payload.possibleDuplicate}><p>May repeat: {props.item.payload.possibleDuplicate}</p></Show>
        <Show when={props.item.payload.retiresNote?.path}>{path => <p>Save also retires {path()}: it becomes a link to its line, so the fact has one home.</p>}</Show>
        <small>Save to {props.item.payload.targetPath}</small>
        <small>Keys: e edit, a save, Shift+R reject, z skip.</small>
      </details>
    </div>
    <Show when={rejecting()}><div class="work-actions"><label>Why reject this learning?
      <input maxlength={REJECT_REASON_MAX} value={reason()} onInput={e => setReason(e.currentTarget.value)} />
    </label><button class="btn small" disabled={busy() || !reason().trim()} onClick={() => void reject()}>Reject with reason</button></div></Show>
    <Show when={actionErrors[id]}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
  </>;
}

export function LearningLine(props: { runId?: string | null }) {
  useLearnings();
  const items = createMemo(() => props.runId ? proposals().filter(p => p.payload.sourceRunId === props.runId) : []);
  return <Show when={items().length || skillLessons().some(p => p.payload.sourceRunId === props.runId)}><div class="learning-line" role="note" aria-label="Learning from this reply">
    <SkillLessons runId={props.runId}/>
    <For each={items().map(p => p.id)}>{id => <Show when={proposals().find(p => p.id === id)}>{p => <div>
      <p><strong>{p().status === 'accepted' ? 'Saved learning' : p().status === 'rejected' ? 'Rejected learning' : p().status === 'snoozed' ? 'Skipped learning' : 'Learning suggestion'}:</strong> {p().payload.statement}</p>
      <Show when={p().status === 'pending'} fallback={<>
        <Show when={p().status === 'accepted' && autoSaveUndoBatch(p().id, p().payload.batchId)}>{batch => <button class="btn-link" disabled={busy()} onClick={() => void change(async () => { const result = await api.undoLearningBatch(batch()!); if (result.items.some(i => !i.ok)) throw new Error(result.items.find(i => !i.ok)?.error ?? 'Undo failed'); }, undoErrorKey(id))}>Undo saved learning</button>}</Show>
        <button class="btn-link" data-learning-result={id} onClick={openLearnings}>Open in Learnings</button>
        <Show when={p().status === 'accepted' && actionErrors[undoErrorKey(id)]}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
      </>}><LearningActions item={p()}/></Show>
    </div>}</Show>}</For>
  </div></Show>;
}

export function Learnings() {
  useLearnings();
  const [tab, setTab] = createSignal<'pending' | 'saved' | 'skipped' | 'rejected'>('pending');
  const [replacements, setReplacements] = createSignal<Record<string, string>>({});
  const [savedState,setSavedState] = createSignal('all');
  const used = createMemo(() => new Set(activeChatRecall().map(r => r.path)));
  const contextAt = (path?: string | null) => contexts().find(c => c.path === path);
  const includeSaved = (path?: string | null) => savedState() === 'all' || !!contextAt(path)?.supersededBy === (savedState() === 'replaced');
  const pending = createMemo(() => proposals().filter(p => p.status === 'pending' || p.status === 'applying'));
  const skipped = createMemo(() => proposals().filter(p => p.status === 'snoozed'));
  const rejected = createMemo(() => proposals().filter(p => p.status === 'rejected'));
  const accepted = createMemo(() => proposals().filter(p => p.status === 'accepted'));
  const legacy = createMemo(() => contexts().filter(c => !accepted().some(p => p.payload.learnedPath === c.path)));
  const selected = createMemo(() => pending().filter(p => p.status === 'pending' && picked().has(p.id)).slice(0, BATCH_MAX).map(p => p.id));
  /** Selected learnings the duplicate check flagged: still selectable, but said. */
  const repeats = () => pending().filter(p => selected().includes(p.id) && !!p.payload.possibleDuplicate?.trim()).length;
  const toggle = (id: string) => setPicked(old => { const next = new Set(old); next.has(id) ? next.delete(id) : next.add(id); return next; });
  const subjectOf = (id: string) => proposals().find(p => p.id === id)?.payload.subject;
  /** Save or Reject the selection, each learning on its own: any that
   *  couldn't be stay ticked. A batch gets the 10 s Undo snackbar and
   *  "Undo last batch" until nothing in it is left to undo. */
  const bulk = async (action: 'accept' | 'reject') => {
    let outcome = null as LearningBatchOutcome | null;
    const ids = selected();
    if (!await change(async () => { outcome = await api.resolveLearningProposalsBatch(ids, action, action === 'reject' ? bulkReason().trim() : undefined); }) || !outcome) return;
    const bad = failures(outcome);
    setPicked(new Set(bad.map(f => f.id)));
    if (!bad.length) setBulkReason('');
    offerUndo(outcome, action, failureDetail(outcome, subjectOf));
  };
  const skillCount = (state:string) => skillLessons().filter(p => skillState(p.status) === state).length;
  const saved = () => accepted().length + legacy().length + skillCount('saved');
  const MaintenanceControls = (props: { entryKey: string }) => {
    const info = () => maintenance()?.entries.find(e => e.key === props.entryKey);
    const [expiry, setExpiry] = createSignal<string | null>(null);
    const edit = (action: 'pin' | 'unpin' | 'restore' | 'expiry') => change(async () => {
      const snapshot = info(); if (!snapshot) return;
      setMaintenance(await api.editMemoryMaintenance(props.entryKey,snapshot.expectedHash,action,action === 'expiry' ? (expiry() ?? snapshot.expiresOn ?? '') : undefined));
    });
    const usage = (e: NonNullable<ReturnType<typeof info>>) => e.kind === 'note' ? `Recalled ${e.uses} times.` : e.ages ? `Read in chats in its workspace on ${e.uses} days.` : 'Always on in every chat.';
    return <Show when={info()}>{entry => <Show when={entry().status !== 'changed'} fallback={<div class="memory-maintenance"><p>changed. This line was edited by hand since it was saved, so Hollow leaves it alone.</p></div>}><div class="memory-maintenance">
      <p>{entry().status}{entry().pinned ? ', pinned' : ''}. {usage(entry())} <Show when={entry().lastUsed}>{date => <>Last used {new Date(date()).toLocaleDateString()}.</>}</Show> <Show when={entry().expiresOn}>{date => <>Valid through {date()}.</>}</Show></p>
      <Show when={entry().proposalId}><p role="note">A proposal to remove or update this line waits in Pending. The line changes only if you save it.</p></Show>
      <div class="work-actions"><button class="btn small ghost" disabled={busy()} onClick={() => void edit(entry().pinned ? 'unpin' : 'pin')}>{entry().pinned ? 'Unpin' : 'Pin to keep'}</button>
      <Show when={entry().status !== 'active'}><button class="btn small" disabled={busy()} onClick={() => void edit('restore')}>{entry().kind === 'line' ? 'Keep this line' : 'Restore memory'}</button></Show>
      <details><summary>Time-bound fact</summary><label>Valid through<input type="date" value={expiry() ?? entry().expiresOn ?? ''} onInput={e => setExpiry(e.currentTarget.value)}/></label><button class="btn small" disabled={busy()} onClick={() => void edit('expiry')}>Save expiry</button><p>Blank means no expiry. Expired facts stay searchable and restorable.</p></details></div>
    </div></Show>}</Show>;
  };

  // One array whose labels read the counts, so a reload never re-creates
  // the tab buttons (focus can rest on one after a decision).
  const states: TabItem<'pending' | 'saved' | 'skipped' | 'rejected'>[] = [
    { value: 'pending', get label() { return `Pending (${pending().length + skillCount('pending')})`; } },
    { value: 'saved', get label() { return `Saved (${saved()})`; } },
    { value: 'skipped', get label() { return `Skipped (${skipped().length})`; } },
    { value: 'rejected', get label() { return `Rejected (${rejected().length + skillCount('rejected')})`; } },
  ];
  const SavedProvenance = (props: { path?: string | null; target: string; source?: string | null; date: string }) => <>
    <span class="review-card-chips"><span class="settings-state-chip">{contextAt(props.path)?.supersededBy ? 'outdated' : 'trusted'}</span><Show when={props.path && used().has(props.path)}><span class="settings-state-chip" data-state="ok">used in this conversation</span></Show></span>
    <dl class="review-provenance"><dt>From</dt><dd>{memorySource(props.source)}</dd><dt>Saved to</dt><dd><code>{props.target}</code> · approved {new Date(props.date).toLocaleDateString()}</dd><Show when={contextAt(props.path)?.supersedes}>{old => <><dt>Replaces</dt><dd><button class="btn-link" onClick={() => openVaultNote(old())}>{contextAt(old())?.subject || old()}</button></dd></>}</Show></dl>
  </>;
  const SavedControls = (props: { item: LearningContext }) => <Show when={props.item.supersededBy} fallback={<details class="review-supersede"><summary>Replace with a saved learning</summary>
    <Select label={`Replacement for ${props.item.subject}`} value={replacements()[props.item.path] ?? ''} onChange={value => setReplacements(old => ({...old,[props.item.path]:value}))} options={[{value:'',label:'Choose replacement'},...contexts().filter(next => next.path !== props.item.path && !next.supersededBy).map(next => ({value:next.path,label:next.subject}))]}/>
    <button class="btn small" disabled={busy() || !replacements()[props.item.path]} onClick={() => void change(() => api.supersedeLearningContext(props.item.path,replacements()[props.item.path]))}>Mark replaced</button>
  </details>}>{path => <p>Replaced by <button class="btn-link" onClick={() => openVaultNote(path())}>{path()}</button></p>}</Show>;
  function move(e: KeyboardEvent) {
    if (!shortcutsEnabled() || e.metaKey || e.ctrlKey || e.altKey || !['j','k'].includes(e.key)
      || !(e.target instanceof HTMLElement) || e.target.closest('input,textarea,select,[contenteditable="true"]')) return;
    const section = e.currentTarget as HTMLElement;
    const actions = [...section.querySelectorAll<HTMLElement>('[data-learning-actions]')];
    if (!actions.length) return;
    const at = actions.findIndex(item => item.contains(e.target as Node));
    const next = at < 0 ? (e.key === 'j' ? 0 : actions.length - 1) : Math.max(0,Math.min(actions.length - 1,at + (e.key === 'j' ? 1 : -1)));
    actions[next].focus(); e.preventDefault(); e.stopPropagation();
  }
  return <section class="learning-review" aria-label="Learnings" onKeyDown={move}>
    <h2>Learnings</h2><p class="settings-row-v">What Hollow learned, the quote behind it and where it was saved. Grounded Know-how can save itself with Undo. You and Project rules wait for Save.</p>
    <Suspense fallback={null}><ContextReviewCard/></Suspense>
    <PrivateMemories/>
    <Show when={maintenance()}>{m => <div class="memory-core-size"><p>Profile source files: {m().coreBytes.toLocaleString()} bytes. Consolidation review threshold: {m().coreBudgetBytes.toLocaleString()} bytes.</p><Show when={m().consolidationProposal}>{proposal => <p role="note">{proposal()}</p>}</Show><p>This measures the five profile Markdown files before prompt compilation. Know-how becomes stale after 14 quiet days and archives after 30. Archived facts stay in Saved, searchable and restorable. Pin keeps a fact active; profile rules never age automatically. A saved You or Project line past its valid-through date, or a Project line whose workspace had no chat for 14 days, gets a proposal in Pending; the line changes only if you save it.</p></div>}</Show>
    <Show when={activeChatRecall().length}><p class="settings-row-v learning-used" role="status">This conversation's last turn recalled {activeChatRecall().length} memories: {activeChatRecall().map(r => r.subject).join(', ')}.</p></Show>
    <Show when={loadError()}><ErrorText class="work-error" text={loadError()}/><button class="btn small" onClick={() => void reload(true)}>Try again</button></Show>
    <Show when={actionErrors['']}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
    <Show when={!loaded() && !loadError()}><Skeleton label="Reading learnings" rows={3}/></Show>
    <Show when={loaded()}>
    <Tabs label="Learning state" value={tab()} onChange={setTab} items={states}/>
    <SkillLessons state={tab()}/>
    <Show when={lastBatch()}>{batch => <button class="btn small ghost" disabled={busy()} onClick={() => void undoLast()}>{undoLastLabel(batch().action, batch().count)}</button>}</Show>
    <Show when={tab() === 'pending'}>
      {/* The profile builder's questions and suggestions: You items in this one queue (Settings PR 4). */}
      <ProfileSuggestions />
      <Show when={pending().length > 1}><div class="work-actions" role="group" aria-label="Bulk review learnings">
        <label><input type="checkbox" checked={selected().length === Math.min(pending().filter(p => p.status === 'pending').length, BATCH_MAX)} onChange={e => setPicked(new Set(e.currentTarget.checked ? pending().filter(p => p.status === 'pending').slice(0,BATCH_MAX).map(p => p.id) : []))}/>Select all learnings (up to {BATCH_MAX})</label>
        <Show when={repeats()}><span class="learning-repeats" role="status">{repeats() === 1 ? 'One may repeat a saved learning' : `${repeats()} may repeat a saved learning`}</span></Show>
        <label>Reason to reject selected<input maxlength={REJECT_REASON_MAX} value={bulkReason()} onInput={e => setBulkReason(e.currentTarget.value)}/></label>
        <button class="btn small" disabled={busy() || !selected().length} onClick={() => void bulk('accept')}>Save selected ({selected().length})</button>
        <button class="btn small ghost" disabled={busy() || !selected().length || !bulkReason().trim()} onClick={() => void bulk('reject')}>Reject selected</button>
      </div></Show>
      <Show when={pending().length} fallback={skillCount('pending') || profileWaitingCount() ? null : <p>No pending learnings.</p>}><For each={pending().map(p => p.id)}>{id => <Show when={proposals().find(p => p.id === id)}>{p => <article class="review-card">
        <label><input type="checkbox" checked={picked().has(id)} disabled={busy() || p().status !== 'pending'} onChange={() => toggle(id)}/> {p().payload.subject}</label>
        <p>{p().payload.statement}</p><Show when={p().error}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show><Show when={p().status === 'pending'} fallback={<p>Saving this learning.</p>}><LearningActions item={p()}/></Show>
      </article>}</Show>}</For></Show>
    </Show>
    <Show when={tab() === 'saved'}>
      <Select label="Saved learning state" value={savedState()} onChange={setSavedState} options={[{value:'all',label:'All saved memories'},{value:'current',label:'Current memories'},{value:'replaced',label:'Replaced memories'}]}/>
      <Show when={!saved()}><p>No saved learnings.</p></Show>
      <For each={accepted().filter(p => includeSaved(p.payload.learnedPath))}>{p => <article class="review-card" data-used={p.payload.learnedPath && used().has(p.payload.learnedPath) ? 'true' : undefined}><strong>{p.payload.subject}</strong><p>{p.payload.statement}</p>
        <Show when={autoSaveUndoBatch(p.id, p.payload.batchId)}>{batch => <button class="btn small ghost" disabled={busy()} onClick={() => void change(async () => { const result = await api.undoLearningBatch(batch()); if (result.items.some(i => !i.ok)) throw new Error(result.items.find(i => !i.ok)?.error ?? 'Undo failed'); })}>Undo saved learning</button>}</Show>
        <SavedProvenance path={p.payload.learnedPath} target={p.payload.targetPath} source={p.payload.sourceRunId} date={p.updatedAt}/>
        <Show when={p.payload.learnedPath || p.payload.alreadyIn}>{path => <button class="btn-link" onClick={() => openVaultNote(path())}>Open saved home</button>}</Show>
        <Show when={contexts().find(c => c.path === p.payload.learnedPath)}>{c => <SavedControls item={c()}/>}</Show>
        <Show when={p.payload.learnedLine ? `line:${p.id}` : p.payload.learnedPath}>{key => <MaintenanceControls entryKey={key()}/>}</Show>
      </article>}</For>
      <For each={legacy().filter(c => includeSaved(c.path))}>{c => <article class="review-card" data-used={used().has(c.path) ? 'true' : undefined}><strong>{c.subject}</strong><p>{c.statement}</p>
        <SavedProvenance path={c.path} target={c.targetPath} source={c.sourceRunId} date={c.learnedAt}/>
        <button class="btn-link" onClick={() => openVaultNote(c.path)}>Open saved home</button>
        <SavedControls item={c}/>
        <MaintenanceControls entryKey={c.path}/>
      </article>}</For>
    </Show>
    <Show when={tab() === 'skipped'}><Show when={skipped().length} fallback={<p>No skipped learnings.</p>}><For each={skipped()}>{p => <article class="review-card"><strong>{p.payload.subject}</strong><p>{p.payload.statement}</p><button class="btn small" disabled={busy()} onClick={() => void change(() => api.wakeReviewItem(p.id))}>Review again</button></article>}</For></Show></Show>
    <Show when={tab() === 'rejected'}><Show when={rejected().length} fallback={skillCount('rejected') ? null : <p>No rejected learnings.</p>}><For each={rejected()}>{p => <article class="review-card"><strong>{p.payload.subject}</strong><p>{p.payload.statement}</p><p>Target: {p.payload.targetPath}</p><Show when={p.payload.rationale}><p>Why proposed: {p.payload.rationale}</p></Show><p>Reason: {p.payload.rejectReason || 'No reason recorded.'}</p><button class="btn small" disabled={busy()} onClick={() => void change(() => api.undoReviewReject(p.id,p.updatedAt))}>Review again</button></article>}</For></Show></Show>
    </Show>
  </section>;
}
