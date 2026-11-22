/**
 * The `hollow-canvas` fence parser (Canvas C3 foundations, plan §4).
 *
 * A fence body is one root element and nothing else:
 *
 *     <hc-canvas id="release-status" title="Release status" [ask] [height="480"]>
 *       …HTML…
 *     </hc-canvas>
 *
 * This module reads that root and classifies the body for the chat skeleton:
 * `pending` (the root start tag hasn't closed yet), `drawing` (the start tag
 * closed and the fence is still streaming; the title is known), `ready` (the
 * fence ended and the canvas is well formed) or `invalid` with a short human
 * reason. It never builds a DOM and never interprets the inner HTML beyond
 * finding where the root ends, so it runs in the node test environment and
 * on every streaming flush.
 *
 * `src-tauri/src/canvas_fence.rs` re-validates with the same rules before a
 * canvas is saved. Both are tested against one case file,
 * `src/canvas/fence.golden.json` (limits, reason texts and cases), so they
 * cannot drift. Change a rule in both files and the golden together.
 *
 * Rules (also in docs/history/CANVAS_C3.md):
 * - Body ≤ 128 KB UTF-8 (`MAX_BODY_BYTES`). Only HTML whitespace (space, tab,
 *   LF, FF, CR) may come before the root or after `</hc-canvas>`.
 * - Tag and attribute names are ASCII case-insensitive (`<HC-Canvas ID=…>`).
 *   Values may be double-quoted, single-quoted or unquoted. The root start
 *   tag must close within `MAX_ROOT_TAG_BYTES` and carry at most
 *   `MAX_ROOT_ATTRIBUTES`; a self-closing `/>`, a stray `/`, `"`, `'` or `<`
 *   in a name, or a quoted value glued to the next attribute is malformed.
 * - `id`: `[a-z0-9-]{1,64}` (the rule whiteboards used, kept). `title`: required,
 *   1–180 characters (code points) that are not all whitespace, with no
 *   control characters other than tab and newline.
 *   `ask`: boolean; bare, `ask=""` or `ask="ask"`. Any other value is refused
 *   so `ask="false"` can't silently mean yes. `height`: optional whole pixels,
 *   80–1600, with or without a `px` suffix. A repeated attribute is refused.
 * - **Unknown root attributes are ignored**, not refused: an older Hollow can
 *   still show a canvas from a newer agent prompt, and the host reads only the
 *   attributes above. The optional texture request only affects presentation;
 *   it adds no validation rule. All attributes still count toward the caps.
 * - Values of the known attributes decode `&amp; &lt; &gt; &quot; &apos;` and
 *   numeric references (`&#233;`, `&#xE9;`: ≤7 decimal or ≤6 hex digits, with
 *   `;`). A malformed or out-of-range numeric reference refuses the attribute.
 *   Any other `&name;` stays literal text.
 * - Inside the root, comments, `<!…>` / `<?…>`, other tags' attribute values
 *   and raw-text elements (script, style, textarea, title, xmp, iframe,
 *   noembed, noframes, noscript) are skipped the way the HTML tokenizer skips
 *   them, so `"</hc-canvas>"` inside a script string doesn't end the root.
 *   `<plaintext>` never ends, so the root can't close after it. A nested
 *   `<hc-canvas` start tag is refused.
 * - While the fence is open, a last line made only of backticks or tildes,
 *   or empty or only spaces and tabs, is ignored, with the newline before it:
 *   it may be the closing fence still arriving. The size limit applies to
 *   what is left.
 *
 * Pure: no DOM, no store.
 */

import { CANVAS_FENCE, CANVAS_ROOT } from './names';
import { requestedTexture, type CanvasTexture } from './kit/texture';

export { CANVAS_FENCE, CANVAS_ROOT };
export const MAX_BODY_BYTES = 128 * 1024;
export const MAX_ROOT_TAG_BYTES = 4096;
export const MAX_ROOT_ATTRIBUTES = 32;
export const MAX_TITLE_CHARS = 180;
export const MIN_HEIGHT = 80;
export const MAX_HEIGHT = 1600;
/** A canvas id: 1–64 lowercase letters, digits or hyphens (Rust: `canvas_fence`). */
export const CANVAS_ID = /^[a-z0-9-]{1,64}$/;

