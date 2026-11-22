import type { IconName } from './icons';
import { ICON_MEANING } from './iconMeaning';
/**
 * One-line descriptions of agent tool calls: the icon, verb and target a
 * chat row shows ("Read …/components/ToolLedger.tsx"), and the turn-level
 * summary a ledger header shows ("Ran 2 commands, read ToolLedger.tsx").
 * Pure: no Solid, no store, so the node vitest suite can load it.
 *
 * A call is described by what it was FOR where the agent said so: a Bash
 * call's `description` ("Record the test results") leads, with the command
 * beside it; an MCP call shows what it acted on (a session key, a query, an
 * action id) rather than only the server's name.
 */
import { toolFamily, type FoldStatus } from './toolFolding';
import { editStats } from './toolDiff';
import { fileKind, type FileTone } from './fileChip';

export interface ToolMeta {
  icon: IconName;
  verb: string;
  target: string;
  mono?: boolean;
  extra?: string;
  /** The full path a file tool acted on; a row shows it as a file chip. */
  file?: string;
}

/** Hollow's own MCP tools, so the row reads as an action rather than as the
 *  server name followed by a restatement of the tool name. */
const MCP_VERBS: Record<string, string> = {
  search_vault: 'Search your vault',
  open_url: 'Open', open_app: 'Open app', search_bookmarks: 'Search bookmarks',
  browser_current_tab: 'Read browser tab', browser_list_tabs: 'List browser tabs',
  quit_app: 'Quit app', ask_codex: 'Ask Codex', ask_antigravity: 'Ask Antigravity',
};

/** An MCP server's icon, so a row shows which of Hollow's tools it used. */
const MCP_ICONS: Record<string, IconName> = {
  hollow_app: 'appWindow', hollow_tasks: 'checkSquare', hollow_vault: 'book',
  hollow_computer: 'globe', hollow_questions: 'message', hollow_team: 'users',
};

/** The argument an MCP call is most plausibly "about", in priority order. */
const MCP_TARGET_KEYS = [
  'subject', 'query', 'q', 'url', 'key', 'session', 'sessionKey', 'workspace', 'runId', 'id',
  'file_path', 'path', 'pattern', 'name', 'title', 'page', 'section', 'prompt', 'description',
];
/** Targets are clipped: the full input is one click away. */
const TARGET_MAX = 80;

function obj(v: unknown): Record<string, unknown> {
  return v && typeof v === 'object' ? (v as Record<string, unknown>) : {};
}
function str(v: unknown): string {
  return typeof v === 'string' ? v : '';
}
function clip(s: string, max = TARGET_MAX): string {
  const one = s.replace(/\s+/g, ' ').trim();
  return one.length > max ? one.slice(0, max - 1) + '…' : one;
}

/** Last two path segments, e.g. `…/tasks/foo.md`. */
export function shortPath(p: string): string {
  if (!p) return '';
  const parts = p.split('/').filter(Boolean);
  return parts.length <= 2 ? p : '…/' + parts.slice(-2).join('/');
}

