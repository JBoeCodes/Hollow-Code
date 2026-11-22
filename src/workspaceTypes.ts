import type { AgentSessionSummary } from './types';
import type {
  BuildInfo, ContextDocHealth, LibraryResult, NeedsYouItem, NeedsYouPage, ResultAcceptance, ResultReview, ReviewEvent, SessionChanges,
  SourceContext, StartedBy, WorkFile, WorkResult, WorkWorkspace, WorkspaceDetail, WorkspaceDigest, WorkspaceReentry, WorkspaceSessions,
} from './generated/ipc';
// Generated from the Rust structs (src/generated/ipc.ts, merged plan slice 6);
// re-exported so existing imports keep working. `WorkWorkspace` is Rust
// `workspaces::Workspace` (`kind: 'quick'` marks Quick chats, `kind: 'main'` the
// Main agent; while a build
// workspace's worktree `exists`, `cwd` is the worktree). `WorkspaceDetail`
// (`read_workspace`) always carries `sources`, `sourcesOmitted`, `reentry`
// and `contextHealth` (empty for Quick chats); `ContextDocHealth.included <
// bytes` means agents see a cut copy. `WorkspaceReentry.briefStale` (overnight
// F3) is absent while the brief looks current. `WorkspaceDigest` is a Your
// work row (next-level P02): `resultsError` is set when the results couldn't
// be counted (B·F08). `WorkspaceSessions` (`workspace_sessions`) is one
// workspace's unarchived conversations for the rail tree, newest first:
// `changes` (always sent) lists the lines each conversation's runs added and
// removed (`SessionChanges`), leaving out conversations that changed nothing;
// `startedBy` (overnight F2, `StartedBy`) says which listed conversations
// another one started, and is absent when none was.
export type {
  BuildInfo, ContextDocHealth, LibraryResult, NeedsYouItem, NeedsYouPage, ResultAcceptance, ResultReview, ReviewEvent, SessionChanges,
  SourceContext, StartedBy, WorkFile, WorkResult, WorkWorkspace, WorkspaceDetail, WorkspaceDigest, WorkspaceReentry, WorkspaceSessions,
};
/** The built-in Quick chats workspace (Rust `workspaces::QUICK_CHATS_ID`). */
export const QUICK_CHATS_ID = 'quick-chats';
export const QUICK_CHATS_TITLE = 'Quick chats';
/** What the quick-chats folder is called on screen: Quick chats again since
 *  2026-11-14 (it read COO from L8, 11-10). The folder id stays
 *  `quick-chats`. */
export const QUICK_CHATS_LABEL = 'Quick chats';
export const isQuickChats = (w: { id: string; kind?: string } | string | null | undefined) =>
  typeof w === 'string' ? w === QUICK_CHATS_ID : !!w && (w.id === QUICK_CHATS_ID || w.kind === 'quick');
/** The built-in Main agent workspace (Rust `workspaces::MAIN_AGENT_ID`,
 *  main agent M1): the orb's conversations, its notepad and standing orders. */
export const MAIN_AGENT_ID = 'main';

/** A Notion project's mirror: `notion-` and the page id (32 hex, or the
 *  dashed UUID), not any id that starts `notion-` (Rust
 *  `workspaces::is_notion_mirror`; overnight 10-11 Y2). */
