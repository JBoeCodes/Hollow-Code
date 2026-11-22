// The footer under a finished agent reply (Conductor-style): how long the run
// took and when it finished, then the files it changed as `name +a −d` chips.
// Pure and dependency-free (`import type` erases), so the node vitest suite
// can load it.
import type { ExecutionReceipt } from './executionReceipt';

/** Chips shown before the rest fold into one `+N more` chip. */
export const FOOTER_CHIPS = 2;

export interface FileChip {
  /** Stable key: `vault:<path>` or `repo:<root>:<path>`. */
  key: string;
  /** The file name shown on the chip. */
  name: string;
  /** Vault-relative path, or the repository-relative path for a repo row. */
  path: string;
  /** Absolute repository root, for a repo row. */
  root?: string;
  /** `create`, `modify`, `delete`, … */
  changeType: string;
  additions: number | null;
  deletions: number | null;
}

export interface FileChipSummary {
  shown: FileChip[];
  /** The listed chips behind `+N more` (empty when none are), so a shown
   *  chip's same-name twin there still gets it a folder (`chipFolders`). */
  folded: FileChip[];
  /** The chips folded into `+N more`, with their summed counts (null when
   *  none of them carried counts). Null when nothing is folded. */
  more: { count: number; additions: number | null; deletions: number | null } | null;
  total: number;
}

function baseName(path: string): string {
  const trimmed = path.replace(/\/+$/, '');
  return trimmed.slice(trimmed.lastIndexOf('/') + 1) || trimmed;
}

/** The receipt's changed files, vault then repository, as chips. Receipt
 *  rows without a real change (`clean` repo rows) and another chat's files
 *  (`shared`, chat concurrency F1) are left out. */
export function fileChips(receipt: ExecutionReceipt | undefined, max = FOOTER_CHIPS): FileChipSummary {
  const chips: FileChip[] = [];
  // `executionReceipt.isOwnReceiptFile`, inline to stay dependency-free.
  for (const file of (receipt?.files ?? []).filter(f => f.attribution !== 'shared')) {
    chips.push({
      key: `vault:${file.path}`,
      name: baseName(file.path),
      path: file.path,
      changeType: file.changeType ?? 'modify',
      additions: file.additions ?? null,
      deletions: file.deletions ?? null,
    });
  }
  let unlisted = 0;
  for (const repo of receipt?.repoChanges ?? []) {
    let listed = 0;
    for (const file of repo.files) {
      if (file.status === 'clean') { listed++; continue; }
      listed++;
      chips.push({
        key: `repo:${repo.root}:${file.path}`,
        name: baseName(file.path),
        path: file.path,
        root: repo.root,
        changeType: file.changeType,
        additions: file.binary || file.withheld ? null : file.additions,
        deletions: file.binary || file.withheld ? null : file.deletions,
      });
    }
    unlisted += Math.max(0, (repo.totalFiles || 0) - listed);
  }
  const total = chips.length + unlisted;
  // Never fold a single chip: `+1 more` is no shorter than the chip itself.
  const cut = total - max === 1 && unlisted === 0 ? chips.length : Math.min(max, chips.length);
  const shown = chips.slice(0, cut);
  const folded = chips.slice(cut);
  if (folded.length === 0 && unlisted === 0) return { shown, folded, more: null, total };
  const counted = folded.filter(c => c.additions != null || c.deletions != null);
  return {
    shown,
    folded,
    more: {
      count: folded.length + unlisted,
      additions: counted.length ? counted.reduce((n, c) => n + (c.additions ?? 0), 0) : null,
      deletions: counted.length ? counted.reduce((n, c) => n + (c.deletions ?? 0), 0) : null,
    },
    total,
  };
}

/** A vault note the Main agent keeps: the workspace brief or its plan. */
function isAgentNote(path: string, workspaceId: string): boolean {
  return path === `workspaces/${workspaceId}/PLAN.md` || path === `workspaces/${workspaceId}/WORKSPACE.md`;
}

/** The Main agent's own brief and plan edits, as one quiet line instead of
 *  file chips. Other files stay chips. A project chat keeps the chips. */
export function notesUpdate(receipt: ExecutionReceipt | undefined, mainWorkspaceId?: string): { updated: boolean; notePath: string | null; chips: FileChipSummary } {
  const plain = { updated: false, notePath: null as string | null, chips: fileChips(receipt) };
  if (!mainWorkspaceId || !receipt) return plain;
  const notes = (receipt.files ?? []).filter(f => f.attribution !== 'shared' && isAgentNote(f.path, mainWorkspaceId));
  if (notes.length === 0) return plain;
  const files = (receipt.files ?? []).filter(f => f.attribution === 'shared' || !isAgentNote(f.path, mainWorkspaceId));
  return { updated: true, notePath: notes[0].path, chips: fileChips({ ...receipt, files }) };
}

