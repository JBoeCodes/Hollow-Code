/**
 * Stray tags in a canvas (overnight 10-11/12 A2).
 *
 * An agent wrote an ask-canvas matrix row `Back to canvas: <title>`. The HTML
 * tokenizer (and `fence.ts`, which reads the fence the same way) opened a
 * real `title` element, whose raw text swallowed the rest of the fence, and
 * the card said only "The canvas ended before </hc-canvas>". Two fixes:
 *
 * - `escapeRowTags`: inside the kit's data-row elements (`ROW_ELEMENTS`),
 *   whose rows the kit reads as text, a `<` that doesn't open an allowed tag
 *   (`ROW_TAGS`, any `hc-…` element, or the row element's own end tag) becomes
 *   `&lt;`, so the row shows it literally. Applied to the fence body before
 *   it is parsed, saved or drawn; the store keeps the escaped body, which
 *   `canvas_fence.rs` validates like any other.
 * - Outside rows (PER-1): a raw-text start tag (`<title>`, `<textarea>`,
 *   `<plaintext>`…) never closed before the root's `</hc-canvas>` is a word
 *   too, written `&lt;`, once the fence has its end.
 * - `diagnoseUnclosed`: anywhere else, a canvas that never reaches its
 *   `</hc-canvas>` says which construct hid it (a raw-text element such as
 *   `<title>` or `<textarea>`, an open comment, `<plaintext>`, a tag whose
 *   quote never closes), on which line, and how to fix it. "Ask to fix"
 *   carries that text.
 *
 * Both read the body the way `fence.ts`'s `scanContent` does (comments,
 * `<!…>`, attribute values and raw-text contents are skipped), so a
 * `"<hc-chart>"` in a script string is not a chart. Pure: no DOM.
 */

/** Kit elements whose content is read as text rows: a `<` in them is
 *  literal unless it opens an allowed tag. */
export const ROW_ELEMENTS: ReadonlySet<string> = new Set([
  'hc-chart', 'hc-bars', 'hc-table', 'hc-flow', 'hc-sparkline', 'hc-timeline', 'hc-heatmap', 'hc-checklist',
  'hc-code', 'hc-file', 'hc-choice', 'hc-multi', 'hc-rank', 'hc-matrix', 'hc-pick',
]);
/** Elements read as plain text (code, a file name): every `<` but their own
 *  end tag is literal, so markup in a code sample shows as written. */
const TEXT_ONLY: ReadonlySet<string> = new Set(['hc-code', 'hc-file']);

/** HTML tags an agent may use inside a data-row element: a `<table>` for
 *  hc-chart / hc-table, a list or one element per option, and inline
 *  phrasing. Anything else there is text. */
export const ROW_TAGS: ReadonlySet<string> = new Set([
  'table', 'thead', 'tbody', 'tfoot', 'tr', 'th', 'td', 'caption', 'colgroup', 'col',
  'ul', 'ol', 'li', 'dl', 'dt', 'dd', 'div', 'p', 'span', 'br', 'wbr', 'hr', 'pre', 'blockquote',
  'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'figure', 'figcaption', 'label', 'option', 'img',
  'a', 'abbr', 'b', 'cite', 'code', 'data', 'del', 'em', 'i', 'ins', 'kbd', 'mark', 'q',
  's', 'samp', 'small', 'strong', 'sub', 'sup', 'time', 'u', 'var',
  'svg', 'g', 'path', 'circle', 'ellipse', 'line', 'polyline', 'polygon', 'rect', 'text', 'tspan', 'defs', 'use',
]);
/** Elements that never take an end tag. */
const VOID = new Set(['br', 'wbr', 'hr', 'img', 'col']);

const RAW_TEXT = new Set(['script', 'style', 'textarea', 'title', 'xmp', 'iframe', 'noembed', 'noframes', 'noscript']);

const isWs = (c: string | undefined) => c === ' ' || c === '\t' || c === '\n' || c === '\f' || c === '\r';
const isAlpha = (c: string | undefined) => c !== undefined && ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z'));
const asciiLower = (s: string) => s.replace(/[A-Z]/g, c => String.fromCharCode(c.charCodeAt(0) + 32));

/** A tag name from `from` (ASCII-lowered) and where it ends; `complete` is
 *  false when the text ends inside it (still streaming). */
function tagName(t: string, from: number): { name: string; end: number; complete: boolean } {
  let k = from;
  while (k < t.length && !isWs(t[k]) && t[k] !== '/' && t[k] !== '>') k++;
  return { name: asciiLower(t.slice(from, k)), end: k, complete: k < t.length };
}