export function isNotionMirrorId(id: string): boolean {
  return /^notion-(?:[0-9a-f]{32}|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$/i.test(id);
}
export const MAIN_AGENT_TITLE = 'COO';
export const isMainAgent = (w: { id: string; kind?: string } | string | null | undefined) =>
  typeof w === 'string' ? w === MAIN_AGENT_ID : !!w && (w.id === MAIN_AGENT_ID || w.kind === 'main');
/** Quick chats or the Main agent: built in, fixed name, never in project lists. */
export const isBuiltinWorkspace = (w: { id: string; kind?: string } | string | null | undefined) =>
  isQuickChats(w) || isMainAgent(w);
/** Why Delete… is off for `w`, or '' (the menu says it beside the item):
 *  the backend's own refusals (`workspaces::trash_checks`), known before the
 *  click, plus a queued turn, which is about to hold the folder (P0-13).
 *  `running`: a conversation in it has a turn running or queued. */
export const workspaceDeleteLock = (w: Pick<WorkWorkspace, 'id' | 'kind' | 'build'>, running = false) =>
  isBuiltinWorkspace(w) ? 'Built in'
  : w.id.startsWith('local-') ? 'Connected project folder'
  : w.build?.exists ? 'Remove its worktree first'
  : running ? 'A conversation here is running' : '';
/** A quick chat's scratch folder (Rust `project_intake::ScratchInfo`, 4.4). */
export interface ScratchInfo { path: string; relative: string; files: number; bytes: number }
/** What Turn into project moved (Rust `AdoptReport`): files now in the project, files left behind, and where. */
export interface AdoptReport { moved: number; kept: number; into: string }
/** A repository cloned into the Projects folder (Rust `ClonedRepo`). */
export interface ClonedRepo { name: string; path: string }
/** A git repository in the connected Projects folder (Rust `RepoCandidate`). */
export interface RepoCandidate {
  name: string; path: string;
  /** Containing folder relative to the Projects folder ("" at the top). */
  parent?: string;
  branch?: string | null;
  /** This checkout is itself a linked worktree of another repository. */
  worktree?: boolean;
  /** origin as host/owner/name. */
  remote?: string | null;
}
/** One line that tells two copies of a repository apart (audit H05):
 *  where it lives, its branch, worktree status and origin. */
export function repoDetail(r: RepoCandidate): string {
  // A folder chosen with Choose folder… has no discovery metadata: show where it is.
  const where = r.parent === undefined ? r.path.replace(/\/[^/]+\/?$/, '').replace(/^\/Users\/[^/]+/, '~') || '/'
    : r.parent ? `in ${r.parent}` : 'Projects folder';
  const branch = r.branch ? `on ${r.branch}` : r.branch === null ? 'detached HEAD' : '';
  return [where, branch, r.worktree ? 'linked worktree' : '', r.remote ?? ''].filter(Boolean).join(' · ');
}
/** What `create_build_workspace` returns. */
export interface CreatedBuildWorkspace { workspace: WorkWorkspace; warnings: string[] }
/** `WORKTREE_UNSAVED:` payload: why a worktree needs a forced removal. */
export interface WorktreeUnsaved { uncommitted: number; unpushed: number }
/** The refusal counts from a `remove_build_worktree` error, or null. */
export function worktreeUnsaved(error: unknown): WorktreeUnsaved | null {
  const text = typeof error === 'string' ? error : (error as { message?: string })?.message ?? '';
  const m = /^WORKTREE_UNSAVED: (\{.*\})$/.exec(text.trim());
  if (!m) return null;
  try { const v = JSON.parse(m[1]); return { uncommitted: Number(v.uncommitted) || 0, unpushed: Number(v.unpushed) || 0 }; } catch { return null; }
}
/** "3 uncommitted changes and 1 unpushed commit". */
export function unsavedText(u: WorktreeUnsaved): string {
  const parts: string[] = [];
  if (u.uncommitted) parts.push(`${u.uncommitted} uncommitted change${u.uncommitted === 1 ? '' : 's'}`);
  if (u.unpushed) parts.push(`${u.unpushed} unpushed commit${u.unpushed === 1 ? '' : 's'}`);
  return parts.join(' and ');
}
/** A workspace's app artwork and GitHub owner (Rust `WorkspaceIcon`). */
export interface WorkspaceIcon { workspaceId: string; imageUrl: string | null; owner: string | null; avatarUrl: string | null }
/** A build workspace's branch and PR state for the rail (Rust `build_changes::RailGitState`). */
export interface RailGitState { workspaceId: string; branch: string; ahead: number; prState: 'open' | 'merged' | 'closed' | string | null; prNumber: number | null }

/** The status Unarchive puts back (P1-38): the one recorded when it was
 *  archived, else active. */
export const unarchiveStatus = (w: Pick<WorkWorkspace, 'archivedFrom'>) => w.archivedFrom?.trim() || 'active';
export const isCurrentWorkspace =(status: string) => ['active','planning','in progress','blocked'].includes(status.toLowerCase());

/** One unchecked `PLAN.md` item offered as a task (Rust `PlanTaskCandidate`). */
export interface PlanTaskCandidate {
  /** The item on one line, at most 240 characters: the task title. */
  text: string;
  /** `plan:<path>\n<sha256 of the normalized text>`; send as `idempotencyKey`. */
  idempotencyKey: string;
  /** `plan:workspaces/<id>/PLAN.md`. */
  origin: string;
  /** The task an earlier create with this key made, or null. */
  existingUrl: string | null;
}

/** A file's content identity in a refresh preview (Rust `workspaces::SourceFingerprint`). */
export interface SourceFingerprint { sha256: string; size: number; modified: string | null }
/** A snapshot compared with its original by content (Rust
 *  `workspaces::SourceRefreshPreview`, plan L3 X5): `origin` is null when the
 *  original can't be read; `text` only when both sides are UTF-8 and ≤ 1 MB. */
export interface SourceRefreshPreview {
  path: string; original: string; available: boolean;
  recorded: SourceFingerprint & { captured: string };
  snapshot: SourceFingerprint; origin: SourceFingerprint | null;
  contentChanged: boolean; snapshotEdited: boolean;
  text: { before: string; after: string } | null;
}

/** One changed file in a build worktree (Rust `build_changes::ChangedFile`). Counts are null for binaries. */
export interface BuildChangedFile { path: string; oldPath?: string; status: 'added' | 'modified' | 'deleted' | 'renamed' | 'untracked'; additions: number | null; deletions: number | null; staged: boolean; uncommitted: boolean }
export interface BuildCommit { sha: string; subject: string; when: string }
export interface BuildPullRequest { url: string; state: string; number: number }
/** What `build_status` returns (Rust `build_changes::BuildStatus`). */
export interface BuildStatus {
  blocked: string | null;
  worktree: string; branch: string; base: string; head: string;
  ahead: number; behind: number; upstream: string | null; unpushed: number;
  files: BuildChangedFile[]; moreFiles: number; commits: BuildCommit[];
  pr: BuildPullRequest | null;
  /** The brief's Outcome section, for a PR body. */
  goal: string;
  /** The last install's commit contains HEAD (P1-46); null when unknown. */
  installedContainsHead?: boolean | null;
}
/** What `build_diff` returns: redacted unified diff text, capped. */
export interface BuildDiff { diff: string; truncated: boolean; truncatedFiles: string[] }

/** An app build from a Hollow worktree (Rust `self_install::AppBuildStatus`, Build PR B5). */
export interface AppBuildStatus {
  blocked: string | null;
  workspaceId: string; isHollow: boolean;
  state: 'idle' | 'building' | 'built' | 'failed';
  /** Another workspace's build holds the build slot. */
  otherJob: string | null;
  log: string | null; logTail: string;
  startedAt: string | null; finishedAt: string | null; error: string | null;
  commit: string | null; branch: string | null;
  /** The finished build's receipt (slice 6, review A·S4/B·F12); an install
   *  re-hashes the bundle against it and refuses anything else. */
  receipt?: AppBuildReceipt | null;
}
export interface QuitRequest { id: string; names: string[]; uncertain?: boolean }
export interface RunningWork { key: string; name: string }
/** Read-only comparison from install_brings.rs; a note explains empty/partial lists. */
export interface InstallBringsReport {
  installed: string | null; installedFrom: string; repo: string | null;
  /** How `repo` was found (Rust `hollow_repo::RepoSource`); absent from a receipt comparison. */
  repoSource?: 'installReceipt' | 'buildWorkspace'; target: string;
  targetCommit: string | null; fetchedAt: string | null; buildId?: string;
  missing: number; note: string | null;
  pullRequests: { number: number; title: string; merge: string; mergedAt: string; untested: string[]; entryFound: boolean }[];
  commits: { sha: string; subject: string; at: string }[];
}
/** Rust `self_install::BuildReceipt`: one finished build, recorded once. */
export interface AppBuildReceipt { buildId: string; workspaceId: string; worktree: string; bundle: string; bundleDigest: string; sourceFingerprint: string; commit: string; dirty: boolean; baseCommit: string; recordedAt: string }
/** What the detached installer helper recorded (Rust `self_install::InstallResult`). */
export interface SelfInstallResult { id: string; ok: boolean; startedAt: string; finishedAt: string; commit: string; branch: string; log: string; message: string; installerRan?: boolean | null; installer?: SelfInstallerReport | null; state?: SelfInstallState; build?: AppBuildReceipt | null }
/** Rust `self_install::InstallState` (slice 6, review B·F02): `failed` means the
 *  previous app is verifiably still installed; `unknown` means nobody could say. */
export type SelfInstallState = 'installed' | 'failed' | 'unknown';
/** Rust `self_install::InstallerReport` (review B·F02): what the installer said,
 *  what it left on disk (`installed`), its journal, and whether the app running
 *  after the reopen is the installed one. */
export interface SelfInstallerReport { ok: boolean; reason: string; installedCommit: string; runningMatches: boolean | null; relaunched: boolean; runningReason: string; installed?: 'yes' | 'no' | 'unknown' | ''; installId?: string; journal?: string | null; steps?: string[] }
/** Rust `self_install::InstallReceipt` (next-level P00/P20): the persistent
 *  receipt, whether its notice was dismissed, and whether the running app's
 *  embedded provenance matches its commit. */
export interface SelfInstallReceipt { result: SelfInstallResult; acknowledged: boolean; runningCommit: string; runningDirty: boolean; matchesRunning: boolean; mismatchReason?: string | null; state?: SelfInstallState; summary: string }

/** A service that outlives a chat turn (Rust `services::ServiceView`, Build PR B6). */
export interface ServiceView {
  scope: string; name: string; command: string; cwd: string;
  port: number | null; kind: 'dev-server' | 'other';
  status: 'running' | 'exited'; exitCode: number | null;
  pid: number; startedAt: string; log: string;
}

// ── Results (next-level P05) · Rust `workspace_results` ─────────────────
export type ResultKind = 'document' | 'table' | 'image' | 'video' | 'page' | 'canvas' | 'review' | 'pdf' | 'data' | 'file';
// `WorkResult` (one deliverable: the file, where it came from, its checks and
// the user's review), `ReviewEvent`, `ResultAcceptance` (only a person
// accepts, decision 3) and `ResultReview` are generated; see the top of this
// file. `checks.total: null` means no receipt names the file, so checks are
// unknown (never zero). `kind` is a `ResultKind` or a newer kind; a `canvas`
// (a saved chat canvas, Canvas C3) also carries `canvas: {id, revision}`.

// `NeedsYouItem` / `NeedsYouPage` (the "needs you" query, Rust `needs_you`)
// are generated; see the top of this file. Pass `nextCursor` back for the next
// page (opaque); `unavailable` names sources that could not be read.
/** Rust `workspaces::SessionTarget`: one conversation found by its exact key or by a search. */
export interface SessionTarget {
  workspaceId: string | null;
  session: AgentSessionSummary;
  /** A search hit whose title lacks a word of the query: the part of the person's message that holds it. */
  snippet?: string | null;
  context?: { request: string; outcome: string; messages: { turn: number; text: string }[]; replacedBy: string[] } | null;
}
/** Rust `workspaces::SessionSearchPage`: conversations by title or by the
 *  person's messages, in every workspace (not only the rail's newest),
 *  keyset-paged. */
export interface SessionSearchPage { memories?: { path: string; targetPath: string; subject: string; statement: string; score: number; lifecycle?: string | null }[]; items: SessionTarget[]; nextCursor: string | null }

// `LibraryResult` (a result with its workspace, next-level P06) is generated.
