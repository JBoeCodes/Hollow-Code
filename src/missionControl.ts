// Mission Control (overnight 10-11 B2): every running and queued agent turn
// at once. Pure: the cards built from the run queue plus the live feed
// (missionFeed.ts), and the orbit layout. No store or Tauri import, so
// the node vitest suite loads it. The store is store/missionControl.ts, the
// view components/MissionControl.tsx.
import type { RunQueueItem } from './types';
import { QUICK_CHATS_LABEL } from './workspaceTypes';
import { describeTool } from './toolDescribe';
import { activityFor, progressLabel, quietLabel, type ActivityMap } from './runActivity';
import { cancelLabel, isLive, readsWorking, stopRowAfterConfirm, waitingText, type RowStop } from './runQueue';
import { isWorkflowFeedKey, shownLines, textLines, workflowFeedKey, type Feed, type FeedEntry } from './missionFeed';

export { FEED_KEYS, FEED_LINES, LINE_CHARS, applyFeedEvent, compactInput, shownLines, textLines, workflowFeedEvent, workflowFeedKey } from './missionFeed';
export type { Feed, FeedEntry, FeedEvent } from './missionFeed';

/** "Read …/Rail.tsx" for a feed entry's tool in use. */
export function toolLine(tool: FeedEntry['tool']): string | null {
  if (!tool) return null;
  const d = describeTool(tool.name, tool.input);
  return [d.verb, d.target].filter(Boolean).join(' ').trim() || tool.name;
}

/** Where a conversation lives, for its card. */
export interface ChatPlace { workspaceId: string | null; workspaceTitle: string; title: string; agent: string | null }

export type CardStatus = 'running' | 'starting' | 'queued' | 'done' | 'failed' | 'stopped' | 'paused' | 'skipped' | 'interrupted';

/** A card holds no clock: the view derives elapsed time and the quiet line
 *  from `now`, so a card keeps its identity (and its DOM) as time passes. */
export interface MissionCard {
  id: string;
  queueId: number | null;
  /** The conversation key (chat turns), else null. */
  key: string | null;
  /** The automation's slug (scheduled, manual, triggered or retried runs),
   *  so Open lands on that automation (consolidation, J8), else null. */
  slug?: string | null;
  source: string;
  status: CardStatus;
  /** The star it orbits: a workspace id, `automations`, or `chats`. */
  system: string;
  systemTitle: string;
  title: string;
  agent: string | null;
  model: string | null;
  /** "Waiting for a free slot", "Queued #2"; null when running. */
  waiting: string | null;
  startedAt: number | null;
  finishedAt: number | null;
  lines: string[];
  tool: string | null;
  progress: string | null;
  lastEventAt: string | null;
  costUsd: number | null;
  error: string | null;
  /** The stop control the queue allows: Stop, Skip (a scheduled run) or
   *  Cancel; null when it can't be stopped or a stop is already asked. */
  stop: 'Stop' | 'Skip' | 'Cancel' | null;
}

const time = (iso: string | null | undefined) => {
  if (!iso) return null;
  const t = Date.parse(iso);
  return Number.isFinite(t) ? t : null;
};

const AUTOMATIONS = { system: 'automations', systemTitle: 'Automations' };
const LOOSE_CHATS = { system: 'chats', systemTitle: QUICK_CHATS_LABEL };

function finishedStatus(item: RunQueueItem): CardStatus {
  switch (item.status) {
    case 'done': return 'done';
    case 'cancelled': return 'stopped';
    case 'paused': return 'paused';
    case 'skipped': return 'skipped';
    case 'interrupted': return 'interrupted';
    default: return 'failed';
  }
}

function liveStatus(item: RunQueueItem): CardStatus {
  if (item.status === 'running') return 'running';
  if (item.status === 'claimed' || readsWorking(item)) return 'starting';
  return 'queued';
}

/** A live card's phase line: the progress phase, else "Running", then the
 *  quiet time once the agent has been silent a while. */
export function phaseLine(entry: { progress: string | null; lastEventAt: string | null } | undefined, now: number): string | null {
  if (!entry) return null;
  const quiet = quietLabel(entry.lastEventAt, now);
  const verb = progressLabel(entry.progress) ?? (quiet ? 'Running' : null);
  return [verb, quiet].filter(Boolean).join(' · ') || null;
}

/** What the queue lets the person do to a live row: Stop a running chat
 *  turn (by its row, `stopCard`); anything else follows the queue's own label. */
function stopOf(item: RunQueueItem): MissionCard['stop'] {
  if (item.cancelRequested) return null;
  if (item.source === 'chat' && item.status === 'running') return 'Stop';
  return cancelLabel(item);
}

/** Queue sources that run an automation, its slug the row's `sourceId`. */
const WORKFLOW_SOURCES: ReadonlySet<string> = new Set(['cron', 'manual', 'trigger', 'retry', 'restrictedResume']);

