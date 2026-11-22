/**
 * Rename, move and trash for Library notes (P1-18): which files offer them
 * and where a rename or a move lands. Library.tsx runs them through
 * `api.vaultMoveNote` / `api.vaultDeleteNote`.
 */

/** A writable vault note: Markdown, outside every workspace (a snapshot is
 *  read only, and a workspace keeps its own brief, plan and outputs where it
 *  expects them) and outside hidden folders such as `.trash`. */
export function canManageNote(path: string): boolean {
  return /\.md$/i.test(path) && !path.startsWith('workspaces/') && !path.split('/').some(s => s.startsWith('.'));
}

const folderOf = (path: string) => { const i = path.lastIndexOf('/'); return i < 0 ? '' : path.slice(0, i); };
const fileOf = (path: string) => path.slice(path.lastIndexOf('/') + 1);
const join = (folder: string, file: string) => folder ? `${folder}/${file}` : file;

export type NoteTarget = { path: string } | { error: string };

/** The note's new path after renaming it to `name`, in the same folder,
 *  keeping its extension (`.md` typed or not). */
export function renamedNotePath(path: string, name: string): NoteTarget {
  const ext = fileOf(path).match(/\.[^.]+$/)?.[0] ?? '';
  const bare = name.trim().replace(new RegExp(`${ext.replace('.', '\\.')}$`, 'i'), '').trim();
  if (!bare) return { error: 'Type a name.' };
  if (/[/\\\n\r]/.test(bare)) return { error: 'Use a name without slashes or line breaks.' };
  if (bare.startsWith('.')) return { error: 'A name can’t start with a dot.' };
  return { path: join(folderOf(path), `${bare}${ext}`) };
}

/** The note's new path in `folder` (vault-relative; empty or `/` is the
 *  vault's top level), keeping its file name. A folder that already exists
 *  in another letter case (`known`: the vault's notes' paths) keeps its own
 *  spelling, since macOS ignores case and the walk reports the disk's. */
export function movedNotePath(path: string, folder: string, known: readonly string[] = []): NoteTarget {
  const parts = folder.trim().split(/[/\\]+/).filter(Boolean);
  if (parts.some(p => p.startsWith('.'))) return { error: 'Choose a folder inside the vault, not a hidden one.' };
  if (parts[0]?.toLowerCase() === 'workspaces') return { error: 'Workspace folders hold their own files; choose another folder.' };
  const folders = new Map<string, string>();
  for (const p of known) for (let f = folderOf(p); f; f = folderOf(f)) folders.set(f.toLowerCase(), f);
  let spelled = '';
  for (const part of parts) { const next = join(spelled, part); spelled = folders.get(next.toLowerCase()) ?? next; }
  return { path: join(spelled, fileOf(path)) };
}

/** The folder a note sits in, to start the Move dialog from. */
export const noteFolder = folderOf;
