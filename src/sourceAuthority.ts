/**
 * What a piece of material is, said the same way at every attachment and
 * read point (plan L3 X3, 2026-10-14; review U library, ASG 5): a plain
 * summary first ("Snapshot captured 2 Oct; the original has changed
 * since"), the exact provenance and limits in a secondary line.
 *
 * - `snapshot`: a copy in a workspace's `sources/`. Never refreshes on its
 *   own; Check for changes compares it with its original.
 * - `live`: a connected project file, read in place each time, never copied.
 * - `tracked`: a task or project mirrored from the task tracker (B3): the
 *   tracker's fields win; the vault's own fields stay in the file.
 * - `output`: work made in a workspace (`outputs/`, a Result).
 * - `note`: a Library note, edited where it is.
 *
 * Nothing here claims a snapshot is current: size-and-date freshness is
 * reported as "not refreshed", and "same content" only follows a content
 * check (`checked: 'same'`). Pure; unit-tested.
 */

export type SourceKind = 'snapshot' | 'live' | 'tracked' | 'output' | 'note';
export type SourceTone = 'ok' | 'warn' | 'unknown';

export interface SourceState {
  kind: SourceKind;
  /** The plain summary, shown first. */
  summary: string;
  /** Exact provenance and limits, for a secondary line. */
  detail: string;
  tone: SourceTone;
}

export type SourceStateInput =
  | {
    kind: 'snapshot';
    /** `SourceContext.freshness`: unchanged_metadata | changed | unavailable | unknown. */
    freshness?: string | null;
    captured?: string | null;
    original?: string | null;
    /** A content check's answer (the refresh preview), when there was one. */
    checked?: 'same' | 'changed' | null;
  }
  | { kind: 'live'; project?: string | null }
  | { kind: 'tracked'; service?: string | null }
  | { kind: 'output'; workspace?: string | null }
  | { kind: 'note' };

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const dayKey = (d: Date) => d.getFullYear() * 10_000 + d.getMonth() * 100 + d.getDate();

/** "today", "yesterday", "2 Oct", or "2 Oct 2025" in another year; null for no date or an unreadable one. */
export function capturedWhen(iso: string | null | undefined, now = new Date()): string | null {
  if (!iso) return null;
  const at = new Date(iso);
  if (!Number.isFinite(at.getTime())) return null;
  if (dayKey(at) === dayKey(now)) return 'today';
  const yesterday = new Date(now); yesterday.setDate(now.getDate() - 1);
  if (dayKey(at) === dayKey(yesterday)) return 'yesterday';
  const day = `${at.getDate()} ${MONTHS[at.getMonth()]}`;
  return at.getFullYear() === now.getFullYear() ? day : `${day} ${at.getFullYear()}`;
}

/** The state of one piece of material: the summary first, then the detail. */
export function sourceStateLine(input: SourceStateInput, now = new Date()): SourceState {
  switch (input.kind) {
    case 'live': return {
      kind: 'live', tone: 'ok',
      summary: `Live file${input.project ? ` from ${input.project}` : ''}: read each time, never copied`,
      detail: 'Agents and this reader see the file as it is now, in its own folder. Hollow keeps no copy.',
    };
    case 'tracked': return {
      kind: 'tracked', tone: 'ok',
      summary: `Tracked in ${input.service || 'the task tracker'}: its fields come from there`,
      detail: 'Hollow keeps a mirror. The tracker wins its own fields (status, assignee, dates and the rest); notes and portfolio fields you add stay in this file and are never sent.',
    };
    case 'output': return {
      kind: 'output', tone: 'ok',
      summary: input.workspace ? `Made in ${input.workspace}` : 'Made in this workspace',
      detail: 'Work saved in the workspace’s outputs/. It isn’t a copy of anything else.',
    };
    case 'note': return {
      kind: 'note', tone: 'ok',
      summary: 'Library note: edits save here',
      detail: 'A workspace that uses it keeps its own snapshot, which changes only when you refresh it.',
    };
    case 'snapshot': return snapshotState(input, now);
  }
}

function snapshotState(input: Extract<SourceStateInput, { kind: 'snapshot' }>, now: Date): SourceState {
  const when = capturedWhen(input.captured, now);
  const head = when ? `Snapshot captured ${when}` : 'Snapshot';
  const from = input.original ? `Its original is ${input.original}.` : '';
  const state = (summary: string, detail: string, tone: SourceTone): SourceState => ({ kind: 'snapshot', summary, detail: [from, detail].filter(Boolean).join(' '), tone });
  if (input.checked === 'same') return state(`${head}; same content as its original when checked`, 'Compared by content just now.', 'ok');
  if (input.checked === 'changed') return state(`${head}; the original has changed since`, 'Compared by content just now. Replace the snapshot to use the new version.', 'warn');
  switch (input.freshness) {
    case 'changed': return state(`${head}; the original has changed since`, 'Its size or date differs from the capture. Check for changes shows what changed before anything is replaced.', 'warn');
    case 'unavailable': return state(`${head}; its original can’t be found`, 'The original is gone or can’t be read. The snapshot is unchanged.', 'warn');
    case 'unchanged_metadata': return state(`${head}; doesn’t update on its own`, 'The original’s size and date match the capture; Check for changes compares the content. Snapshots never refresh on their own.', 'unknown');
    default: return { kind: 'snapshot', summary: `${head}; nothing records where it came from`, detail: 'With no record of an original, nothing compares it with one. Snapshots never refresh on their own.', tone: 'unknown' };
  }
}

