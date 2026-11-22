// Chat file widgets (tool chips, reply-footer chips, file links in a reply)
// open in the Preview pane. These helpers turn what a widget carries into the
// absolute path the pane reads, and a reply's file link into that path plus
// the lines it cites (`fileLinkFromHref`). Pure and dependency-free, so the
// node vitest suite can load it.

/** Videos that play inside Hollow (inline video, 2026-10-17): mp4, m4v and
 *  mov. Mirrors `VIDEO_EXTS` in `src-tauri/src/media.rs` (`mediaKinds.test.ts`
 *  checks). WebM is left out: WebKit's WebM player asks for the whole file in
 *  one request, which the bounded `hollow-media` scheme can't answer. */
export const VIDEO_EXTS: readonly string[] = ['mp4', 'm4v', 'mov'];

/** Whether a path or file name is a video Hollow plays. */
export function isVideoPath(p: string): boolean {
  const ext = /\.([a-z0-9]+)$/i.exec(p)?.[1]?.toLowerCase() ?? '';
  return VIDEO_EXTS.includes(ext);
}

/** A preview target that is an inline image (an attached or generated
 *  picture), not a file on disk. */
export function isInlineImage(target: string): boolean {
  return /^data:image\//i.test(target);
}

/** Collapse `.` and `..` segments and repeated slashes in an absolute path. */
export function normalizePath(path: string): string {
  const out: string[] = [];
  for (const seg of path.split('/')) {
    if (!seg || seg === '.') continue;
    if (seg === '..') out.pop();
    else out.push(seg);
  }
  return `/${out.join('/')}`;
}

/** Join a base directory and a relative path, normalized. */
export function joinPath(base: string, rel: string): string {
  return normalizePath(`${base.replace(/\/+$/, '')}/${rel}`);
}

/** The lines a reply's file link cites: `line`, and `end` for a range. */
export type CitedLines = { line: number; end?: number };

/** A file a reply's link opens in the Preview pane, at the lines it cites. */
export type FileLink = { path: string } & Partial<CitedLines>;

/** A trailing line citation, as agents write them: `:12`, `:12:3` (line and
 *  column), `:12-20`, `#L12`, `#L12-L20` and `#L12-20`. */
