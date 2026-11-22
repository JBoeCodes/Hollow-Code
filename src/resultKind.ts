/**
 * Paperclip PR 14a: the result kinds a chat turn, a workflow run or a
 * receipt can carry, and their labels. A mirror of
 * `src-tauri/src/result_kind.rs`; its `labels_match_frontend_mirror` test
 * reads this file, so each entry must stay on one line as `kind: 'label'`.
 * Pure and dependency-free (vitest loads it in node).
 */

export const RESULT_KIND_LABELS: Record<string, string> = {
  success: 'completed',
  refusal: 'the model declined the request',
  maxTurns: 'turn limit reached',
  providerQuota: 'usage limit reached',
  transient: 'rate limited, overloaded or network error',
  auth: 'sign-in problem',
  resumeFailed: 'the previous agent session could not be resumed',
  permissionDenied: 'actions were denied by the permission mode',
  stalled: 'stalled: no output',
  budgetStop: 'budget reached',
  cancelled: 'cancelled',
  timedOut: 'timed out',
  error: 'failed',
};

/** The kinds whose error text gets the label in front (Rust's
 *  `ResultKind::labels_error`). */
const LABELLED_KINDS = new Set(['maxTurns', 'providerQuota', 'transient', 'auth', 'resumeFailed']);

/** The neutral notice a refusal is shown with (Rust's `REFUSAL_ERROR`). */
export const REFUSAL_NOTICE = 'The model declined the request (refusal).';

/** A kind's label, or null for an unknown or missing kind. */
export function resultKindLabel(kind: string | null | undefined): string | null {
  return kind ? RESULT_KIND_LABELS[kind] ?? null : null;
}

/** How an error `result` event reads in the transcript: the provider's raw
 *  text, labelled once for the kinds that say more than the text does.
 *  The backend labels `error` events itself; those render verbatim. */
export function resultErrorText(text: string, kind?: string | null): string {
  const label = kind && LABELLED_KINDS.has(kind) ? resultKindLabel(kind) : null;
  const body = label && !text.startsWith(label) ? `${label}: ${text}` : text;
  return `(error: ${body})`;
}

/** What a chat `result` event adds to the transcript (Paperclip PR 14a):
 *  a refusal is a neutral system notice (not an error the user caused), an
 *  error result is its labelled text appended to the reply, anything else
 *  adds nothing. `store/chatRuntime.ts` applies it. */
export type ResultEventTurn = { notice: string } | { append: string } | null;

export function resultEventTurn(p: { result: string | null; isError: boolean; resultKind?: string | null }): ResultEventTurn {
  if (p.resultKind === 'refusal') return { notice: REFUSAL_NOTICE };
  if (p.isError && p.result) return { append: resultErrorText(p.result, p.resultKind) };
  return null;
}
