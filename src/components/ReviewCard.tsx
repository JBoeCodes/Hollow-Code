import { For, Show, createMemo, createSignal } from 'solid-js';
import { api } from '../api';
import { shortcutsEnabled } from '../shortcuts';
import { refreshCommitments } from '../commitments';
import { refreshReviewInbox } from '../reviewInbox';
import { enableWorkflowsResource, flashFooter, flashWithUndo, openVaultNote, refetchTasks, refetchProjects, refetchWorkflows, refetchBudgets, showConfirm, workflowsList } from '../store';
import type { BudgetMetric, WorkflowPermissionDiffEntry } from '../types';
import { canResume, formatBudgetAmount, suggestionPrefill } from '../budgets';
import { errMsg } from '../utils';
import { pinEditVersion, versionForSave } from '../learningBulk';
import { REVIEW_CARD_ACTION, reviewKey, snoozeUntil, wakeLabel } from '../reviewKeys';
import {
  isWorkspaceReviewItem, WORKSPACE_REVIEW_KINDS as KIND_LABELS, WORKSPACE_REVIEW_SOURCES as SOURCE_LABELS, type WorkspaceReviewItem,
} from '../workspaceContent';
import { permissionConfirmBody, permissionPayload, reviewStatusLabel } from '../workflowPermissions';
import { canPauseStreak, streakPayload, streakSummary } from '../workflowHealth';
import { defaultTicked, gardenActionLabel, gardenLines, gardenSelection } from '../gardener';
import '../styles/budget-pauses.css';
import { PlainErrorText } from './ui';
import { rejectedLine } from '../decisionRows';

/** Permission reviews are approve/reject only: no editing, no snooze. */
function isPermissionReview(item: WorkspaceReviewItem): boolean {
  return item.kind === 'workflow_permissions';
}

/** Failure-streak cards (Paperclip PR 9): Pause schedule / Dismiss, and
 *  snooze; nothing to edit. */
function isStreakReview(item: WorkspaceReviewItem): boolean {
  return item.kind === 'workflow_failure_streak';
}

function isBudgetPause(item: WorkspaceReviewItem): boolean {
  return item.kind === 'budget_pause';
}

/** What a card offers, per kind. Kinds whose approval is re-derived outside
 *  the vault (permission changes, budget pauses) are never edited or
 *  snoozed, so `e`, `z`, Edit and Tomorrow do nothing for them; failure
 *  streaks have nothing to edit but can be snoozed. */
interface KindActions {
  editable: boolean;
  snoozable: boolean;
  acceptLabel: string;
  rejectLabel: string;
}

const DEFAULT_ACTIONS: KindActions = { editable: true, snoozable: true, acceptLabel: 'Accept', rejectLabel: 'Reject' };

const KIND_ACTIONS: Partial<Record<WorkspaceReviewItem['kind'], KindActions>> = {
  workflow_permissions: { editable: false, snoozable: false, acceptLabel: 'Approve…', rejectLabel: 'Reject' },
  // Failure streaks (Paperclip PR 9): nothing to edit, but snoozable.
  workflow_failure_streak: { editable: false, snoozable: true, acceptLabel: 'Pause schedule', rejectLabel: 'Dismiss' },
  budget_pause: { editable: false, snoozable: false, acceptLabel: 'Raise and resume', rejectLabel: 'Keep paused' },
  // The gardener (master plan #19): tick lines, Apply ticked applies only
  // those ("Do it" is only the Main agent's suggestions, P1-03).
  gardener: { editable: false, snoozable: true, acceptLabel: 'Apply ticked', rejectLabel: 'Keep all' },
  // A field changed on a companion device and on the Mac (companion protocol v1).
  companion_conflict: { editable: false, snoozable: true, acceptLabel: "Use the device's value", rejectLabel: "Keep the Mac's" },
  // A companion op sent more than 14 days ago waits for the person (companion protocol).
  companion_old_op: { editable: false, snoozable: true, acceptLabel: 'Apply it', rejectLabel: 'Discard it' },
  // A stale memory or home file (ME2): the person rechecks and re-dates it.
  memory_recheck: { editable: false, snoozable: true, acceptLabel: 'Rechecked', rejectLabel: 'Dismiss' },
  // NV7: a chat that missed a skill. Add needs the reviewer's own expected
  // skill (written under Edit); the chat's answer is never the expectation.
  eval_candidate: { editable: true, snoozable: true, acceptLabel: 'Add as eval case', rejectLabel: 'Not a case' },
};