/** Past the rest of a start tag, honouring quoted values; null if it never ends. */
function skipTag(t: string, from: number): number | null {
  let k = from;
  while (k < t.length) {
    const ch = t[k];
    if (ch === '>') return k + 1;
    k++;
    if (ch === '=') {
      while (k < t.length && isWs(t[k])) k++;
      if (k < t.length && (t[k] === '"' || t[k] === "'")) {
        const e = t.indexOf(t[k], k + 1);
        if (e < 0) return null;
        k = e + 1;
      }
    }
  }
  return null;
}

/** Index of the `</name` ending a raw-text element, or null. */
function rawTextEnd(t: string, from: number, name: string): number | null {
  let k = from;
  for (;;) {
    const at = t.indexOf('</', k);
    if (at < 0) return null;
    const after = at + 2 + name.length;
    if (after >= t.length) return null;
    if (asciiLower(t.slice(at + 2, after)) === name && (isWs(t[after]) || t[after] === '/' || t[after] === '>')) return at;
    k = at + 2;
  }
}

const allowedInRows = (name: string) => ROW_TAGS.has(name) || name.startsWith('hc-');
const ROW_HINT = new RegExp(`<(?:${[...ROW_ELEMENTS].join('|')})\\b`, 'i');
const RAW_HINT = new RegExp(`<(?:${[...RAW_TEXT, 'plaintext'].join('|')})\\b`, 'i');

/**
 * The body with every stray `<` inside a data-row element written `&lt;`.
 * Returns `body` itself when nothing changes. Safe on a body still
 * streaming (a tag name cut off at the end is left alone until it arrives)
 * and idempotent.
 */
export function escapeRowTags(body: string): string {
  if (!ROW_HINT.test(body) && !RAW_HINT.test(body)) return body;
  const t = body;
  const n = t.length;
  // The root's end tag, once the fence has it (PER-1): a raw-text start tag
  // outside rows (`<title>`, `<textarea>`, `<plaintext>`) whose own end tag
  // doesn't come before it would hide the rest of the canvas, so it is a
  // word, written `&lt;`. While the fence streams there is no end yet, and
  // the tag waits as before.
  const rootEnd = asciiLower(t).lastIndexOf('</hc-canvas');
  let out = '';
  let copied = 0;
  let k = 0;
  // The data-row element we're inside, or null, and the allowed elements
  // opened inside it that haven't closed.
  let row: string | null = null;
  const open: string[] = [];
  while (k < n) {
    const lt = t.indexOf('<', k);
    if (lt < 0) break;
    if (row !== null) {
      const c = t[lt + 1];
      if (c === undefined) break; // a `<` at the end: wait for the next flush
      const textOnly = TEXT_ONLY.has(row);
      if (c === '/' && isAlpha(t[lt + 2])) {
        const tag = tagName(t, lt + 2);
        if (!tag.complete) break;
        if (tag.name === row || (!textOnly && allowedInRows(tag.name))) {
          // The row's own end tag, or one closing what the row opened; any
          // other allowed end tag closes the row the way the browser would
          // (an `hc-chart` left open inside a tab ends with the tab's `</div>`).
          const at = open.lastIndexOf(tag.name);
          if (at >= 0) open.length = at;
          else { row = null; open.length = 0; }
          const e = t.indexOf('>', tag.end);
          if (e < 0) break;
          k = e + 1;
          continue;
        }
      } else if (!textOnly && isAlpha(c)) {
        const tag = tagName(t, lt + 1);
        if (!tag.complete) break;
        if (allowedInRows(tag.name)) {
          const e = skipTag(t, tag.end);
          if (e === null) break;
          if (!VOID.has(tag.name) && t[e - 2] !== '/') open.push(tag.name);
          k = e;
          continue;
        }
      }
      // Text: the row shows this `<` as it is.
      out += t.slice(copied, lt) + '&lt;';
      copied = lt + 1;
      k = lt + 1;
      continue;
    }
    if (t.startsWith('<!--', lt)) {
      const e = t.indexOf('-->', lt + 4);
      if (e < 0) break;
      k = e + 3;
      continue;
    }
    const c = t[lt + 1];
    if (c === '!' || c === '?' || c === '/') {
      const e = t.indexOf('>', lt + 2);
      if (e < 0) break;
      k = e + 1;
      continue;
    }
    if (!isAlpha(c)) { k = lt + 1; continue; }
    const tag = tagName(t, lt + 1);
    const end = skipTag(t, tag.end);
    if (end === null) break;
    if (tag.name === 'plaintext' || RAW_TEXT.has(tag.name)) {
      const close = tag.name === 'plaintext' ? null : rawTextEnd(t, end, tag.name);
      if (rootEnd > lt && (close === null || close > rootEnd)) {
        out += t.slice(copied, lt) + '&lt;';
        copied = lt + 1;
        k = lt + 1;
        continue;
      }
      if (close === null) break;
      k = close;
      continue;
    }
    if (ROW_ELEMENTS.has(tag.name) && t[end - 2] !== '/') { row = tag.name; open.length = 0; }
    k = end;
  }
  return copied === 0 ? body : out + t.slice(copied);
}

