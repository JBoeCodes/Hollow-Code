import { Show, createEffect, createMemo, createSignal, onCleanup } from 'solid-js';
import { WorkingMark } from './ChatMotion';
import { Icon } from '../icons';
import { AGENT_LABELS, flashFooter, openContextMenu, openVaultHistory, type ContextMenuItem } from '../store';
import { ReplyFileChips } from './ReplyFileChips';
import { replyFinishedAt, replyRowButtons, replyTime, voteErrorText, voteSavedText, type ReplyRowButton } from '../replyActions';
import { ensureReplyVote, replyVotes, saveReplyVote } from '../store/replyFeedback';
import type { FeedbackVote } from '../types';
import { canvasesLine, receiptAttentionLabel, type ExecutionReceipt } from '../executionReceipt';
import { cfoFooterLine } from '../cfo';
import { openWorkspaceResult } from '../store/workspaces';
import { chipFolders, chipLabel, chipTarget, durationText, notesUpdate, runDuration, type FileChip } from '../turnFooter';
import '../styles/turn-footer.css';

export interface TurnFooterProps {
  receipt?: ExecutionReceipt;
  /** The reply turn's own time (`ChatTurn.at`): when it finished, for a
   *  reply whose receipt has none (`replyActions.ts` `replyFinishedAt`). */
  at?: string;
  /** Copy the whole reply. */
  onCopy: () => void;
  onExplain?: () => void;
  onTeach?: () => void;
  /** Whether the full receipt is shown under the reply. */
  detailsOpen: boolean;
  onToggleDetails: () => void;
  /** The conversation's latest reply: its actions stay visible; older
   *  replies show them on hover or keyboard focus (frontier F5). */
  latest?: boolean;
  /** The run's steps are folded above ("Worked for 1m 34s"), which already
   *  says how long it took: show only when it finished. */
  hideDuration?: boolean;
  /** Usual length of this kind of job, already phrased. The fold shows it
   *  when the steps are folded, so pass null then. */
  typical?: string | null;
  /** Continue from this reply in a new conversation (a written handoff). */
  onBranch?: () => void;
  /** CH1: Branch from here (a new chat with this chat's turns up to this reply). */
  onBranchHere?: () => void;
  /** CP1: Restore to before this turn (only in a chat's own isolated worktree, with a checkpoint). */
  onRestore?: () => void;
  /** Save this reply as a Markdown note (One Ask, 2026-11-03;
   *  `store/replyNote.ts`). */
  onSaveNote?: () => void;
  /** Read this reply aloud, or stop (overnight 11-07, D1; `store/readAloud.ts`):
   *  the row's Read aloud button, which says Stop reading while `reading`
   *  (overnight 11-12, C2: it was in More). Absent without a voice. */
  onReadAloud?: () => void;
  reading?: boolean;
  /** Retry (C3, ChatGPT's regenerate): only on the latest reply, once its
   *  run finished and nothing runs or waits. It sends your last message
   *  again as a new message; this reply stays. */
  onRetry?: () => void;
  /** The same Retry with another agent, in More (one item each). */
  retryAgents?: { label: string; action: () => void }[];
  /** The same Retry with another model of this agent, in More (one item
   *  each, after the agents; overnight 11-07, D1, audit gap 10). */
  retryModels?: { label: string; action: () => void }[];
  /** The workspace whose conversation this is: a changed file in its
   *  `outputs/` opens in its Results (matrix N8). Absent outside one. */
  workspace?: { id: string; kind?: string } | null;
  /** The Main agent's conversation: its own brief and plan edits are one
   *  quiet line, not file chips. */
  mainAgent?: boolean;
  /** The reply's exact run id: Useful and Not useful save against it. */
  runId?: string;
  /** Whether the thumbs show (`replyActions.canVote`). */
  canVote?: boolean;
  /** The changed-file chips are drawn above the reply's "Worked for" line,
   *  so the footer leaves them out. */
  chipsAbove?: boolean;
}

/** A run that finished moments ago, in this session: its footer gets one
 *  brief accent (motion contract: only for real completions, never on
 *  reload). It marks completion, not verification. */
