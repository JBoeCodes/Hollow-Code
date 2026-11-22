// The task tracker's typed IPC wrappers (overhaul A2; backend
// `src-tauri/src/commands/tracker_api.rs`, contract docs/TRACKER_SYNC.md).
// Settings' Task tracker panel, the rebuilt Tasks page and the project
// detail (Tasks, Library) import this
// module lazily, so none of it reaches the App chunk; calls keep the shared
// mock routing and IPC tallying of `api.ts`'s invoke. The key goes in and
// never comes back: `TrackerStatus.key` says only whether one is saved.
// Inputs take Partial shapes: every Rust input field but the named ones has
// a serde default.

import { invoke } from './api';
import type {
  PortfolioUpdate,
  ProjectPortfolio,
  TrackerEdit,
  TrackerFridayStatus,
  TrackerHistory,
  TrackerIssueDetail,
  TrackerIssueWork,
  TrackerNewProject,
  TrackerNewTask,
  TrackerSettingsUpdate,
  TrackerStatus,
  TrackerSyncOutcome,
  TrackerTeamMeta,
  TrackerView,
  TrackerViewQuery,
  TrackerWriteOutcome,
} from './generated/ipc';

// X2: one row of Hollow's audit record (`tracker_issue_work`'s `audit`).
export type { AuditEntry } from './generated/ipc';

export const trackerStatus = () => invoke<TrackerStatus>('tracker_status');
export const trackerView = (query?: Partial<TrackerViewQuery>) => invoke<TrackerView>('tracker_view', { query: query ?? null });
export const trackerIssue = (id: string) => invoke<TrackerIssueDetail>('tracker_issue', { id });
/** The issue detail's activity: one issue's change history, read from the tracker now (never mirrored); `more` when older entries were left out. */
export const trackerIssueHistory = (id: string) => invoke<TrackerHistory>('tracker_issue_history', { id });
/** The Custom link value to paste into Linear's coding-tool setting. */
export const linearLinkSetup = () => invoke<{ value: string; steps: string }>('tracker_link_setup');
export const trackerTeamMeta = () => invoke<TrackerTeamMeta>('tracker_team_meta');
// X1: the agent runs tied to an issue (id or key); X2: its audit rows, newest first.
export const trackerIssueWork = (issueId: string) => invoke<TrackerIssueWork>('tracker_issue_work', { issueId });
export const trackerSync = () => invoke<TrackerSyncOutcome>('tracker_sync');
export const editTrackerItem = (edit: TrackerEdit) => invoke<TrackerWriteOutcome>('edit_tracker_item', { edit });
export const createTrackerTask = (task: Pick<TrackerNewTask, 'title'> & Partial<TrackerNewTask>) =>
  invoke<TrackerWriteOutcome>('create_tracker_task', { task });
export const createTrackerProject = (project: Pick<TrackerNewProject, 'name'> & Partial<TrackerNewProject>) =>
  invoke<TrackerWriteOutcome>('create_tracker_project', { project });
export const archiveTrackerTask = (id: string) => invoke<TrackerWriteOutcome>('archive_tracker_task', { id });
/** The issue detail's comment box (B5); reuse `requestKey` on a retry. */
export const commentTrackerIssue = (id: string, body: string, requestKey: string) =>
  invoke<TrackerWriteOutcome>('comment_tracker_issue', { id, body, requestKey });
/** A18: the Friday status of one tracked workspace now (every one, with
 *  the COO's digest, when `workspaceId` is omitted). Notes first; the
 *  tracker copy only with Settings' "Also post it to the tracker". */
export const writeFridayStatus = (workspaceId?: string | null) =>
  invoke<TrackerFridayStatus>('write_friday_status', { workspaceId: workspaceId ?? null });
/** A19: a copy of a saved canvas on an issue of team AI; the canvas stays in Hollow. */
export const attachCanvasToIssue = (workspaceId: string, canvasId: string, revision: number | null, issueId: string) =>
  invoke<TrackerWriteOutcome>('attach_canvas_to_issue', { workspaceId, canvasId, revision, issueId });
export const setTrackerKey = (key: string) => invoke<TrackerStatus>('set_tracker_key', { key });
export const clearTrackerKey = () => invoke<TrackerStatus>('clear_tracker_key');
export const copyKeychainTrackerKey = () => invoke<TrackerStatus>('use_keychain_tracker_key');
export const setTrackerSettings = (update: Partial<TrackerSettingsUpdate>) => invoke<TrackerStatus>('set_tracker_settings', { update });
// A workspace's project record (overhaul B3; `commands/workspaces_api.rs`):
// the tracker's fields when tracked, the vault's portfolio fields always.
export const projectPortfolio = (workspaceId: string) => invoke<ProjectPortfolio>('project_portfolio', { workspaceId });
export const setProjectPortfolio = (update: PortfolioUpdate) => invoke<ProjectPortfolio>('set_project_portfolio', { update });