export type CanvasErrorCode =
  | 'too-large' | 'no-root' | 'bad-tag' | 'duplicate-attribute' | 'bad-id' | 'bad-title'
  | 'bad-ask' | 'bad-height' | 'nested' | 'unclosed' | 'two-roots' | 'after-root';

/** Short, human reasons, shown under a broken canvas. Pinned by the golden. */
export const CANVAS_ERROR_REASONS: Readonly<Record<CanvasErrorCode, string>> = {
  'too-large': 'The canvas is larger than 128 KB.',
  'no-root': 'A canvas must start with <hc-canvas id="…" title="…">.',
  'bad-tag': 'The <hc-canvas> tag is malformed.',
  'duplicate-attribute': 'An <hc-canvas> attribute appears twice.',
  'bad-id': 'The canvas id must be 1–64 lowercase letters, digits or hyphens.',
  'bad-title': 'The canvas needs a title of 1–180 characters of plain text.',
  'bad-ask': 'ask takes no value: write <hc-canvas ask …>.',
  'bad-height': 'height must be whole pixels from 80 to 1600.',
  'nested': 'A canvas cannot contain another <hc-canvas>.',
  'unclosed': 'The canvas ended before </hc-canvas>.',
  'two-roots': 'A fence holds one canvas; this one has two.',
  'after-root': 'Nothing may follow </hc-canvas> in the fence.',
};

export type CanvasRoot = { id: string; title: string; ask: boolean; height: number | null; texture?: CanvasTexture };

/** Display/export only: retain the root's named presentation after stripping
 * the fence wrapper. Legacy root plain now uses default dither. The saved
 * source and answer identities are never rewritten. */
export function canvasFrameBody(p: { root: CanvasRoot; inner: string }): string {
  const texture = requestedTexture(p.root.texture ?? null);
  return `<div data-hc-texture-root texture="${texture === 'plain' ? 'dither' : texture}" style="display:contents">${p.inner}</div>`;
}

export type CanvasParse =
  | { state: 'pending' }
  | { state: 'drawing'; root: CanvasRoot }
  | { state: 'ready'; root: CanvasRoot; inner: string }
  | { state: 'invalid'; code: CanvasErrorCode; reason: string };

const invalid = (code: CanvasErrorCode): CanvasParse => ({ state: 'invalid', code, reason: CANVAS_ERROR_REASONS[code] });

const isWs = (c: string | undefined) => c === ' ' || c === '\t' || c === '\n' || c === '\f' || c === '\r';
const isAlpha = (c: string | undefined) => c !== undefined && ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z'));
const asciiLower = (s: string) => s.replace(/[A-Z]/g, c => String.fromCharCode(c.charCodeAt(0) + 32));

/** UTF-8 length of a JS string (a lone surrogate counts as U+FFFD, 3 bytes, as TextEncoder writes it). */
export function utf8Length(s: string): number {
  let n = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c < 0x80) n += 1;
    else if (c < 0x800) n += 2;
    else if (c >= 0xd800 && c <= 0xdbff && i + 1 < s.length && (s.charCodeAt(i + 1) & 0xfc00) === 0xdc00) { n += 4; i++; }
    else n += 3;
  }
  return n;
}

/** While streaming, drop a last line that may be the closing fence
 *  arriving, with the newline before it: one made only of backticks or
 *  tildes, or still empty or only spaces and tabs (the newline after the
 *  root has arrived, the fence characters not yet). What is left is the body
 *  exactly as it will read once the fence closes, so the size check never
 *  flashes `too-large` for a canvas at the limit (wave 2 C1). */
