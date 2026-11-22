// The workflow failure-streak card's action URL (overnight P3: split from
// workflowHealth.ts so the launch path carries only this).

/** Mirrors Rust `workflow_hygiene::STREAK_ACTION_SCHEME`: the action URL of
 *  the streak notification (`workflow-failures:<slug>`). */
export const STREAK_ACTION_SCHEME = 'workflow-failures:';

/** The slug a `workflow-failures:<slug>` action URL names, or null. */
export function streakActionSlug(actionUrl: string | null | undefined): string | null {
  if (!actionUrl?.startsWith(STREAK_ACTION_SCHEME)) return null;
  const slug = actionUrl.slice(STREAK_ACTION_SCHEME.length).trim();
  return slug || null;
}