function justFinished(at: string | null | undefined): boolean {
  const t = at ? Date.parse(at) : NaN;
  return Number.isFinite(t) && Date.now() - t >= 0 && Date.now() - t < 4000;
}

/**
 * The quiet row under a finished agent reply (overnight 11-12, C2): Copy,
 * Useful and Not useful, Read aloud, Retry on the latest reply, More, then
 * when it finished ("Sunday 11:27 PM") with any duration, usual time, canvas
 * and CFO words. An unfolded reply's changed-file chips sit above it.
 */
export function TurnFooter(props: TurnFooterProps) {
  const duration = () => (props.hideDuration ? null : runDuration(props.receipt));
  const finishedAt = () => replyFinishedAt(props.receipt, props.at);
  const finished = () => replyTime(finishedAt());
  // COO rebuild L7: what the CFO chose ("Sonnet 5.5 · Medium · review").
  const cfoLine = () => cfoFooterLine(props.receipt?.cfo, undefined, agent => AGENT_LABELS[agent as keyof typeof AGENT_LABELS] ?? agent);
  const foldedNotes = createMemo(() => notesUpdate(props.receipt, props.mainAgent ? props.workspace?.id : undefined));
  const chips = createMemo(() => foldedNotes().chips);
  const hasDetails = () => !!props.receipt;
  // The More actions labels name a Result chip's folder when two share a name (EH4).
  const folders = createMemo(() => {
    const { shown, folded } = chips();
    return chipFolders([...shown, ...folded], props.workspace?.id).slice(0, shown.length);
  });
  const folderOf = (chip: FileChip) => folders()[chips().shown.indexOf(chip)] ?? null;
  const opensResult = (chip: FileChip) => chipTarget(chip, props.workspace).to === 'results';

  // Useful and Not useful (overnight 11-12, C2; `replyActions.ts`): one tap
  // saves the vote; Not useful then asks for an optional one-line reason.
  const vote = () => (props.runId ? replyVotes()[props.runId] : undefined);
  createEffect(() => { if (props.canVote) ensureReplyVote(props.runId); });
  const [reasonOpen, setReasonOpen] = createSignal(false);
  const [reasonDraft, setReasonDraft] = createSignal('');
  async function castVote(choice: FeedbackVote, reason?: string) {
    const runId = props.runId;
    if (!runId) return;
    try {
      await saveReplyVote(runId, choice, reason);
      flashFooter(voteSavedText(choice, !!reason?.trim()), 1600);
      setReasonOpen(choice === 'notUseful' && reason === undefined);
      if (choice === 'useful' || reason !== undefined) setReasonDraft('');
    } catch (e) {
      flashFooter(`× ${voteErrorText(e)}`, 4000);
    }
  }
  const buttons = createMemo(() => replyRowButtons({
    canVote: !!props.canVote && !!props.runId,
    canRead: !!props.onReadAloud,
    canRetry: !!props.onRetry,
    hasMore: !!(props.onExplain || props.onTeach || hasDetails() || props.onBranch || props.onBranchHere || props.onRestore || props.onSaveNote || props.retryAgents?.length || props.retryModels?.length),
  }));
  const has = (b: ReplyRowButton) => buttons().includes(b);

  // P0-23: the check chip shows the Run record (never hides it) and moves
  // focus to it. ChatBody renders the record right after this footer.
  function showRunRecord(from: HTMLElement) {
    if (!props.detailsOpen) props.onToggleDetails();
    requestAnimationFrame(() => {
      let el = from.closest('.turn-footer')?.nextElementSibling ?? null;
      while (el && !el.classList.contains('run-record')) el = el.nextElementSibling;
      const receipt = el?.querySelector(':scope > details.execution-receipt');
      if (!(receipt instanceof HTMLDetailsElement)) return;
      receipt.open = true;
      receipt.querySelector<HTMLElement>(':scope > summary')?.focus();
    });
  }

  function menu(e: MouseEvent) {
    const items: ContextMenuItem[] = [];
    if (props.onExplain) items.push({ label: 'Explain visually', action: props.onExplain });
    if (props.onTeach) items.push({ label: 'Teach Hollow…', action: props.onTeach });
    if (props.onSaveNote) items.push({ label: 'Save as note', action: props.onSaveNote });
    if (props.onBranchHere) items.push({ label: 'Branch from here…', action: props.onBranchHere });
    if (props.onBranch) items.push({ label: 'Continue from here in a new conversation', action: props.onBranch });
    if (props.onRestore) items.push({ label: 'Restore files to before this turn…', action: props.onRestore });
    for (const agent of props.retryAgents ?? []) items.push({ label: `Retry with ${agent.label}`, action: agent.action });
    for (const model of props.retryModels ?? []) items.push({ label: `Retry with ${model.label}`, action: model.action });
    // The changed files' right-click "Show change in history", by a click
    // (L4, N3): this reply's vault changes in Library history.
    const runId = props.receipt?.historyRunId;
    const historyPath = chips().shown.find(c => !c.root)?.path ?? foldedNotes().notePath;
    if (historyPath) items.push({ label: 'Show this reply’s changes in history', action: () => runId ? openVaultHistory(undefined, runId) : openVaultHistory(historyPath) });
    // A Result chip's right-click "Review result", by a click.
    for (const chip of chips().shown.filter(opensResult)) {
      const target = chipTarget(chip, props.workspace);
      if (target.to === 'results') items.push({ label: `Review ${chipLabel(chip.name, folderOf(chip))}`, action: () => void openWorkspaceResult(target.workspaceId, target.path) });
    }
    if (hasDetails()) {
      if (items.length) items.push({ separator: true });
      items.push({ label: props.detailsOpen ? 'Hide run record' : 'Show run record', action: props.onToggleDetails });
    }
    if (!items.length) return;
    // A left click would be closed by the document click listener in the
    // same event (see Quirks), so open on the next tick.
    const { clientX, clientY } = e;
    const target = e.currentTarget as HTMLElement;
    e.stopPropagation();
    setTimeout(() => {
      const rect = target.getBoundingClientRect();
      openContextMenu(
        new MouseEvent('contextmenu', { clientX: clientX || rect.left, clientY: (clientY || rect.bottom) + 4 }),
        items,
      );
    }, 0);
  }

  const meta = () => [duration(), props.typical, canvasesLine(props.receipt?.canvases), cfoLine()].filter(Boolean).join(' · ');

  return (
    <>
    {/* Unfolded replies keep their chips here, above the row; a folded
        reply's chips sit above its "Worked for" line (ChatBody). */}
    <Show when={!props.chipsAbove}>
      <ReplyFileChips receipt={props.receipt} detailsOpen={props.detailsOpen} onToggleDetails={props.onToggleDetails} workspace={props.workspace} mainAgent={props.mainAgent} />
    </Show>
    <div class="turn-footer" role="group" aria-label="Reply details" classList={{ 'just-finished': justFinished(props.receipt?.finishedAt), latest: !!props.latest, voted: !!vote() || reasonOpen(), reading: !!props.reading }}>
      <span class="turn-footer-actions">
      <button type="button" class="turn-footer-btn" title="Copy reply" aria-label="Copy reply" onClick={() => props.onCopy()}>
        <Icon name="copy" size={14} />
      </button>
      <Show when={has('useful')}>
        <button type="button" class="turn-footer-btn turn-footer-vote" classList={{ active: vote() === 'useful' }} aria-pressed={vote() === 'useful'}
          title="Useful" aria-label="Useful" onClick={() => void castVote('useful')}><Icon name="thumbsUp" size={14} /></button>
        <button type="button" class="turn-footer-btn turn-footer-vote" classList={{ active: vote() === 'notUseful' }} aria-pressed={vote() === 'notUseful'}
          title="Not useful" aria-label="Not useful" onClick={() => void castVote('notUseful')}><Icon name="thumbsDown" size={14} /></button>
      </Show>
      <Show when={has('read')}>
        <button type="button" class="turn-footer-btn" classList={{ 'turn-footer-reading': !!props.reading, active: !!props.reading }}
          title={props.reading ? 'Stop reading' : 'Read aloud'} aria-label={props.reading ? 'Stop reading' : 'Read aloud'} aria-pressed={!!props.reading}
          onClick={() => props.onReadAloud?.()}>
          <Show when={props.reading} fallback={<SpeakerIcon />}><Icon name="stop" size={13} /></Show>
        </button>
      </Show>
      <Show when={has('retry')}>
        <button type="button" class="turn-footer-btn turn-footer-retry" title="Send your last message again as a new message" aria-label="Retry" onClick={() => props.onRetry?.()}>
          <Icon name="refresh" size={14} />
        </button>
      </Show>
      <Show when={has('more')}>
        <button type="button" class="turn-footer-btn" title="More" aria-label="More actions" aria-haspopup="menu" onClick={menu}>
          <Icon name="more" size={14} />
        </button>
      </Show>
      <Show when={finished() || meta()}>
        <span class="turn-footer-time" title={finishedAt() ? new Date(finishedAt()!).toLocaleString() : undefined}>
          {[finished(), meta()].filter(Boolean).join(' · ')}
        </span>
      </Show>
      </span>
      {/* Outside the actions, so an older reply's chip isn't faded with them. */}
      <Show when={receiptAttentionLabel(props.receipt)}>
        {(label) => (
          <button type="button" class="turn-footer-check execution-receipt-claim" title="Show the Run record" aria-expanded={props.detailsOpen} onClick={e => showRunRecord(e.currentTarget)}>
            {label()}
          </button>
        )}
      </Show>
      <Show when={foldedNotes().updated}>
        <span class="turn-footer-notes">Updated my notes</span>
      </Show>
    </div>
    <Show when={reasonOpen() && vote() === 'notUseful'}>
      <div class="chat-feedback-reason">
        <input
          type="text"
          aria-label="What was wrong? (optional)"
          placeholder="What was wrong? (optional)"
          maxLength={2000}
          value={reasonDraft()}
          ref={el => requestAnimationFrame(() => el.focus({ preventScroll: true }))}
          onInput={e => setReasonDraft(e.currentTarget.value)}
          onKeyDown={e => {
            e.stopPropagation();
            if (e.key === 'Enter' && reasonDraft().trim()) { e.preventDefault(); void castVote('notUseful', reasonDraft()); }
            else if (e.key === 'Escape') { e.preventDefault(); setReasonOpen(false); }
          }}
        />
        <button type="button" class="btn ghost sm" disabled={!reasonDraft().trim()} onClick={() => void castVote('notUseful', reasonDraft())}>Save</button>
      </div>
    </Show>
    </>
  );
}