export function withoutTypingFence(body: string): string {
  const nl = body.lastIndexOf('\n');
  const last = body.slice(nl + 1);
  return /^[ \t]*(?:`+|~+)?$/.test(last) ? body.slice(0, Math.max(nl, 0)) : body;
}

/** Characters a title refuses (control characters other than tab and newline). */
const CONTROL = /[\u0000-\u0008\u000b-\u001f\u007f]/;
/** Whitespace for the "not blank" rule; the same list as canvas_fence.rs. */
const BLANK = /^[\t\n\u000b\f\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000\ufeff]*$/;

const NAMED: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'" };
const NUMERIC = /&#(?:[xX]([0-9a-fA-F]{1,6})|([0-9]{1,7}));/y;
const NAMED_REF = /&(amp|lt|gt|quot|apos);/y;

/** Decode character references in a known attribute's value; null when a numeric one is malformed. */
export function decodeAttribute(v: string): string | null {
  if (!v.includes('&')) return v;
  let out = '';
  let i = 0;
  for (;;) {
    const a = v.indexOf('&', i);
    if (a < 0) return out + v.slice(i);
    out += v.slice(i, a);
    if (v[a + 1] === '#') {
      NUMERIC.lastIndex = a;
      const m = NUMERIC.exec(v);
      if (!m) return null;
      const cp = m[1] !== undefined ? parseInt(m[1], 16) : parseInt(m[2], 10);
      if (cp === 0 || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff)) return null;
      out += String.fromCodePoint(cp);
      i = a + m[0].length;
      continue;
    }
    NAMED_REF.lastIndex = a;
    const m = NAMED_REF.exec(v);
    if (m) { out += NAMED[m[1]]; i = a + m[0].length; continue; }
    out += '&';
    i = a + 1;
  }
}

type TagScan =
  | { kind: 'incomplete' }
  | { kind: 'bad' }
  | { kind: 'ok'; attrs: [string, string][]; end: number };

/** Strict scan of the root start tag's attributes, from just after `<hc-canvas`. */
function scanRootTag(t: string, from: number): TagScan {
  const n = t.length;
  const attrs: [string, string][] = [];
  let k = from;
  for (;;) {
    while (k < n && isWs(t[k])) k++;
    if (k >= n) return { kind: 'incomplete' };
    const ch = t[k];
    if (ch === '>') return { kind: 'ok', attrs, end: k + 1 };
    if (ch === '/' || ch === '=' || ch === '"' || ch === "'" || ch === '<') return { kind: 'bad' };
    const start = k;
    while (k < n && !isWs(t[k]) && t[k] !== '/' && t[k] !== '>' && t[k] !== '=') {
      if (t[k] === '"' || t[k] === "'" || t[k] === '<') return { kind: 'bad' };
      k++;
    }
    if (k >= n) return { kind: 'incomplete' };
    const name = asciiLower(t.slice(start, k));
    let value = '';
    let j = k;
    while (j < n && isWs(t[j])) j++;
    if (j >= n) return { kind: 'incomplete' };
    if (t[j] === '=') {
      j++;
      while (j < n && isWs(t[j])) j++;
      if (j >= n) return { kind: 'incomplete' };
      const q = t[j];
      if (q === '"' || q === "'") {
        const e = t.indexOf(q, j + 1);
        if (e < 0) return { kind: 'incomplete' };
        value = t.slice(j + 1, e);
        k = e + 1;
        if (k < n && !isWs(t[k]) && t[k] !== '>') return { kind: 'bad' };
      } else if (q === '>') {
        return { kind: 'bad' };
      } else {
        const s = j;
        while (j < n && !isWs(t[j]) && t[j] !== '>') {
          if (t[j] === '"' || t[j] === "'" || t[j] === '<' || t[j] === '=' || t[j] === '`') return { kind: 'bad' };
          j++;
        }
        if (j >= n) return { kind: 'incomplete' };
        value = t.slice(s, j);
        k = j;
      }
    }
    attrs.push([name, value]);
    if (attrs.length > MAX_ROOT_ATTRIBUTES) return { kind: 'bad' };
  }
}

