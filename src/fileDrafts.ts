/**
 * Unsaved file drafts that survive an app restart (next-level P08).
 *
 * The editor used to keep drafts in memory and sessionStorage, which a quit
 * or crash loses. Drafts now live in localStorage (WKWebView keeps it in the
 * app's data store across launches) under `hollow.fileDraft.<path>`, with the
 * baseline the draft started from and when it was last changed, so a restart
 * can offer recovery and a later save can still detect an external edit.
 * Older sessionStorage drafts (`hollow.file.<path>`) are migrated on read.
 * Dismissing a draft is explicit; nothing is discarded silently.
 */

export interface FileDraft { text: string; baseline: string; savedAt: string }

const PREFIX = 'hollow.fileDraft.';
const LEGACY = 'hollow.file.';

type Store = Pick<Storage, 'getItem' | 'setItem' | 'removeItem' | 'key' | 'length'>;
const local = (): Store | null => { try { return localStorage; } catch { return null; } };
const session = (): Store | null => { try { return sessionStorage; } catch { return null; } };

function parse(raw: string | null): FileDraft | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw);
    if (v && typeof v.text === 'string' && typeof v.baseline === 'string') return { text: v.text, baseline: v.baseline, savedAt: typeof v.savedAt === 'string' ? v.savedAt : '' };
  } catch { /* malformed */ }
  return null;
}

export function readDraft(path: string, store = local(), legacy = session()): FileDraft | null {
  const found = parse(store?.getItem(PREFIX + path) ?? null);
  if (found) return found;
  const old = parse(legacy?.getItem(LEGACY + path) ?? null);
  if (old) {
    // Migrate: the next restart must still find it.
    if (writeDraft(path, old.text, old.baseline, store)) legacy?.removeItem(LEGACY + path);
    return { ...old, savedAt: old.savedAt || new Date().toISOString() };
  }
  return null;
}

/** False when storage is full or unavailable (the caller says so). */
export function writeDraft(path: string, text: string, baseline: string, store = local(), now = new Date()): boolean {
  if (!store) return false;
  try { store.setItem(PREFIX + path, JSON.stringify({ text, baseline, savedAt: now.toISOString() })); return true; }
  catch { return false; }
}

export function clearDraft(path: string, store = local(), legacy = session()) {
  try { store?.removeItem(PREFIX + path); } catch { /* ignore */ }
  try { legacy?.removeItem(LEGACY + path); } catch { /* ignore */ }
}

/** Paths an editor has open now, each with how many editors hold it: Review
 *  never lists a draft someone is typing (P1-14, D3). */
const open = new Map<string, number>();
/** Note that an editor shows `path`; call the result when it stops. */
export function holdDraftOpen(path: string): () => void {
  open.set(path, (open.get(path) ?? 0) + 1);
  let held = true;
  return () => {
    if (!held) return;
    held = false;
    const left = (open.get(path) ?? 1) - 1;
    if (left > 0) open.set(path, left); else open.delete(path);
  };
}
export const openDraftPaths = (): ReadonlySet<string> => new Set(open.keys());

/** Who keeps a copy of drafts beside storage (the editor's memory), told
 *  when one is discarded from outside an editor. */
const discardListeners = new Set<(path: string) => void>();
export function onDraftDiscarded(listener: (path: string) => void): () => void {
  discardListeners.add(listener);
  return () => discardListeners.delete(listener);
}
/** Discard `path`'s draft from outside an editor (Review's Discard): gone
 *  from storage and from every copy. Returns it for Undo, or null. */
export function discardDraft(path: string, store = local(), legacy = session()): FileDraft | null {
  const draft = readDraft(path, store, legacy);
  clearDraft(path, store, legacy);
  for (const listener of discardListeners) listener(path);
  return draft;
}

/** Every stored draft that differs from its baseline, newest first. */
export function listDrafts(store = local()): ({ path: string } & FileDraft)[] {
  if (!store) return [];
  const out: ({ path: string } & FileDraft)[] = [];
  for (let i = 0; i < store.length; i++) {
    const key = store.key(i);
    if (!key?.startsWith(PREFIX)) continue;
    const d = parse(store.getItem(key));
    if (d && d.text !== d.baseline) out.push({ path: key.slice(PREFIX.length), ...d });
  }
  return out.sort((a, b) => b.savedAt.localeCompare(a.savedAt));
}