export function baseName(p: string): string {
  const parts = p.split('/').filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

function humanize(raw: string): string {
  const s = raw.replace(/_/g, ' ');
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/** The one-line summary of a tool call (icon, verb, target). */
export function describeTool(name: string, input: unknown): ToolMeta {
  const i = obj(input);
  switch (name) {
    case 'Read': {
      const offset = typeof i.offset === 'number' ? i.offset : 0;
      const limit = typeof i.limit === 'number' ? i.limit : 0;
      const extra = limit ? `lines ${offset || 1}–${(offset || 1) + limit - 1}` : str(i.pages) ? `pages ${str(i.pages)}` : '';
      return { icon: 'fileText', verb: 'Read', target: shortPath(str(i.file_path)), mono: true, extra, file: str(i.file_path) };
    }
    case 'Write':      return { icon: 'filePen', verb: 'Write', target: shortPath(str(i.file_path)), mono: true, file: str(i.file_path) };
    case 'Edit':       return { icon: 'filePen', verb: 'Edit', target: shortPath(str(i.file_path)), mono: true, extra: i.replace_all === true ? 'every match' : '', file: str(i.file_path) };
    case 'MultiEdit': {
      const n = Array.isArray(i.edits) ? (i.edits as unknown[]).length : 0;
      return { icon: 'filePen', verb: 'Edit', target: shortPath(str(i.file_path)), mono: true, extra: n ? `${n} edits` : '', file: str(i.file_path) };
    }
    case 'NotebookEdit': return { icon: 'filePen', verb: 'Edit', target: shortPath(str(i.notebook_path)), mono: true, file: str(i.notebook_path) };
    case 'Bash': {
      // The agent's own one-line reason leads; the command sits beside it.
      const why = clip(str(i.description));
      const cmd = clip(str(i.command));
      return why
        ? { icon: 'terminal', verb: why, target: cmd, mono: true, extra: i.run_in_background === true ? 'background' : '' }
        : { icon: 'terminal', verb: 'Run', target: cmd, mono: true, extra: i.run_in_background === true ? 'background' : '' };
    }
    case 'Grep':       return { icon: 'search', verb: 'Search', target: str(i.pattern), mono: true, extra: str(i.path) ? `in ${shortPath(str(i.path))}` : '' };
    case 'Glob':       return { icon: 'search', verb: 'Find', target: str(i.pattern), mono: true };
    case 'WebFetch':   return { icon: 'globe', verb: 'Fetch', target: str(i.url), mono: true };
    case 'WebSearch':  return { icon: 'globe', verb: 'Search web', target: str(i.query), mono: false };
    case 'LS':         return { icon: 'folder', verb: 'List', target: shortPath(str(i.path)), mono: true };
    case 'Task':
    case 'Agent':      return { icon: 'bot', verb: 'Delegate', target: clip(str(i.description) || str(i.subagent_type)), extra: str(i.description) ? str(i.subagent_type) : '' };
    case 'Skill':      return { icon: ICON_MEANING.skill, verb: 'Use skill', target: str(i.skill), mono: true };
    case 'ToolSearch': return { icon: 'wrench', verb: 'Load tools', target: clip(str(i.query).replace(/^select:/, '')), mono: true };
    case 'TodoWrite': {
      const n = Array.isArray(i.todos) ? (i.todos as unknown[]).length : 0;
      const done = Array.isArray(i.todos) ? (i.todos as unknown[]).filter(t => obj(t).status === 'completed').length : 0;
      return { icon: 'listTodo', verb: 'Update plan', target: n ? `${done}/${n} done` : '' };
    }
    default:
      if (name.startsWith('mcp__')) {
        const parts = name.split('__');
        const raw = parts[parts.length - 1] || name;
        const server = (parts[1] || '').replace(/[-_]/g, ' ').slice(0, 18);
        const icon = MCP_ICONS[parts[1] ?? ''] ?? 'plug';
        // Hollow's on-demand reader: the file it read and the lines asked.
        if (raw === 'read_source' && parts[1] === 'hollow_vault') {
          const path = str(i.path);
          const from = typeof i.from_line === 'number' ? i.from_line : null;
          const to = typeof i.to_line === 'number' ? i.to_line : null;
          const lines = from == null ? '' : to == null ? `from line ${from}` : `lines ${from}–${to}`;
          // Asked by intent (repo features R3): what it looked for.
          const query = str(i.query).trim();
          const extra = query ? findLabel(query) : lines;
          return { icon, verb: 'Read a source', target: path ? shortPath(path) : '', extra };
        }
        // A conversation read (repo features R4): what it looked for, or
        // the turns it read by number.
        if (raw === 'read_session' && parts[1] === 'hollow_app') {
          const query = str(i.query).trim();
          const from = typeof i.fromTurn === 'number' ? i.fromTurn : null;
          const to = typeof i.toTurn === 'number' ? i.toTurn : from;
          const turns = from == null ? '' : to === from ? `turn ${from}` : `turns ${from}–${to}`;
          const extra = query ? findLabel(query) : turns;
          return { icon, verb: 'Read a conversation', target: clip(str(i.key)), mono: true, extra: extra || `via ${server}` };
        }
        // A subagent started or messaged (COO rebuild L3): the transcript
        // draws it as its own line (SubagentLine); ledgers and summaries say
        // it plainly, with the first words of what it was asked.
        if ((raw === 'start_session' || raw === 'send_to_session') && parts[1] === 'hollow_app') {
          return { icon: 'bot', verb: raw === 'start_session' ? 'Started a subagent' : 'Messaged a subagent', target: clip(str(i.prompt), 60) };
        }
        // Hollow's own MCP verbs get a real verb and carry the argument as the
        // target; the server name is not a verb.
        const hit = MCP_VERBS[raw];
        if (hit) {
          return { icon, verb: hit, target: str(i.subject) || str(i.query) || str(i.url) || str(i.name) || '' };
        }
        const about = MCP_TARGET_KEYS.map(k => str(i[k])).find(Boolean) ?? '';
        return about
          ? { icon, verb: humanize(raw), target: clip(about), mono: true, extra: server ? `via ${server}` : '' }
          : { icon, verb: humanize(raw), target: server ? `via ${server}` : '' };
      }
      return { icon: 'zap', verb: name, target: '' };
  }
}

/** A query call's `find “…”`, cut at 40 code points (never inside a
 *  surrogate pair). */
function findLabel(query: string): string {
  const points = Array.from(query);
  return `find “${points.length > 40 ? `${points.slice(0, 39).join('')}…` : query}”`;
}

const IMAGE_EXT = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'heic', 'bmp', 'ico', 'avif']);
function ext(path: string): string {
  const m = /\.([A-Za-z0-9]+)$/.exec(path);
  return m ? m[1].toLowerCase() : '';
}

