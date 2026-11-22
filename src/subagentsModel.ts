// The Subagents tab's words (COO rebuild L3, 2026-11-10): what the chat's
// Subagents chip, the tab's rows and a subagent's header say, from the
// `session_subagents` read (`SubagentsView`, L2). Pure: no Solid, no store,
// so the node vitest suite loads it. Model names come from the agents' own
// catalogs (passed in), never from one provider's naming.
import type { CfoDecision, SubagentRow, SubagentsView } from './generated/ipc';
import type { AgentConversationTurn, RunQueueItem } from './types';
import { agentLabel } from './agentLabel';
import { cfoFooterLine } from './cfo';
import { durationText } from './turnFooter';
import { stopRowAfterConfirm, type RowStop } from './runQueue';

/** Glyphs the chip shows at most. */
export const CHIP_GLYPHS = 3;

export interface WorkGroup {
  id: string;
  /** Key of the first row's root: stable while the team moves between Active and Done. */
  teamKey: string;
  title: string;
  kind: 'swarm' | 'team' | 'single';
  parts: number;
  rows: SubagentRow[];
  active: boolean;
}

/** Stable recorded membership, including a parent's handover keys. */
export function workGroups(rows: readonly SubagentRow[]): WorkGroup[] {
  const byKey = new Map<string, SubagentRow>();
  for (const row of rows) for (const key of row.conversations) byKey.set(key, row);
  const groups = new Map<string, WorkGroup>();
  for (const row of rows) {
    let root = row;
    const seen = new Set([row.key]);
    while (root.parentKey && byKey.has(root.parentKey)) {
      const parent = byKey.get(root.parentKey)!;
      if (seen.has(parent.key)) break;
      seen.add(parent.key);
      root = parent;
    }
    // One team per root: a child's own swarm group never splits it from its root.
    const swarm = root.group;
    const hasChildren = rows.some(r => r.parentKey && root.conversations.includes(r.parentKey));
    const kind = swarm ? 'swarm' : root !== row || hasChildren ? 'team' : 'single';
    const id = swarm ? `swarm:${swarm.id}` : `session:${root.key}`;
    let group = groups.get(id);
    if (!group) {
      group = { id, teamKey: root.key, title: swarm?.title ?? root.title ?? 'Untitled subagent', kind, parts: swarm?.parts ?? 0, rows: [], active: false };
      groups.set(id, group);
    }
    group.rows.push(row);
    group.active ||= row.status === 'running' || row.status === 'queued';
    if (!swarm) group.parts = group.rows.length;
  }
  return [...groups.values()];
}

export function costWords(cost: number | null | undefined): string {
  return cost != null && Number.isFinite(cost) ? `$${Math.max(0, cost).toFixed(2)}` : 'Cost unavailable';
}

export function groupWords(group: WorkGroup): string {
  const running = group.rows.filter(r => r.status === 'running').length;
  const queued = group.rows.filter(r => r.status === 'queued').length;
  const priced = group.rows.filter(r => r.costUsd != null && Number.isFinite(r.costUsd));
  return [`${group.parts} parts`, running ? `${running} running` : '', queued ? `${queued} queued` : '',
    priced.length ? `${costWords(priced.reduce((n, r) => n + r.costUsd!, 0))} recorded` : ''].filter(Boolean).join(' · ');
}

/** Who started the row (11-14 card 2): "by the lead", "by the COO", "by
 *  another chat" or "by you"; empty when the read doesn't say. */
export function startedWords(row: Pick<SubagentRow, 'startedBy'>): string {
  switch (row.startedBy) {
    case 'lead': return 'by the lead';
    case 'main': return 'by the COO';
    case 'chat': return 'by another chat';
    case 'you': return 'by you';
    default: return '';
  }
}

export function outcomeWords(row: SubagentRow): string {
  if (row.status !== 'done') return statusWords(row);
  return row.activity?.outcome?.split(':')[0] || 'Finished';
}

