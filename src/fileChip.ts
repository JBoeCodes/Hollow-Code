/**
 * File chips (2026-11-10; Alex: "having the file link be the full path is
 * ridiculous"): every file a chat names shows as its name with a small icon
 * for its type, the path kept for the tooltip, the accessible name and copy.
 * One map serves a reply's file links (ReplyMarkdown) and the tool ledger's
 * file chips (ToolCall), so a type looks the same in both. Pure (`import
 * type` erases), so the node suite can load it.
 */
import type { IconName } from './icons';

/** A chip's icon colour, by family (`tone-*` in tool-ledger.css). */
export type FileTone = 'image' | 'video' | 'code' | 'doc' | 'plain' | 'sheet' | 'word' | 'slides' | 'pdf' | 'web' | 'folder';

const IMAGE_EXT = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'heic', 'bmp', 'ico', 'avif', 'tif', 'tiff']);
const VIDEO_EXT = new Set(['mp4', 'm4v', 'mov', 'webm']);
const SHEET_EXT = new Set(['csv', 'tsv', 'xlsx', 'xls', 'xlsm', 'numbers', 'ods']);
const WORD_EXT = new Set(['docx', 'doc', 'rtf', 'odt', 'pages']);
const SLIDES_EXT = new Set(['pptx', 'ppt', 'key', 'odp']);
const DATA_EXT = new Set(['json', 'jsonl', 'yaml', 'yml', 'toml', 'xml', 'ipynb']);
const CODE_EXT = new Set([
  'ts', 'tsx', 'js', 'jsx', 'mjs', 'cjs', 'rs', 'py', 'go', 'rb', 'java', 'kt', 'swift', 'c', 'h', 'cpp',
  'css', 'scss', 'vue', 'svelte', 'sh', 'zsh', 'bash', 'sql', 'php', 'cs', 'lua', 'r',
]);
const ARCHIVE_EXT = new Set(['zip', 'tar', 'gz', 'tgz', 'bz2', 'xz', '7z', 'dmg']);

/** A path's extension, lower case, after any line citation is cut off. */
export function fileExt(path: string): string {
  const m = /\.([A-Za-z0-9]+)$/.exec(path.replace(/(?::\d+(?:[-:]\d+)?|#L\d+(?:-L?\d+)?)$/, ''));
  return m ? m[1].toLowerCase() : '';
}

/** A file chip's icon and colour, by file type; a path ending in `/` is a
 *  folder. Anything unknown gets the plain file. */
export function fileKind(path: string): { icon: IconName; tone: FileTone } {
  if (/\/$/.test(path.trim())) return { icon: 'folder', tone: 'folder' };
  const e = fileExt(path);
  if (IMAGE_EXT.has(e)) return { icon: 'fileImage', tone: 'image' };
  if (VIDEO_EXT.has(e)) return { icon: 'play', tone: 'video' };
  if (e === 'md' || e === 'mdx' || e === 'markdown') return { icon: 'markdown', tone: 'doc' };
  if (SHEET_EXT.has(e)) return { icon: 'fileSpreadsheet', tone: 'sheet' };
  if (WORD_EXT.has(e)) return { icon: 'fileType', tone: 'word' };
  if (SLIDES_EXT.has(e)) return { icon: 'presentation', tone: 'slides' };
  if (e === 'pdf') return { icon: 'filePdf', tone: 'pdf' };
  if (e === 'html' || e === 'htm') return { icon: 'globe', tone: 'web' };
  if (DATA_EXT.has(e)) return { icon: 'fileJson', tone: 'code' };
  if (CODE_EXT.has(e)) return { icon: 'fileCode', tone: 'code' };
  if (ARCHIVE_EXT.has(e)) return { icon: 'archive', tone: 'plain' };
  if (e === 'txt' || e === 'log' || e === 'text') return { icon: 'fileText', tone: 'plain' };
  return { icon: 'file', tone: 'plain' };
}

/** What a chip shows for a path as written: its name, and the lines it cites
 *  (`:12`, `:12-20`) apart, so they can be muted. A folder keeps no slash. */
export function chipName(path: string): { name: string; lines: string } {
  const t = path.trim().replace(/\/+$/, '');
  const m = /(?::\d+(?:[-:]\d+)?|#L\d+(?:-L?\d+)?)$/.exec(t);
  const ref = m ? t.slice(0, m.index) : t;
  const name = ref.slice(ref.lastIndexOf('/') + 1) || ref;
  const lines = m ? m[0].replace(/^#L(\d+)(?:-L?(\d+))?$/, (_, a, b) => `:${a}${b ? `-${b}` : ''}`) : '';
  return { name, lines };
}

/** The folders of `path` (no name, no line citation), as written. */
function folders(path: string): string[] {
  const t = path.trim().replace(/\/+$/, '').replace(/(?::\d+(?:[-:]\d+)?|#L\d+(?:-L?\d+)?)$/, '');
  return t.split('/').slice(0, -1).filter(s => s && s !== '.');
}

/**
 * For each path, the folder words that tell it apart from another path with
 * the same name in the same reply: '' when its name is the only one (or every
 * path with that name is this same path), else the fewest trailing folders
 * that differ from the others (`email-intake`, `a/outputs`). Names compare
 * without case, as macOS folders do.
 */
export function chipFolders(paths: readonly string[]): string[] {
  const norm = (p: string) => p.trim().replace(/\/+$/, '').toLowerCase();
  const byName = new Map<string, Set<string>>();
  for (const p of paths) {
    const key = chipName(p).name.toLowerCase();
    if (!byName.has(key)) byName.set(key, new Set());
    byName.get(key)!.add(norm(p));
  }
  return paths.map(p => {
    const same = [...(byName.get(chipName(p).name.toLowerCase()) ?? [])].filter(o => o !== norm(p));
    if (!same.length) return '';
    const mine = folders(p);
    const others = same.map(folders);
    for (let k = 1; k <= mine.length; k++) {
      const tail = mine.slice(-k).join('/').toLowerCase();
      if (others.every(o => o.slice(-k).join('/').toLowerCase() !== tail)) return mine.slice(-k).join('/');
    }
    return mine.join('/');
  });
}

/** A chip's accessible name: what a click does, the name and where it is. */
export function chipLabel(path: string): string {
  const { name, lines } = chipName(path);
  const where = path.trim();
  const shown = `${name}${lines}`;
  return where && where !== shown ? `Open ${shown}, ${where}` : `Open ${shown}`;
}
