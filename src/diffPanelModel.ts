// The card's Diff tab, as pure functions (parity 5.3): the file tree, Find,
// the Viewed state and the scope words. DiffPanel.tsx renders them;
// diffPanelModel.test.ts holds them. Provider-neutral and local only.

export interface DiffFileRow { path: string; status: string; additions: number | null; deletions: number | null }

export type DiffTreeNode =
  | { kind: 'dir'; name: string; path: string; children: DiffTreeNode[] }
  | { kind: 'file'; name: string; path: string; file: DiffFileRow };

/** Paths as a folder tree: folders first, then files, each alphabetical; a
 *  folder holding only one folder joins it into one row ("src/components"). */
export function buildTree(files: readonly DiffFileRow[]): DiffTreeNode[] {
  type Dir = { name: string; path: string; dirs: Map<string, Dir>; files: DiffFileRow[] };
  const root: Dir = { name: '', path: '', dirs: new Map(), files: [] };
  for (const f of files) {
    const parts = f.path.split('/').filter(Boolean);
    let at = root;
    for (const part of parts.slice(0, -1)) {
      const path = at.path ? `${at.path}/${part}` : part;
      let next = at.dirs.get(part);
      if (!next) { next = { name: part, path, dirs: new Map(), files: [] }; at.dirs.set(part, next); }
      at = next;
    }
    at.files.push(f);
  }
  const convert = (d: Dir): DiffTreeNode[] => {
    const dirs = [...d.dirs.values()].sort((a, b) => a.name.localeCompare(b.name)).map((c): DiffTreeNode => {
      let cur = c;
      let name = c.name;
      while (cur.files.length === 0 && cur.dirs.size === 1) { cur = [...cur.dirs.values()][0]; name = `${name}/${cur.name}`; }
      return { kind: 'dir', name, path: cur.path, children: convert(cur) };
    });
    const leaves = [...d.files].sort((a, b) => a.path.localeCompare(b.path)).map((file): DiffTreeNode => ({ kind: 'file', name: file.path.split('/').pop() ?? file.path, path: file.path, file }));
    return [...dirs, ...leaves];
  };
  return convert(root);
}

/** Files whose path contains `query` (case-insensitive); all of them for a blank query. */
export function filterFiles<T extends { path: string }>(files: readonly T[], query: string): T[] {
  const q = query.trim().toLowerCase();
  return q ? files.filter(f => f.path.toLowerCase().includes(q)) : [...files];
}

/** A line cut into plain and matching pieces for Find (case-insensitive). */
export function splitMatches(text: string, query: string): { text: string; hit: boolean }[] {
  const q = query.trim().toLowerCase();
  if (!q) return [{ text, hit: false }];
  const lower = text.toLowerCase();
  const out: { text: string; hit: boolean }[] = [];
  let from = 0;
  for (let at = lower.indexOf(q, from); at >= 0; at = lower.indexOf(q, from)) {
    if (at > from) out.push({ text: text.slice(from, at), hit: false });
    out.push({ text: text.slice(at, at + q.length), hit: true });
    from = at + q.length;
  }
  if (from < text.length) out.push({ text: text.slice(from), hit: false });
  return out.length ? out : [{ text, hit: false }];
}

/** How many lines of `lines` contain `query` (Find's count). */
export function countLineMatches(lines: readonly string[], query: string): number {
  const q = query.trim().toLowerCase();
  return q ? lines.filter(l => l.toLowerCase().includes(q)).length : 0;
}

/** What the tab compares: everything uncommitted, or one turn's checkpoints. */
export type DiffScope = { kind: 'all' } | { kind: 'turn'; ord: number; before: string; after: string };
export const scopeKey = (s: DiffScope): string => s.kind === 'all' ? 'all' : `turn:${s.ord}:${s.after.slice(0, 12)}`;

/** The scope menu's rows: all changes, then each turn that changed files, newest first. */
export function scopeChoices(turns: readonly { ord: number; before: string; after: string | null; files: readonly string[] }[]): { scope: DiffScope; label: string }[] {
  const rows: { scope: DiffScope; label: string }[] = [{ scope: { kind: 'all' }, label: 'All uncommitted changes' }];
  for (const t of [...turns].sort((a, b) => b.ord - a.ord)) {
    if (!t.after || t.files.length === 0) continue;
    rows.push({ scope: { kind: 'turn', ord: t.ord, before: t.before, after: t.after }, label: `Turn ${t.ord}: ${t.files.length} file${t.files.length === 1 ? '' : 's'}` });
  }
  return rows;
}

// ── Viewed ──────────────────────────────────────────────────────────────
// A file is viewed for one folder and scope while its signature is unchanged:
// when the file changes again it is unviewed on its own.

export type ViewedStore = Record<string, Record<string, string>>;
export const VIEWED_KEY = 'hollow.diffViewed';
const MAX_FOLDERS = 20;
const MAX_ENTRIES = 500;

export const fileSig = (f: DiffFileRow): string => `${f.status}:${f.additions ?? '?'}:${f.deletions ?? '?'}`;

export function parseViewed(raw: string | null): ViewedStore {
  if (!raw) return {};
  try {
    const v = JSON.parse(raw) as unknown;
    if (!v || typeof v !== 'object' || Array.isArray(v)) return {};
    const out: ViewedStore = {};
    for (const [folder, entries] of Object.entries(v as Record<string, unknown>)) {
      if (!entries || typeof entries !== 'object' || Array.isArray(entries)) continue;
      const clean: Record<string, string> = {};
      for (const [k, sig] of Object.entries(entries as Record<string, unknown>)) if (typeof sig === 'string') clean[k] = sig;
      out[folder] = clean;
    }
    return out;
  } catch { return {}; }
}

const entryKey = (scope: DiffScope, path: string) => `${scopeKey(scope)}|${path}`;

export function isViewed(store: ViewedStore, cwd: string, scope: DiffScope, f: DiffFileRow): boolean {
  return store[cwd]?.[entryKey(scope, f.path)] === fileSig(f);
}

/** A new store with `f` marked (or unmarked) viewed; the oldest folders and
 *  entries fall off at the caps. */
export function setViewed(store: ViewedStore, cwd: string, scope: DiffScope, f: DiffFileRow, viewed: boolean): ViewedStore {
  const mine = { ...(store[cwd] ?? {}) };
  const key = entryKey(scope, f.path);
  delete mine[key];
  if (viewed) mine[key] = fileSig(f);
  const keys = Object.keys(mine);
  for (const k of keys.slice(0, Math.max(0, keys.length - MAX_ENTRIES))) delete mine[k];
  const rest = { ...store };
  delete rest[cwd];
  const folders = Object.entries(rest).slice(-(MAX_FOLDERS - 1));
  return { ...Object.fromEntries(folders), [cwd]: mine };
}

export const viewedCount = (store: ViewedStore, cwd: string, scope: DiffScope, files: readonly DiffFileRow[]): number =>
  files.filter(f => isViewed(store, cwd, scope, f)).length;
