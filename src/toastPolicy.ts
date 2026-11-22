/**
 * In-app notification policy, independent of the current page or focus.
 * Routine notices stay in the bell. Errors, low disk/login warnings,
 * run-guard, budgets, permission reviews and failure streaks still toast.
 * run-guard sends `run-guard-docs` (bell only) for a run whose only alerts
 * are shared-doc notes, and nothing for routine context-doc edits (PER-36).
 * Native banners retain their separate backend foreground policy.
 * - An expected cancellation (a queued run the user cancelled, a stop while
 *   waiting for a background agent slot) reads as a neutral "Stopped", not an
 *   error.
 */

/** The fields the rule reads from a notification. */
export interface ToastNotification {
  kind?: string;
  actionUrl?: string | null;
  source?: string | null;
}

const NEVER_SUPPRESSED_SOURCES = new Set(['run-guard', 'budgets', 'disk-guard', 'claude-accounts']);

/** Whether the in-app toast for this notification is skipped. */
export function suppressToast(n: ToastNotification): boolean {
  // Ordinary completions live in the bell. Only failures and time-critical
  // safety/permission notices interrupt the conversation (overnight U2).
  const urgent = n.kind === 'error' || (n.source && NEVER_SUPPRESSED_SOURCES.has(n.source))
    || /^(workflow-permissions:|workflow-failures:|budget-pauses:)/.test(n.actionUrl ?? '');
  return !urgent;
}

/** Backend error texts that mean the user (or Hollow on their behalf)
 *  stopped the run before it did anything: `run_queue::CANCELLED_BEFORE_START`
 *  and the run permit's stop while waiting for a slot. */
export const EXPECTED_CANCELLATIONS = [
  'cancelled before it started',
  'was stopped while waiting for a background agent slot',
] as const;

/** Whether an error (a Tauri rejection is a plain string) is one of those. */
export function isExpectedCancellation(error: unknown): boolean {
  const text = typeof error === 'string'
    ? error
    : error instanceof Error
      ? error.message
      : error && typeof error === 'object' && typeof (error as { message?: unknown }).message === 'string'
        ? (error as { message: string }).message
        : '';
  return EXPECTED_CANCELLATIONS.some(needle => text.includes(needle));
}
