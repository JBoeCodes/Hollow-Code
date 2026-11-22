/**
 * Tool-call folding and the Raw view's window math (Paperclip PR 21). Pure:
 * no Solid, no store, so the node vitest suite can load it.
 *
 * A long agent turn is mostly runs of the same kind of action (read eight
 * files, search five times). Three or more consecutive calls of one family
 * fold into a single group row ("Read 4 files") that expands to its members.
 * A call carrying an approval badge never folds, and it splits the run
 * around it, so an approval is always visible at a glance. The per-turn Raw
 * view shows every call unfolded; past `RAW_VIRTUAL_THRESHOLD` rows it
 * becomes a fixed-height windowed list of one-line summaries.
 *
 * Families use the tool names after PR 13's Codex normalization: an MCP call
 * is `mcp__<server>__<tool>` from every agent, so it groups by server.
 */

/** Runs shorter than this stay as individual rows. */
export const FOLD_MIN = 3;
/** Past this many calls the Raw view windows its rows. */
export const RAW_VIRTUAL_THRESHOLD = 300;
/** The fixed height of one windowed Raw row, in px. */
export const RAW_ROW_PX = 28;

export type FoldStatus = 'running' | 'done' | 'error';

/** The fields folding reads from a chat tool call. */
export interface FoldableTool {
  name: string;
  id?: string;
  status?: FoldStatus;
  /** Any approval state at all keeps the call out of a group. */
  approval?: unknown;
}

export type LedgerItem<T> =
  | { kind: 'single'; key: string; index: number; tool: T }
  | { kind: 'group'; key: string; family: string; start: number; tools: T[] };

/** The family a tool belongs to. Unknown tools are their own family. */
export function toolFamily(name: string): string {
  switch (name) {
    case 'Read': case 'NotebookRead': case 'LS':
      return 'read';
    case 'Grep': case 'Glob':
      return 'search';
    case 'Edit': case 'MultiEdit': case 'Write': case 'NotebookEdit':
      return 'edit';
    case 'Bash':
      return 'shell';
    case 'WebFetch': case 'WebSearch': case 'web_search':
      return 'web';
    case 'TodoWrite':
      return 'todo';
    default: {
      const mcp = /^mcp__(.+?)__/.exec(name);
      return mcp ? `mcp:${mcp[1]}` : name;
    }
  }
}

function keyOf(tool: FoldableTool, index: number): string {
  return tool.id ? tool.id : `#${index}`;
}

/**
 * Fold consecutive same-family calls (at least `FOLD_MIN`) into groups.
 *
 * Every item's key is unique within the result and stable while a turn
 * streams: a single is keyed by its call, a group by its FIRST call, so a
 * group that gains a member keeps its key. `ToolLedger` renders rows by key,
 * which keeps a growing group's row (and whether the user opened it) mounted
 * instead of rebuilding it. A repeated call id gets a `~n` suffix.
 */
export function foldTools<T extends FoldableTool>(tools: readonly T[]): LedgerItem<T>[] {
  const out: LedgerItem<T>[] = [];
  const seen = new Map<string, number>();
  const unique = (key: string): string => {
    const n = seen.get(key) ?? 0;
    seen.set(key, n + 1);
    return n === 0 ? key : `${key}~${n}`;
  };
  let i = 0;
  while (i < tools.length) {
    const tool = tools[i];
    if (tool.approval !== undefined) {
      out.push({ kind: 'single', key: unique(`t:${keyOf(tool, i)}`), index: i, tool });
      i++;
      continue;
    }
    const family = toolFamily(tool.name);
    let j = i + 1;
    while (j < tools.length && tools[j].approval === undefined && toolFamily(tools[j].name) === family) j++;
    if (j - i >= FOLD_MIN) {
      out.push({ kind: 'group', key: unique(`g:${keyOf(tool, i)}`), family, start: i, tools: tools.slice(i, j) });
    } else {
      for (let k = i; k < j; k++) out.push({ kind: 'single', key: unique(`t:${keyOf(tools[k], k)}`), index: k, tool: tools[k] });
    }
    i = j;
  }
  return out;
}

/** A group is running while any member runs, else failed if any failed. */
export function groupStatus(tools: readonly FoldableTool[]): FoldStatus {
  if (tools.some(t => t.status === 'running')) return 'running';
  if (tools.some(t => t.status === 'error')) return 'error';
  return 'done';
}

/** The one-line label of a group row. */
export function groupLabel(family: string, count: number): string {
  switch (family) {
    case 'read': return `Read ${count} files`;
    case 'search': return `Searched ${count} times`;
    case 'edit': return `Edited ${count} files`;
    case 'shell': return `Ran ${count} commands`;
    case 'web': return `${count} web lookups`;
    case 'todo': return `Updated the plan ${count} times`;
    default: {
      if (family.startsWith('mcp:')) {
        const server = family.slice(4).replace(/^hollow_/, '').replace(/[-_]+/g, ' ').trim();
        return `${count} ${server || 'tool'} calls`;
      }
      return `${family} ×${count}`;
    }
  }
}

/**
 * The rows a fixed-height windowed list renders: the visible ones plus
 * `overscan` on each side, clamped to `[0, total]`. `end` is exclusive.
 */
export function windowRange(
  scrollTop: number,
  viewportPx: number,
  rowPx: number,
  total: number,
  overscan: number,
): { start: number; end: number } {
  if (total <= 0 || rowPx <= 0) return { start: 0, end: 0 };
  const first = Math.max(0, Math.floor(Math.max(0, scrollTop) / rowPx));
  const visible = Math.ceil(Math.max(0, viewportPx) / rowPx);
  const start = Math.min(total, Math.max(0, first - overscan));
  const end = Math.min(total, first + visible + overscan);
  return { start, end: Math.max(start, end) };
}
