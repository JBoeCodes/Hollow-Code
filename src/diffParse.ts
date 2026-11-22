// Pure unified-diff parser for the Changes pane (Build PR B2). Input is
// `git diff` text (`build_diff`); output is files → hunks → lines with old
// and new line numbers. Deliberately count-free: a hunk runs until the next
// `@@` or `diff --git`, so a diff whose text redaction folded several lines
// into one still parses (its later line numbers are then approximate).

export type DiffLineKind = 'context' | 'add' | 'del' | 'note';
export interface DiffLine { kind: DiffLineKind; text: string; oldNo: number | null; newNo: number | null }
export interface DiffHunk { header: string; oldStart: number; newStart: number; section: string; lines: DiffLine[] }
export interface DiffFile {
  /** The path shown: the new path, or the old one for a deletion. */
  path: string;
  oldPath: string | null;
  newPath: string | null;
  status: 'added' | 'deleted' | 'modified' | 'renamed';
  binary: boolean;
  hunks: DiffHunk[];
  additions: number;
  deletions: number;
}

/** Undo git's C-style quoting of a path (`"a\tb"`), else trim a trailing tab. */
export function unquotePath(raw: string): string {
  const s = raw.replace(/\t$/, '');
  if (!(s.startsWith('"') && s.endsWith('"') && s.length >= 2)) return s;
  const body = s.slice(1, -1);
  const bytes: number[] = [];
  for (let i = 0; i < body.length; i++) {
    const c = body[i];
    if (c !== '\\') { bytes.push(...new TextEncoder().encode(c)); continue; }
    const n = body[++i];
    if (n === undefined) break;
    if (/[0-7]/.test(n)) { bytes.push(parseInt(body.slice(i, i + 3), 8)); i += 2; continue; }
    bytes.push(({ n: 10, t: 9, r: 13, '"': 34, '\\': 92, a: 7, b: 8, f: 12, v: 11 } as Record<string, number>)[n] ?? n.charCodeAt(0));
  }
  return new TextDecoder().decode(new Uint8Array(bytes));
}

/** `a/path` → `path`; `/dev/null` → null. */
function stripSide(raw: string, side: 'a/' | 'b/'): string | null {
  const p = unquotePath(raw.trim());
  if (p === '/dev/null') return null;
  return p.startsWith(side) ? p.slice(2) : p;
}

/** `diff --git a/x b/x` → x, used only when no ---/+++ lines follow. */
function pathsFromHeader(line: string): [string | null, string | null] {
  const rest = line.slice('diff --git '.length);
  if (rest.startsWith('"')) {
    const m = /^("(?:[^"\\]|\\.)*")\s+(.*)$/.exec(rest);
    if (m) return [stripSide(m[1], 'a/'), stripSide(m[2], 'b/')];
  }
  // Unquoted: split on " b/" (ambiguous only for paths containing " b/").
  const i = rest.indexOf(' b/');
  if (i < 0) return [null, null];
  return [stripSide(rest.slice(0, i), 'a/'), stripSide(rest.slice(i + 1), 'b/')];
}

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@ ?(.*)$/;

export function parseDiff(text: string): DiffFile[] {
  const files: DiffFile[] = [];
  let file: DiffFile | null = null;
  let hunk: DiffHunk | null = null;
  let oldNo = 0, newNo = 0;
  let sawNew = false, sawDeleted = false, sawRename = false;
  const finish = () => {
    if (!file) return;
    if (sawNew) file.status = 'added';
    else if (sawDeleted) file.status = 'deleted';
    else if (sawRename || (file.oldPath && file.newPath && file.oldPath !== file.newPath)) file.status = 'renamed';
    file.path = file.newPath ?? file.oldPath ?? file.path;
    files.push(file);
  };
  for (const line of text.split('\n')) {
    if (line.startsWith('diff --git ')) {
      finish();
      const [a, b] = pathsFromHeader(line);
      file = { path: b ?? a ?? '', oldPath: a, newPath: b, status: 'modified', binary: false, hunks: [], additions: 0, deletions: 0 };
      hunk = null; sawNew = sawDeleted = sawRename = false;
      continue;
    }
    if (!file) continue;
    const h = HUNK.exec(line);
    if (h) {
      oldNo = Number(h[1]); newNo = Number(h[2]);
      hunk = { header: line, oldStart: oldNo, newStart: newNo, section: h[3] ?? '', lines: [] };
      file.hunks.push(hunk);
      continue;
    }
    if (!hunk) {
      if (line.startsWith('new file mode')) { sawNew = true; file.oldPath = null; }
      else if (line.startsWith('deleted file mode')) { sawDeleted = true; file.newPath = null; }
      else if (line.startsWith('rename from ')) { sawRename = true; file.oldPath = unquotePath(line.slice(12)); }
      else if (line.startsWith('rename to ')) { sawRename = true; file.newPath = unquotePath(line.slice(10)); }
      else if (line.startsWith('--- ')) file.oldPath = stripSide(line.slice(4), 'a/');
      else if (line.startsWith('+++ ')) file.newPath = stripSide(line.slice(4), 'b/');
      else if (line.startsWith('Binary files ') || line === 'GIT binary patch') file.binary = true;
      continue;
    }
    const tag = line[0];
    if (tag === '+') { hunk.lines.push({ kind: 'add', text: line.slice(1), oldNo: null, newNo: newNo++ }); file.additions++; }
    else if (tag === '-') { hunk.lines.push({ kind: 'del', text: line.slice(1), oldNo: oldNo++, newNo: null }); file.deletions++; }
    else if (tag === ' ') hunk.lines.push({ kind: 'context', text: line.slice(1), oldNo: oldNo++, newNo: newNo++ });
    else if (tag === '\\') hunk.lines.push({ kind: 'note', text: line.slice(1).trim(), oldNo: null, newNo: null });
    else if (line === '') continue; // the text's final newline, or a blank separator
    else hunk = null; // anything else ends the hunk (headers of a malformed diff)
  }
  finish();
  return files;
}

/** The hunk's line range on the new side (the old side for a pure deletion). */
export function hunkRange(h: DiffHunk): { side: 'new' | 'old'; start: number; end: number } {
  const news = h.lines.map(l => l.newNo).filter((n): n is number => n != null);
  if (news.length) return { side: 'new', start: news[0], end: news[news.length - 1] };
  const olds = h.lines.map(l => l.oldNo).filter((n): n is number => n != null);
  return { side: 'old', start: olds[0] ?? h.oldStart, end: olds[olds.length - 1] ?? h.oldStart };
}

/** The hunk as diff text again (header + its lines). */
export function hunkText(h: DiffHunk): string {
  const body = h.lines.map(l => l.kind === 'note' ? `\\ ${l.text}` : `${l.kind === 'add' ? '+' : l.kind === 'del' ? '-' : ' '}${l.text}`);
  return [h.header, ...body].join('\n');
}

/** The composer draft "Ask about this" appends: file, line range, the hunk. */
export function askAboutHunk(file: DiffFile, h: DiffHunk): string {
  const r = hunkRange(h);
  const where = r.start === r.end ? `line ${r.start}` : `lines ${r.start}-${r.end}`;
  const fence = hunkText(h).includes('```') ? '````' : '```';
  return `About this change in ${file.path} (${where}${r.side === 'old' ? ', before the change' : ''}):\n\n${fence}diff\n${hunkText(h)}\n${fence}\n\n`;
}