function card(item: RunQueueItem, feed: Feed, place: (key: string) => ChatPlace | null, activity: ActivityMap, paused = false): MissionCard {
  const chat = item.source === 'chat' && item.sourceId ? item.sourceId : null;
  const where = chat ? place(chat) : null;
  // The feed follows a conversation's (or an automation's, A7) newest turn:
  // use it only for this row.
  const raw = chat ? feed[chat] : WORKFLOW_SOURCES.has(item.source) && item.sourceId ? feed[workflowFeedKey(item.sourceId)] : undefined;
  const live = isLive(item);
  // A legacy unbound live event may describe current work, but never prove a finished row’s cost.
  const entry = raw && (raw.queueId === item.id || (live && raw.queueId == null && raw.running)) ? raw : undefined;
  const sys = chat ? (where?.workspaceId ? { system: where.workspaceId, systemTitle: where.workspaceTitle } : LOOSE_CHATS) : AUTOMATIONS;
  // P1a PR 2: a paused chat's turn ended `cancelled`; its newest row says
  // paused instead of stopped (`missionCards` decides which row).
  const status = live ? liveStatus(item) : paused ? 'paused' : finishedStatus(item);
  // Activity survives a reload without a started event. An exact run id
  // also keeps another run of the same automation from lending its phase.
  // No slug fallback after done removes this run from the activity map.
  const progress = !chat && WORKFLOW_SOURCES.has(item.source) && item.historyRunId
    ? activityFor(activity, item.historyRunId)
    : entry?.running ? entry : null;
  return {
    id: `q${item.id}`,
    queueId: item.id,
    key: chat,
    slug: !chat && WORKFLOW_SOURCES.has(item.source) ? item.sourceId ?? null : null,
    source: item.source,
    status,
    ...sys,
    title: where?.title || item.label || (chat ? 'Conversation' : 'Automation'),
    agent: where?.agent ?? null,
    model: entry?.model ?? null,
    waiting: status === 'queued' || status === 'starting' ? waitingText(item) : null,
    startedAt: time(item.claimedAt) ?? (live ? null : time(item.requestedAt)),
    finishedAt: live ? null : time(item.finishedAt),
    lines: entry && status !== 'queued' ? shownLines(entry) : (item.detail ? textLines(item.detail).slice(0, 1).map(l => `Asked: ${l}`) : []),
    tool: live && status !== 'queued' ? toolLine(entry?.tool ?? null) : null,
    progress: status === 'running' ? progress?.progress ?? null : null,
    lastEventAt: status === 'running' ? progress?.lastEventAt ?? null : null,
    costUsd: entry && !entry.running ? entry.costUsd : null,
    error: item.error ?? entry?.error ?? null,
    stop: live ? stopOf(item) : null,
  };
}

/** A paused chat's own row (P1a PR 2): the chat is paused, this row ended
 *  `cancelled` (how a paused turn's row ends) and no newer row of the chat
 *  exists, so an older stop of the same chat still reads stopped. */
function isPausedRow(queue: readonly RunQueueItem[], item: RunQueueItem, pausedKeys: ReadonlySet<string>): boolean {
  const key = item.source === 'chat' ? item.sourceId : null;
  if (!key || item.status !== 'cancelled' || !pausedKeys.has(key)) return false;
  return !queue.some(other => other.source === 'chat' && other.sourceId === key && other.id > item.id);
}

/** A card Resume applies to (P1a PR 2): a paused chat turn. */
export function canResume(c: Pick<MissionCard, 'status' | 'key'>): boolean {
  return c.status === 'paused' && !!c.key;
}

/** Same calendar day, in the machine's zone. */
function sameDay(a: number, b: number): boolean {
  return new Date(a).toDateString() === new Date(b).toDateString();
}

const ORDER: Record<CardStatus, number> = { running: 0, starting: 1, queued: 2, done: 3, failed: 3, stopped: 3, skipped: 3, interrupted: 3, paused: 3 };

/** Live cards (running first, then starting, then queued in claim order) and
 *  finished ones from `today` (any instant of that day), newest first. A
 *  conversation the feed shows running with no live queue row (a turn the
 *  queue no longer lists) still gets a running card. */