function actionsFor(item: WorkspaceReviewItem): KindActions {
  return KIND_ACTIONS[item.kind] ?? DEFAULT_ACTIONS;
}

function num(payload: Record<string, unknown>, key: string): number | null {
  const value = payload[key];
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function budgetMetric(item: WorkspaceReviewItem): BudgetMetric {
  return text(item.payload, 'metric') === 'costUsd' ? 'costUsd' : 'tokens';
}

/** A budget pause's facts (display only: the backend re-derives them). */
/** The gardener's lines, each with its tick, reason and undo. */
function GardenerLines(props: { item: WorkspaceReviewItem; ticked: string[]; toggle: (id: string) => void }) {
  const lines = () => gardenLines(props.item.payload);
  return (
    <ul class="review-gardener-lines">
      <For each={lines()}>
        {line => (
          <li>
            <label onClick={event => event.stopPropagation()}>
              <input
                type="checkbox"
                disabled={line.action === 'manual' || line.action === 'resolve' || props.item.status !== 'pending'}
                checked={props.ticked.includes(line.id)}
                onChange={() => props.toggle(line.id)}
              />
              <span><strong>{line.title}</strong> · {gardenActionLabel(line.action)}</span>
            </label>
            <p class="settings-note">{line.reason} Undo: {line.undo}</p>
          </li>
        )}
      </For>
    </ul>
  );
}

function BudgetPauseDetails(props: { item: WorkspaceReviewItem }) {
  const metric = () => budgetMetric(props.item);
  const period = () => text(props.item.payload, 'window') || 'window';
  const estimated = () => props.item.payload.estimated === true;
  return (
    <dl class="review-budget-facts">
      <div><dt>Scope</dt><dd>{text(props.item.payload, 'scopeLabel') || text(props.item.payload, 'scope')}</dd></div>
      <div><dt>This {period()}</dt><dd>{formatBudgetAmount(metric(), num(props.item.payload, 'observed'), estimated())} of {formatBudgetAmount(metric(), num(props.item.payload, 'limit'))}</dd></div>
      <div><dt>Paused</dt><dd>{shortDate(text(props.item.payload, 'pausedAt'))}</dd></div>
    </dl>
  );
}

function text(payload: Record<string, unknown>, key: string): string {
  const value = payload[key];
  return typeof value === 'string' ? value : '';
}

export function itemTitle(item: WorkspaceReviewItem): string {
  const payload = item.payload;
  return text(payload, 'subject') || text(payload, 'title') || text(payload, 'projectName') || text(payload, 'text') || KIND_LABELS[item.kind];
}

/** Suggestions whose acceptance creates a task: heartbeat, meeting and
 *  agent-proposed (hollow_tasks, Paperclip PR 12). One editor, one refresh. */
function isTaskProposal(item: WorkspaceReviewItem): boolean {
  return item.kind === 'meeting_task' || item.kind === 'heartbeat_task' || item.kind === 'agent_task';
}

function itemSummary(item: WorkspaceReviewItem): string {
  const payload = item.payload;
  if (item.kind === 'learning') return text(payload, 'statement');
  if (item.kind === 'commitment') {
    const counterparty = text(payload, 'counterparty');
    if (text(payload, 'direction') === 'unknown') {
      return `Who owes whom isn't clear (with ${counterparty || 'someone'}): choose before accepting.`;
    }
    const direction = text(payload, 'direction') === 'owed_to_me' ? 'owes me' : 'I owe';
    return counterparty ? `${direction} ${counterparty}` : text(payload, 'context');
  }
  if (item.kind === 'project_activation') {
    return text(payload, 'desiredState') || text(payload, 'currentState') || 'Fill missing project outcome fields.';
  }
  if (item.kind === 'goal') return text(payload, 'why') || text(payload, 'successEvidence');
  if (isTaskProposal(item)) return text(payload, 'note') || text(payload, 'title');
  if (item.kind === 'workflow_permissions') return text(payload, 'summary') || 'Automation settings changed outside the editor.';
  if (item.kind === 'workflow_failure_streak') return streakSummary(streakPayload(item));
  if (item.kind === 'record') {
    const who = text(payload, 'kind') === 'person_fact' ? text(payload, 'counterparty') : text(payload, 'owner');
    const what = text(payload, 'kind') === 'person_fact' ? 'About' : 'Decided by';
    const at = [text(payload, 'sourceTitle'), text(payload, 'at')].filter(Boolean).join(' · ');
    return [who ? `${what} ${who}` : `${what}: not stated`, at].filter(Boolean).join(' — ');
  }
  if (item.kind === 'person_alias') {
    const where = [text(payload, 'sourceTitle'), text(payload, 'anchor')].filter(Boolean).join(' · ');
    const many = Array.isArray(payload.candidates) && payload.candidates.length > 1;
    return `${many ? 'Pick who it is under Edit.' : 'Accept links the name; Hollow never guesses.'}${where ? ` Heard in ${where}.` : ''}`;
  }
  if (item.kind === 'eval_candidate') return evalCandidateSummary(payload);
  if (item.kind === 'gardener') return 'Tick what to tidy. Nothing is deleted: each line says how to undo it, and Notion is never changed.';
  if (item.kind === 'budget_pause') {
    return `Automations under this budget wait until you raise its limit above this ${text(payload, 'window') || 'window'}'s usage, or until it resets. Keep paused holds them past the reset. Chat keeps working.`;
  }
  return text(payload, 'body') || text(payload, 'message') || text(payload, 'context') || text(payload, 'description');
}

function evidence(item: WorkspaceReviewItem): string {
  return text(item.payload, 'evidenceExcerpt');
}

/** Set by Hollow only when the optional Jev duplicate check leans toward a
 *  match (`jev_decide.rs`); a hint for the reviewer, never an action. */
function possibleDuplicate(item: WorkspaceReviewItem): string {
  return isTaskProposal(item) || item.kind === 'learning' || item.kind === 'record' ? text(item.payload, 'possibleDuplicate') : '';
}

/** One home per fact (PX5): the saved learning note a learning card's Save
 *  retires (it becomes a link to its me/ line). */
function retiresNote(item: WorkspaceReviewItem): string {
  const note = item.kind === 'learning' ? item.payload.retiresNote : undefined;
  return note && typeof note === 'object' ? text(note as Record<string, unknown>, 'path') : '';
}

/** DR1: a proposed decision's turned-down options, so the person sees them
 *  before accepting (read only; an agent may have written them). */
function rejectedNote(item: WorkspaceReviewItem): string {
  return item.kind === 'record' ? rejectedLine(item.payload.rejected) : '';
}

function shortDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

function typingTarget(target: EventTarget | null): boolean {
  const element = target as HTMLElement | null;
  return !!element?.closest('input, textarea, select, button, a, summary, [role="button"], [contenteditable="true"]');
}

/** NV7: what an eval candidate card says (the redacted ask, what the turn
 *  loaded), and that the expected skill is the reviewer's to write. */
function evalCandidateSummary(payload: Record<string, unknown>): string {
  const ask = text(payload, 'prompt').replace(/\s+/g, ' ').trim();
  const shown = ask.length > 220 ? `${ask.slice(0, 220)}…` : ask;
  const loaded = Array.isArray(payload.skillsLoaded) ? (payload.skillsLoaded as unknown[]).filter(s => typeof s === 'string') as string[] : null;
  const used = loaded == null ? '' : loaded.length ? ` It loaded ${loaded.join(', ')}.` : ' It loaded no skill.';
  return `Add as an eval case? Asked: “${shown}”.${used} Write the expected skill under Edit.`;
}

function ReviewEditor(props: {
  item: WorkspaceReviewItem;
  draft: Record<string, unknown>;
  setField: (key: string, value: string) => void;
}) {
  const field = (key: string) => text(props.draft, key);
  return (
    <div class="review-editor">
      {/* NV7: blank until the reviewer writes it; the hint is not prefilled. */}
      <Show when={props.item.kind === 'eval_candidate'}>
        <EditInput label="Expected skill" value={field('expectedSkill')} placeholder={`e.g. ${text(props.item.payload, 'missedSkill') || 'a skill name'}`} onInput={value => props.setField('expectedSkill', value)} />
        <EditArea label="What a good run does (optional)" value={field('expectedNote')} onInput={value => props.setField('expectedNote', value)} />
      </Show>
      <Show when={props.item.kind === 'learning'}>
        <EditArea label="Approved wording" value={field('statement')} onInput={value => props.setField('statement', value)} />
      </Show>
      <Show when={props.item.kind === 'commitment'}>
        <EditInput label="Commitment" value={field('title')} onInput={value => props.setField('title', value)} />
        {/* Master plan #2: an extracted promise whose direction the source left
            unclear arrives as "unknown"; only the person picks. */}
        <label class="review-edit-label"><span>Who owes it</span>
          <select value={field('direction')} onChange={event => props.setField('direction', event.currentTarget.value)}>
            <Show when={field('direction') === 'unknown'}><option value="unknown">Not clear: choose one</option></Show>
            <option value="owed_by_me">I owe {text(props.draft, 'counterparty') || 'them'}</option>
            <option value="owed_to_me">{text(props.draft, 'counterparty') || 'They'} owes me</option>
          </select>
        </label>
        <EditInput label="Due date" value={field('due')} placeholder="YYYY-MM-DD or blank" onInput={value => props.setField('due', value)} />
        <EditArea label="Context" value={field('context')} onInput={value => props.setField('context', value)} />
      </Show>
      {/* Master plan F4: an extracted decision or fact. A fact whose person
          isn't exactly one people/ note waits for the person to pick it. */}
      <Show when={props.item.kind === 'record'}>
        <EditInput label={text(props.item.payload, 'kind') === 'person_fact' ? 'Fact' : 'Decision'} value={field('text')} onInput={value => props.setField('text', value)} />
        <Show when={text(props.item.payload, 'kind') === 'decision'}>
          <EditInput label="Decided by" value={field('owner')} placeholder="A name, or blank if not said" onInput={value => props.setField('owner', value)} />
          <EditInput label="Scope" value={field('scope')} placeholder="global, or a workspace id (blank: the recording's workspace)" onInput={value => props.setField('scope', value)} />
        </Show>
        <Show when={text(props.item.payload, 'kind') === 'person_fact'}>
          <EditInput label="About" value={field('person')} placeholder={`people/${text(props.item.payload, 'counterparty') || 'Name'}.md`} onInput={value => props.setField('person', value)} />
        </Show>
      </Show>
      {/* Master plan #3: which people/ note a name means. */}
      <Show when={props.item.kind === 'person_alias'}>
        <label class="review-edit-label"><span>Who is '{text(props.item.payload, 'name')}'?</span>
          <select value={field('person')} onChange={event => props.setField('person', event.currentTarget.value)}>
            <option value="">{(props.item.payload.candidates as unknown[] | undefined)?.length ? 'Choose a person' : 'Add a new page'}</option>
            <Show when={(props.item.payload.candidates as unknown[] | undefined)?.length}><option value="__new__">Someone else: add a new page</option></Show>
            <For each={(Array.isArray(props.item.payload.candidates) ? props.item.payload.candidates : []) as string[]}>{path => <option value={path}>{path.replace(/^people\//, '').replace(/\.md$/, '')}</option>}</For>
          </select>
        </label>
      </Show>
      <Show when={props.item.kind === 'project_activation'}>
        <For each={[
          ['currentState', 'Current state'],
          ['desiredState', 'Desired state'],
          ['nonGoals', 'Non-goals'],
          ['successEvidence', 'Success evidence'],
        ] as Array<[string, string]>}>
          {([key, label]) => (
            <Show when={key in props.draft}>
              <EditArea label={label} value={field(key)} onInput={value => props.setField(key, value)} />
            </Show>
          )}
        </For>
      </Show>
      <Show when={props.item.kind === 'goal'}>
        <EditInput label="Goal" value={field('title')} onInput={value => props.setField('title', value)} />
        <EditArea label="Why" value={field('why')} onInput={value => props.setField('why', value)} />
        <EditArea label="Success evidence" value={field('successEvidence')} onInput={value => props.setField('successEvidence', value)} />
      </Show>
      <Show when={isTaskProposal(props.item)}>
        <EditInput label="Task" value={field('title')} onInput={value => props.setField('title', value)} />
        <EditInput label="Due date" value={field('due')} placeholder="YYYY-MM-DD or blank" onInput={value => props.setField('due', value)} />
        <EditInput label="Priority" value={field('priority')} placeholder="P1–P4 or blank" onInput={value => props.setField('priority', value)} />
        <EditArea label="Context" value={field('note')} onInput={value => props.setField('note', value)} />
      </Show>
    </div>
  );
}

function PermissionDiffEntry(props: { entry: WorkflowPermissionDiffEntry }) {
  // Entries are replaced wholesale when the payload refreshes, so reading the
  // discriminant once per row is safe.
  const entry = props.entry;
  if (entry.kind === 'scalar') {
    return (
      <div class="review-permission-row" data-kind="scalar">
        <span class="review-permission-field">{entry.field}</span>
        <span class="review-permission-value">
          <span class="review-permission-before">{entry.before ?? '(none)'}</span>
          {' → '}
          <span class="review-permission-after">{entry.after ?? '(none)'}</span>
        </span>
      </div>
    );
  }
  if (entry.kind === 'list') {
    return (
      <div class="review-permission-row" data-kind="list">
        <span class="review-permission-field">{entry.field}</span>
        <span class="review-permission-value">
          <For each={entry.added}>{value => <span class="review-permission-chip added">+{value}</span>}</For>
          <For each={entry.removed}>{value => <span class="review-permission-chip removed">−{value}</span>}</For>
          <Show when={entry.reordered}><span class="review-permission-chip">reordered</span></Show>
        </span>
      </div>
    );
  }
  return (
    <div class="review-permission-row" data-kind="text">
      <span class="review-permission-field">prompt{entry.truncated ? ' (excerpt)' : ''}</span>
      <div class="review-permission-prompt">
        <Show when={entry.before !== null} fallback={<span class="review-permission-note">No approved version to compare with.</span>}>
          <span class="review-permission-note">Approved</span>
          <pre class="review-permission-pre">{entry.before}</pre>
        </Show>
        <span class="review-permission-note">Now</span>
        <pre class="review-permission-pre">{entry.after}</pre>
      </div>
    </div>
  );
}

function PermissionDiff(props: { item: WorkspaceReviewItem }) {
  const payload = createMemo(() => permissionPayload(props.item));
  return (
    <Show when={payload()}>{p => (
      <div class="review-permission-diff" aria-label="Changed automation settings">
        <For each={p().diff}>{entry => <PermissionDiffEntry entry={entry} />}</For>
      </div>
    )}</Show>
  );
}

function EditInput(props: { label: string; value: string; placeholder?: string; onInput: (value: string) => void }) {
  return <label class="review-edit-label"><span>{props.label}</span><input value={props.value} placeholder={props.placeholder} onInput={event => props.onInput(event.currentTarget.value)} /></label>;
}

function EditArea(props: { label: string; value: string; onInput: (value: string) => void }) {
  return <label class="review-edit-label"><span>{props.label}</span><textarea rows={3} value={props.value} onInput={event => props.onInput(event.currentTarget.value)} /></label>;
}

/** One review item, with every action its kind offers: the card the
 *  "waiting on you" surface opens by the item's exact review id (decision 7;
 *  the five per-page review inboxes it came from are gone). Every action here
 *  is a person's: only a person accepts, rejects, edits or snoozes a review
 *  item or proposal (decision 3), and the backend refuses an agent caller.
 *  Keys while the card has focus (`reviewKeys.ts`): e edit, a accept,
 *  Shift-R reject, z snooze until 09:00. */
export function ReviewCard(props: { item: WorkspaceReviewItem; onResolved?: () => void }) {
  const item = () => props.item;
  const streakKind = () => isStreakReview(item());
  // Pause reads the LIVE schedule (a routine toggle since the card opened
  // makes its payload stale), so a failure-streak card needs the workflows list.
  if (isStreakReview(props.item)) enableWorkflowsResource();
  const canPause = () => canPauseStreak(streakPayload(item()), workflowsList());
  const [editing, setEditing] = createSignal(false);
  const [draft, setDraft] = createSignal<Record<string, unknown>>({});
  // The version the open edit began at (ME1); the item refreshes under it.
  const [editedAt, setEditedAt] = createSignal<string | undefined>(undefined);
  const [busy, setBusy] = createSignal(false);
  // Budget pause cards: the raise field and the backend's refusal.
  const [raise, setRaise] = createSignal<string | undefined>(undefined);
  const [cardError, setCardError] = createSignal('');
  // Gardener cards: the lines the person keeps ticked.
  const [ticked, setTicked] = createSignal<string[]>(defaultTicked(gardenLines(props.item.payload)));
  const toggleTick = (id: string) => setTicked(previous => (previous.includes(id) ? previous.filter(x => x !== id) : [...previous, id]));
  function raiseValue(): string {
    const typed = raise();
    if (typed !== undefined) return typed;
    const prefill = suggestionPrefill(num(item().payload, 'observed'), num(item().payload, 'suggestedLimit'));
    return prefill == null ? '' : String(prefill);
  }
  function raiseOk(): boolean {
    return canResume(Number(raiseValue()), num(item().payload, 'observed'));
  }

  function beginEdit() {
    if (item().status !== 'pending' || !actionsFor(item()).editable) return;
    if (editing()) {
      setEditing(false);
      return;
    }
    setDraft(structuredClone(item().payload));
    setEditedAt(pinEditVersion(undefined, item().version));
    setEditing(true);
  }

  async function resolve(action: 'accept' | 'reject') {
    const current = item();
    if (busy() || !isWorkspaceReviewItem(current) || current.status !== 'pending') return;
    const permission = isPermissionReview(current);
    const streak = isStreakReview(current);
    // Nothing to pause (already manual, or the automation is gone): accept
    // stays out of reach from the `a` key too; Dismiss still works.
    if (action === 'accept' && streak && !canPause()) return;
    if (action === 'accept' && permission) {
      // Approving can enable bypassPermissions or a wider tool list, so it is
      // always confirmed — from the button and from the `a` key alike.
      const payload = permissionPayload(current);
      if (!payload) {
        flashFooter('× this permission review is malformed; reject it instead', 3500);
        return;
      }
      const ok = await showConfirm({
        title: `Approve changes to “${itemTitle(current)}”?`,
        body: permissionConfirmBody(payload),
        confirmLabel: payload.escalation ? 'Approve and allow' : 'Approve',
        cancelLabel: 'Keep paused',
        danger: payload.escalation,
      });
      if (!ok) return;
    }
    // NV7: an eval case is added only with the reviewer's own expectation.
    if (action === 'accept' && current.kind === 'eval_candidate' && !(editing() && text(draft(), 'expectedSkill').trim())) {
      if (!editing()) beginEdit();
      flashFooter('Write the expected skill first', 2600);
      return;
    }
    const budget = isBudgetPause(current);
    if (action === 'accept' && budget && !raiseOk()) {
      setCardError('Enter a limit above this window’s usage to raise and resume.');
      return;
    }
    setBusy(true);
    setCardError('');
    try {
      const edited = action === 'accept' && current.kind === 'gardener'
        ? gardenSelection(gardenLines(current.payload), ticked())
        : action === 'accept' && budget
        ? { ...current.payload, newLimit: Number(raiseValue()) }
        : action === 'accept' && !permission && editing() ? draft() : undefined;
      // An approval is bound to the exact card confirmed above, not only to
      // its id (the backend refuses a card that changed since, or a queue
      // with duplicate ids).
      // A learning's Save also carries the version the card read (ME1): an
      // open edit's, else the one shown now.
      const version = versionForSave(edited && editing() ? editedAt() : undefined, current.version);
      const resolved = await api.resolveReviewItem(current.id, action, edited, action === 'accept' ? current.fingerprint : undefined, action === 'accept' ? version : undefined);
      if (current.kind === 'commitment') refreshCommitments();
      if (action === 'accept') {
        // An old companion op applied now may have made or changed a task.
        if (isTaskProposal(current) || current.kind === 'companion_old_op') void refetchTasks();
        if (current.kind === 'project_activation' || current.kind === 'goal') void refetchProjects();
        if (permission || streak) void refetchWorkflows();
      }
      if (budget) {
        void refetchWorkflows();
        void refetchBudgets();
      }
      setEditing(false);
      refreshReviewInbox();
      props.onResolved?.();
      // One Reject has an Undo (P0-10): the same item back to pending. Not a
      // budget pause's Keep paused, whose Reject also holds the pause.
      if (action === 'reject' && !budget && current.kind !== 'companion_old_op') {
        // The rejection as the backend wrote it: an Undo from this notice
        // never takes back a later one (review finding).
        const { id, updatedAt } = resolved;
        flashWithUndo({
          message: permission ? 'Change rejected; the automation stays paused' : streak ? 'Dismissed until the automation succeeds again' : 'Rejected',
          kind: 'review-reject',
          ttlMs: 8000,
          undo: async () => {
            try { await api.undoReviewReject(id, updatedAt); flashFooter('Back in Needs you', 2200); }
            catch (error) { flashFooter(`× Couldn’t undo: ${errMsg(error)}`, 3500); }
            finally { refreshReviewInbox(); props.onResolved?.(); }
          },
        });
        return;
      }
      flashFooter(permission
        ? (action === 'accept' ? 'Automation approved' : 'Change rejected; the automation stays paused')
        : streak
          ? (action === 'accept' ? 'Schedule paused; triggers keep running' : 'Dismissed until the automation succeeds again')
        : budget
          ? (action === 'accept' ? 'Budget raised; automations resume' : 'Kept paused until you resume it')
          : (action === 'accept' ? 'Suggestion applied' : 'Suggestion rejected'), 2200);
    } catch (error: any) {
      // Tauri rejects with a plain string: read it through errMsg.
      if (budget) setCardError(errMsg(error));
      flashFooter(`× suggestion failed: ${errMsg(error)}`, 3500);
      // A failed approval may have superseded this card (the automation
      // changed again); show the current queue rather than the stale card.
      refreshReviewInbox();
      props.onResolved?.();
    } finally {
      setBusy(false);
    }
  }

  async function snooze() {
    if (busy() || item().status !== 'pending' || !actionsFor(item()).snoozable) return;
    setBusy(true);
    try {
      // Until 09:00 the next morning, with an Undo (P1-13); the Decisions
      // view lists what is snoozed, each with Wake now.
      const id = item().id;
      const until = snoozeUntil(new Date());
      await api.snoozeReviewItem(id, until.toISOString());
      refreshReviewInbox();
      props.onResolved?.();
      flashWithUndo({
        message: `Snoozed until ${wakeLabel(until)}`,
        kind: 'status',
        ttlMs: 8000,
        undo: async () => {
          try { await api.wakeReviewItem(id); refreshReviewInbox(); flashFooter('Back in Needs you', 2200); }
          catch (error) { flashFooter(`× Couldn’t wake it: ${errMsg(error)}`, 3500); }
        },
      });
    } catch (error: any) {
      flashFooter(`× snooze failed: ${errMsg(error)}`, 3500);
    } finally {
      setBusy(false);
    }
  }

  function keydown(event: KeyboardEvent) {
    if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey
      || typingTarget(event.target)) return;
    // The letter keys are shortcuts (off with Settings, Keyboard shortcuts),
    // from Review's one keymap (P1-13): Shift-R rejects, so a stray r doesn't.
    if (!shortcutsEnabled()) return;
    const action = reviewKey('card', event);
    if (action === 'edit') beginEdit();
    else if (action === 'accept') void resolve('accept');
    else if (action === 'reject') void resolve('reject');
    else if (action === 'snooze') void snooze();
    else return;
    event.preventDefault();
    event.stopPropagation();
  }

  /** Incoming's a and x on this card's row (REVIEW_CARD_ACTION). */
  function cardAction(event: Event) {
    const action = (event as CustomEvent<unknown>).detail;
    if (action === 'accept' || action === 'reject') void resolve(action);
  }

  const keyHint = () => isPermissionReview(item())
    ? 'a approve · ⇧R reject'
    : streakKind()
      ? 'a pause · ⇧R dismiss · z snooze'
      : isBudgetPause(item())
        ? 'a raise and resume · ⇧R keep paused'
        : 'e edit · a accept · ⇧R reject · z snooze';

  return (
    <article class="review-card review-inbox-card" tabIndex={0} onKeyDown={keydown} ref={el => el.addEventListener(REVIEW_CARD_ACTION, cardAction)} aria-label={`${KIND_LABELS[item().kind]}: ${itemTitle(item())}`} data-review-id={item().id}>
      <div class="review-card-head">
        <div class="review-inbox-title">
          <strong>{itemTitle(item())}</strong>
          <span>{KIND_LABELS[item().kind]} · {SOURCE_LABELS[item().source]} · {shortDate(item().createdAt)}</span>
        </div>
        <span class="settings-state-chip">{reviewStatusLabel(item())}</span>
      </div>
      <Show when={editing()} fallback={<p class="review-statement">{itemSummary(item())}</p>}>
        <ReviewEditor item={item()} draft={draft()} setField={(key, value) => setDraft(previous => ({ ...previous, [key]: value || null }))} />
      </Show>
      <Show when={isPermissionReview(item())}><PermissionDiff item={item()} /></Show>
      <Show when={isBudgetPause(item())}><BudgetPauseDetails item={item()} /></Show>
      <Show when={item().kind === 'gardener'}><GardenerLines item={item()} ticked={ticked()} toggle={toggleTick} /></Show>
      <Show when={evidence(item())}><blockquote class="review-evidence">{evidence(item())}</blockquote></Show>
      <Show when={rejectedNote(item())}><p class="settings-note review-rejected">{rejectedNote(item())}</p></Show>
      <Show when={retiresNote(item())}><p class="settings-note review-retires-note">Save also retires {retiresNote(item())}: it becomes a link to its line, so the fact has one home.</p></Show>
      <Show when={possibleDuplicate(item())}><p class="settings-note review-possible-duplicate">Possibly the same as “{possibleDuplicate(item())}” (Jev duplicate check).</p></Show>
      <Show when={item().sourcePath}><button class="panel-link review-source-link" onClick={event => { event.stopPropagation(); openVaultNote(item().sourcePath); }}>{item().sourcePath}</button></Show>
      <Show when={item().error}><p class={reviewStatusLabel(item()) === 'superseded' ? 'settings-note' : 'settings-error'}><PlainErrorText text={item().error!} plain={reviewStatusLabel(item()) !== 'superseded'} /></p></Show>
      <Show when={isBudgetPause(item()) && cardError()}><p class="settings-error review-budget-error"><PlainErrorText text={cardError() ?? ''} /></p></Show>
      <Show when={item().status === 'pending'}>
        <div class="review-card-actions">
          <Show when={actionsFor(item()).editable}>
            <button class="btn ghost sm" disabled={busy()} onClick={beginEdit}>{editing() ? 'Cancel edit' : 'Edit'}</button>
          </Show>
          <Show when={actionsFor(item()).snoozable}>
            <button class="btn ghost sm" disabled={busy()} onClick={() => void snooze()}>Tomorrow</button>
          </Show>
          <Show when={isBudgetPause(item())}>
            <label class="review-budget-raise" onClick={event => event.stopPropagation()}>
              <span>New limit</span>
              <input
                type="number"
                min="0"
                step="any"
                inputMode="decimal"
                aria-label="New limit"
                value={raiseValue()}
                onInput={event => {
                  setRaise(event.currentTarget.value);
                  setCardError('');
                }}
              />
            </label>
          </Show>
          <button class="btn sm" disabled={busy()} onClick={() => void resolve('reject')}>{actionsFor(item()).rejectLabel}</button>
          <Show when={!streakKind() || canPause()}>
            <button
              class="btn primary sm"
              disabled={busy() || (isBudgetPause(item()) && !raiseOk())}
              onClick={() => void resolve('accept')}
            >{busy() ? 'Applying…' : actionsFor(item()).acceptLabel}</button>
          </Show>
          {/* Only while the letter keys work (P1-13). */}
          <Show when={shortcutsEnabled()}><span class="review-key-hint">{keyHint()}</span></Show>
        </div>
      </Show>
    </article>
  );
}
