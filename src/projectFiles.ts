// A project's files in the Preview pane (COO rebuild L4, 2026-11-10). The
// workspace panel's sections are gone: WORKSPACE.md, PLAN.md, SOURCES.md and
// AGENTS.md open as files, pinned at the top of the Files picker; a build
// workspace's changed files are a group there too; a Result (a file in
// `outputs/`) opens with its review bar; PLAN.md's checkboxes tick in the
// Markdown view. Pure: paths in, paths out, so the picker, the bar and the
// plan toggle agree on what a file is.

/** The project notes pinned at the top of the Files picker, in this order. */
export const PINNED_NOTES = ['WORKSPACE.md', 'PLAN.md', 'SOURCES.md', 'AGENTS.md'] as const;

/** `root` joined with a vault-relative path. */
export function inVault(root: string, rel: string): string {
  return `${root.replace(/\/+$/, '')}/${rel.replace(/^\/+/, '')}`;
}

/** An absolute path's vault-relative form, or null outside the vault. */
export function vaultRelative(abs: string, root: string | null | undefined): string | null {
  if (!root || !abs.startsWith('/')) return null;
  const base = `${root.replace(/\/+$/, '')}/`;
  return abs.startsWith(base) && abs.length > base.length ? abs.slice(base.length) : null;
}

type Place = { id: string; path: string };

/** The project's pinned notes that exist (vault-relative), in pin order. */
export function pinnedNotes(workspace: Place, files: readonly { path: string }[]): string[] {
  const have = new Set(files.map(f => f.path));
  return PINNED_NOTES.map(name => `${workspace.path}/${name}`).filter(p => have.has(p));
}

/** The workspace whose folder holds vault-relative `rel`, the longest match. */
function owner(rel: string, workspaces: readonly Place[]): Place | null {
  let best: Place | null = null;
  for (const w of workspaces) if (rel.startsWith(`${w.path}/`) && (!best || w.path.length > best.path.length)) best = w;
  return best;
}

/** A Result's identity when `rel` is a file in a workspace's `outputs/`:
 *  the workspace and the path its review record is keyed by. Whether it
 *  has a record is the review bar's read. */
export function resultFileOf(rel: string | null, workspaces: readonly Place[]): { workspaceId: string; path: string } | null {
  if (!rel) return null;
  const w = owner(rel, workspaces);
  if (!w || !rel.startsWith(`${w.path}/outputs/`) || rel.endsWith('/')) return null;
  return { workspaceId: w.id, path: rel };
}

/** The workspace whose PLAN.md `rel` is, or null. */
export function planFileOf(rel: string | null, workspaces: readonly Place[]): string | null {
  if (!rel) return null;
  return workspaces.find(w => rel === `${w.path}/PLAN.md`)?.id ?? null;
}

/** One option of the Files picker. */
export interface FilesOption { value: string; label: string; detail?: string; group?: string }

/** The Files picker's options: the project's notes first, then a build
 *  workspace's changed files, then the conversation's files, each path once. */
export function filesPickerOptions(input: {
  notes: readonly string[];
  changed: readonly string[];
  conversation: readonly { value: string; label: string; detail?: string }[];
}): FilesOption[] {
  const seen = new Set<string>();
  const out: FilesOption[] = [];
  const name = (p: string) => p.slice(p.lastIndexOf('/') + 1);
  const dir = (p: string) => (p.includes('/') ? p.replace(/\/[^/]+$/, '') : undefined);
  const add = (o: FilesOption) => { if (!seen.has(o.value)) { seen.add(o.value); out.push(o); } };
  for (const p of input.notes) add({ value: p, label: name(p), group: 'Project notes' });
  for (const p of input.changed) add({ value: p, label: name(p), detail: dir(p), group: 'Changed files' });
  for (const o of input.conversation) add({ ...o, group: input.notes.length || input.changed.length ? 'This conversation' : undefined });
  return out;
}
