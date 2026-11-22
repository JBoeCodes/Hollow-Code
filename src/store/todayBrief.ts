// Today (overnight 10-11 A3): whether the view is open, the day's facts
// both it and Home's hero read, and the brief's state. Its own module so the
// shell can mount the lazy view without importing it.
import { createMemo, createResource, createRoot, createSignal } from 'solid-js';
import { api } from '../api';
import { localIso } from '../agenda';
import { localDateKey, errMsg } from '../utils';
import { tasksData, enableTasksResource, refetchTasks, setChatPrefillRequest, requestChatFocus, flashFooter, commitmentsList, commitmentsError, enableCommitmentsResource, refetchCommitments } from './runtime';
import { unsuspended } from '../stableResource';
import { waitingBadge, waitingList } from './waitingOnYou';
import { openMainAgent } from './mainAgent';
import { commitLabel, failedLabel, prLabel } from '../overnightCard';
import type { OvernightSummary } from '../types';
import { briefFingerprint, commitmentsDue, dueBuckets, factsDigest, todayCoverage, todaysMeetings, type TodayBrief, type TodayCached, type TodayFacts } from '../todayBrief';

/** Last night in lines (empty after a quiet night). */
export function overnightLines(s: OvernightSummary | null | undefined): string[] {
  if (!s || !s.ran) return [];
  return [...s.pullRequests.map(prLabel), ...s.commits.map(commitLabel), ...s.failed.map(failedLabel)];
}

// The Today screen was cut (overnight 11-08 S3). These facts stay for agents
// and for a later reader; nothing in the window opens a Today view.

/** The local day's range for `agenda_events`: midnight to next midnight. */
function rangeFor(day: string): [string, string] {
  const start = new Date(`${day}T00:00:00`);
  const next = new Date(start.getTime() + 36 * 3600_000);
  return [localIso(start), localIso(new Date(`${localDateKey(next)}T00:00:00`))];
}

/** The day's deterministic facts, from what Hollow already reads (no model).
 *  Resources are keyed by the day string, so they load once a day (and on
 *  `refresh`), never on a clock tick. `tick` re-derives soon/past. */
export function createTodayFacts(day: () => string, tick: () => number = () => 0) {
  enableTasksResource();
  const tasks = unsuspended(tasksData);
  // The shared ledger snapshot (cross-PR review S2): it follows every
  // commitment change, where a per-day read kept "2 due" after both were kept.
  enableCommitmentsResource();
  const commitments = commitmentsList;
  // A failed read keeps its reason (P0-04): the facts call it unknown, never empty.
  const [eventsError, setEventsError] = createSignal<string | null>(null);
  const [overnightError, setOvernightError] = createSignal<string | null>(null);
  const [events, { refetch: refetchEvents }] = createResource(day, d => api.agendaEvents(...rangeFor(d)).then(
    v => { setEventsError(null); return v; }, e => { setEventsError(errMsg(e)); return null; }));
  const [overnight, { refetch: refetchOvernight }] = createResource(day, () => api.overnightSummary().then(
    v => { setOvernightError(null); return v; }, e => { setOvernightError(errMsg(e)); return null; }));
  const coverage = createMemo(() => todayCoverage({
    calendar: events.latest, calendarError: eventsError(), calendarLoading: events.loading, commitmentsError: commitmentsError(), overnightError: overnightError(),
    waitingFailed: waitingList.failed(), waitingUnavailable: waitingList.unavailable(),
  }));
  const facts = createMemo<TodayFacts>(() => {
    tick();
    const today = day();
    const now = new Date();
    const due = dueBuckets(tasks.latest?.items ?? [], today);
    return {
      date: today,
      meetings: todaysMeetings(events.latest?.events ?? [], now),
      overdue: due.overdue,
      dueToday: due.today,
      commitments: commitmentsDue(commitments.latest ?? [], today),
      waiting: { total: waitingBadge(), titles: waitingList.items().map(i => i.title), ids: waitingList.items().map(i => i.id) },
      overnight: overnightLines(overnight.latest),
      coverage: coverage(),
    };
  });
  // What the paragraph still waits for, in words (empty = ready): every
  // source has answered (a failed read counts as an answer: it is unknown)
  // and the tasks were read without error.
  const missing = (): string[] => {
    const out: string[] = [];
    if (events.loading) out.push('the calendar');
    if (commitments.loading) out.push('commitments');
    if (overnight.loading) out.push('last night');
    if (!waitingList.loaded() && !waitingList.failed()) out.push('what waits on you');
    const taskError = tasksData.error ? errMsg(tasksData.error) : tasks.latest?.syncError;
    if (taskError) out.push(`tasks (couldn't read them: ${taskError})`);
    else if (tasks.loading || tasks.latest === undefined) out.push('tasks');
    return out;
  };
  const ready = () => missing().length === 0;
  const calendar = () => events.latest ?? null;
  const refreshTasks = () => { void Promise.resolve(refetchTasks()).catch(() => {}); };
  const refresh = () => { refreshTasks(); void refetchEvents(); void refetchCommitments(); void refetchOvernight(); void waitingList.load(); };
  /** Read every source again and settle once each has answered (P1-12:
   *  Today back from away writes from these, not from the 00:01 read). */
  const reads: (() => unknown)[] = [() => refetchTasks(), () => refetchEvents(), () => refetchCommitments(), () => refetchOvernight(), () => waitingList.load()];
  const refreshAll = () => Promise.allSettled(reads.map(read => Promise.resolve().then(read))).then(() => {});
  return { facts, ready, missing, coverage, calendar, refresh, refreshAll, tasks, refreshTasks };
}