/** Where a click on a changed-file chip goes (matrix N8, overnight 10-14/15
 *  H3): a Result opens in its workspace's Results, beside Accept and Ask for
 *  a revision; anything else opens in the side panel's Files view as before. */
export type ChipTarget =
  | { to: 'results'; workspaceId: string; path: string }
  | { to: 'files' };

/** Deepest Result the Results walk reaches (Rust `workspace_results::list`,
 *  `WalkDir::new(outputs).max_depth(6)`): folders below `outputs/`, plus the
 *  file itself. */
const RESULT_MAX_DEPTH = 6;

/** Whether `rel` (workspace-relative, `outputs/x.md`) is a file Results
 *  lists: a mirror of Rust `workspace_results::is_listed` and its walk. Not
 *  listed: a dot-prefixed or `..` segment (the walk prunes dot folders, and
 *  `relative_output` refuses both), anything past the walk's depth, a
 *  compacted brief's archive (`context_compact::ARCHIVE_DIR`), a process
 *  capture's frames (`process_capture::is_frame`), a saved canvas's
 *  `.canvas.json` sidecar (its `.canvas.html` is the Result) and either file
 *  of a curated section canvas (`_plan.canvas.*`). A page review's capture
 *  folder (`outputs/reviews/<slug>/`) is pruned when `<slug>.annotations.json`
 *  sits beside it, which a path alone can't tell, so every file inside a
 *  `reviews/` subfolder stays with Files: opening Files is never wrong, and
 *  Results would say "That result is no longer here". */
export function isListedResult(rel: string): boolean {
  const parts = rel.split('/');
  if (parts[0] !== 'outputs' || parts.length < 2 || parts.length - 1 > RESULT_MAX_DEPTH) return false;
  if (parts.some(p => !p || p === '..' || p.startsWith('.'))) return false;
  if (rel.startsWith('outputs/workspace-archive/')) return false;
  if (parts[1] === 'process-captures' && parts[3] === 'frames') return false;
  if (parts[1] === 'reviews' && parts.length > 3) return false;
  const canvas = /^outputs\/canvases\/(.+)\.canvas\.(html|json)$/.exec(rel);
  if (canvas) {
    if (/^_(overview|brief|plan|sources)$/.test(canvas[1])) return false;
    if (canvas[2] === 'json' && /^[a-z0-9-]{1,64}$/.test(canvas[1])) return false;
  }
  return true;
}

/** Where a click on `chip` goes, in a conversation of `workspace` (null
 *  outside one, such as Conversation history). Results only when it will show the
 *  file: a vault row (not a repository's), not deleted, under this
 *  workspace's own `outputs/` and listed there. Quick chats has no Results
 *  (workspaceTypes `isQuickChats`). Vault rows are vault-relative
 *  (`workspaces/<id>/outputs/x.md`, the path Results selects by). */
export function chipTarget(
  chip: Pick<FileChip, 'path' | 'root' | 'changeType'>,
  workspace: { id: string; kind?: string } | null | undefined,
): ChipTarget {
  if (!workspace || workspace.kind === 'quick' || workspace.id === 'quick-chats') return { to: 'files' };
  if (chip.root || chip.changeType === 'delete') return { to: 'files' };
  const prefix = `workspaces/${workspace.id}/`;
  if (!chip.path.startsWith(prefix)) return { to: 'files' };
  if (!isListedResult(chip.path.slice(prefix.length))) return { to: 'files' };
  return { to: 'results', workspaceId: workspace.id, path: chip.path };
}

/** `path` with a leading `/Users/<name>` (the home folder) shown as `~`;
 *  any other path unchanged. */
export function tildeHome(path: string): string {
  // /Users/Shared is no one's home: it stays as it is (review nit).
  return path.replace(/^\/Users\/(?!Shared(?:\/|$))[^/]+(?=\/|$)/, '~');
}

/** The folder each chip names when another of `chips` has the same file
 *  name (EH4), so `report.md` in `outputs/` and in `sources/` read apart on
 *  the chips and the reply's More actions; null when the name is unique.
 *  Relative to the open workspace's folder (`outputs/tables`), else to the
 *  vault; a repository row starts with the repository's folder name. Two
 *  that would still read alike (two repositories with one folder name, or
 *  `outputs/` in and outside the workspace) give their full folder instead,
 *  a repository's with the home folder as `~` (`tildeHome`).
 *  An empty string is a file at the top of that folder. */
