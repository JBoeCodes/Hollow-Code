import { createEffect, createMemo, onCleanup } from 'solid-js';
import { marked } from 'marked';
import DOMPurify from 'dompurify';
import { blockRemoteImages } from '../remoteImages';
import { markdownOptions } from '../markdownOptions';
import { handleRemoteImageClick, loadedRemoteImage, remoteImagesVersion } from '../remoteImageLoader';

// Defense in depth: even though chat output comes from our own model, skill
// markdown, log_activity entries, and other "trusted" sources can include
// user-controlled fragments (email bodies, tool output). Sanitize before
// injecting via innerHTML so an inline <script> or `javascript:` href can't
// fire on render. `<style>` is dropped too: it would restyle the whole app,
// and its `url()`s are a remote-image request no attribute rewrite can see.
//
// Remote images then become click-to-load placeholders (merged plan slice 1,
// review A·S3): an `![](https://…?q=SECRET)` in a reply must not fire a
// request just by rendering. See remoteImages.ts.
//
// `forbidMedia` (chat replies, inline video 2026-10-17) also drops `<video>`,
// `<audio>`, `<source>` and `<track>`: the CSP leaves an agent's raw
// `<video src="/abs/x.mp4">` a dead black box, and the reply's video strip
// (`ReplyMedia`) shows the playable copy instead.
//
// `breaks={false}` (a whole file: the preview window, Skills) renders a soft
// line break as the Library reader does (P1-18); messages keep them.
//
// `apply` puts the HTML in the view itself instead of a whole `innerHTML`
// swap (a streaming reply keeps its settled blocks mounted, overnight 11-07
// D1 gap 8: `ReplyMarkdown`'s `keepBlocks`).
// Internal UI classes that Hollow's own post-processors attach after
// sanitization (`ReplyMarkdown`, `blockRemoteImages`, `fileChip`). Untrusted
// HTML/Markdown must never smuggle them past DOMPurify.
export const INTERNAL_UI_CLASSES = new Set([
  'file-ref',
  'file-chip',
  'file-chip-icon',
  'file-chip-name',
  'file-chip-lines',
  'file-chip-folder',
  'md-remote-img',
  'md-remote-img-label',
  'md-remote-img-alt',
  'reply-local-image',
  'reply-table',
  'reply-table-bar',
  'reply-table-copy',
  'reply-table-scroll',
  'reply-thread-chip',
]);

const RAW_TEXT_TAGS = new Set(['style', 'script', 'textarea', 'title', 'xmp', 'iframe', 'noembed', 'noframes', 'noscript']);
const isAsciiWs = (c: string | undefined) => c === ' ' || c === '\t' || c === '\n' || c === '\r' || c === '\f';
const escHtmlAttr = (s: string) => s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

function decodeClassEntities(s: string): string {
  return s.replace(/&(#[xX][0-9a-fA-F]+|#[0-9]+|amp|lt|gt|quot|apos|nbsp);?/gi, (m, e: string) => {
    if (e[0] === '#') {
      const code = e[1] === 'x' || e[1] === 'X' ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10);
      return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : m;
    }
    switch (e.toLowerCase()) {
      case 'amp': return '&';
      case 'lt': return '<';
      case 'gt': return '>';
      case 'quot': return '"';
      case 'apos': return "'";
      case 'nbsp': return '\u00a0';
      default: return m;
    }
  });
}

function stripClassAttrInTag(tagText: string): string | null {
  let i = 1;
  const n = tagText.length;
  while (i < n && !isAsciiWs(tagText[i]) && tagText[i] !== '/' && tagText[i] !== '>') i++;
  let out = '';
  let last = 0;
  let changed = false;
  while (i < n) {
    const wsStart = i;
    while (i < n && (isAsciiWs(tagText[i]) || tagText[i] === '/')) i++;
    if (i >= n || tagText[i] === '>') break;
    const nameStart = i++;
    while (i < n && !isAsciiWs(tagText[i]) && tagText[i] !== '/' && tagText[i] !== '>' && tagText[i] !== '=') i++;
    const attrName = tagText.slice(nameStart, i).toLowerCase();
    while (i < n && isAsciiWs(tagText[i])) i++;
    let value = '';
    if (tagText[i] === '=') {
      i++;
      while (i < n && isAsciiWs(tagText[i])) i++;
      const q = tagText[i];
      if (q === '"' || q === "'") {
        const close = tagText.indexOf(q, i + 1);
        if (close < 0) break;
        value = tagText.slice(i + 1, close);
        i = close + 1;
      } else {
        const vs = i;
        while (i < n && !isAsciiWs(tagText[i]) && tagText[i] !== '>') i++;
        value = tagText.slice(vs, i);
      }
    }
    if (attrName === 'class') {
      const tokens = decodeClassEntities(value).split(/\s+/).filter(Boolean);
      const kept = tokens.filter(tok => !INTERNAL_UI_CLASSES.has(tok.toLowerCase()));
      if (kept.length !== tokens.length) {
        changed = true;
        out += tagText.slice(last, wsStart);
        if (kept.length > 0) out += ` class="${escHtmlAttr(kept.join(' '))}"`;
        else if (i < n && !isAsciiWs(tagText[i]) && tagText[i] !== '>' && tagText[i] !== '/') out += ' ';
        last = i;
      }
    }
  }
  return changed ? out + tagText.slice(last) : null;
}