/** A file in a workspace's `sources/` (a snapshot), as a vault-relative path:
 *  `workspaces/<id>/sources/…`, no hidden segment. */
export function isWorkspaceSnapshot(path: string): boolean {
  return /^workspaces\/[^/]+\/sources\/[^/]/.test(path) && !path.split('/').some(p => p.startsWith('.'));
}

/** A workspace deliverable (`workspaces/<id>/outputs/…`). */
export const isWorkspaceOutput = (path: string) => /^workspaces\/[^/]+\/outputs\//.test(path);

/** `{ workspaceId, path }` of a snapshot: `path` is workspace-relative (`sources/<name>`), as `SourceContext.path`. */
export function snapshotParts(path: string): { workspaceId: string; path: string } | null {
  if (!isWorkspaceSnapshot(path)) return null;
  const [, id, ...rest] = path.split('/');
  return { workspaceId: id, path: rest.join('/') };
}

/** What a vault file is, for a reader's state line. */
export function vaultFileKind(path: string): SourceKind {
  if (isWorkspaceSnapshot(path)) return 'snapshot';
  if (isWorkspaceOutput(path)) return 'output';
  return 'note';
}

/** The toolbar's read-only state word for a vault file: a snapshot says so. */
export function readOnlyWord(path: string): string {
  return isWorkspaceSnapshot(path) ? 'Snapshot · read only' : 'Read only';
}

const stripPrivate = (p: string) => p.replace(/^\/private(?=\/)/, '');

/** `original` relative to the vault at `root` (both absolute), or null when it
 *  lies outside. macOS's `/private` prefix is ignored on either side, since
 *  attach records a canonical path. */
export function vaultRelativePath(original: string | null | undefined, root: string | null | undefined): string | null {
  if (!original || !root) return null;
  const base = stripPrivate(root).replace(/\/+$/, '');
  const file = stripPrivate(original);
  if (!base || !file.startsWith(`${base}/`)) return null;
  const rel = file.slice(base.length + 1);
  return rel && !rel.split('/').some(p => p === '..' || p === '') ? rel : null;
}

/** The tracker a file is mirrored from (`tracker` with `tracker_id` or
 *  `tracker_project` in its frontmatter), or null. */
export function trackerMirrorOf(text: string | null | undefined): string | null {
  const front = /^---\r?\n([\s\S]*?)\r?\n---/.exec(text ?? '')?.[1] ?? '';
  const provider = /^tracker:[ \t]*["']?([a-z0-9_-]+)/m.exec(front)?.[1];
  return provider && /^tracker_(id|project):[ \t]*["']?[0-9a-zA-Z-]{2,}/m.test(front) ? provider : null;
}

/** Whether the refresh command refused because the original or the snapshot moved on after the preview. */
export const isSourceChanged = (error: unknown) => /^(Error: )?SOURCE_CHANGED:/.test(String(error));

/** A path Open original may open in its app: a document with an extension
 *  that isn't an app, installer or script. Anything else is revealed. */
export function opensAsDocument(path: string): boolean {
  const ext = /\.([A-Za-z0-9]+)\/?$/.exec(path)?.[1]?.toLowerCase();
  return !!ext && !RUNNABLE.has(ext);
}
// Web pages open in the browser and run their scripts, so they're revealed too
// (integration review, 2026-10-14). The backend's copy, RUNNABLE_EXTENSIONS in
// src-tauri/src/open_guard.rs, decides again on a link's target (H4); a Rust
// test holds the two lists together.
const RUNNABLE = new Set(['html', 'htm', 'xhtml', 'shtml', 'app', 'command', 'sh', 'zsh', 'bash', 'tool', 'terminal', 'pkg', 'mpkg', 'dmg', 'workflow', 'scpt', 'scptd', 'applescript', 'jar', 'exe', 'py', 'pl', 'rb', 'js', 'webloc', 'inetloc', 'fileloc', 'shortcut', 'action', 'prefpane', 'kext', 'plugin', 'bundle', 'osax', 'definition', 'url', 'webarchive', 'svg', 'xht', 'mht', 'mhtml', 'afploc', 'ftploc', 'mobileconfig']);
