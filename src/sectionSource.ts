/**
 * Where a workspace section comes from, and when it must be read again
 * (Canvas C5a P2, docs/history/CANVAS_C5.md). Pure, no DOM.
 *
 * The native section footers expose which file they were drawn from
 * (a focusable source icon and tooltip) and re-read that file when the
 * vault watcher reports it changed, so an edit made in another app shows
 * without "Reload file".
 */

import { relativeWhen } from './homeReturn';
import type { VaultChange } from './types';
import type { WorkFile } from './workspaceTypes';

/** The section toolbar's third button (P1-09): Markdown opens the section's
 *  own file in WorkFileEditor; Results' is its Full review instead. */
export const SECTION_MARKDOWN = {
  overview: { label: 'Markdown', file: 'WORKSPACE.md' },
  brief: { label: 'Markdown', file: 'WORKSPACE.md' },
  plan: { label: 'Markdown', file: 'PLAN.md' },
  sources: { label: 'Markdown', file: 'SOURCES.md' },
  results: { label: 'Full review', file: null },
} as const;

/** The watcher's `paths` cap (Rust `vault_watch::MAX_PATHS`). A burst that
 *  fills it may have dropped the path asked about. */
export const VAULT_CHANGE_MAX_PATHS = 200;

/** Whether `change` may have touched the vault-relative `path`: it is listed,
 *  or the list was cut at the cap and the path's top-level folder changed. */
export function vaultChangeTouches(change: VaultChange | null | undefined, path: string): boolean {
  if (!change || !path) return false;
  const paths = change.paths ?? [];
  if (paths.includes(path)) return true;
  const top = path.split('/')[0];
  return paths.length >= VAULT_CHANGE_MAX_PATHS && (change.folders ?? []).includes(top);
}

/** The newest modification time among `names` in the workspace file list
 *  (names relative to the workspace, e.g. `WORKSPACE.md`); null when none of
 *  them has one. */
export function newestModified(files: readonly WorkFile[] | undefined, names: readonly string[]): string | null {
  let best: string | null = null, bestAt = -Infinity;
  for (const f of files ?? []) {
    if (!names.includes(f.name) || !f.modified) continue;
    const at = Date.parse(f.modified);
    if (Number.isFinite(at) && at > bestAt) { best = f.modified; bestAt = at; }
  }
  return best;
}

/** "From WORKSPACE.md · updated 2m ago"; "From WORKSPACE.md and PLAN.md";
 *  the time is left out when it is unknown, never guessed. */
export function sourceLineText(names: readonly string[], modified: string | null | undefined, now = Date.now()): string {
  const list = names.length <= 1 ? names.join('') : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
  const when = modified ? relativeWhen(modified, now) : '';
  return `From ${list}${when ? ` · updated ${when}` : ''}`;
}