/** Remove Hollow-internal UI class tokens from sanitized HTML before host
 *  post-processors attach legitimate chips and image buttons. */
export function stripInternalUiClasses(html: string): string {
  let out = '';
  let last = 0;
  let i = 0;
  const n = html.length;
  while (i < n) {
    const lt = html.indexOf('<', i);
    if (lt < 0) break;
    if (html.startsWith('<!--', lt)) {
      const end = html.indexOf('-->', lt + 4);
      i = end < 0 ? n : end + 3;
      continue;
    }
    const next = html[lt + 1];
    if (!next || !/[a-zA-Z]/.test(next)) {
      const end = html.indexOf('>', lt + 1);
      i = end < 0 ? n : end + 1;
      continue;
    }
    // Find tag end while respecting quoted attribute values.
    let j = lt + 1;
    const nameStart = j;
    while (j < n && !isAsciiWs(html[j]) && html[j] !== '/' && html[j] !== '>') j++;
    const tagName = html.slice(nameStart, j).toLowerCase();
    let quote = '';
    while (j < n) {
      const c = html[j++];
      if (quote) { if (c === quote) quote = ''; }
      else if (c === '"' || c === "'") quote = c;
      else if (c === '>') break;
    }
    const rewritten = stripClassAttrInTag(html.slice(lt, j));
    if (rewritten !== null) {
      out += html.slice(last, lt) + rewritten;
      last = j;
    }
    i = j;
    if (tagName === 'plaintext') break;
    if (RAW_TEXT_TAGS.has(tagName)) {
      const lower = html.toLowerCase();
      let k = i;
      for (;;) {
        const at = lower.indexOf(`</${tagName}`, k);
        if (at < 0) { i = n; break; }
        const c = html[at + 2 + tagName.length];
        if (c === undefined || isAsciiWs(c) || c === '/' || c === '>') { i = at; break; }
        k = at + 1;
      }
    }
  }
  return out + html.slice(last);
}

export function MarkdownView(props: { text: string; class?: string; parse?: (text: string, breaks: boolean) => string; decorate?: (root: HTMLElement) => void | (() => void); transform?: (html: string) => string; apply?: (root: HTMLElement, html: string) => void; onLinkClick?: (e: MouseEvent | KeyboardEvent) => void; replyFiles?: boolean; forbidMedia?: boolean; breaks?: boolean }) {
  const sanitized = createMemo(() => {
    const t = props.text ?? '';
    if (!t) return '';
    const raw = props.parse ? props.parse(t, props.breaks ?? true) : marked.parse(t, markdownOptions(props.breaks ?? true)) as string;
    const forbid = props.forbidMedia ? ['style', 'video', 'audio', 'source', 'track'] : ['style'];
    const clean = DOMPurify.sanitize(raw, { USE_PROFILES: { html: true }, FORBID_TAGS: forbid,
      ALLOW_DATA_ATTR: false,
      // Native MathML only, without SVG, mglyph, embedding or network tags.
      ADD_TAGS: props.parse ? ['math', 'semantics', 'mrow', 'mi', 'mo', 'mn', 'mtext', 'mspace', 'mfrac', 'msqrt', 'mroot', 'msub', 'msup', 'msubsup', 'munder', 'mover', 'munderover', 'mtable', 'mtr', 'mtd', 'menclose', 'mpadded', 'mstyle', 'annotation'] : [],
      ADD_ATTR: props.parse ? ['mathvariant', 'stretchy', 'fence', 'separator', 'lspace', 'rspace', 'accent', 'accentunder', 'columnalign', 'rowspacing', 'columnspacing', 'displaystyle', 'scriptlevel', 'encoding', 'display'] : [],
      // Reply links are intercepted before navigation; this retains file:
      // and unfamiliar schemes so the host can resolve or explain them.
      // DOMPurify still refuses script/data links and executable markup.
      ALLOW_UNKNOWN_PROTOCOLS: !!props.replyFiles });
    return stripInternalUiClasses(clean);
  });
  // Separate memo, so a finished image load re-runs only the cheap rewrite;
  // views whose output doesn't change keep their DOM (string equality).
  const html = createMemo(() => {
    remoteImagesVersion();
    const clean = sanitized();
    return blockRemoteImages(props.transform ? props.transform(clean) : clean, loadedRemoteImage);
  });
  let root: HTMLDivElement | undefined;
  // Runs after each innerHTML swap, so a host can mark up the rendered DOM
  // (chat marks the file paths it can open).
  createEffect(() => {
    const h = html();
    if (root && props.apply) props.apply(root, h);
    if (root && props.decorate) { const dispose = props.decorate(root); if (dispose) onCleanup(dispose); }
  });
  return <div ref={el => (root = el)} class={`md-view ${props.class ?? ''}`}
    onClick={e => { if (!handleRemoteImageClick(e)) props.onLinkClick?.(e); }}
    onAuxClick={e => { if (e.button === 1) props.onLinkClick?.(e); }}
    onKeyDown={e => { if ((e.key === 'Enter' || e.key === ' ') && (e.target as Element).matches?.('.file-ref')) props.onLinkClick?.(e); }}
    innerHTML={props.apply ? '' : html()} />;
}
