// Mission Control's live feed (overnight 10-11 B2): what each conversation's
// turn last said and did, folded from the `chat-event` stream. No imports at
// all: store/missionControl.ts loads it at launch, so it must stay tiny; the
// cards (missionControl.ts) describe the tool lazily.

/** Streamed lines a card keeps, and conversations the feed remembers. */
export const FEED_LINES = 3;
export const FEED_KEYS = 24;
/** A line is cut to this many characters. */
export const LINE_CHARS = 160;

/** What one conversation's latest turn has shown so far. */
export interface FeedEntry {
  key: string;
  /** The run-queue row of the turn this entry follows (from `runStarted`),
   *  so a card never shows another turn's lines or cost. */
  queueId: number | null;
  running: boolean;
  startedAt: number | null;
  finishedAt: number | null;
  updatedAt: number;
  asked: string | null;
  /** Finished streamed lines, oldest first (at most FEED_LINES). */
  lines: string[];
  /** The text block still streaming. */
  partial: string;
  /** The tool in use (its name and a compact copy of its input, described
   *  when a card shows it), null between tools. */
  tool: { name: string; input: Record<string, string> } | null;
  progress: string | null;
  lastEventAt: string | null;
  costUsd: number | null;
  model: string | null;
  error: string | null;
}
export type Feed = Readonly<Record<string, FeedEntry>>;

/** The subset of a `chat-event` payload the feed reads. */
export interface FeedEvent {
  type: string;
  conversationKey?: string | null;
  queueId?: number | null;
  sessionKey?: string | null;
  text?: string;
  name?: string;
  input?: unknown;
  asked?: string | null;
  message?: string;
  lastEventAt?: string | null;
  progress?: string | null;
  usage?: { costUsd?: number | null; model?: string | null } | null;
}

const blank = (key: string, now: number): FeedEntry => ({
  key, queueId: null, running: false, startedAt: null, finishedAt: null, updatedAt: now, asked: null,
  lines: [], partial: '', tool: null, progress: null, lastEventAt: null, costUsd: null, model: null, error: null,
});

/** A tool input's string fields, each cut short: enough to describe the
 *  call ("Read …/Rail.tsx"), never a whole file a Write tool carries. */
export function compactInput(input: unknown): Record<string, string> {
  const out: Record<string, string> = {};
  if (!input || typeof input !== 'object') return out;
  for (const [k, v] of Object.entries(input as Record<string, unknown>).slice(0, 12)) {
    if (typeof v === 'string') out[k] = v.length > 300 ? v.slice(0, 300) : v;
  }
  return out;
}

/** Non-empty lines of `text`, each cut to LINE_CHARS. */
export function textLines(text: string): string[] {
  return text.split('\n').map(l => l.replace(/\s+/g, ' ').trim()).filter(Boolean)
    .map(l => (l.length > LINE_CHARS ? `${l.slice(0, LINE_CHARS - 1)}…` : l));
}

const keep = (lines: string[]) => lines.slice(-FEED_LINES);

/** The lines a card shows: finished lines, then the block still streaming. */
export function shownLines(entry: Pick<FeedEntry, 'lines' | 'partial'>): string[] {
  return keep([...entry.lines, ...textLines(entry.partial)]);
}

/** The feed key of automation `slug` (overnight 10-11/12 A7). Chat keys
 *  are session keys (`s-…`), so the two never meet. */
export const workflowFeedKey = (slug: string) => `workflow:${slug}`;
export const isWorkflowFeedKey = (key: string) => key.startsWith('workflow:');

/** The subset of a `workflow-event` payload the feed reads. */
export interface WorkflowFeedInput {
  type: string;
  slug?: string;
  queueId?: number | null;
  name?: string;
  input?: unknown;
  text?: string;
  lastEventAt?: string | null;
  progress?: string | null;
}

/** A `workflow-event` as the `chat-event` the feed folds, keyed by the
 *  automation; null for one the feed doesn't read. A `toolResult` clears the
 *  tool line, as a chat's does. */
export function workflowFeedEvent(evt: WorkflowFeedInput): FeedEvent | null {
  if (!evt.slug) return null;
  const conversationKey = workflowFeedKey(evt.slug);
  switch (evt.type) {
    case 'started': return { type: 'runStarted', conversationKey, queueId: typeof evt.queueId === 'number' ? evt.queueId : null };
    case 'toolUse': return { type: 'toolUse', conversationKey, name: evt.name, input: evt.input };
    case 'toolResult': return { type: 'toolResult', conversationKey };
    case 'assistantText': return { type: 'assistantText', conversationKey, text: evt.text };
    case 'activity': return { type: 'activity', conversationKey, lastEventAt: evt.lastEventAt ?? null, progress: evt.progress ?? null };
    case 'done': return { type: 'done', conversationKey };
    default: return null;
  }
}

/** Fold one `chat-event` into the feed. Events without a conversation key
 *  are ignored; the feed forgets the least recently updated conversation past
 *  FEED_KEYS. Returns the same object when nothing changed. */
export function applyFeedEvent(feed: Feed, evt: FeedEvent, now: number): Feed {
  const key = evt.conversationKey || (evt.type === 'usage' || evt.type === 'activity' ? evt.sessionKey : null);
  if (!key) return feed;
  const prev = feed[key] ?? blank(key, now);
  let next: FeedEntry | null = null;
  switch (evt.type) {
    case 'runStarted':
      next = { ...blank(key, now), queueId: typeof evt.queueId === 'number' ? evt.queueId : null, running: true, startedAt: now, asked: evt.asked?.trim() || null, model: prev.model };
      break;
    case 'assistantDelta':
      if (!evt.text) return feed;
      next = { ...prev, running: true, partial: (prev.partial + evt.text).slice(-4000) };
      break;
    case 'assistantText':
      // A whole block. No agent sends both deltas and a block for the same
      // text (Claude drops the block once deltas came), so anything still
      // streaming is kept first (an API error after deltas, say).
      if (!evt.text) return feed;
      next = { ...prev, running: true, lines: keep([...prev.lines, ...textLines(prev.partial), ...textLines(evt.text)]), partial: '' };
      break;
    case 'turnBoundary':
      next = { ...prev, lines: keep([...prev.lines, ...textLines(prev.partial)]), partial: '' };
      break;
    case 'toolUse':
      next = { ...prev, running: true, tool: { name: evt.name ?? 'tool', input: compactInput(evt.input) } };
      break;
    case 'toolResult':
      next = { ...prev, tool: null };
      break;
    case 'activity':
      next = { ...prev, progress: evt.progress ?? null, lastEventAt: evt.lastEventAt ?? null };
      break;
    case 'usage':
      next = { ...prev, costUsd: typeof evt.usage?.costUsd === 'number' ? evt.usage.costUsd : prev.costUsd, model: evt.usage?.model || prev.model };
      break;
    case 'error':
      next = { ...prev, error: evt.message ?? 'The turn failed' };
      break;
    case 'cancelled':
    case 'done':
      next = { ...prev, running: false, tool: null, progress: null, lastEventAt: null, finishedAt: now, lines: keep([...prev.lines, ...textLines(prev.partial)]), partial: '' };
      break;
    default:
      return feed;
  }
  next.updatedAt = now;
  const out: Record<string, FeedEntry> = { ...feed, [key]: next };
  const keys = Object.keys(out);
  if (keys.length > FEED_KEYS) {
    keys.sort((a, b) => out[a].updatedAt - out[b].updatedAt);
    for (const k of keys.slice(0, keys.length - FEED_KEYS)) if (k !== key) delete out[k];
  }
  return out;
}
