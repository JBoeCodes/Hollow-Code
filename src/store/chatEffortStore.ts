import { api } from '../api';
import { chatEffortStorageKey, parseStoredEffort } from '../ultracode';

// A chat's own reasoning effort lives in Rust (the session store, table
// `chat_efforts`), so every run of the chat resolves it: an automation in the
// chat, a wake-up, a continuation, a handover successor, a resume after a
// restart, a phone send. This is the window's cache of it: reads are
// synchronous (the composer, the model menu, the resume card), writes go
// through `chat_effort_set`, and a refresh from Rust picks up what the backend
// changed (a handover successor inherits its predecessor's level).

const cache = new Map<string, string>();
/** In-flight writes per key, and the sequence of each key's last write: a
 *  refresh that started before a write must not undo it. */
const pending = new Map<string, number>();
const lastWrite = new Map<string, number>();
/** Each key's newest save, so the next one waits for it: saves reach Rust
 *  in the order they were picked and the last pick wins there too. */
const chains = new Map<string, Promise<boolean>>();
let writeSeq = 0;

/** The chat's own level, or null (no choice: the app default). */
export function cachedChatEffort(key: string | null): string | null {
  return key ? parseStoredEffort(cache.get(key) ?? null) : null;
}

/** Set (a level) or clear (null: Default) one chat's effort: the cache now,
 *  Rust through the command, after this chat's earlier saves have answered
 *  (two IPC calls in flight can commit in either order). A failed write is
 *  corrected by the next refresh. */
export function putChatEffort(key: string, level: string | null): Promise<boolean> {
  return write(key, level).save;
}

/** How a save picked in the model menu went: saved, failed while still this
 *  chat's newest pick, or failed after a newer pick for the same chat (which
 *  then wins, so nothing is put back). R11b review, 2026-11-15. */
export type ChatEffortSave = 'saved' | 'failed-latest' | 'failed-superseded';

/** `putChatEffort`, telling a failed save that is still the newest for its
 *  key from one a later pick has already replaced. */
export function putChatEffortTracked(key: string, level: string | null): Promise<ChatEffortSave> {
  const { seq, save } = write(key, level);
  return save.then(ok => ok ? 'saved' : lastWrite.get(key) === seq ? 'failed-latest' : 'failed-superseded');
}

function write(key: string, level: string | null): { seq: number; save: Promise<boolean> } {
  if (level) cache.set(key, level);
  else cache.delete(key);
  const seq = ++writeSeq;
  lastWrite.set(key, seq);
  pending.set(key, (pending.get(key) ?? 0) + 1);
  const done = () => {
    const left = (pending.get(key) ?? 1) - 1;
    if (left > 0) pending.set(key, left);
    else pending.delete(key);
  };
  const send = (): Promise<boolean> => {
    try {
      return Promise.resolve(api.chatEffortSet(key, level)).then(
        () => { done(); return true; },
        () => { done(); return false; },
      );
    } catch {
      done();
      return Promise.resolve(false);
    }
  };
  const before = chains.get(key);
  const save = before ? before.then(send) : send();
  chains.set(key, save);
  void save.then(() => {
    if (chains.get(key) === save) chains.delete(key);
  });
  return { seq, save };
}

/** Whether a write for this chat is still on its way to Rust. */
export const chatEffortWriteInFlight = (key: string): boolean => pending.has(key);

/** The levels the window stored itself before Rust kept them
 *  (localStorage `hollow.effort.<key>`), as `[key, level]`. */
function legacyEfforts(): [string, string][] {
  const prefix = chatEffortStorageKey('');
  const found: [string, string][] = [];
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const name = localStorage.key(i);
      if (!name || !name.startsWith(prefix)) continue;
      const level = parseStoredEffort(localStorage.getItem(name));
      if (level) found.push([name.slice(prefix.length), level]);
    }
  } catch { /* no storage */ }
  return found;
}

function dropLegacy(): void {
  try {
    const prefix = chatEffortStorageKey('');
    const names: string[] = [];
    for (let i = 0; i < localStorage.length; i++) {
      const name = localStorage.key(i);
      if (name && name.startsWith(prefix)) names.push(name);
    }
    for (const name of names) localStorage.removeItem(name);
  } catch { /* no storage */ }
}

/** Move the old localStorage levels into Rust once: only chats Rust has no
 *  level for are filled (never an overwrite), and the old keys go only after
 *  Rust took them, so a failure retries at the next start. */
export async function migrateLegacyEfforts(): Promise<void> {
  const entries = legacyEfforts();
  if (!entries.length) return;
  try {
    await api.chatEffortsImport(entries);
    dropLegacy();
  } catch { /* keep them; the next start tries again */ }
}

/** Read every chat's level from Rust into the cache, except keys written
 *  since this read began. Returns false when Rust couldn't be read. */
export async function refreshChatEfforts(): Promise<boolean> {
  const started = writeSeq;
  // A write already on its way when this read began may land after the read
  // saw the old value: its key is left alone until the refresh is applied.
  const inFlight = new Set(pending.keys());
  let rows: unknown;
  try {
    rows = await api.chatEffortsList();
  } catch {
    return false;
  }
  if (!Array.isArray(rows)) return false;
  const next = new Map<string, string>();
  for (const row of rows) {
    const level = Array.isArray(row) ? parseStoredEffort(String(row[1] ?? '')) : null;
    if (level) next.set(String(row[0]), level);
  }
  const fresh = (key: string) => !inFlight.has(key) && !pending.has(key) && (lastWrite.get(key) ?? 0) <= started;
  for (const key of [...cache.keys()]) if (fresh(key) && !next.has(key)) cache.delete(key);
  for (const [key, level] of next) if (fresh(key)) cache.set(key, level);
  return true;
}

/** Window start: migrate the old values, then read Rust's. */
export async function loadChatEfforts(): Promise<boolean> {
  await migrateLegacyEfforts();
  return refreshChatEfforts();
}

/** Tests: forget everything. */
export function resetChatEffortCache(): void {
  cache.clear();
  pending.clear();
  lastWrite.clear();
  chains.clear();
}
