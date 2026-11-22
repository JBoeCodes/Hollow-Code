// Links from Linear's Custom link coding tool (2026-11-11; backend
// `src-tauri/src/linear_link.rs`, docs/TRACKER_SYNC.md "Linear link").
// `hollow://issue?prompt=…` or `?id=ENG-123` reaches the window as a queued
// result: where to open (an issue's project-lead chat, else the COO) and a
// draft. This opens that chat and puts the draft in its composer. It never
// sends: the person presses Send, so a link from outside can't start a run.
// Loaded lazily by `appBridge.ts` on the backend's `linear-link-pending`
// nudge, so none of it reaches the App chunk.
import { invoke } from './api';
import { setChatPrefillRequest, requestChatFocus, flashFooter } from './store/runtime';
import { workspaceDetail, workspaceId, openProjectAgent, refreshWorkspaces, workspaceListLoaded, workspaces } from './store/workspaces';
import { askMainAgent } from './store/todayBrief';

/** What the backend queued for one link (camelCase of `LinkOpen`). */
export interface LinkOpen {
  seq: number;
  source: string;
  identifier: string | null;
  tracked: boolean;
  /** The project whose lead chat opens; null opens the COO. */
  workspaceId: string | null;
  /** The composer's text; empty when the link was refused. */
  draft: string;
  /** One footer line: what was ignored, or why the COO opened. */
  note: string | null;
}

/** Links already opened here, so a repeated nudge never doubles one. */
const opened = new Set<number>();
let draining: Promise<void> = Promise.resolve();

/** A cold launch can nudge before the workspace list has loaded; without it
 *  a project's lead chat looks missing and the draft would go to the COO. */
async function listReady() {
  if (!workspaceListLoaded()) await refreshWorkspaces({ join: true }).catch(() => { /* the fallback below still works */ });
}

const RETRIES = 3;
const RETRY_MS = 1500;
const ATTEMPT_MS = 15_000;

/** Open one link's chat and fill its composer. False when nothing opened. */
export async function openLink(link: LinkOpen): Promise<boolean> {
  if (!link.draft.trim()) { if (link.note) flashFooter(link.note, 6000); return false; }
  await listReady();
  if (link.workspaceId && !workspaces().some(w => w.id === link.workspaceId)) {
    flashFooter(`The Linear link's project isn't in the workspace list, so its draft went to the COO.`, 6000);
  }
  if (link.workspaceId && workspaces().some(w => w.id === link.workspaceId)) {
    if (await openProjectAgent(link.workspaceId) && workspaceId() === link.workspaceId && workspaceDetail()) {
      setChatPrefillRequest(link.draft);
      requestChatFocus();
      return true;
    }
    flashFooter(`× Couldn't open that project's chat; the Linear link's draft went to the COO instead.`, 6000);
  }
  if (await askMainAgent(link.draft) === 'opened') {
    if (link.note) flashFooter(link.note, 6000);
    return true;
  }
  return false;
}

/** Drain the backend's queue and open each link, one at a time, oldest first. */
export function openPendingLinearLinks(): Promise<void> {
  draining = draining.then(async () => {
    let links: LinkOpen[] = [];
    try { links = (await invoke<LinkOpen[] | undefined>('take_linear_links')) ?? []; } catch { return; }
    for (const link of links) {
      if (opened.has(link.seq)) continue;
      opened.add(link.seq);
      // The queue is already drained, so a chat that won't open yet (a message
      // still sending, the COO mid-open) is retried here, not lost.
      let done = false;
      for (let attempt = 0; attempt < RETRIES && !done; attempt++) {
        if (attempt) await new Promise(resolve => setTimeout(resolve, RETRY_MS));
        // Bounded: an open that never settles must not stall every later link.
        const stuck = new Promise<boolean>(resolve => setTimeout(() => resolve(false), ATTEMPT_MS));
        try { done = await Promise.race([openLink(link), stuck]) || !link.draft.trim(); } catch { /* try again */ }
      }
      if (!done) flashFooter(`× Couldn't open the Linear link${link.identifier ? ` for ${link.identifier}` : ''}. Click it in Linear again.`, 8000);
    }
  }).catch(() => { /* a failed pass never blocks the next */ });
  return draining;
}
