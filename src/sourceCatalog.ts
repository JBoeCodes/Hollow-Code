import { CALENDAR_SOURCE, KEPT_COPIES } from './meetingsCopy';

/** One vocabulary for shared material and workspace attachment. No connector reads here.
 *  No label reuses a Library tab's name; a collection holding a tab's files names that
 *  tab (`tab`, `tabLabel`) so Library can link to it (P1-19). Meetings and Kept
 *  transcripts use the shared meetings wording (meetingsCopy.ts, P1-20). */
export const SOURCE_COLLECTIONS = [
  { id: 'shared', label: 'Reference material', description: 'Notes and reusable local context.' },
  { id: 'projects', label: 'Projects & tasks', description: 'Task files and local project records; tracked ones are mirrored from the task tracker. Attachments are snapshots, not two-way links.' },
  { id: 'records', label: 'Records', description: 'Meetings, domains, War Room use cases and captures in library/records/, and each project’s record (PROJECT.md). People notes stay with the agents.' },
  { id: 'calendar', label: 'Meetings', description: `Retained local meeting notes. ${CALENDAR_SOURCE}` },
  { id: 'recordings', label: 'Kept transcripts', description: `${KEPT_COPIES} Audio is not included in agent context.`, tab: 'recordings', tabLabel: 'Recordings' },
  { id: 'outputs', label: 'Result files', description: 'Result files from workspaces and previous automations (saved in outputs/), as plain files.', tab: 'results', tabLabel: 'Results' },
  { id: 'all', label: 'All files', description: 'Retained local files, including older Hollow folders.' },
] as const;
/** A workspace's project record: `workspaces/<id>/PROJECT.md`. Library opens it as the project detail. */
export const isProjectRecord = (path: string) => /^workspaces\/[^/]+\/PROJECT\.md$/.test(path);
export const projectRecordWorkspace = (path: string) => (isProjectRecord(path) ? path.split('/')[1] : null);

export function inSourceCollection(path: string, collection: string): boolean {
  if (collection === 'all') return true;
  if (collection === 'projects') return /^(projects|tasks|goals|commitments)\//.test(path);
  // Records (B3): library/records/ but its people, and every project record.
  if (collection === 'records') return (path.startsWith('library/records/') && !path.startsWith('library/records/people/')) || isProjectRecord(path);
  if (collection === 'outputs') return /^(briefings|outputs)\//.test(path) || /^workspaces\/[^/]+\/outputs\//.test(path);
  if (collection === 'shared') return !/^(workspaces|sessions|calendar|recordings|people|projects|tasks|goals|commitments|briefings|outputs|workflows|me)\//.test(path);
  return path.startsWith(collection + '/');
}
export const SOURCE_COVERAGE: Record<string, string> = {
  complete: 'Text included', excerpt: 'Sent when relevant', not_selected: 'Sent when relevant', budget_exceeded: 'Outside context limit', requires_reader: 'Needs a file reader',
};
export const SOURCE_FRESHNESS: Record<string, string> = {
  unchanged_metadata: 'Original size and date unchanged', changed: 'Original changed since capture', unavailable: 'Original unavailable', unknown: 'Origin not tracked',
};

/** What a workspace's sources can and can't do, in one sentence set for the
 *  Sources section and its list (P1-04). Hollow extracts PDF, Word,
 *  PowerPoint and Excel text itself (`src-tauri/src/doc_text.rs`). */
export const SOURCE_LIMITS = 'Snapshots never refresh on their own. The size and date check doesn’t compare content, and the checksum is the one recorded when the file was captured; Check for changes compares the content, shows what changed, and replaces the snapshot only when you choose to. Connected project files are read live, and tracked tasks and projects follow their tracker. Hollow reads the text of PDF, Word, PowerPoint and Excel files itself; images and other formats need a runtime with a file reader, or a text export.';

/** How many Library files the source picker lists at once. One limit for the
 *  list and its message (review F04, 2026-10-13). */
export const SOURCE_PICKER_LIMIT = 100;

/** The picker's window over `total` matches: how many it shows, and the
 *  message to show only when that is fewer than all of them. */
export function sourcePickerWindow(total: number, limit = SOURCE_PICKER_LIMIT): { shown: number; message: string | null } {
  const shown = Math.min(total, limit);
  return { shown, message: shown < total ? `Showing the first ${shown.toLocaleString()} of ${total.toLocaleString()} matching files. Type in Find to narrow them.` : null };
}
