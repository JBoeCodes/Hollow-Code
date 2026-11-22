// The triage inbox (overnight 10-11 B6): whether it is open, its sources and
// the person's actions on a row. Its own module so the shell mounts the lazy
// view without importing it (App.tsx). Pure parts in `../triage.ts`.
import { createMemo, createRoot, createSignal } from 'solid-js';
import { api } from '../api';
import { errMsg } from '../utils';
import { latestRequest } from '../latestRequest';
import type { NeedsYouItem } from '../workspaceTypes';
import type { RecordingLink, RecordingList } from '../recordingTypes';
import { closeTriage, openTriage, triageOpen } from './triageOpen';
import {
  RECORDING_DAYS, buildTriage, needsKind, needsSuggestion, suggestText, taskFrom,
  type TriageArrivals, type TriageItem, type TriageState, type TriageSuggestion,
} from '../triage';

// The open state lives in `triageOpen.ts` (the shell imports only that).
export { closeTriage, openTriage, triageOpen };

const EMPTY_STATE: TriageState = { dismissed: {}, filed: {}, tasked: {} };
/** Most Waiting-on-you rows the inbox reads (two of the query's pages). */
const NEEDS_MAX = 400;

const [needs, setNeeds] = createSignal<NeedsYouItem[]>([]);
const [recordings, setRecordings] = createSignal<RecordingList | null>(null);
const [arrivals, setArrivals] = createSignal<TriageArrivals | null>(null);
const [state, setState] = createSignal<TriageState>(EMPTY_STATE);
const [suggestions, setSuggestions] = createSignal<Record<string, TriageSuggestion>>({});
const [loaded, setLoaded] = createSignal(false);
const [loading, setLoading] = createSignal(false);
/** Sources that failed, in words; a partial list never reads as complete. */
const [problems, setProblems] = createSignal<string[]>([]);
/** Undo stack of what this window dismissed or filed here (newest last); a
 *  filed recording keeps the link it had before, which undo puts back. */
interface UndoEntry { id: string; title: string; recording?: { id: string; before: RecordingLink | null } }
const [undoable, setUndoable] = createSignal<UndoEntry[]>([]);
/** Cancelled by every mark here, so a load that started before one never
 *  writes its older state over it (the load only `peek`s it). */
const stateWrites = latestRequest();
/** Rows handled this session, for the caught-up line. */
const [handledToday, setHandledToday] = createSignal(0);

export const triageLoaded = loaded;
export const triageLoading = loading;
export const triageProblems = problems;
export const triageSuggestions = suggestions;
export const triageRecordings = recordings;
export const triageUndoable = undoable;
export const triageHandledCount = handledToday;

/** Every row now (one memo, so every reader shares one build). */
export const triageItems = createRoot(() => createMemo((): TriageItem[] => {
  const rec = recordings();
  return buildTriage({
    needs: needs(),
    recordings: rec?.state === 'ok' ? rec.recordings : [],
    recordingLinks: rec?.links ?? {},
    arrivals: arrivals(),
    state: state(),
  });
}));

async function readNeeds(): Promise<NeedsYouItem[]> {
  const out: NeedsYouItem[] = [];
  let cursor: string | null = null;
  do {
    const page = await api.listNeedsYou(cursor, 200);
    out.push(...page.items.filter(item => needsKind(item.kind)));
    for (const name of page.unavailable) setProblems(p => (p.includes(name) ? p : [...p, name]));
    cursor = page.nextCursor;
  } while (cursor && out.length < NEEDS_MAX);
  return out;
}

const loads = latestRequest();
/** Read every source again (each on its own: one failing leaves the rest). */
export async function loadTriage(): Promise<void> {
  const gen = loads.start();
  const version = stateWrites.peek();
  setLoading(true);
  setProblems([]);
  const fail = (what: string) => (error: unknown) => { if (loads.isLatest(gen)) setProblems(p => [...p, `${what} (${errMsg(error)})`]); };
  await Promise.all([
    readNeeds().then(items => { if (loads.isLatest(gen)) setNeeds(items); }, fail('what waits on you')),
    api.triageArrivals().then(a => {
      if (!loads.isLatest(gen)) return;
      setArrivals(a);
      if (stateWrites.isLatest(version)) setState(a.state ?? EMPTY_STATE);
      for (const u of a.unavailable) setProblems(p => [...p, u]);
    }, fail('imports and new tasks')),
    // Plaud is slow and may be signed out: its own state, never a blocker.
    api.listRecordings(RECORDING_DAYS).then(r => { if (loads.isLatest(gen)) setRecordings(r); }, error => {
      if (loads.isLatest(gen)) setRecordings({ state: 'unavailable', fetchedAt: '', source: 'plaud', recordings: [], unparsed: 0, error: errMsg(error), links: {} });
    }),
  ]);
  if (!loads.isLatest(gen)) return;
  setLoaded(true);
  setLoading(false);
  await refreshSuggestions();
}