export function missionCards(queue: readonly RunQueueItem[], feed: Feed, place: (key: string) => ChatPlace | null, today: number, activity: ActivityMap = new Map(), pausedKeys: ReadonlySet<string> = new Set()): { live: MissionCard[]; finished: MissionCard[] } {
  const live = queue.filter(isLive).map(item => card(item, feed, place, activity))
    .sort((a, b) => ORDER[a.status] - ORDER[b.status] || position(queue, a) - position(queue, b) || (a.queueId ?? 0) - (b.queueId ?? 0));
  const liveKeys = new Set(live.map(c => c.key).filter(Boolean));
  for (const entry of Object.values(feed)) {
    // An automation's entry belongs to its queue row only, never a chat card.
    if (!entry.running || liveKeys.has(entry.key) || isWorkflowFeedKey(entry.key)) continue;
    const where = place(entry.key);
    live.push({
      id: `f${entry.key}`, queueId: null, key: entry.key, source: 'chat', status: 'running',
      ...(where?.workspaceId ? { system: where.workspaceId, systemTitle: where.workspaceTitle } : LOOSE_CHATS),
      title: where?.title || entry.asked || 'Conversation', agent: where?.agent ?? null, model: entry.model,
      waiting: null, startedAt: entry.startedAt, finishedAt: null, lines: shownLines(entry), tool: toolLine(entry.tool),
      progress: entry.progress, lastEventAt: entry.lastEventAt, costUsd: null, error: entry.error, stop: 'Stop',
    });
  }
  const finished = queue.filter(item => !isLive(item) && item.status !== 'coalesced')
    .filter(item => { const t = time(item.finishedAt); return t !== null && sameDay(t, today); })
    .map(item => card(item, feed, place, activity, isPausedRow(queue, item, pausedKeys)))
    .sort((a, b) => (b.finishedAt ?? 0) - (a.finishedAt ?? 0));
  return { live, finished };
}

/** Stop (Skip, Cancel) a card once the person confirms. The row is read when
 *  clicked, before the confirm opens: the card's queue row, else the row the
 *  feed saw its turn start on. A card with no known row (a turn the queue no
 *  longer lists) is refused (`noRow`): no check here can prove which turn
 *  holds the key's stop slot when a stop by key lands (overnight 11-13,
 *  Codex review). */
export async function stopCard(c: MissionCard, feed: () => Feed, deps: { confirm: () => Promise<boolean>; cancelRow: (id: number) => Promise<RunQueueItem> }): Promise<RowStop> {
  const row = c.queueId ?? (c.key ? feed()[c.key]?.queueId ?? null : null);
  if (row === null) return 'noRow';
  return stopRowAfterConfirm(row, deps.confirm, deps.cancelRow);
}

function position(queue: readonly RunQueueItem[], c: MissionCard): number {
  return queue.find(item => item.id === c.queueId)?.position ?? Number.MAX_SAFE_INTEGER;
}

/** "42 s", "3 min", "1 h 12 min". */
export function elapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  return m % 60 ? `${h} h ${m % 60} min` : `${h} h`;
}

/** "$0.42", "<$0.01", or null when unknown. */
export function money(usd: number | null | undefined): string | null {
  if (typeof usd !== 'number' || !Number.isFinite(usd)) return null;
  if (usd > 0 && usd < 0.01) return '<$0.01';
  return `$${usd.toFixed(2)}`;
}

/** One star in the orbit view and the turns circling it. Positions are
 *  fractions of the sky (0–1); angles in degrees; periods in seconds. */
export interface OrbitSystem {
  id: string;
  title: string;
  x: number;
  y: number;
  satellites: { id: string; card: MissionCard; ring: 1 | 2; angle: number; period: number }[];
}

/** A stable angle for a string, so a satellite keeps its place across
 *  refreshes. */
export function hashAngle(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 16777619); }
  return (h >>> 0) % 360;
}

/** Stars spread over the sky: one centred; more on a ring around the
 *  centre; automations (when present) last. Running turns orbit the inner
 *  ring, evenly spread from a stable start; queued turns hold still on the
 *  outer ring. */
export function orbitLayout(cards: readonly MissionCard[]): OrbitSystem[] {
  const groups = new Map<string, { title: string; cards: MissionCard[] }>();
  for (const c of cards) {
    const g = groups.get(c.system) ?? { title: c.systemTitle, cards: [] };
    g.cards.push(c);
    groups.set(c.system, g);
  }
  const ids = [...groups.keys()].sort((a, b) => (a === 'automations' ? 1 : 0) - (b === 'automations' ? 1 : 0)
    || groups.get(b)!.cards.length - groups.get(a)!.cards.length || a.localeCompare(b));
  const n = ids.length;
  return ids.map((id, i) => {
    const g = groups.get(id)!;
    let x = 0.5, y = 0.5;
    if (n === 2) { x = i === 0 ? 0.32 : 0.68; }
    else if (n > 2) {
      const a = (i / n) * Math.PI * 2 - Math.PI / 2;
      x = 0.5 + Math.cos(a) * 0.32;
      y = 0.5 + Math.sin(a) * 0.3;
    }
    const inner = g.cards.filter(c => c.status === 'running' || c.status === 'starting');
    const outer = g.cards.filter(c => c.status === 'queued');
    const start = hashAngle(id);
    const satellites = [
      ...inner.map((card, k) => ({ id: card.id, card, ring: 1 as const, angle: (start + (k * 360) / inner.length) % 360, period: 16 + k * 3 })),
      ...outer.map((card, k) => ({ id: card.id, card, ring: 2 as const, angle: (start + 180 + (k * 360) / outer.length) % 360, period: 0 })),
    ];
    return { id, title: g.title, x, y, satellites };
  });
}