/** Read aloud's speaker (drawn here, not in the shared icon set, so the
 *  launch chunk does not grow). */
export function SpeakerIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M11 5 6 9H2v6h4l5 4V5Z" /><path d="M15.5 8.5a5 5 0 0 1 0 7" /><path d="M19 5a10 10 0 0 1 0 14" />
    </svg>
  );
}

/**
 * The same slot while the agent is working: a spinner, what it is doing now
 * and a live elapsed clock, so the finished footer takes its place.
 */
export function LiveTurnFooter(props: { verb: string; startedAt: number | null; quiet?: string | null; typical?: string | null; compact?: boolean }) {
  const [now, setNow] = createSignal(Date.now());
  createEffect(() => {
    if (props.startedAt == null) return;
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), 1_000);
    onCleanup(() => clearInterval(id));
  });
  const elapsed = () => props.startedAt == null ? null : durationText(now() - props.startedAt);
  return (
    <div class="turn-footer live chat-ember" classList={{ 'chat-ember-compact': props.compact }} aria-live="polite">
      <WorkingMark queued={props.verb === 'Queued'} />
      <span class="turn-footer-verb chat-ember-verb">{props.verb === 'Queued' ? 'Queued' : elapsed() ? `Working for ${elapsed()}` : 'Working'}</span>
      <Show when={props.verb !== 'Queued' && props.verb !== 'Working'}>
        <span class="turn-footer-time">{props.verb}</span>
      </Show>
      <Show when={props.typical}>
        <span class="turn-footer-typical">{props.typical}</span>
      </Show>
      <Show when={props.quiet}>
        <span class="chat-ember-quiet" title="Nothing printed by the agent for a while. It may still be working; Stop ends it.">{props.quiet}</span>
      </Show>
    </div>
  );
}
