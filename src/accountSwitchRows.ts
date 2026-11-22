// Overnight D4: a reply's account moves as rows in the chat timeline, not
// only on its receipt. The receipt's `switches` (turn guards #420/#421, R1)
// come back with the conversation, so these rows survive a reload; the live
// `accountSwitch` notice (pushed when the move happens) does not. A reply
// that still shows its live notices skips that many account rows.
// Type-only imports, so the node vitest suite can load it.
import type { ReceiptSwitchWire } from './executionReceipt';

export interface SwitchRow {
  kind: ReceiptSwitchWire['kind'];
  /** What happened, with which account or agent. */
  text: string;
  /** Why, as the backend worded it. */
  why: string;
}

/** One row per move: "Switched to Tech" · "Admin passed its ceiling";
 *  "Moved to Codex from Claude" · the budget's reason; "Stayed on Claude
 *  Code login, not an added account" · "claude.ai connector: Notion". */
export function switchRow(switched: ReceiptSwitchWire): SwitchRow {
  const from = switched.from.trim();
  const to = switched.to.trim();
  const why = switched.reason.trim();
  switch (switched.kind) {
    case 'login':
      return { kind: 'login', text: 'Stayed on Claude Code login, not an added account', why };
    case 'agent':
      return { kind: 'agent', text: from ? `Moved to ${to || 'another agent'} from ${from}` : `Moved to ${to || 'another agent'}`, why };
    case 'retry':
      return { kind: 'retry', text: `${to || 'The agent'} resumed once after an outage`, why };
    case 'rested':
      return { kind: 'rested', text: 'Started fresh', why };
    default:
      return { kind: 'account', text: `Switched to ${to || 'another account'}`, why };
  }
}

/** How many live `accountSwitch` notices the reply holding `index` still
 *  shows: the whole reply, from the user message before it to the next one
 *  (a failed resume can leave the receipt on the turn before its notice),
 *  counting only turns at or after `from`, the first one rendered. */
export function liveSwitchNotices(turns: ReadonlyArray<{ role: string; notice?: string }>, index: number, from = 0): number {
  let start = index;
  while (start > 0 && turns[start - 1]?.role !== 'user') start--;
  let count = 0;
  for (let i = Math.max(start, Math.min(from, index)); i < turns.length && turns[i]?.role !== 'user'; i++) {
    if (turns[i]?.role === 'system' && turns[i]?.notice === 'accountSwitch') count++;
  }
  return count;
}

/** The rows a reply's timeline shows for its receipt's `switches`: every
 *  agent move and login stay, and the account moves its live notices don't
 *  already show (the first `live` of them, in order). */
export function switchRows(switches: ReadonlyArray<ReceiptSwitchWire> | undefined, live = 0): SwitchRow[] {
  let skip = Math.max(0, live);
  const rows: SwitchRow[] = [];
  for (const switched of switches ?? []) {
    // Overnight 9 A9: the chat already says so in its fresh-session notice.
    if (switched.kind === 'rested') continue;
    if (switched.kind === 'account' && skip > 0) {
      skip--;
      continue;
    }
    rows.push(switchRow(switched));
  }
  return rows;
}