// A file chip's icon and colour, by file type: one map for the tool ledger
// and a reply's file links (fileChip.ts, 2026-11-10).
export { fileKind, type FileTone } from './fileChip';

/** Lines in a Read result: Claude Code numbers every line it returns
 *  (`  12→text` or `12\ttext`), so only numbered lines count and an
 *  appended system note does not. */
export function readLineCount(result: string | undefined): number {
  if (!result) return 0;
  let n = 0;
  for (const line of result.split('\n')) if (/^\s*\d+(→|\t)/.test(line)) n++;
  return n;
}

function lines(n: number): string {
  return `${n} ${n === 1 ? 'line' : 'lines'}`;
}

/** A chat row: the call's one-line description plus what only a row shows,
 *  once the result is in: how many lines a read returned or a write wrote,
 *  an edit's `+a −d`, a picture read as an image. */
export interface ToolRow extends ToolMeta {
  stats?: { additions: number; deletions: number };
}

/** `readLines`: a trimmed Read result's line count (WD1), else counted. */
export function describeRow(name: string, input: unknown, result?: string, readLines?: number): ToolRow {
  const meta: ToolRow = { ...describeTool(name, input) };
  const i = obj(input);
  switch (name) {
    case 'Read': {
      const e = ext(str(i.file_path));
      const n = readLines ?? readLineCount(result);
      if (IMAGE_EXT.has(e)) meta.verb = 'Read image';
      else if (e === 'pdf') meta.verb = 'Read PDF';
      else if (n > 0) {
        meta.verb = `Read ${lines(n)}`;
        // "Read 20 lines" already says how much; keep where it started.
        if (typeof i.offset === 'number' && i.offset > 1) meta.extra = `from line ${i.offset}`;
        else meta.extra = '';
      }
      break;
    }
    case 'Write': {
      const stats = editStats(name, input);
      if (stats && stats.additions > 0) meta.verb = `Write ${lines(stats.additions)}`;
      break;
    }
    case 'Edit': case 'MultiEdit': case 'NotebookEdit': {
      const stats = editStats(name, input);
      if (stats) meta.stats = stats;
      break;
    }
  }
  return meta;
}

/** What a ledger header needs from each call. */
export interface SummaryTool { name: string; input?: unknown; status?: FoldStatus }

export interface LedgerSummary {
  /** "Ran 2 commands, read ToolLedger.tsx, edited 3 files". */
  text: string;
  /** The icon of the family with the most calls. */
  icon: IconName;
  /** The call running now, when one is. */
  running?: ToolMeta;
  failed: number;
}

/** A family's phrase inside a summary: the call itself when there is one,
 *  a count when there are several. */
function familyPhrase(family: string, calls: SummaryTool[]): string {
  const n = calls.length;
  if (n === 1) {
    const m = describeTool(calls[0].name, calls[0].input);
    // A command the agent explained is summarized by its explanation alone.
    if (family === 'shell' && m.verb !== 'Run') return m.verb;
    const target = m.mono && /[/\\]/.test(m.target) && family !== 'shell' ? baseName(m.target) : m.target;
    return clip(target ? `${m.verb} ${target}` : m.verb, 60);
  }
  switch (family) {
    case 'read': return `read ${n} files`;
    case 'search': return `searched ${n} times`;
    case 'edit': {
      const files = new Set(calls.map(c => str(obj(c.input).file_path) || str(obj(c.input).notebook_path)).filter(Boolean));
      return files.size === 1 ? `edited ${baseName([...files][0])} ${n} times` : `edited ${files.size || n} files`;
    }
    case 'shell': return `ran ${n} commands`;
    case 'web': return `${n} web lookups`;
    case 'todo': return `updated the plan ${n} times`;
    default: {
      if (family.startsWith('mcp:')) {
        const server = family.slice(4).replace(/[-_]+/g, ' ').trim();
        const kinds = new Set(calls.map(c => c.name));
        if (kinds.size === 1) return `${describeTool(calls[0].name, calls[0].input).verb.toLowerCase()} ×${n}`;
        return `${n} ${server || 'tool'} calls`;
      }
      return `${family} ×${n}`;
    }
  }
}

