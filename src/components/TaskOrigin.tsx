import { Show, createRoot } from 'solid-js';
import { unsuspended } from '../stableResource';
import { describeTaskOrigin, type OriginLink, type TaskOrigin as Origin } from '../taskOrigin';
import { goToPage, openWorkspaceFile, visitWorkspace, workspaces } from '../store/workspaces';
import { api } from '../api';
import { planItemLine } from '../planModel';
import { setLibraryMode } from '../store/libraryMode';
import { commitmentsList, enableCommitmentsResource, openVaultHistory, openVaultNote } from '../store/runtime';
import { requestRecording } from '../store/originFocus';

/** A task's origin in words (consolidation G), or null without one. With the
 *  task's title, a plan origin can mark the exact item it was copied from;
 *  a promise is named with its counterparty once Commitments has loaded. */
// Read without suspending: the task pane sits under the Tasks page's
// Suspense boundary, and a commitments fetch must not swap the page for its
// loading screen (stableResource.ts). Enabled once, outside render.
const promises = createRoot(() => unsuspended(commitmentsList));
let promisesOn = false;
export const taskOrigin = (origin: string | null | undefined, taskTitle?: string): Origin | null => {
  if (origin?.startsWith('commitment:') && !promisesOn) { promisesOn = true; queueMicrotask(enableCommitmentsResource); }
  return describeTaskOrigin(origin, id => workspaces().find(w => w.id === id)?.title ?? null, {
    taskTitle,
    promiseOf: path => { const c = (promises.latest ?? []).find(x => x.vaultPath === path); return c ? { counterparty: c.counterparty, direction: c.direction } : null; },
  });
};

/** The plan a task came from: its workspace, then its PLAN.md in the Files
 *  tab at the row the task was copied from (COO rebuild L4: the Plan
 *  section marked the row); the top of the file when the row was edited or
 *  is gone. */
async function openPlanItem(workspaceId: string, item?: string | null) {
  await visitWorkspace(workspaceId);
  const folder = workspaces().find(w => w.id === workspaceId)?.path;
  if (!folder) return;
  const rel = `${folder}/PLAN.md`;
  let line: number | undefined;
  if (item) {
    try { line = planItemLine((await api.readWorkFile(rel)).content, item) ?? undefined; }
    catch { line = undefined; }
  }
  await openWorkspaceFile(rel, line);
}

/** Go where a task came from, to the exact item where the origin names one
 *  (plan completion item 4): the plan item, the recording, the promise, the
 *  agent run; else the workspace or Incoming. */
export async function followTaskOrigin(link: OriginLink) {
  switch (link.kind) {
    case 'recording': requestRecording(link.id); setLibraryMode('recordings'); goToPage('vault'); return;
    case 'run': openVaultHistory(undefined, link.runId); return;
    case 'note': openVaultNote(link.path); return;
    case 'workspace': await visitWorkspace(link.workspaceId); return;
    case 'plan': await openPlanItem(link.workspaceId, link.item); return;
  }
}

/** "An item in “Q3 review”’s plan · Open the plan item", for the task detail
 *  and editor, with how the task and its source relate (copied, or a promise
 *  that stays open). `before` runs first (the editor closes itself). */
export function TaskOriginText(props: { origin: Origin; before?: () => void }) {
  return <>{props.origin.text}<Show when={props.origin.link}>{link => <> <button type="button" class="btn-link" onClick={() => { props.before?.(); void followTaskOrigin(link()); }}>{props.origin.linkLabel}</button></>}</Show>
    <Show when={props.origin.relation}>{r => <span class="task-origin-relation">{r()}</span>}</Show></>;
}