/** The local day the shared facts are for, and a minute tick for soon/past. */
const [factsDay, setFactsDay] = createSignal(localDateKey(new Date()));
const [factsTick, setFactsTick] = createSignal(0);
let shared: ReturnType<typeof createTodayFacts> | null = null;

/** The day's facts, shared by Today and Home's hero (read once a day, and
 *  again when Today opens), owned by their own root. */
export function todayFacts() {
  if (!shared) shared = createRoot(() => createTodayFacts(factsDay, factsTick));
  return shared;
}
export const todayFactsDay = factsDay;

/** Bring the facts up to now: the day (a new one reloads them) and soon/past. */
export function touchToday() {
  setFactsDay(localDateKey(new Date()));
  setFactsTick(n => n + 1);
}

const [cached, setCached] = createSignal<TodayCached | null>(null);
const [writing, setWriting] = createSignal(false);
const [briefError, setBriefError] = createSignal('');
export const todayCached = cached;
export const todayWriting = writing;
export const todayBriefError = briefError;

/** Read today's cached brief (no model). */
export async function loadTodayBrief(day: string) {
  try { setCached(await api.todayBriefCached(day)); } catch { setCached({ brief: null, firstName: null }); }
}

// The facts today's brief was written from (P1-12): a later change shows
// "Facts changed since h:mm" beside Write it again, instead of passing the
// old paragraph off as current. A brief from before this has none: no cue.
const FACTS_KEY = 'hollow.todayBrief.facts';
const [briefFacts, setBriefFacts] = createSignal<{ date: string; fp: string } | null>((() => {
  try { const v = JSON.parse(localStorage.getItem(FACTS_KEY) ?? 'null'); return v && typeof v.date === 'string' && typeof v.fp === 'string' ? v : null; } catch { return null; }
})());
function rememberBriefFacts(v: { date: string; fp: string }) {
  setBriefFacts(v);
  try { localStorage.setItem(FACTS_KEY, JSON.stringify(v)); } catch { /* this session only */ }
}
/** Today's facts differ from the ones its brief was written from. */
export function todayFactsChanged(facts: TodayFacts): boolean {
  const b = briefFacts();
  return !!b && b.date === facts.date && b.fp !== briefFingerprint(facts);
}

/** Write today's brief from the facts (once a day unless `regenerate`). */
export async function writeTodayBrief(facts: TodayFacts, regenerate = false): Promise<TodayBrief | null> {
  if (writing()) return null;
  setWriting(true); setBriefError('');
  try {
    const brief = await api.todayBrief(facts.date, factsDigest(facts), regenerate);
    setCached(c => ({ brief, firstName: c?.firstName ?? null }));
    rememberBriefFacts({ date: brief.date, fp: briefFingerprint(facts) });
    return brief;
  } catch (e) {
    setBriefError(errMsg(e));
    return null;
  } finally {
    setWriting(false);
  }
}

/** Draft for the Main agent (Today, the weekly review): open today's Main
 *  agent conversation with the request in its composer, for the person to
 *  read, edit and send (agent-written text is never sent on its own): says
 *  'opened', and the view that asked closes. `false` when nothing was
 *  filled. (P1-03's 'drawer' answer went with the drawer, run 15 P3.) */
export async function askMainAgent(text: string): Promise<'opened' | false> {
  const message = text.trim();
  if (!message) return false;
  if (!await openMainAgent()) { flashFooter("× Couldn't open the COO; try again in a moment.", 5000); return false; }
  setChatPrefillRequest(message);
  requestChatFocus();
  return 'opened';
}

/** A new day or a new open: a failure from before no longer applies. */
export function clearTodayError() {
  setBriefError('');
}