/** The turn-level header of a ledger: up to three family phrases in the
 *  order the agent first reached for them, then "+N more". */
export function ledgerSummary(tools: readonly SummaryTool[], maxParts = 3): LedgerSummary {
  const failed = tools.filter(t => t.status === 'error').length;
  const live = [...tools].reverse().find(t => t.status === 'running');
  const running = live ? describeTool(live.name, live.input) : undefined;
  if (tools.length === 0) return { text: '', icon: 'terminal', running, failed };

  const byFamily = new Map<string, SummaryTool[]>();
  for (const t of tools) {
    const f = toolFamily(t.name);
    const list = byFamily.get(f);
    if (list) list.push(t); else byFamily.set(f, [t]);
  }
  const families = [...byFamily.entries()];
  // Phrases join mid-sentence, so each starts lower-case; the text is
  // capitalized once at the end.
  const phrases = families.slice(0, maxParts).map(([f, calls]) => {
    const p = familyPhrase(f, calls);
    return p.charAt(0).toLowerCase() + p.slice(1);
  });
  const rest = families.length - phrases.length;
  let text = phrases.join(', ');
  if (rest > 0) text += ` +${rest} more`;
  text = text.charAt(0).toUpperCase() + text.slice(1);

  const [topFamily] = families.reduce((a, b) => (b[1].length > a[1].length ? b : a));
  const top = byFamily.get(topFamily)![0];
  return { text, icon: describeTool(top.name, top.input).icon, running, failed };
}

/** The header's count: "7 tool calls, 1 subagent". A delegation is a
 *  subagent, not a tool call. */
export function ledgerCounts(tools: readonly SummaryTool[]): string {
  const agents = tools.filter(t => t.name === 'Agent' || t.name === 'Task').length;
  const calls = tools.length - agents;
  const parts: string[] = [];
  if (calls > 0 || agents === 0) parts.push(`${calls} ${calls === 1 ? 'tool call' : 'tool calls'}`);
  if (agents > 0) parts.push(`${agents} ${agents === 1 ? 'subagent' : 'subagents'}`);
  return parts.join(', ');
}

export interface LedgerGlyph { icon: IconName; status: FoldStatus; title: string }

/** One small icon per call for the header, in call order, capped at `max`
 *  (the rest is counted in `more`). */
export function ledgerGlyphs(tools: readonly SummaryTool[], max = 14): { glyphs: LedgerGlyph[]; more: number } {
  const glyphs = tools.slice(0, max).map(t => {
    const m = describeTool(t.name, t.input);
    return { icon: m.icon, status: t.status ?? 'done', title: m.target ? `${m.verb} ${m.target}` : m.verb };
  });
  return { glyphs, more: Math.max(0, tools.length - max) };
}

export interface GroupFile { path: string; name: string; icon: IconName; tone: FileTone }

/** The distinct files a folded run touched, first-seen order, capped at
 *  `max` (the rest is counted in `more`), plus the run's summed `+a −d` when
 *  it edited. */
export function groupFiles(
  tools: readonly SummaryTool[],
  max = 3,
): { files: GroupFile[]; more: number; stats: { additions: number; deletions: number } | null } {
  const seen = new Map<string, GroupFile>();
  let additions = 0, deletions = 0, edited = false;
  for (const t of tools) {
    const m = describeTool(t.name, t.input);
    if (m.file && !seen.has(m.file)) seen.set(m.file, { path: m.file, name: baseName(m.file), ...fileKind(m.file) });
    const st = editStats(t.name, t.input);
    if (st) { edited = true; additions += st.additions; deletions += st.deletions; }
  }
  const all = [...seen.values()];
  return {
    files: all.slice(0, max),
    more: Math.max(0, all.length - max),
    stats: edited ? { additions, deletions } : null,
  };
}

/** A finished ledger with at most this many calls stays open, so its rows
 *  (chips, pills, counts) are visible without a click; a longer one folds to
 *  its header. */
export const LEDGER_OPEN_MAX = 8;

/** Whether a ledger the user hasn't toggled is open: always while a call
 *  runs, and afterwards only when it is short. */
export function ledgerDefaultOpen(count: number, running: boolean): boolean {
  return running || count <= LEDGER_OPEN_MAX;
}
