import type { SessionMeta, TaskBucket } from './types';
import { monthDay } from './monthDay';

export const projectName = (full: string) => full.split('/').filter(Boolean).pop() || full;

/// Display title for a session row: the short AI-generated title when present,
/// else the (context-stripped) first prompt, else the project name. Capped so
/// CSS ellipsis handles the visual overflow.
export function sessionTitle(s: SessionMeta): string {
  return (s.title?.trim() || s.firstPrompt?.trim() || projectName(s.project)).slice(0, 80)
    || projectName(s.project);
}

/// Open a URL in a new tab/window without exposing `window.opener` to the
/// target page. `noopener` also implies `noreferrer` in modern browsers,
/// but we set both for older WebKit (and to be explicit).
export function openExternal(url: string | null | undefined): void {
  if (!url) return;
  window.open(url, '_blank', 'noopener,noreferrer');
}

export function fmtTime(iso: string | null | undefined): string {
  if (!iso) return '';
  const d = new Date(iso);
  const diff = (Date.now() - d.getTime()) / 1000;
  if (diff < 60) return `${Math.floor(diff)}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  if (diff < 86400 * 7) return `${Math.floor(diff / 86400)}d ago`;
  return d.toLocaleDateString();
}

/// `YYYY-MM-DD` for a date in *local* time. `Date#toISOString()` is UTC, so
/// near midnight it reports tomorrow's or yesterday's date — wrong for
/// comparing against local-time calendar strings.
export function localDateKey(d: Date = new Date()): string {
  const y = d.getFullYear();
  const mo = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${mo}-${day}`;
}

/**
 * Human-readable due-date label for a task.
 *
 * Default (long) form is used in the Tasks list. The `compact` form returns
 * the terser strings the Today dashboard's brief rows use ("overdue",
 * "2d late", "today") and keys off the task's `bucket` rather than recomputing
 * the day delta — pass the full Task there so `bucket` is available.
 */
export function dueLabel(
  t: { due: string | null; daysUntilDue: number | null; bucket?: TaskBucket },
  compact = false,
): string {
  // Completed work is never late. The terminal `done` bucket wins over
  // `daysUntilDue`, which the backend still computes — without this the
  // Completed group reads as a backlog of work that already shipped.
  if (t.bucket === 'done') return t.due ? 'done' : compact ? '' : '—';
  if (compact) {
    if (t.bucket === 'overdue') {
      const days = Math.abs(t.daysUntilDue ?? 0);
      return days <= 1 ? 'overdue' : `${days}d late`;
    }
    if (t.bucket === 'today') return 'today';
    return t.due ?? '';
  }
  if (!t.due) return '—';
  const d = t.daysUntilDue;
  if (d == null) return t.due;
  if (d < 0) return `${Math.abs(d)}d overdue`;
  if (d === 0) return 'due today';
  if (d === 1) return 'due tomorrow';
  if (d <= 7) return `due in ${d}d`;
  return monthDay(new Date(t.due));
}

/** Stable hue (0–359) from a string so each person/conversation gets a
 *  consistent avatar tint. Pairs with the shared `.hue-avatar` primitive
 *  (set `--avatar-hue` inline from this). */
export function hueFor(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) % 360;
  return h;
}

export type FrontmatterValue = string | string[] | number | boolean;
export interface ParsedFrontmatter {
  props: Record<string, FrontmatterValue>;
  /** Byte offset where the body content starts (after the closing ---). */
  bodyStart: number;
  /** Inclusive end position of the closing --- line. */
  blockEnd: number;
}

const FM_KEY_RE = /^([\w-]+):\s*(.*)$/;
const FM_LIST_ITEM_RE = /^\s+-\s+(.+)$/;

/**
 * Parse a YAML frontmatter block at the start of a markdown document.
 * Returns null if the doc doesn't start with `---`.
 *
 * Supports the limited shape we use across the vault:
 *   key: value
 *   listKey:
 *     - item
 *     - item
 */
export function parseFrontmatter(content: string): ParsedFrontmatter | null {
  if (!content.startsWith('---\n') && !content.startsWith('---\r\n')) return null;
  const headLen = content.startsWith('---\r\n') ? 5 : 4;
  // Find the closing fence: a line containing only `---`.
  const re = /\n---\s*(?:\r?\n|$)/g;
  re.lastIndex = headLen;
  const m = re.exec(content);
  if (!m) return null;
  const blockEnd = m.index + m[0].length; // position right after closing ---\n
  const yaml = content.slice(headLen, m.index);

  const props: Record<string, FrontmatterValue> = {};
  let currentList: string[] | null = null;
  let currentKey: string | null = null;

  for (const rawLine of yaml.split(/\r?\n/)) {
    if (!rawLine.trim()) continue;
    const itemMatch = rawLine.match(FM_LIST_ITEM_RE);
    if (itemMatch && currentList) {
      currentList.push(unquote(itemMatch[1].trim()));
      continue;
    }
    const keyMatch = rawLine.match(FM_KEY_RE);
    if (keyMatch) {
      const key = keyMatch[1];
      const val = keyMatch[2].trim();
      currentKey = key;
      if (val === '') {
        currentList = [];
        props[key] = currentList;
      } else {
        currentList = null;
        props[key] = parseScalar(val);
      }
    }
  }

  return { props, bodyStart: blockEnd, blockEnd };
}

function unquote(s: string): string {
  if ((s.startsWith('"') && s.endsWith('"')) || (s.startsWith("'") && s.endsWith("'"))) {
    return s.slice(1, -1);
  }
  return s;
}

function parseScalar(s: string): FrontmatterValue {
  const u = unquote(s);
  if (u === 'true') return true;
  if (u === 'false') return false;
  if (/^-?\d+(\.\d+)?$/.test(u)) {
    const n = Number(u);
    if (Number.isFinite(n)) return n;
  }
  return u;
}

/// Tolerant clock-time parser for free-text time fields. Accepts the same
/// vocabulary as quick-add ("4pm", "11:30 AM", "16:00", "noon") plus bare
/// digits ("930", "1630", "11"). Returns canonical 24h "HH:MM", or null when
/// the text isn't a readable time.
export function parseClockTime(raw: string): string | null {
  const s = raw.trim().toLowerCase();
  if (!s) return null;
  if (s === 'noon') return '12:00';
  if (s === 'midnight') return '00:00';
  const hhmm = (h: number, m: number) =>
    `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}`;
  // 12-hour: "4pm", "4 pm", "11:30am", "11.30 PM"
  let m = /^(\d{1,2})(?:[:.](\d{2}))?\s*(am|pm)$/.exec(s);
  if (m) {
    let h = Number(m[1]);
    const min = Number(m[2] ?? '0');
    if (h < 1 || h > 12 || min > 59) return null;
    if (m[3] === 'pm' && h < 12) h += 12;
    if (m[3] === 'am' && h === 12) h = 0;
    return hhmm(h, min);
  }
  // 24-hour: "16:00", "9.30"
  m = /^(\d{1,2})[:.](\d{2})$/.exec(s);
  if (m) {
    const h = Number(m[1]);
    const min = Number(m[2]);
    return h <= 23 && min <= 59 ? hhmm(h, min) : null;
  }
  // Bare digits: "930" / "1630" → 9:30 / 16:30; "11" → 11:00.
  m = /^(\d{1,2})(\d{2})$/.exec(s);
  if (m) {
    const h = Number(m[1]);
    const min = Number(m[2]);
    return h <= 23 && min <= 59 ? hhmm(h, min) : null;
  }
  m = /^(\d{1,2})$/.exec(s);
  if (m) {
    const h = Number(m[1]);
    return h <= 23 ? hhmm(h, 0) : null;
  }
  return null;
}

/// Render a canonical "HH:MM" as the compact 12-hour form ("14:30" → "2:30pm").
/// Anything that isn't HH:MM passes through unchanged so stored oddities stay
/// visible/editable instead of vanishing.
export function fmtClockHHMM(hhmm: string): string {
  const m = /^(\d{1,2}):(\d{2})$/.exec(hhmm.trim());
  if (!m) return hhmm;
  const h = Number(m[1]);
  if (h > 23 || Number(m[2]) > 59) return hhmm;
  const ampm = h >= 12 ? 'pm' : 'am';
  return `${h % 12 || 12}:${m[2]}${ampm}`;
}

/// Normalize an unknown thrown value into a display string. Use in place of
/// ad-hoc `e?.message || e` / `e instanceof Error ? e.message : String(e)`.
export function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/// A task-mutation error for display: `errMsg` (Tauri rejects with a plain
/// string, so `e?.message` is undefined) minus the machine-readable
/// `TASK_CONFLICT:` / `TASK_LINKED:` / `TASK_CYCLE:` / `TASK_BLOCKER:` /
/// `TASK_KEY:` prefix the backend adds.
export function taskErrorText(e: unknown): string {
  return errMsg(e).replace(/^TASK_(CONFLICT|LINKED|CYCLE|BLOCKER|KEY):\s*/, '');
}

/// A task write refused because the file changed since the caller's
/// baseline (`TASK_CONFLICT: …`, review F01).
export const isTaskConflict = (e: unknown) => /^TASK_CONFLICT:/.test(errMsg(e));

/// When an error starts with the backend's machine-readable `PREFIX:` code,
/// the message after it; otherwise null. Reads through `errMsg`, because
/// Tauri rejects with a plain string and `e?.message` is undefined.
export function errorWithPrefix(e: unknown, prefix: string): string | null {
  const message = errMsg(e);
  const head = `${prefix}:`;
  return message.startsWith(head) ? message.slice(head.length).trimStart() : null;
}

/// Drop a leading `---` YAML frontmatter block (and an optional BOM) from a
/// markdown string, returning the body. Returns the input unchanged when
/// there's no well-formed frontmatter.
export function stripFrontmatter(s: string): string {
  if (!s.startsWith('---')) return s;
  const m = /^---\r?\n[\s\S]*?\r?\n---\r?\n?/.exec(s);
  return m ? s.slice(m[0].length) : s;
}

/// Canonical wikilink matcher. Capture group 1 is the target note (anchor
/// `#heading` excluded), group 2 is the optional `|alias`. The `#heading`
/// portion is matched but discarded — it isn't part of the link target.
/// Global flag: callers that iterate must reset `lastIndex` (or use matchAll).
export const WIKILINK_RE = /\[\[([^\]\n|#]+?)(?:#[^\]\n|]+)?(?:\|([^\]\n]+))?\]\]/g;