/** What hid a canvas's `</hc-canvas>`, and on which line of the fence body (from 1). */
export type UnclosedCause =
  | { kind: 'raw'; name: string; line: number }
  | { kind: 'comment'; line: number }
  | { kind: 'plaintext'; line: number }
  | { kind: 'declaration'; line: number }
  | { kind: 'tag'; name: string; line: number };

const lineAt = (t: string, at: number) => { let n = 1; for (let i = t.indexOf('\n'); i >= 0 && i < at; i = t.indexOf('\n', i + 1)) n++; return n; };

/**
 * Read a finished fence body the way `fence.ts` does and name the construct
 * still open at its end: the reason the root's `</hc-canvas>` was never
 * seen. Null when nothing is open (the end tag is simply missing) or the
 * body has no root start tag.
 */
export function unclosedCause(body: string): UnclosedCause | null {
  const t = body;
  const n = t.length;
  const root = /<hc-canvas(?=[\s>])/i.exec(t);
  if (!root) return null;
  const rootEnd = skipTag(t, root.index + root[0].length);
  if (rootEnd === null) return null;
  // No `</hc-canvas` at all: the canvas was cut off (Stop, an agent error, a
  // dropped fence) or never closed, and the plain reason says so.
  const last = asciiLower(t).lastIndexOf('</hc-canvas');
  if (last < 0) return null;
  // A construct still open at the end, or one that ends only past the end
  // tag (`<!DOCTYPE x` with no `>` ends at the end tag's own `>`), hid it.
  const swallows = (from: number, end: number) => last > from && (end < 0 || last < end);
  let k = rootEnd;
  while (k < n) {
    const lt = t.indexOf('<', k);
    if (lt < 0 || lt + 1 >= n) return null;
    if (t.startsWith('<!--', lt)) {
      const e = t.indexOf('-->', lt + 4);
      if (swallows(lt, e)) return { kind: 'comment', line: lineAt(t, lt) };
      if (e < 0) return null;
      k = e + 3;
      continue;
    }
    const c = t[lt + 1];
    if (c === '!' || c === '?') {
      const e = t.indexOf('>', lt + 2);
      if (swallows(lt, e)) return { kind: 'declaration', line: lineAt(t, lt) };
      if (e < 0) return null;
      k = e + 1;
      continue;
    }
    if (c === '/') {
      if (asciiLower(t.slice(lt, lt + 11)) === '</hc-canvas') return null;
      const e = t.indexOf('>', lt + 2);
      if (swallows(lt, e)) return isAlpha(t[lt + 2]) ? { kind: 'tag', name: `/${tagName(t, lt + 2).name}`, line: lineAt(t, lt) } : { kind: 'declaration', line: lineAt(t, lt) };
      if (e < 0) return null;
      k = e + 1;
      continue;
    }
    if (!isAlpha(c)) { k = lt + 1; continue; }
    const tag = tagName(t, lt + 1);
    const end = skipTag(t, tag.end);
    if (swallows(lt, end ?? -1)) return { kind: 'tag', name: tag.name, line: lineAt(t, lt) };
    if (end === null) return null;
    if (tag.name === 'plaintext') return last > lt ? { kind: 'plaintext', line: lineAt(t, lt) } : null;
    if (RAW_TEXT.has(tag.name)) {
      const close = rawTextEnd(t, end, tag.name);
      if (swallows(end - 1, close ?? -1)) return { kind: 'raw', name: tag.name, line: lineAt(t, lt) };
      if (close === null) return null;
      k = close;
      continue;
    }
    k = end;
  }
  return null;
}

/** The reason a broken canvas shows (and "Ask to fix" quotes) for a cause. */
export function unclosedReason(cause: UnclosedCause): string {
  const at = `Line ${cause.line} of the canvas`;
  switch (cause.kind) {
    case 'raw':
      return `${at}: <${cause.name}> is never closed, so it hides the rest of the canvas and its </hc-canvas>. `
        + `For the text, write &lt;${cause.name}&gt; or put it in words; otherwise close it with </${cause.name}>.`;
    case 'comment':
      return `${at}: a comment (<!--) is never closed with -->, so it hides the rest of the canvas. Close it, or write &lt;!-- for the text.`;
    case 'plaintext':
      return `${at}: <plaintext> hides the rest of the canvas. Write &lt;plaintext&gt; or put it in words.`;
    case 'declaration':
      return `${at}: a <! or <? never ends with >, so it hides the rest of the canvas. Write &lt; for the text.`;
    case 'tag':
      return cause.name.startsWith('/')
        ? `${at}: the <${cause.name}> tag never ends (its > is missing), so it hides the rest of the canvas.`
        : `${at}: the <${cause.name}> tag never ends (a quote or > is missing), so it hides the rest of the canvas.`;
  }
}