export function chipFolders(
  chips: readonly Pick<FileChip, 'name' | 'path' | 'root'>[],
  workspaceId?: string | null,
): (string | null)[] {
  const prefix = workspaceId ? `workspaces/${workspaceId}/` : null;
  const dir = (path: string) => { const at = path.lastIndexOf('/'); return at > 0 ? path.slice(0, at) : ''; };
  // Counted once, so a receipt with many files stays linear (A10 review).
  const names = new Map<string, number>();
  for (const chip of chips) names.set(chip.name, (names.get(chip.name) ?? 0) + 1);
  const short = chips.map(chip => {
    if ((names.get(chip.name) ?? 0) < 2) return null;
    return dir(chip.root ? `${baseName(chip.root)}/${chip.path}`
      : prefix && chip.path.startsWith(prefix) ? chip.path.slice(prefix.length) : chip.path);
  });
  const pair = (i: number) => `${chips[i].name}\u0000${short[i]}`;
  const pairs = new Map<string, number>();
  short.forEach((folder, i) => { if (folder !== null) pairs.set(pair(i), (pairs.get(pair(i)) ?? 0) + 1); });
  return short.map((folder, i) => {
    const alike = folder !== null && (pairs.get(pair(i)) ?? 0) > 1;
    const chip = chips[i];
    return alike ? dir(chip.root ? tildeHome(`${chip.root.replace(/\/+$/, '')}/${chip.path}`) : chip.path) : folder;
  });
}

/** A chip's name for a label: `name (folder)` when `chipFolders` gave one. */
export function chipLabel(name: string, folder: string | null | undefined): string {
  return folder ? `${name} (${folder})` : name;
}

/** `8s`, `2m 5s`, `1h 4m`: whole seconds, for a person rather than a log. */
export function durationText(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1_000));
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3_600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  const minutes = Math.floor(seconds / 60);
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** "typically 4m 0s for code", beside the time this reply took. Null when
 *  this kind of job has no usual length yet. */
export function typicalLine(ms: number | null | undefined, kind: string | null | undefined): string | null {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return null;
  const name = kind?.trim();
  if (!name) return null;
  return `typically ${durationText(ms)} for ${name}`;
}

/** How long a finished run took, from its receipt; null without both times. */
export function runDuration(receipt: Pick<ExecutionReceipt, 'startedAt' | 'finishedAt'> | undefined): string | null {
  const start = Date.parse(receipt?.startedAt ?? '');
  const end = Date.parse(receipt?.finishedAt ?? '');
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return null;
  return durationText(end - start);
}

/** The finish time as the local clock reads it (`7:12 PM`); an older day
 *  adds its date (`Sep 24, 7:12 PM`). Null for a missing or bad time. */
export function finishedText(finishedAt: string | undefined, now = Date.now(), locale?: string): string | null {
  const at = Date.parse(finishedAt ?? '');
  if (!Number.isFinite(at)) return null;
  const when = new Date(at);
  const time = when.toLocaleTimeString(locale, { hour: 'numeric', minute: '2-digit' });
  if (when.toDateString() === new Date(now).toDateString()) return time;
  const date = when.toLocaleDateString(locale, { month: 'short', day: 'numeric' });
  return `${date}, ${time}`;
}

/** Whether the turn at `index` ends its agent reply: an assistant turn with
 *  no assistant turn after it before the next user message. System notices
 *  (a stop, a budget warning) don't end a reply. */
export function endsReply(turns: ReadonlyArray<{ role: string }>, index: number): boolean {
  if (turns[index]?.role !== 'assistant') return false;
  for (let i = index + 1; i < turns.length; i++) {
    if (turns[i].role === 'assistant') return false;
    if (turns[i].role === 'user') return true;
  }
  return true;
}

/** A message without its blank lines before and its space after, but with its
 *  first line's indentation: four spaces open an indented code block, and
 *  `trim()` turned one into a paragraph in a saved note or a copy (a Codex
 *  read of the save-as-note path). */
export function trimBlock(text: string): string {
  return text.replace(/^(?:[ \t]*\r?\n)+/, '').trimEnd();
}

/** The whole reply's text (every assistant turn from the last user message
 *  up to `index`), for Copy and Explain visually. An `index` past the end
 *  reads as no reply: a row's getters can run once more after the turns were
 *  cleared (a new conversation opening), and a throw there failed the open
 *  (Continue from here, 2026-11-15). */
export function replyText(turns: ReadonlyArray<{ role: string; text: string }>, index: number): string {
  const parts: string[] = [];
  if (index >= turns.length) return '';
  for (let i = index; i >= 0 && turns[i].role !== 'user'; i--) {
    if (turns[i].role === 'assistant' && turns[i].text.trim()) parts.unshift(trimBlock(turns[i].text));
  }
  return parts.join('\n\n');
}