const LINE_SUFFIX = /(?::(\d+)(?::\d+|-(\d+))?|#L(\d+)(?:-L?(\d+))?)$/;

/** `ref` without its trailing line citation, and the lines that citation
 *  names: null when there is none, or when the line is 0 or too large to be
 *  a line. An end that isn't after the line is dropped (one line). */
export function splitLineSuffix(ref: string): { ref: string; lines: CitedLines | null } {
  const m = LINE_SUFFIX.exec(ref);
  if (!m) return { ref, lines: null };
  const line = Number(m[1] ?? m[3]), end = Number(m[2] ?? m[4] ?? NaN);
  const lines = Number.isSafeInteger(line) && line >= 1
    ? (Number.isSafeInteger(end) && end > line ? { line, end } : { line })
    : null;
  return { ref: ref.slice(0, m.index), lines };
}

/** A bare file name with its line (`README.md:12`, `app.ts:3:9`,
 *  `notes.txt:4-9`), which reads like a URL scheme (`README.md:`) but is a
 *  file: a scheme has no `.ext` before its colon and no digit after it
 *  (2026-11-04: such links only said "Hollow can't open 'README.md:' links"). */
export function isFileWithLine(ref: string): boolean {
  return /^[^/:?#\s]+\.[A-Za-z][A-Za-z0-9]{0,7}:\d+(?:[-:]\d+)?(?:[?#].*)?$/.test(ref.trim());
}

/**
 * The absolute file a reply's link points at, or null when it isn't a local
 * file (a web link, `mailto:`, an in-page `#anchor`, or a relative link with
 * no base to resolve against). Handles `file://` URLs, percent-encoding and
 * the line suffixes agents write (`path:12`, `path:12:3`, `path:12-20`,
 * `path#L12`), which it drops: `fileLinkFromHref` keeps them.
 */
export function localFileFromHref(href: string, base: string | null, home: string | null = null): string | null {
  let ref = href.trim();
  if (!ref || ref.startsWith('#')) return null;
  if (/^file:\/\//i.test(ref)) ref = ref.replace(/^file:\/\/(localhost)?/i, '');
  else if (/^[a-z][a-z0-9+.-]*:/i.test(ref) && !isFileWithLine(ref)) return null;
  ref = ref.replace(/[?#].*$/, '');
  try { ref = decodeURIComponent(ref); } catch { /* keep it as written */ }
  ref = splitLineSuffix(ref).ref;
  if (!ref) return null;
  if (ref.startsWith('~')) return ref.startsWith('~/') && home?.startsWith('/') ? joinPath(home, ref.slice(2)) : null;
  if (ref.startsWith('/')) return normalizePath(ref);
  return base ? joinPath(base, ref) : null;
}

/**
 * What a click on a reply's file link opens (S1b, 2026-11-03): the file
 * `localFileFromHref` reads, and the lines its suffix cites, from a `#L12` /
 * `#L12-L20` fragment or a `:12` / `:12:3` / `:12-20` suffix on the path.
 * Null when the link isn't a local file. A link with no line opens the file
 * at its top, as before.
 */
export function fileLinkFromHref(href: string, base: string | null, home: string | null = null): FileLink | null {
  const path = localFileFromHref(href, base, home);
  if (!path) return null;
  let ref = href.trim();
  // Only a `#L` fragment cites lines; any other fragment, and a query, don't.
  const fragment = /#L\d[^#]*$/.exec(ref)?.[0] ?? '';
  ref = ref.replace(/[?#].*$/, '');
  try { ref = decodeURIComponent(ref); } catch { /* keep it as written */ }
  const lines = splitLineSuffix(fragment || ref).lines;
  return lines ? { path, ...lines } : { path };
}

/** Extensions that make a bare name (no `/`) in inline code read as a file.
 *  Deliberately short: `console.log`, `e.target` and `Node.js` stay code. */
const BARE_FILE_EXTS = new Set([
  'md', 'markdown', 'txt', 'pdf', 'png', 'jpg', 'jpeg', 'gif', 'webp', 'svg',
  'html', 'htm', 'csv', 'json', 'yaml', 'yml', 'toml', 'ts', 'tsx', 'rs', 'py',
  'xlsx', 'docx', 'pptx', 'ipynb', 'mp4', 'm4v', 'mov', 'webm', 'mmd', 'mermaid',
  // 2026-11-04: more files agents name bare (`style.css`, `run.sh`). Still
  // not js, log, c or h, so `Node.js` and `console.log` stay code.
  'mjs', 'cjs', 'jsx', 'css', 'scss', 'go', 'sh', 'tsv', 'xml', 'sql', 'swift',
  'kt', 'java', 'cpp', 'rb', 'php', 'heic', 'ini', 'vue', 'svelte', 'xls',
  'doc', 'ppt', 'odt', 'rtf', 'mp3', 'm4a', 'wav', 'zip',
]);


/**
 * The file an inline-code span in a reply names (`outputs/report.pdf`,
 * `src/app.ts:42`, `src/a.ts:10-20`, `/abs/path/My File.md`), without its
 * line suffix (`fileHrefFromCode` keeps it), or null
 * when it reads as code rather than a path. A path with a `/` needs a
 * filename with an extension; a bare name needs one of BARE_FILE_EXTS. Web
 * URLs, commands (anything with a space and no `/`) and globs are not files.
 * A leading `~/` needs the OS home explicitly supplied by the caller.
 */
export function fileRefFromCode(text: string, home: string | null = null): string | null {
  let t = text.trim();
  if (!t || t.length > 400 || /[\n*?<>|"`$]/.test(t)) return null;
  if (t.startsWith('~')) {
    if (!t.startsWith('~/') || !home?.startsWith('/')) return null;
    t = joinPath(home, t.slice(2));
  }
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(t)) return null;
  const { ref } = splitLineSuffix(t);
  const name = ref.slice(ref.lastIndexOf('/') + 1);
  const ext = /\.([A-Za-z][A-Za-z0-9]{0,7})$/.exec(name)?.[1]?.toLowerCase();
  if (!ext || (name.startsWith('.') && name.lastIndexOf('.') === 0)) return null;
  if (ref.includes('/')) {
    // A path may have spaces in folder names, but not a command's flags.
    if (/\s-{1,2}[A-Za-z]/.test(ref) || /^[a-z]+\s/.test(ref)) return null;
    return ref;
  }
  return !/\s/.test(ref) && BARE_FILE_EXTS.has(ext) ? ref : null;
}

/**
 * The link an inline-code file reference in a reply carries (its
 * `data-file-ref`): the path `fileRefFromCode` reads, with the lines the code
 * cites as a `#L12` or `#L12-20` fragment, so the click reads both through
 * `fileLinkFromHref`. A fragment, not `:12`, so a bare `README.md:12` never
 * reads as a `README.md:` scheme. Null when the code isn't a file.
 */
export function fileHrefFromCode(text: string, home: string | null = null): string | null {
  const ref = fileRefFromCode(text, home);
  if (!ref) return null;
  const lines = splitLineSuffix(text.trim()).lines;
  // The click reads this back through `fileLinkFromHref`, which cuts at `?`
  // or `#` and percent-decodes, so those three characters are escaped here
  // and a path like `src/C#/a.cs` or `docs/50%20off.md` round-trips exactly.
  const safe = ref.replace(/%/g, '%25').replace(/#/g, '%23').replace(/\?/g, '%3F');
  return lines ? `${safe}#L${lines.line}${lines.end ? `-${lines.end}` : ''}` : safe;
}

/** A reply-footer chip's absolute path: repository rows carry their root;
 *  vault rows are vault-relative. Null when the root isn't known yet. */
export function footerChipPath(chip: { path: string; root?: string }, vaultRoot: string | null): string | null {
  if (chip.path.startsWith('/')) return normalizePath(chip.path);
  const root = chip.root ?? vaultRoot;
  return root ? joinPath(root, chip.path) : null;
}

/** The text with fenced code blocks (``` or ~~~) emptied, line by line. */
function blankFences(md: string): string {
  let fence: string | null = null;
  return md.split('\n').map(line => {
    const m = /^ {0,3}(`{3,}|~{3,})(.*)$/.exec(line);
    if (fence) {
      // Only a bare fence (no info string) closes one.
      if (m && m[1][0] === fence[0] && m[1].length >= fence.length && !m[2].trim()) fence = null;
      return '';
    }
    if (m) { fence = m[1]; return ''; }
    return line;
  }).join('\n');
}

/** The absolute file a literal path names (inline code, a bare path in
 *  prose): read as written, unlike an href, so `%`, `#` and `?` stay part of
 *  the name. A relative one resolves against `base`; `~` is never resolved. */
function localFileFromPath(ref: string, base: string | null): string | null {
  if (ref.startsWith('/')) return normalizePath(ref);
  return base && !ref.startsWith('~') ? joinPath(base, ref) : null;
}

/** A markdown link, and a raw-HTML `<a href>`, `<video src>` or `<source src>`. */
const MD_LINK = /\[[^\]\n]*\]\(\s*(?:<([^>\n]+)>|([^)\s]+))(?:\s+"[^"\n]*")?\s*\)/g;
const MEDIA_TAG = /<(?:a|video|source)\b[^>\n]*?\s(?:href|src)\s*=\s*(?:"([^"\n]+)"|'([^'\n]+)')/gi;

/**
 * The videos a reply names, as absolute paths in the order they appear, each
 * once: markdown links, raw-HTML `<a href>`, `<video src>` and
 * `<source src>`, inline-code paths `fileRefFromCode` accepts, and bare
 * absolute paths. Fenced code blocks are skipped; web links and `~` paths are
 * ignored; relative paths resolve against `base` (the conversation's folder).
 * Links and tags are hrefs (percent-decoded, cut at `?` or `#`); inline code
 * and bare paths are literal paths, read as written, as cited file links are.
 */
export function videoRefsFromText(md: string, base: string | null): string[] {
  const text = blankFences(md ?? '');
  const blank = (m: string) => ' '.repeat(m.length);
  // Inline code is read only as a path (`fileRefFromCode`): a command's
  // arguments (`ffmpeg -i /a/in.mov /a/out.mp4`) are never bare paths.
  const prose = text.replace(/`[^`\n]+`/g, blank);
  const found: { at: number; ref: string; literal: boolean }[] = [];
  const take = (src: string, re: RegExp, map: (m: RegExpMatchArray) => string | null, literal: (ref: string) => boolean) => {
    for (const m of src.matchAll(re)) {
      const ref = map(m);
      if (ref) found.push({ at: m.index ?? 0, ref, literal: literal(ref) });
    }
  };
  take(prose, MD_LINK, m => m[1] ?? m[2], () => false);
  take(prose, MEDIA_TAG, m => m[1] ?? m[2], () => false);
  take(text, /`([^`\n]+)`/g, m => fileRefFromCode(m[1]), () => true);
  // A link's own target was read above, as an href: blank links and tags so
  // the bare-path pass doesn't read it again, literally, as a second file.
  const bare = prose.replace(MD_LINK, blank).replace(MEDIA_TAG, blank);
  take(bare, /(?:^|[\s(*])((?:file:\/\/)?\/[^\s"'<>`()*]+\.(?:mp4|m4v|mov))(?=$|[\s.,;:!?)*])/gim, m => m[1], ref => !/^file:/i.test(ref));
  const out: string[] = [];
  const seen = new Set<string>();
  for (const { ref, literal } of found.sort((a, b) => a.at - b.at)) {
    const path = literal ? localFileFromPath(ref, base) : localFileFromHref(ref, base);
    // macOS folders are case-insensitive: one file, one player.
    if (path && isVideoPath(path) && !seen.has(path.toLowerCase())) { seen.add(path.toLowerCase()); out.push(path); }
  }
  return out;
}
