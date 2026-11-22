// Settings, Advanced, Hollow's databases (repo features R7b): the words
// around `hollow_databases`'s report (Rust `hollow_databases.rs`). Pure, so
// the unit tests import it directly. Removing other vaults' old indexes is
// Disk space's (Remove old indexes…, diskGuard.ts), not this panel's.
import type { HollowDatabases, HollowDbFile } from './types';

const KB = 1024;
const MB = 1024 ** 2;
const GB = 1024 ** 3;

/** What each kind of file holds, as a row's label. */
export const KIND_LABEL: Record<HollowDbFile['kind'], string> = {
  sessions: 'Conversations',
  searchIndex: 'Search index',
  embedIndex: 'Meaning-search index',
  companion: 'Companion ledger',
  projectIndex: 'Project contents index',
};

/** `40 KB`, `0.3 MB`, `86 MB`, `1.2 GB`: small files stay readable. */
export function dbSize(bytes: number): string {
  if (bytes >= GB) return `${(bytes / GB).toFixed(1)} GB`;
  if (bytes >= 10 * MB) return `${Math.round(bytes / MB)} MB`;
  if (bytes >= MB / 10) return `${(bytes / MB).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / KB))} KB`;
}

/** Whose file it is. Another vault's conversations are kept (primary
 *  data); another vault's index is rebuildable. */
export function ownerWords(f: HollowDbFile): string {
  if (f.owner === 'thisVault') return 'this vault';
  if (f.owner === 'app') return 'Hollow';
  const name = f.ownerName ?? 'another vault';
  return f.kind === 'sessions' ? `${name}: another vault's conversations · kept` : `${name}: another vault's`;
}

/** `Search index · this vault · 86 MB + 16 MB WAL · version 2`. */
export function dbLine(f: HollowDbFile): string {
  const wal = f.walBytes > 0 ? ` + ${dbSize(f.walBytes)} WAL` : '';
  const version = f.schemaVersion == null ? '' : ` · version ${f.schemaVersion}`;
  return `${KIND_LABEL[f.kind]} · ${ownerWords(f)} · ${dbSize(f.bytes)}${wal}${version}`;
}

/** A check's outcome: `ok` (also `ok (FTS index not validated
 *  read-only)`), `unknown` when it ran out of time, else `failed`; null
 *  before a check. */
export function checkState(f: HollowDbFile): 'ok' | 'unknown' | 'failed' | null {
  const q = f.quickCheck;
  if (q == null) return null;
  if (q === 'ok' || q.startsWith('ok (')) return 'ok';
  return q.startsWith('not finished') ? 'unknown' : 'failed';
}

/** After Check integrity: `Integrity ok · notes 5,200 rows`, the problem
 *  SQLite found, that it didn't finish, or why the file couldn't be read.
 *  Empty before a check. */
export function checkWords(f: HollowDbFile): string {
  const state = checkState(f);
  if (state == null) return f.error ? `Couldn't read it: ${f.error}` : '';
  const q = f.quickCheck!;
  const head = state === 'ok' ? `Integrity ${q}` : state === 'unknown' ? `Integrity unknown: ${q}` : `Integrity: ${q}`;
  const tables = (f.tables ?? []).map(t => `${t.name} ${t.rows.toLocaleString('en-US')} row${t.rows === 1 ? '' : 's'}`);
  const error = f.error ? ` (${f.error})` : '';
  return [head + error, ...tables].join(' · ');
}

/** True when a check found a problem (not when it ran out of time). */
export function checkFailed(f: HollowDbFile): boolean {
  return checkState(f) === 'failed';
}

/** `8 files, 950 MB, in /…/Hollow`; after a check (`checked`: the report
 *  came from Check integrity), how many passed. A file the check couldn't
 *  open has an `error` and no `quickCheck` (Rust `check_file`); it is
 *  counted, so the summary never says every file checked ok when one wasn't
 *  checked, even when none could be (A4). */
export function dbSummary(r: HollowDatabases, checked: boolean): string {
  const total = r.files.reduce((n, f) => n + f.bytes + f.walBytes, 0);
  const head = `${r.files.length} file${r.files.length === 1 ? '' : 's'}, ${dbSize(total)}, in ${r.folder}`;
  const unopened = checked ? r.files.filter(f => f.quickCheck == null && !!f.error).length : 0;
  if (!unopened && !r.files.some(f => f.quickCheck != null)) return head;
  const bad = r.files.filter(checkFailed).length;
  const unknown = r.files.filter(f => checkState(f) === 'unknown').length;
  const parts = [
    bad ? `${bad} need${bad === 1 ? 's' : ''} attention.` : '',
    unknown ? `${unknown} not checked in time.` : '',
    unopened ? `${unopened} couldn't be opened.` : '',
  ].filter(Boolean);
  return `${head}. ${parts.length ? parts.join(' ') : 'Every file checked ok.'}`;
}

/** The panel's last line: where the old indexes go from. Empty when none. */
export function staleLine(r: HollowDatabases): string {
  if (!r.staleCount) return '';
  const n = r.staleCount;
  return `${n} old ${n === 1 ? 'index' : 'indexes'} of other vaults ${n === 1 ? 'uses' : 'use'} ${dbSize(r.staleBytes)}: Disk space → Remove old indexes… removes ${n === 1 ? 'it' : 'them'}.`;
}