function rootFrom(attrs: [string, string][]): CanvasRoot | CanvasErrorCode {
  const seen = new Set<string>();
  for (const [name] of attrs) {
    if (seen.has(name)) return 'duplicate-attribute';
    seen.add(name);
  }
  const get = (name: string) => attrs.find(a => a[0] === name)?.[1];
  const rawId = get('id');
  const id = rawId === undefined ? null : decodeAttribute(rawId);
  if (id === null || !CANVAS_ID.test(id)) return 'bad-id';
  const rawTitle = get('title');
  const title = rawTitle === undefined ? null : decodeAttribute(rawTitle);
  if (title === null || BLANK.test(title) || [...title].length > MAX_TITLE_CHARS || CONTROL.test(title)) return 'bad-title';
  const rawAsk = get('ask');
  let ask = false;
  if (rawAsk !== undefined) {
    const v = decodeAttribute(rawAsk);
    if (v === null || (v !== '' && asciiLower(v) !== 'ask')) return 'bad-ask';
    ask = true;
  }
  const rawHeight = get('height');
  let height: number | null = null;
  if (rawHeight !== undefined) {
    const v = decodeAttribute(rawHeight);
    const m = v === null ? null : /^([0-9]{1,5})(?:px)?$/.exec(v);
    const px = m ? Number(m[1]) : NaN;
    if (!(px >= MIN_HEIGHT && px <= MAX_HEIGHT)) return 'bad-height';
    height = px;
  }
  const texture = requestedTexture(decodeAttribute(get('texture') ?? '') || 'plain');
  return { id, title, ask, height, ...(texture === 'plain' ? {} : { texture }) };
}

/** Read a tag name from `from` (ASCII-lowered) up to whitespace, `/`, `>` or the end. */
function tagName(t: string, from: number): [string, number] {
  let k = from;
  while (k < t.length && !isWs(t[k]) && t[k] !== '/' && t[k] !== '>') k++;
  return [asciiLower(t.slice(from, k)), k];
}

/** Skip the rest of a non-root start tag, honouring quoted attribute values. */
function skipTag(t: string, from: number): number | null {
  const n = t.length;
  let k = from;
  while (k < n) {
    const ch = t[k];
    if (ch === '>') return k + 1;
    k++;
    if (ch === '=') {
      while (k < n && isWs(t[k])) k++;
      if (k < n && (t[k] === '"' || t[k] === "'")) {
        const e = t.indexOf(t[k], k + 1);
        if (e < 0) return null;
        k = e + 1;
      }
    }
  }
  return null;
}

const RAW_TEXT = new Set(['script', 'style', 'textarea', 'title', 'xmp', 'iframe', 'noembed', 'noframes', 'noscript']);

/** Index of the `</name` that ends a raw-text element, or null if it hasn't arrived. */
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

type ContentScan =
  | { kind: 'incomplete' }
  | { kind: 'error'; code: CanvasErrorCode }
  | { kind: 'closed'; innerEnd: number; after: number };

/** Find the root's `</hc-canvas>`, from the end of its start tag. */
function scanContent(t: string, from: number): ContentScan {
  const n = t.length;
  const incomplete: ContentScan = { kind: 'incomplete' };
  let k = from;
  for (;;) {
    const lt = t.indexOf('<', k);
    if (lt < 0 || lt + 1 >= n) return incomplete;
    if (t.startsWith('<!--', lt)) {
      const e = t.indexOf('-->', lt + 4);
      if (e < 0) return incomplete;
      k = e + 3;
      continue;
    }
    const c = t[lt + 1];
    if (c === '!' || c === '?') {
      const e = t.indexOf('>', lt + 2);
      if (e < 0) return incomplete;
      k = e + 1;
      continue;
    }
    if (c === '/') {
      const d = t[lt + 2];
      if (d === undefined) return incomplete;
      if (isAlpha(d)) {
        const [name, p] = tagName(t, lt + 2);
        if (p >= n) return incomplete;
        if (name === CANVAS_ROOT) {
          let q = p;
          while (q < n && isWs(t[q])) q++;
          if (q >= n) return incomplete;
          if (t[q] !== '>') return { kind: 'error', code: 'bad-tag' };
          return { kind: 'closed', innerEnd: lt, after: q + 1 };
        }
      }
      // Any other end tag, `</>` or a bogus comment: skip to its `>`.
      const e = t.indexOf('>', lt + 2);
      if (e < 0) return incomplete;
      k = e + 1;
      continue;
    }
    if (isAlpha(c)) {
      const [name, p] = tagName(t, lt + 1);
      if (p >= n) return incomplete;
      if (name === CANVAS_ROOT) return { kind: 'error', code: 'nested' };
      const end = skipTag(t, p);
      if (end === null) return incomplete;
      if (name === 'plaintext') return incomplete;
      if (RAW_TEXT.has(name)) {
        const close = rawTextEnd(t, end, name);
        if (close === null) return incomplete;
        k = close;
        continue;
      }
      k = end;
      continue;
    }
    k = lt + 1;
  }
}