/** Ask for suggestions for the rows that don't have one yet. */
async function refreshSuggestions(): Promise<void> {
  const known = suggestions();
  const ask = triageItems().filter(item => needsSuggestion(item) && !(item.id in known)).map(item => ({ id: item.id, text: suggestText(item) }));
  if (!ask.length) return;
  try {
    const got = await api.triageSuggest(ask);
    setSuggestions(prev => ({ ...prev, ...got }));
  } catch (error) {
    setProblems(p => [...p, `suggested workspaces (${errMsg(error)})`]);
  }
}

async function mark(item: TriageItem, action: 'dismiss' | 'file' | 'task', workspaceId?: string, recording?: UndoEntry['recording'], taskRef?: string): Promise<void> {
  stateWrites.cancel();
  setState(await api.triageMark(item.id, action, workspaceId ?? null, taskRef ?? null));
  setUndoable(u => [...u.slice(-19), { id: item.id, title: item.title, recording }]);
  setHandledToday(n => n + 1);
}

/** Re-read what waits on you (after a review card acted, which may also
 *  have failed: the row goes only if the query no longer lists it). */
export async function refreshNeeds(): Promise<void> {
  try { setNeeds(await readNeeds()); } catch (error) { setProblems(p => [...p, `what waits on you (${errMsg(error)})`]); }
}

/** Bring back the last row dismissed or filed here (the store's own record
 *  only: a linked recording stays linked, attached sources stay attached). */
export async function undoTriage(): Promise<string | null> {
  const last = undoable()[undoable().length - 1];
  if (!last) return null;
  // A recording filed here is linked back to what it had before.
  if (last.recording) {
    const { id, before } = last.recording;
    await api.setRecordingLink(id, { workspaceId: null, eventId: before?.eventId ?? null, eventTitle: before?.eventTitle ?? null, name: before?.name ?? null });
    setRecordings(r => {
      if (!r) return r;
      const links = { ...r.links };
      if (before) links[id] = before; else delete links[id];
      return { ...r, links };
    });
  }
  stateWrites.cancel();
  setState(await api.triageMark(last.id, 'restore', null));
  setUndoable(u => u.slice(0, -1));
  setHandledToday(n => Math.max(0, n - 1));
  return last.title;
}

/** File a recording or an import into `workspaceId`. Returns what to say. */
export async function fileItem(item: TriageItem, workspaceId: string, workspaceTitle: string): Promise<string> {
  if (item.kind === 'recording') {
    const before = recordings()?.links[item.recording.id] ?? null;
    // Keep a calendar event it is linked to.
    await api.setRecordingLink(item.recording.id, { workspaceId, eventId: before?.eventId ?? null, eventTitle: before?.eventTitle ?? null, name: item.recording.name });
    setRecordings(r => (r ? { ...r, links: { ...r.links, [item.recording.id]: { workspaceId, eventId: before?.eventId ?? null, eventTitle: before?.eventTitle ?? null, name: item.recording.name, linkedAt: new Date().toISOString(), keptCopy: before?.keptCopy ?? null } } } : r));
    await mark(item, 'file', workspaceId, { id: item.recording.id, before });
    return `Linked “${item.title}” to ${workspaceTitle}`;
  }
  if (item.kind === 'import') {
    let attached = 0;
    for (const file of item.imp.files) {
      await api.attachWorkspaceSource(workspaceId, file);
      attached += 1;
    }
    await mark(item, 'file', workspaceId);
    const more = item.imp.fileCount > attached ? ` (the first ${attached} of ${item.imp.fileCount})` : '';
    return `Attached ${attached} note${attached === 1 ? '' : 's'} to ${workspaceTitle}${more}`;
  }
  throw new Error('Only recordings and imports file from the inbox.');
}

export async function dismissItem(item: TriageItem): Promise<void> {
  await mark(item, 'dismiss');
}

/** Drop a row the store resolved elsewhere (a review card, a failure read). */
export function dropNeeds(id: string) {
  setNeeds(n => n.filter(x => x.id !== id));
  setHandledToday(n => n + 1);
}

/** Make a task from the row (idempotent per row). */
export async function makeTask(item: TriageItem): Promise<{ title: string; existing: boolean }> {
  const t = taskFrom(item);
  const created = await api.createTask({ title: t.title, description: t.description, idempotencyKey: t.idempotencyKey, origin: t.origin });
  // An arrival is linked to its task and leaves the inbox (u brings it back; the task stays);
  // a review, question, Result or failure still waits. A failed link leaves the row, not an error.
  if (item.kind === 'recording' || item.kind === 'import' || item.kind === 'task') try { await mark(item, 'task', undefined, undefined, created.url); } catch (error) { setProblems(p => [...p, `linking the task to its row (${errMsg(error)})`]); }
  return { title: t.title, existing: !!created.existing };
}