/** What follows the colon of a finished row's outcome ("Partial: what is
 *  missing"), trimmed; empty when there is none or the row did not finish. */
export function outcomeDetail(row: SubagentRow): string {
  if (row.status !== 'done') return '';
  const text = row.activity?.outcome ?? '';
  const at = text.indexOf(':');
  return at < 0 ? '' : text.slice(at + 1).trim();
}

/** The small mark on a finished row: a check for Done and Finished, a half
 *  circle for any other outcome word (Partial, Blocked), and an exclamation
 *  for a row that did not finish (Failed or Stopped come through `status`). */
export function finishedMark(row: SubagentRow): string {
  if (row.status !== 'done') return '!';
  const word = outcomeWords(row);
  return word === 'Done' || word === 'Finished' ? '✓' : '◐';
}

export function currentStep(row: SubagentRow): string {
  if (row.status === 'queued') return waitingWords(row) || 'Waiting in line';
  const step = row.activity?.current;
  if (!step) return 'Working on the next step';
  return activityWords(step);
}

/** Display vocabulary is closed: command strings, paths and unknown tool
 * names never enter the side view, including older persisted checkpoints. */
export function activityWords(step: { name: string; target: string }): string {
  const name = step.name.split('__').pop()!.replace(/^functions\./, '').toLowerCase();
  if (/^(bash|exec|exec_command|shell|run_command|terminal)$/.test(name)) {
    const command = step.target.trim();
    if (/[;&|><`\n]|\$\(/.test(command)) return 'Working in the terminal';
    if (/^(?:hollow-slot\s+--remote\s+)(?:npm run gate|npm test|cargo test)\b/.test(command)) return 'Running the checks';
    if (/^cargo fmt\b/.test(command)) return command.includes('--check') ? 'Checking code formatting' : 'Formatting the code';
    if (/^(?:npm (?:test|run (?:gate|test\S*|typecheck))|npx (?:vitest|playwright)|cargo (?:test|clippy))\b/.test(command)) return 'Checking the changes';
    if (/^git (?:status|diff|log|show|ls-files)\b/.test(command)) return 'Reviewing repository changes';
    if (/^git (?:fetch|pull)\b/.test(command)) return 'Refreshing repository history';
    if (/^(?:rg|cat|head|tail|ls|pwd)\b/.test(command)) return 'Inspecting project files';
    return 'Working in the terminal';
  }
  if (/^(read|read_file|read_source|get_file|read_workspace)$/.test(name)) return 'Reading project context';
  if (/^(edit|multiedit|write|write_file|apply_patch)$/.test(name)) return 'Updating project files';
  if (/search|^(grep|glob|find)$/.test(name)) return 'Finding relevant information';
  if (/^(todowrite|update_plan|todo_list)$/.test(name)) return 'Updating the work plan';
  if (/^(start_session|ask_agent|spawn_agent)$/.test(name)) return 'Delegating a part of the work';
  if (/^(send_to_session|send_message)$/.test(name)) return 'Coordinating with another agent';
  if (/^(wait|wait_for_session|wait_agent|write_stdin)$/.test(name)) return 'Waiting for work to finish';
  if (/screenshot|view_image/.test(name)) return 'Inspecting the visual result';
  if (/browser|navigate/.test(name)) return 'Checking the page';
  if (/^(thinking|model)$/.test(name)) return 'Planning the next step';
  return 'Working on the task';
}

/** Each conversation has a stable identity, even when providers match. */
export function agentGlyph(key: string): string {
  let hash = 0;
  for (const char of key) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return ['◈', '✳', '◉', '✦', '▧', '◎', '❖', '⊕'][hash % 8];
}

export function recentSteps(row: SubagentRow): { label: string; status: string }[] {
  const out: { label: string; status: string }[] = [];
  for (const step of row.activity?.recent ?? []) {
    const label = activityWords(step);
    if (out.some(s => s.label === label && s.status === step.status)) continue;
    out.push({ label, status: step.status });
    if (out.length === 3) break;
  }
  return out.toReversed();
}

/** How long it worked: "45s", "4m 12s", "1h 4m" (seconds dropped from
 *  an hour on), the chat's own form (`durationText`, "Worked for 1m 34s").
 *  Never negative. */
export function durationWords(ms: number): string {
  return durationText(Number.isFinite(ms) ? ms : 0);
}

/** A token count: "950", "1.2K", "182K", "1.2M". */
export function tokenWords(n: number): string {
  const v = Math.max(0, Math.round(Number.isFinite(n) ? n : 0));
  if (v < 1000) return String(v);
  const short = (x: number) => (x < 10 ? x.toFixed(1).replace(/\.0$/, '') : String(Math.round(x)));
  const k = v / 1000;
  if (Math.round(k) < 1000) return `${short(k)}K`;
  return `${short(v / 1_000_000)}M`;
}

/** 1 → "1st", 2 → "2nd", 11 → "11th", 23 → "23rd". */
export function ordinal(n: number): string {
  const tens = n % 100;
  const suffix = tens >= 11 && tens <= 13 ? 'th' : n % 10 === 1 ? 'st' : n % 10 === 2 ? 'nd' : n % 10 === 3 ? 'rd' : 'th';
  return `${n}${suffix}`;
}

/** A queued row's place: "Queued, 2nd"; "Queued" when it has none. */
export function placeWords(place: number | null | undefined): string {
  return place && place > 0 ? `Queued, ${ordinal(place)}` : 'Queued';
}

/** What a queued row waits for, as the pump words it ("Waiting for a free
 *  subagent slot: this chat is at its limit of 6 subagents at once (Settings, Chat, Chats at
 *  once)"); empty when it isn't queued or nothing but its place holds it. */
export function waitingWords(row: Pick<SubagentRow, 'status' | 'waitingFor'>): string {
  return row.status === 'queued' && row.waitingFor ? `Waiting for ${row.waitingFor}` : '';
}

/** A row's state in words. */
export function statusWords(row: Pick<SubagentRow, 'status' | 'queuePlace'>): string {
  switch (row.status) {
    case 'running': return 'Running';
    case 'queued': return placeWords(row.queuePlace);
    case 'failed': return 'Failed';
    case 'stopped': return 'Stopped';
    default: return 'Done';
  }
}

/** Milliseconds since the read (`at`), never negative. */
function since(at: string, now: number): number {
  const t = Date.parse(at);
  return Number.isFinite(t) ? Math.max(0, now - t) : 0;
}

/** Working time now: a running row's `workingMs` is as of the read, so it
 *  grows by the time since; any other row's is final. */
export function liveWorkingMs(row: Pick<SubagentRow, 'status' | 'workingMs'>, at: string, now: number): number {
  return row.status === 'running' ? row.workingMs + since(at, now) : row.workingMs;
}

/** The wall span now, the same way. */
export function liveWallMs(row: Pick<SubagentRow, 'status' | 'wallMs'>, at: string, now: number): number {
  return row.status === 'running' ? row.wallMs + since(at, now) : row.wallMs;
}

/** How old a read may be and still tick a running row. */
export const TICK_MAX_AGE_MS = 30_000;

/** The time live elapsed counts from: the read's `at` while it is recent,
 *  else now, so a cached view (the window was hidden, nothing used it for a
 *  while) shows its recorded time without ticking until a fresh read comes. */
export function tickFrom(at: string, now: number, maxAgeMs = TICK_MAX_AGE_MS): string {
  const t = Date.parse(at);
  return Number.isFinite(t) && now - t <= maxAgeMs ? at : new Date(now).toISOString();
}

/** "16h ago", "5m ago", "3d ago", "just now". Empty for no time. */
export function agoWords(at: string | null | undefined, now: number): string {
  const t = Date.parse(at ?? '');
  if (!Number.isFinite(t)) return '';
  const minutes = Math.floor(Math.max(0, now - t) / 60_000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

/** Older finished subagents one "Show older" click pages in (the backend's `OLDER_BATCH`). */
export const OLDER_BATCH = 25;

/** How many older finished subagents "Show older" still has to list. */
export function olderLeft(view: Pick<SubagentsView, 'doneOlder' | 'olderTotal'> | null | undefined): number {
  return view ? Math.max(0, view.olderTotal - view.doneOlder.length) : 0;
}

/** The chip's count: "2 running, 5 done", "1 queued", "3 done". Empty when
 *  the chat started none (the chip hides then). */
export function countsText(view: Pick<SubagentsView, 'running' | 'queued' | 'doneCount'> | null | undefined): string {
  if (!view) return '';
  const parts: string[] = [];
  if (view.running) parts.push(`${view.running} running`);
  if (view.queued) parts.push(`${view.queued} queued`);
  if (view.doneCount) parts.push(`${view.doneCount} done`);
  return parts.join(', ');
}

/** Whether the chat has any subagents to show (earlier chats alone don't count). */
export function hasSubagents(view: Pick<SubagentsView, 'running' | 'queued' | 'doneCount' | 'active' | 'done'> | null | undefined): boolean {
  return !!view && (view.running + view.queued + view.doneCount > 0 || view.active.length + view.done.length > 0);
}

/** Whether the chat header shows the Subagents chip: subagents, or earlier
 *  chats alone (L8, 11-10: a project's other chats left the sidebar for its
 *  Earlier chats, so the chip is their way in; the COO's quick chats list in
 *  the sidebar's Quick chats section since 2026-11-14, not here). */
export function chipShown(view: Pick<SubagentsView, 'running' | 'queued' | 'doneCount' | 'active' | 'done' | 'earlier'> | null | undefined): boolean {
  return hasSubagents(view) || (view?.earlier.length ?? 0) > 0;
}

/** The chip's words: its counts, or "4 earlier chats" when it has only
 *  earlier chats. */
export function chipWords(view: Pick<SubagentsView, 'running' | 'queued' | 'doneCount' | 'earlier'> | null | undefined): string {
  const counts = countsText(view);
  if (counts || !view?.earlier.length) return counts;
  const n = view.earlier.length;
  return `${n} earlier chat${n === 1 ? '' : 's'}`;
}

/** Up to three distinct agents for the chip's glyphs: active ones first,
 *  then the newest done. */
export function chipAgents(view: Pick<SubagentsView, 'active' | 'done'> | null | undefined, max = CHIP_GLYPHS): string[] {
  const out: string[] = [];
  for (const row of [...(view?.active ?? []), ...(view?.done ?? [])]) {
    if (!out.includes(row.agent)) out.push(row.agent);
    if (out.length >= max) break;
  }
  return out;
}

/** What the catalogs say about one agent's models. */
export interface ModelNames {
  models?: readonly { id: string; label: string }[];
  /** An alias's pinned model (`sonnet` → its full id), so a pinned id is
   *  named by its alias's label. */
  newestByFamily?: Readonly<Record<string, string>>;
}

/** A model's name: its catalog label, or the label of the alias that
 *  resolves to it, else the id as recorded. Null for no model. */
export function modelWords(model: string | null | undefined, names?: ModelNames | null): string | null {
  const id = model?.trim();
  if (!id) return null;
  const models = names?.models ?? [];
  const direct = models.find(m => m.id === id)?.label;
  if (direct) return direct;
  const alias = Object.entries(names?.newestByFamily ?? {}).find(([, pinned]) => pinned === id)?.[0];
  const aliased = alias ? models.find(m => m.id === alias)?.label : undefined;
  return aliased || id;
}

/** "Medium", "Extra high"; null for none. */
export function effortWords(effort: string | null | undefined): string | null {
  const e = effort?.trim();
  if (!e) return null;
  return e === 'xhigh' ? 'Extra high' : e.charAt(0).toUpperCase() + e.slice(1);
}

/** "182K tokens", "1 token". */
export function tokensLabel(n: number): string {
  return `${tokenWords(n)} ${Math.round(n) === 1 ? 'token' : 'tokens'}`;
}

/** Model and effort: "Sonnet 5.5 · Medium"; the agent's name when the
 *  model wasn't recorded. */
export function modelEffort(row: Pick<SubagentRow, 'agent' | 'model' | 'effort'>, names?: ModelNames | null): string {
  return [modelWords(row.model, names) ?? agentLabel(row.agent) ?? row.agent, effortWords(row.effort)].filter(Boolean).join(' · ');
}

/** A row's right-aligned word, the reference's "16h ago": when it ended
 *  ("Failed 2h ago" for one that didn't finish), the live elapsed while it
 *  runs, its place while it waits. */
export function rowWhen(row: SubagentRow, at: string, now: number): string {
  if (row.status === 'running') return durationWords(liveWorkingMs(row, at, now));
  if (row.status === 'queued') return placeWords(row.queuePlace);
  const ago = agoWords(row.endedAt, now);
  if (row.status === 'failed' || row.status === 'stopped') return ago ? `${statusWords(row)} ${ago}` : statusWords(row);
  return ago;
}

/** A row's quiet second line, Hollow's addition under the title: model and
 *  effort, how long it worked (a running row's ticks at the right instead)
 *  and tokens. A queued row that hasn't worked yet has neither. */
export function rowMeta(row: SubagentRow, at: string, now: number, names?: ModelNames | null): string[] {
  const worked = row.status !== 'running' && row.workingMs > 0;
  return [
    waitingWords(row),
    modelEffort(row, names),
    worked ? durationWords(liveWorkingMs(row, at, now)) : '',
    row.status === 'queued' && !row.tokens ? '' : tokensLabel(row.tokens),
  ].filter(Boolean);
}

/** The row's tooltip: wall span beside the working time, the cached tokens
 *  beside input and output. */
export function rowTitle(row: SubagentRow, at: string, now: number): string {
  return [
    row.title ?? 'Subagent',
    row.status === 'queued'
      ? [statusWords(row), waitingWords(row)].filter(Boolean).join(' · ')
      : `${outcomeWords(row)} · worked ${durationWords(liveWorkingMs(row, at, now))} over ${durationWords(liveWallMs(row, at, now))}`,
    `${tokenWords(row.inputTokens)} in (${tokenWords(row.cachedInputTokens)} cached) · ${tokenWords(row.outputTokens)} out`,
    outcomeDetail(row),
    row.trackerIssue ? `Issue ${row.trackerIssue}` : '',
  ].filter(Boolean).join('\n');
}

/** The session view's header: "Sonnet 5.5 · Medium · 4m 12s · 182K tokens";
 *  a queued one's place instead of its time and tokens. */
export function headerMeta(row: SubagentRow, at: string, now: number, names?: ModelNames | null): string {
  if (row.status === 'queued' && !row.workingMs) return [modelEffort(row, names), placeWords(row.queuePlace), waitingWords(row)].filter(Boolean).join(' · ');
  return [modelEffort(row, names), durationWords(liveWorkingMs(row, at, now)), tokensLabel(row.tokens)].join(' · ');
}

/** The CFO's choice for its latest turn, under the session header: "CFO:
 *  Claude · Sonnet 5.5 · Medium · <reason>", the reason's first line only.
 *  Null without a decision (a turn before the CFO, or one you chose). */
export function cfoHeaderLine(decision: CfoDecision | null | undefined, names?: ModelNames | null): string | null {
  if (!decision) return null;
  const reason = decision.reason?.split('\n').find(l => l.trim())?.trim() ?? '';
  const agent = agentLabel(decision.agent) ?? decision.agent;
  const line = cfoFooterLine({ ...decision, reason }, model => modelWords(model, names) ?? undefined, () => agent);
  if (!line) return null;
  return decision.model && agent ? `CFO: ${agent} · ${line}` : `CFO: ${line}`;
}

/** The first words of a prompt, for a subagent with no title yet. */
export function firstWords(text: string, max = 8): string {
  const words = text.replace(/\s+/g, ' ').trim().split(' ').filter(Boolean);
  if (!words.length) return '';
  return words.length > max ? `${words.slice(0, max).join(' ')}…` : words.join(' ');
}

/** A row by its conversation (the started one, or any it handed over to). */
export function findRow(view: Pick<SubagentsView, 'active' | 'done' | 'doneOlder' | 'earlier'> | null | undefined, key: string | null | undefined): SubagentRow | null {
  if (!view || !key) return null;
  return [...view.active, ...view.done, ...view.doneOlder, ...view.earlier].find(r => r.key === key || r.current === key || r.conversations.includes(key)) ?? null;
}

/** Stop a subagent once the person confirms. Its live queue row is read when
 *  clicked, before the confirm opens, and the stop goes by that row: by then
 *  its turn may have ended and a held message or a send_to_session started
 *  the next one on the same key. A row with no queue row (a turn the queue no
 *  longer lists) is refused (`noRow`), never stopped by key: the polled row
 *  can't prove which turn holds the key's stop slot when the stop lands
 *  (overnight 11-13, Codex review). */
export async function stopSubagent(row: () => SubagentRow | null, deps: { confirm: () => Promise<boolean>; cancelRow: (id: number) => Promise<RunQueueItem> }): Promise<RowStop> {
  const r = row();
  if (!r) return 'declined';
  if (r.queueId == null) return 'noRow';
  return stopRowAfterConfirm(r.queueId, deps.confirm, deps.cancelRow);
}

/** Poll every 3 s while anything is active, else every 20 s. */
export function pollMs(view: Pick<SubagentsView, 'running' | 'queued'> | null | undefined): number {
  return view && view.running + view.queued > 0 ? 3_000 : 20_000;
}

/** A tool call that starts or messages a subagent, by Hollow's app tools
 *  (`start_session`, `send_to_session`, or `run_action` with
 *  `sessions.start`): its kind, the words to show and, once its result came
 *  back, the conversation's key. Null for any other call. */
export function subagentCall(name: string, input: unknown, result?: string | null): { kind: 'start' | 'message'; label: string; sessionKey: string | null } | null {
  const parts = name.split('__');
  const tool = parts[parts.length - 1] ?? '';
  if (parts[0] !== 'mcp' || parts[1] !== 'hollow_app') return null;
  const i = input && typeof input === 'object' ? (input as Record<string, unknown>) : {};
  const args = tool === 'run_action' && i.id === 'sessions.start' && i.args && typeof i.args === 'object' ? (i.args as Record<string, unknown>) : i;
  const kind = tool === 'start_session' || (tool === 'run_action' && i.id === 'sessions.start') ? 'start' : tool === 'send_to_session' ? 'message' : null;
  if (!kind) return null;
  let sessionKey: string | null = null;
  try {
    const parsed = result ? JSON.parse(result) as { sessionKey?: unknown } : null;
    if (parsed && typeof parsed.sessionKey === 'string' && parsed.sessionKey) sessionKey = parsed.sessionKey;
  } catch { /* not JSON yet (running), or an error message */ }
  if (!sessionKey && kind === 'message' && typeof args.session === 'string' && args.session) sessionKey = args.session;
  const title = typeof args.title === 'string' ? args.title.trim() : '';
  const prompt = typeof args.prompt === 'string' ? args.prompt : '';
  const about = title || firstWords(prompt);
  if (kind === 'message') return { kind, label: about ? `Messaged a subagent: ${about}` : 'Messaged a subagent', sessionKey };
  return { kind, label: about ? `Started a subagent: ${about}` : 'Started a subagent', sessionKey };
}

/** One exchange of a subagent's transcript: what it was asked, its answer,
 *  how long it worked and the History run that holds its changes. */
export interface Reply {
  id: string;
  ask: string | null;
  answer: string;
  workedMs: number | null;
  /** Set when the reply changed files (History & recovery shows them). */
  historyRunId: string | null;
  error: string | null;
  /** When its last receipt finished, for the footer's time. */
  finishedAt: string | null;
  /** What the CFO chose for its turn, when the receipt recorded it. */
  cfo: CfoDecision | null;
  /** The run id of the turn that answered: Useful and Not useful save
   *  against it (overnight 11-12, C10). A turn of another run starts its own
   *  reply, so a vote never lands on a later run (overnight 11-13). */
  runId: string | null;
  /** A turn of it is still running (the turn's `status`): its answer is
   *  still streaming, so it offers no Copy, Read aloud or vote yet. */
  running: boolean;
}

/** The transcript as exchanges: each request, then the text of the turns
 *  that answered it, the span of its last receipt and, when a receipt
 *  recorded file or repository changes, its History run. */
export function replies(turns: readonly (Pick<AgentConversationTurn, 'id' | 'role' | 'text' | 'error' | 'receipt'> & Partial<Pick<AgentConversationTurn, 'runId' | 'status'>>)[]): Reply[] {
  const out: Reply[] = [];
  let current: Reply | null = null;
  const fresh = (id: string, ask: string | null): Reply => ({ id, ask, answer: '', workedMs: null, historyRunId: null, error: null, finishedAt: null, cfo: null, runId: null, running: false });
  for (const t of turns) {
    if (t.role === 'user') {
      current = fresh(t.id, t.text);
      if (t.status === 'running') current.running = true;
      out.push(current);
      continue;
    }
    // An answer from another run is its own reply, with its own vote.
    // An earlier answer with no run id splits too, so a vote never moves to a later run.
    if (!current || (current.answer && t.runId && t.runId !== current.runId)) { current = fresh(t.id, null); out.push(current); }
    if (t.status === 'running') current.running = true;
    if (t.text.trim()) current.answer = current.answer ? `${current.answer}\n\n${t.text}` : t.text;
    if (t.error) current.error = t.error;
    if (t.runId) current.runId = t.runId;
    const r = t.receipt;
    if (r) {
      const from = Date.parse(r.startedAt), to = Date.parse(r.finishedAt);
      if (Number.isFinite(from) && Number.isFinite(to) && to >= from) current.workedMs = to - from;
      if (Number.isFinite(to)) current.finishedAt = r.finishedAt;
      if (r.cfo) current.cfo = r.cfo;
      if (r.historyRunId && ((r.files?.length ?? 0) > 0 || (r.repoChanges?.length ?? 0) > 0)) current.historyRunId = r.historyRunId;
    }
  }
  return out;
}

/** The latest CFO decision in the transcript. */
export function latestCfo(list: readonly Pick<Reply, 'cfo'>[]): CfoDecision | null {
  for (let i = list.length - 1; i >= 0; i--) if (list[i].cfo) return list[i].cfo;
  return null;
}

/** What a send to a subagent says: held behind its running turn, waiting in
 *  the queue, or sent. */
export function sentWords(accepted: { heldId?: string | null; queuePosition?: number | null }): string {
  if (accepted.heldId) return 'Queued: it goes when the running turn ends';
  if (accepted.queuePosition && accepted.queuePosition > 0) return `Queued, ${ordinal(accepted.queuePosition)}`;
  return 'Sent';
}

/** Row numbers for live subagents, stable within one chat: a number per key
 *  in first-seen order, restarting when the chat (`parent`) changes. */
export function createIdentities() {
  let owner: string | null = null;
  const numbers = new Map<string, number>();
  return (parent: string, key: string): number => {
    if (owner !== parent) { owner = parent; numbers.clear(); }
    if (!numbers.has(key)) numbers.set(key, numbers.size + 1);
    return numbers.get(key)!;
  };
}