/** The message a reply answers: the nearest person's turn before `index`,
 *  or null when the conversation starts with the reply. */
export function replyQuestion(turns: ReadonlyArray<{ role: string; text: string }>, index: number): string | null {
  if (index >= turns.length) return null;
  for (let i = index; i >= 0; i--) if (turns[i].role === 'user') return turns[i].text;
  return null;
}

// Save as note (One Ask, 2026-11-03): a chat reply becomes a Markdown note,
// as Ask your vault's Save as note did before that view was removed.

/** The saved note's title: the question's first line, else the reply's
 *  (a heading's `#` dropped), on one line and at most 120 characters, cut by
 *  code point so an emoji is never split in half (half a surrogate pair is
 *  not valid text, and the save would fail). */
export function replyNoteTitle(question: string | null | undefined, reply: string): string {
  const line = Array.from((noteTitleLine(question ?? '') || noteTitleLine(reply) || 'Saved reply').replace(/\s+/g, ' '));
  if (line.length <= 120) return line.join('');
  // Cut at 119 code points, backed off to a whole grapheme so a combining
  // accent or a joined emoji is never split from its base (ENG-93).
  return `${graphemeCut(line.join(''), 119)}…`;
}

/** The first line worth a title: a Markdown heading (at most three spaces
 *  in, its `#` run followed by a space or the end) loses its `#` run; an
 *  indented code line, a line inside a fenced block and a `#123` keep theirs
 *  (ENG-83, ENG-93); fence lines themselves are skipped. */
export function noteTitleLine(text: string): string {
  let fence: string | null = null;
  for (const raw of text.split('\n')) {
    const open = /^ {0,3}(`{3,}|~{3,})/.exec(raw);
    if (open) {
      if (fence === null) fence = open[1][0];
      else if (open[1][0] === fence) fence = null;
      continue;
    }
    const heading = fence === null && /^ {0,3}#{1,6}(?=[ \t]|$)/.exec(raw);
    const line = (heading ? raw.slice(heading[0].length) : raw).trim();
    if (line) return line;
  }
  return '';
}

/** `text` cut to at most `max` code points without splitting a grapheme. */
export function graphemeCut(text: string, max: number): string {
  const Segmenter = (Intl as unknown as { Segmenter?: new (l?: string, o?: { granularity: string }) => { segment(t: string): Iterable<{ segment: string }> } }).Segmenter;
  if (!Segmenter) return Array.from(text).slice(0, max).join('');
  let out = '';
  let count = 0;
  for (const { segment } of new Segmenter(undefined, { granularity: 'grapheme' }).segment(text)) {
    const size = Array.from(segment).length;
    if (count + size > max) break;
    out += segment;
    count += size;
  }
  return out;
}

/** A file name from a title: lower case letters and digits joined by `-`,
 *  at most 60 characters; `reply` when nothing is left. */
export function replyNoteSlug(title: string): string {
  const slug = title.normalize('NFKD').replace(/[̀-ͯ]/g, '').toLowerCase()
    .replace(/[^a-z0-9]+/g, '-').replace(/^-+/, '').slice(0, 60).replace(/-+$/, '');
  return slug || 'reply';
}

/** `date` as the local `YYYY-MM-DD`. */
function localDay(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** Where a reply is saved, vault-relative: a workspace conversation's into
 *  that workspace's `outputs/answers/` (so it is one of its Results), a quick
 *  chat's, the Main agent's or one outside any workspace into the Library's
 *  `library/`. Named `<date>-<slug>.md`; `attempt` 2 and on add `-2`, `-3`…,
 *  since a save never overwrites a file (the caller tries the next one). */
export function replyNotePath(
  workspace: { id: string; kind?: string; path?: string } | null | undefined,
  title: string,
  date: Date,
  attempt = 1,
): string {
  const library = !workspace || workspace.kind === 'quick' || workspace.kind === 'main' || workspace.id === 'quick-chats' || workspace.id === 'main';
  const dir = library ? 'library' : `${(workspace.path || `workspaces/${workspace.id}`).replace(/\/+$/, '')}/outputs/answers`;
  return `${dir}/${localDay(date)}-${replyNoteSlug(title)}${attempt > 1 ? `-${attempt}` : ''}.md`;
}

/** The note's Markdown: the title as its heading, where it came from, then
 *  the reply as written. */
export function replyNoteBody(title: string, reply: string, date: Date, what = 'chat reply'): string {
  return `# ${title}\n\n*Saved from a ${what} on ${localDay(date)}.*\n\n${trimBlock(reply)}\n`;
}
