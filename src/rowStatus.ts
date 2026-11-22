// The small marks on a chat row in the sidebar (t3code parity 3.3, B8), beside the one ring
// (`SessionIndicator`): what the chat is waiting for (an approval or your input), a plan to review,
// new messages since you last had it open, and the pull request it is linked to. Pure: the facts
// come from `store/rowStatus.ts`, the drawing from `components/RowMarks.tsx`.
import type { SessionState } from './sessionIndicator';

export type RowMarkId = 'approval' | 'input' | 'plan' | 'unread' | 'pr';
export type RowMarkTone = 'attention' | 'info' | 'open' | 'merged' | 'closed';

export interface RowMark {
  id: RowMarkId;
  /** The chip's words; empty for the unread dot. */
  text: string;
  /** The full words for a screen reader and the tooltip. */
  label: string;
  tone: RowMarkTone;
}

export interface RowFacts {
  state: SessionState;
  /** What the pending ask is, when one is open for this chat. */
  ask: 'approval' | 'input' | null;
  planReady: boolean;
  /** Turns added since the person last had the chat open (0 when never opened). */
  unread: number;
  pr: { number: number; state: string | null } | null;
  /** The chat on screen needs none of these: they are looking at it. */
  current: boolean;
}

/** Most important first: the ask, the plan, new messages, the pull request. */
export function rowMarks(f: RowFacts): RowMark[] {
  const out: RowMark[] = [];
  if (f.state === 'waiting' && f.ask === 'approval') out.push({ id: 'approval', text: 'Approval', label: 'Waiting for your approval', tone: 'attention' });
  else if (f.state === 'waiting' && f.ask === 'input') out.push({ id: 'input', text: 'Input', label: 'Waiting for your answer', tone: 'attention' });
  if (f.planReady && !f.current) out.push({ id: 'plan', text: 'Plan', label: 'A plan is ready to review', tone: 'info' });
  if (f.unread > 0 && !f.current) out.push({ id: 'unread', text: '', label: f.unread === 1 ? '1 new message' : `${f.unread} new messages`, tone: 'info' });
  if (f.pr) {
    const state = f.pr.state === 'merged' || f.pr.state === 'open' || f.pr.state === 'closed' ? f.pr.state : null;
    out.push({ id: 'pr', text: `#${f.pr.number}`, label: `Pull request #${f.pr.number}${state ? ` ${state}` : ''}`, tone: state ?? 'open' });
  }
  return out;
}

/** Turns added since `seen` turns were read; 0 for a chat never opened here. */
export function unreadCount(seen: number | null, total: number): number {
  return seen !== null && Number.isInteger(total) && total > seen ? total - seen : 0;
}
