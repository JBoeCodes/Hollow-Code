/**
 * Line-level Markdown reading shared by the workspace section models
 * (Canvas C5a: `briefModel.ts` `readBrief`, `planModel.ts`, `overviewModel.ts`).
 * Pure: no DOM, no Tauri, so vitest runs it in node.
 *
 * Everything here keeps source line numbers (1-based, the way an editor and
 * Rust's `str::lines()` count them: split on `\n`, one trailing `\r` dropped),
 * so a model can always point back at the exact text it came from and a
 * section that isn't understood can be shown as the Markdown it is.
 */

export interface SourceLine {
  /** 1-based line number. */
  n: number;
  /** The line without its line ending. */
  text: string;
  /** Inside a fenced code block, delimiters included. */
  fenced: boolean;
}

export interface MdSection {
  /** `#`/`##` heading text, or null for the text before the first heading. */
  heading: string | null;
  /** 0 before any heading, otherwise the heading's level (1 or 2). */
  level: number;
  /** The heading's line, or the first body line before any heading. */
  line: number;
  /** The last line of the section (heading included); blank lines at the end are not counted. */
  endLine: number;
  /** Lines after the heading, up to the next `#`/`##` heading. */
  body: SourceLine[];
}

export interface MdDocument {
  lines: SourceLine[];
  /** YAML frontmatter between `---` lines at the very top, without the delimiters. */
  frontmatter: { text: string; line: number; endLine: number } | null;
  sections: MdSection[];
}

/** A run of non-blank lines (fenced blocks stay whole, blank lines included). */
export interface MdBlock { line: number; endLine: number; lines: SourceLine[]; text: string }

const FENCE_OPEN = /^\s*(`{3,}|~{3,})(.*)$/;
const ATX = /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$/;

/** Split into lines, marking fenced code (CommonMark: a fence closes on the same character, at least as long). */
export function sourceLines(text: string): SourceLine[] {
  const raw = text.split('\n');
  const out: SourceLine[] = [];
  let fence: { ch: string; len: number } | null = null;
  for (let i = 0; i < raw.length; i++) {
    let t = raw[i];
    if (t.endsWith('\r')) t = t.slice(0, -1);
    if (i === 0 && t.startsWith('﻿')) t = t.slice(1);
    if (fence) {
      const close = /^\s*(`{3,}|~{3,})\s*$/.exec(t);
      if (close && close[1][0] === fence.ch && close[1].length >= fence.len) fence = null;
      out.push({ n: i + 1, text: t, fenced: true });
      continue;
    }
    const open = FENCE_OPEN.exec(t);
    if (open && !(open[1][0] === '`' && open[2].includes('`'))) {
      fence = { ch: open[1][0], len: open[1].length };
      out.push({ n: i + 1, text: t, fenced: true });
      continue;
    }
    out.push({ n: i + 1, text: t, fenced: false });
  }
  // A final empty string after a trailing newline is not a line.
  if (out.length > 1 && out[out.length - 1].text === '' && text.endsWith('\n')) out.pop();
  if (out.length === 1 && out[0].text === '') return [];
  return out;
}

export const isBlank = (l: SourceLine) => l.text.trim() === '';
export const joinLines = (lines: SourceLine[]) => lines.map(l => l.text).join('\n');

/** An ATX heading on a line outside code, as `[level, text]`. */
export function headingOf(l: SourceLine): [number, string] | null {
  if (l.fenced) return null;
  const m = ATX.exec(l.text);
  return m ? [m[1].length, (m[2] ?? '').trim()] : null;
}

/** The document split at `#` and `##` headings, frontmatter apart. */
export function readSections(text: string): MdDocument {
  const lines = sourceLines(text);
  let start = 0;
  let frontmatter: MdDocument['frontmatter'] = null;
  if (lines.length && lines[0].text === '---') {
    const end = lines.findIndex((l, i) => i > 0 && (l.text === '---' || l.text === '...'));
    if (end > 0) {
      frontmatter = { text: joinLines(lines.slice(1, end)), line: 1, endLine: end + 1 };
      start = end + 1;
    }
  }
  const sections: MdSection[] = [];
  let cur: MdSection | null = null;
  const close = () => {
    if (!cur) return;
    const last = [...cur.body].reverse().find(l => !isBlank(l));
    cur.endLine = last ? last.n : cur.line;
    if (cur.heading !== null || last) sections.push(cur);
  };
  for (let i = start; i < lines.length; i++) {
    const l = lines[i];
    const h = headingOf(l);
    if (h && h[0] <= 2) {
      close();
      cur = { heading: h[1], level: h[0], line: l.n, endLine: l.n, body: [] };
      continue;
    }
    if (!cur) {
      if (isBlank(l)) continue;
      cur = { heading: null, level: 0, line: l.n, endLine: l.n, body: [] };
    }
    cur.body.push(l);
  }
  close();
  return { lines, frontmatter, sections };
}