/**
 * How many start tags of each element a ready canvas's inner HTML has,
 * read the way `scanContent` reads it: comments, `<!…>`/`<?…>`, end tags,
 * attribute values and raw-text contents are skipped, so `"<hc-chart>"` in a
 * script string or a comment is not a chart. Names are ASCII-lowered. A tag
 * cut off at the end, and anything after `<plaintext>`, is not counted.
 * Pure text scanning: it never builds a DOM, so the results card (Canvas
 * C5b) can describe a canvas without putting its markup anywhere.
 */
export function innerElementCounts(inner: string): Map<string, number> {
  const t = inner;
  const n = t.length;
  const counts = new Map<string, number>();
  let k = 0;
  while (k < n) {
    const lt = t.indexOf('<', k);
    if (lt < 0 || lt + 1 >= n) break;
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
    const [name, p] = tagName(t, lt + 1);
    const end = skipTag(t, p);
    if (end === null) break;
    counts.set(name, (counts.get(name) ?? 0) + 1);
    if (name === 'plaintext') break;
    if (RAW_TEXT.has(name)) {
      const close = rawTextEnd(t, end, name);
      if (close === null) break;
      k = close;
      continue;
    }
    k = end;
  }
  return counts;
}

/** Does `t` at `at` start with `<hc-canvas` (ASCII case-insensitive)? `partial` when `t` ends inside it. */
function rootOpenAt(t: string, at: number): 'yes' | 'partial' | 'no' {
  const want = '<' + CANVAS_ROOT;
  for (let i = 0; i < want.length; i++) {
    if (at + i >= t.length) return 'partial';
    if (asciiLower(t[at + i]) !== want[i]) return 'no';
  }
  return 'yes';
}

/**
 * Classify a `hollow-canvas` fence body. `fenceClosed` is whether the closing
 * fence line has arrived; until then the reply is still streaming.
 */
export function parseCanvasFence(body: string, fenceClosed: boolean): CanvasParse {
  // Measured after the typing-fence strip: the closing fence line is not part of the body.
  const t = fenceClosed ? body : withoutTypingFence(body);
  if (utf8Length(t) > MAX_BODY_BYTES) return invalid('too-large');
  const n = t.length;
  let i = 0;
  while (i < n && isWs(t[i])) i++;
  if (i >= n) return fenceClosed ? invalid('no-root') : { state: 'pending' };
  const open = rootOpenAt(t, i);
  if (open === 'no') return invalid('no-root');
  if (open === 'partial') return fenceClosed ? invalid('no-root') : { state: 'pending' };
  const p = i + 1 + CANVAS_ROOT.length;
  if (p >= n) return fenceClosed ? invalid('unclosed') : { state: 'pending' };
  if (t[p] === '/') return invalid('bad-tag');
  if (!isWs(t[p]) && t[p] !== '>') return invalid('no-root');

  const tag = scanRootTag(t, p);
  if (tag.kind === 'bad') return invalid('bad-tag');
  if (utf8Length(t.slice(i, tag.kind === 'ok' ? tag.end : n)) > MAX_ROOT_TAG_BYTES) return invalid('bad-tag');
  if (tag.kind === 'incomplete') return fenceClosed ? invalid('unclosed') : { state: 'pending' };
  const root = rootFrom(tag.attrs);
  if (typeof root === 'string') return invalid(root);

  const content = scanContent(t, tag.end);
  if (content.kind === 'error') return invalid(content.code);
  if (content.kind === 'incomplete') return fenceClosed ? invalid('unclosed') : { state: 'drawing', root };
  let k = content.after;
  while (k < n && isWs(t[k])) k++;
  if (k < n) {
    const again = rootOpenAt(t, k);
    const next = t[k + 1 + CANVAS_ROOT.length];
    const twoRoots = again === 'yes' && (next === undefined || isWs(next) || next === '>' || next === '/');
    return invalid(twoRoots ? 'two-roots' : 'after-root');
  }
  if (!fenceClosed) return { state: 'drawing', root };
  return { state: 'ready', root, inner: t.slice(tag.end, content.innerEnd) };
}
