/**
 * File paths written in a reply's prose, not in backticks (2026-11-04:
 * "I saved it to outputs/q3-review.md." wasn't clickable). Loaded with a
 * reply's decoration, after it draws (its own chunk: the App launch chunk's
 * budget), and marks each path like a backticked one (ReplyMarkdown).
 */
import { fileHrefFromCode, splitLineSuffix } from './chatFileLinks';

/** Extensions that make a bare name in a reply's prose a file (`PLAN.md`,
 *  `report.pdf`): documents, data, pictures and video, not code, so prose
 *  naming a library (`Node.js`, `Vue.js`) or an object stays words. */
const PROSE_BARE_EXTS = new Set([
  'md', 'markdown', 'txt', 'pdf', 'csv', 'tsv', 'xlsx', 'xls', 'docx', 'doc',
  'pptx', 'ppt', 'html', 'htm', 'json', 'png', 'jpg', 'jpeg', 'gif', 'webp',
  'svg', 'heic', 'mp4', 'mov', 'm4v', 'ipynb', 'rtf', 'odt',
]);

/** A path-shaped run of prose: absolute (`/…`, `~/…`), relative with a folder
 *  (`outputs/q3.md`, `../main/x.md`) or a bare name, then an extension and
 *  any `:12` / `:12-20` lines. Not inside a word, a URL or an address. */
const PROSE_PATH = /(?<![\w/.:~@#%-])(?:~\/|\.{1,2}\/|\/)?[\w@+-][\w.@+-]*(?:\/[\w@+-][\w.@+-]*)*\.[A-Za-z][A-Za-z0-9]{0,7}(?::\d+(?:-\d+)?)?(?![\w/@+-]|\.[A-Za-z0-9])/g;

/** A first folder that is a web host (`example.com/a.html`): a URL written
 *  without its scheme, not a file. */
const HOST_FIRST = /^(?:www\.)?[\w-]+(?:\.[\w-]+)*\.(?:com|org|net|io|dev|ai|co|gov|edu|app|me|us|uk|ca)\//i;

/**
 * File paths written in a reply's prose rather than in backticks (2026-11-04:
 * "I saved it to outputs/q3-review.md." wasn't clickable): where each starts
 * and ends in `text`, and the link its click opens (`fileHrefFromCode`'s,
 * with any cited lines). A bare name counts only with a document-like
 * extension (PROSE_BARE_EXTS); `~/` paths need `home`.
 */
export function prosePathRefs(text: string, home: string | null = null): { start: number; end: number; href: string }[] {
  const out: { start: number; end: number; href: string }[] = [];
  for (const m of text.matchAll(PROSE_PATH)) {
    const token = m[0], start = m.index ?? 0;
    const { ref } = splitLineSuffix(token);
    if (!ref.includes('/')) {
      // A bare name starts with a letter: `1.pdf` is more likely the tail of
      // a path with a space in it ("Invoice 1.pdf") than a file of its own.
      const ext = /\.([A-Za-z][A-Za-z0-9]*)$/.exec(ref)?.[1]?.toLowerCase();
      if (!ext || !PROSE_BARE_EXTS.has(ext) || ref.length < 4 || !/^[A-Za-z_]/.test(ref)) continue;
    } else if (HOST_FIRST.test(ref)) continue;
    const href = fileHrefFromCode(token, home);
    if (href) out.push({ start, end: start + token.length, href });
  }
  return out;
}

/** Each prose path in `el` becomes a file reference like a backticked path
 *  (`span.file-ref.prose-file-ref` with its link), except inside code, a
 *  link, a button or one already marked. Nodes are collected first, then
 *  replaced, so the walk never sees its own spans. */
export function markProsePaths(el: HTMLElement, homePath: string | null) {
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT, {
    acceptNode: n => n.parentElement?.closest('pre, code, a, button, kbd, samp, .file-ref') ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT,
  });
  const nodes: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) if ((n as Text).data.length >= 4 && /[./]/.test((n as Text).data)) nodes.push(n as Text);
  for (const node of nodes) {
    const refs = prosePathRefs(node.data, homePath);
    if (!refs.length) continue;
    const parts = document.createDocumentFragment();
    let at = 0;
    for (const r of refs) {
      if (r.start > at) parts.append(node.data.slice(at, r.start));
      const span = document.createElement('span');
      span.className = 'file-ref prose-file-ref';
      span.dataset.fileRef = r.href;
      span.setAttribute('role', 'button');
      span.tabIndex = 0;
      span.title = 'Preview';
      span.textContent = node.data.slice(r.start, r.end);
      parts.append(span);
      at = r.end;
    }
    if (at < node.data.length) parts.append(node.data.slice(at));
    node.replaceWith(parts);
  }
}