/** Blank-line separated blocks; a fenced block is never split. */
export function blocksOf(lines: SourceLine[]): MdBlock[] {
  const out: MdBlock[] = [];
  let cur: SourceLine[] = [];
  const flush = () => {
    while (cur.length && isBlank(cur[cur.length - 1])) cur.pop();
    if (cur.length) out.push({ line: cur[0].n, endLine: cur[cur.length - 1].n, lines: cur, text: joinLines(cur) });
    cur = [];
  };
  for (const l of lines) {
    if (!l.fenced && isBlank(l)) { flush(); continue; }
    cur.push(l);
  }
  flush();
  return out;
}

export interface MdLink { text: string; href: string }
/** Inline `[text](href)` links, images excluded, in order. */
export function linksOf(markdown: string): MdLink[] {
  const out: MdLink[] = [];
  const re = /(!?)\[([^\]\n]*)\]\(\s*(?:<([^>\n]*)>|([^)\s]+))(?:\s+"[^"]*")?\s*\)/g;
  for (let m = re.exec(markdown); m; m = re.exec(markdown)) if (!m[1]) out.push({ text: m[2], href: m[3] ?? m[4] });
  return out;
}

/** Markdown reduced to readable text for a headline: links to their text, emphasis and code marks dropped, whitespace collapsed. */
export function plainText(markdown: string): string {
  return markdown
    .replace(/!\[([^\]\n]*)\]\([^)\n]*\)/g, '$1')
    .replace(/\[([^\]\n]*)\]\([^)\n]*\)/g, '$1')
    .replace(/`([^`\n]*)`/g, '$1')
    .replace(/(\*\*|__)(?=\S)([\s\S]*?\S)\1/g, '$2')
    .replace(/(^|[\s(])[*_](?=\S)([^*_\n]*?\S)[*_](?=[\s).,;:!?]|$)/g, '$1$2')
    .replace(/\s+/g, ' ')
    .trim();
}

/** The first sentence of plain text (up to `.`, `!` or `?` followed by a space and a capital, digit or quote). */
export function firstSentence(text: string): string {
  const m = /^(.+?[.!?])(?=\s+["“'(\[A-Z0-9])/.exec(text);
  return (m ? m[1] : text).trim();
}

export interface ListItemLine { indent: number; marker: string; rest: string }
const LIST_ITEM = /^([ \t]*)([-*+]|\d{1,9}[.)])(?:[ \t]+(.*)|[ \t]*)$/;
/** A list item's indent (tabs count four), marker and text, or null. Thematic breaks (`***`, `- - -`) are not items. */
export function listItemOf(l: SourceLine): ListItemLine | null {
  if (l.fenced) return null;
  if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(l.text)) return null;
  const m = LIST_ITEM.exec(l.text);
  if (!m) return null;
  return { indent: m[1].replace(/\t/g, '    ').length, marker: m[2], rest: m[3] ?? '' };
}

export const indentOf = (l: SourceLine) => (/^[ \t]*/.exec(l.text)![0]).replace(/\t/g, '    ').length;

// ── SHA-256 (sync, pure; matches Rust's `sha2` on the UTF-8 bytes) ──

const K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

/** Lowercase hex SHA-256 of a string's UTF-8 bytes. */
export function sha256Hex(text: string): string {
  const data = new TextEncoder().encode(text);
  const bitLen = data.length * 8;
  const padded = new Uint8Array(((data.length + 9 + 63) >> 6) << 6);
  padded.set(data);
  padded[data.length] = 0x80;
  const view = new DataView(padded.buffer);
  view.setUint32(padded.length - 8, Math.floor(bitLen / 0x100000000));
  view.setUint32(padded.length - 4, bitLen >>> 0);
  const h = new Uint32Array([0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]);
  const w = new Uint32Array(64);
  const rot = (x: number, n: number) => (x >>> n) | (x << (32 - n));
  for (let off = 0; off < padded.length; off += 64) {
    for (let i = 0; i < 16; i++) w[i] = view.getUint32(off + i * 4);
    for (let i = 16; i < 64; i++) {
      const s0 = rot(w[i - 15], 7) ^ rot(w[i - 15], 18) ^ (w[i - 15] >>> 3);
      const s1 = rot(w[i - 2], 17) ^ rot(w[i - 2], 19) ^ (w[i - 2] >>> 10);
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) >>> 0;
    }
    let [a, b, c, d, e, f, g, hh] = h;
    for (let i = 0; i < 64; i++) {
      const t1 = (hh + (rot(e, 6) ^ rot(e, 11) ^ rot(e, 25)) + ((e & f) ^ (~e & g)) + K[i] + w[i]) >>> 0;
      const t2 = ((rot(a, 2) ^ rot(a, 13) ^ rot(a, 22)) + ((a & b) ^ (a & c) ^ (b & c))) >>> 0;
      hh = g; g = f; f = e; e = (d + t1) >>> 0; d = c; c = b; b = a; a = (t1 + t2) >>> 0;
    }
    h[0] += a; h[1] += b; h[2] += c; h[3] += d; h[4] += e; h[5] += f; h[6] += g; h[7] += hh;
  }
  return Array.from(h, x => x.toString(16).padStart(8, '0')).join('');
}
