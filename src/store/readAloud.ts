// Read aloud on a chat reply (overnight 11-07, D1): the reply row's Read aloud button
// reads it through the system voice (`speechSynthesis`, which WKWebView has),
// and says Stop reading while it does. One reply at a time; the text is the
// reply's Markdown made speakable (no symbols, a code block is skipped and
// said so), cut into sentences so Stop answers at once and a long reply never
// hits a voice's length limit.
import { createSignal } from 'solid-js';
import { flashFooter } from './snackbars';

/** Longest piece spoken at once, in characters. */
const CHUNK = 240;

/** What is read for `md`: the Markdown without its markup. Headings, list
 *  items and table rows end like sentences so the voice pauses; a fenced
 *  code block becomes "Code block skipped."; links read as their text and a
 *  bare address as "link". */
export function speakableText(md: string): string {
  const out: string[] = [];
  let fence: string | null = null;
  let fenceEnds = 0; // a fence under a list item ends at a line indented less than the item's text
  let inList = false; // under a list item, where a fence may be indented four or more spaces
  let itemText = 0; // the column where the latest list item's text starts
  for (const raw of md.replace(/\r\n?/g, '\n').split('\n')) {
    const indent = /^[ \t]*/.exec(raw)![0].replace(/\t/g, '    ').length;
    // A backtick fence's info string holds no backtick ("```inline```" is
    // inline code, not a fence).
    const open = /^[ \t]*(?:(`{3,})[^`]*|(~{3,}).*)$/.exec(raw);
    const marker = open ? (open[1] ?? open[2]) : undefined;
    if (fence) {
      // A fence closes on a line of its own marker (or longer) and nothing else.
      if (marker && marker[0] === fence[0] && marker.length >= fence.length && /^[ \t]*[`~]+\s*$/.test(raw)) { fence = null; continue; }
      // One under a list item ends with the item: a line indented less than the item's text.
      if (fenceEnds && raw.trim() && indent < fenceEnds) fence = null;
      else continue;
    }
    // Indented four or more spaces, a fence belongs to a list item; elsewhere that is an indented code block.
    if (marker && (indent <= 3 || inList)) { fence = marker; fenceEnds = inList && indent >= itemText ? itemText : 0; out.push('Code block skipped.'); continue; }
    const item = /^[ \t]*(?:[-*+]|\d{1,3}\.|\d{1,9}\))\s+/.exec(raw);
    if (item) { inList = true; itemText = item[0].replace(/\t/g, '    ').length; }
    else if (raw.trim() && indent === 0) inList = false;
    let line = raw.trim();
    if (!line || /^([-*_])(\s*\1){2,}$/.test(line)) { out.push(''); continue; }
    if (/^\|?(\s*:?-{2,}:?\s*\|?)+$/.test(line)) continue; // a table's rule row
    // A heading, list item or table row stands alone, so it ends like a
    // sentence; a paragraph's line does not (a hard-wrapped one runs on).
    // A number with a full stop has at most three digits to be a list item
    // ("2024. Revenue grew." is a sentence); "1000)" is a list item.
    const stands = /^(#{1,6}\s|([-*+]|\d{1,3}\.|\d{1,9}\))\s|\|.*\|$)/.test(line);
    line = line.replace(/^#{1,6}\s+/, '').replace(/^>\s?/, '').replace(/^([-*+]|\d{1,3}\.|\d{1,9}\))\s+/, '').replace(/^\[[ xX]\]\s+/, '');
    if (/^\|.*\|$/.test(line)) line = line.slice(1, -1).split('|').map(c => c.trim()).filter(Boolean).join(', ');
    // Inline code is set aside first, so no rewrite below touches it.
    const code: string[] = [];
    line = line.replace(/(`+)([^`](?:.*?[^`])?)\1(?!`)/g, (_m, _t, c: string) => `\u0002${code.push(c.trim()) - 1}\u0003`);
    line = line
      .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
      .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
      .replace(/<(https?:\/\/[^>]+)>/g, 'link')
      .replace(/<\/?[a-zA-Z][^>]*>/g, '')
      // The punctuation that ends a sentence after an address stays.
      .replace(/https?:\/\/\S+/g, m => `link${/[.,;:!?)\]'"”’]+$/.exec(m)?.[0] ?? ''}`)
      .replace(/(\*\*|__|~~)(.+?)\1/g, '$2')
      // Emphasis hugs its words: "2 * 3 * 4" is arithmetic.
      .replace(/(^|[\s(])[*_](?=\S)([^*_\n]*[^*_\s])[*_](?=[\s).,;:!?]|$)/g, '$1$2')
      .replace(/\\([\\`*_{}[\]()#+\-.!|>~])/g, '$1')
      .replace(/\u0002(\d+)\u0003/g, (_m, i: string) => code[Number(i)])
      .replace(/\s+/g, ' ')
      .trim();
    if (!line) continue;
    out.push(stands && !/[.!?…:;]["')\]”’]*$/.test(line) ? `${line}.` : line);
  }
  return out.join('\n').replace(/\n{2,}/g, '\n\n').trim();
}

/** `text` in spoken pieces of at most `max` characters, cut after a
 *  sentence, else a comma, else a space (a word is never split). */
export function speechChunks(text: string, max = CHUNK): string[] {
  const pieces: string[] = [];
  for (const line of text.split('\n')) {
    for (const s of line.replace(/([.!?…]+["')\]”’]*)\s+/g, '$1\u0001').split('\u0001')) {
      let rest = s.trim();
      while (rest.length > max) {
        const w = rest.slice(0, max + 1);
        // After a comma, semicolon or colon; else at a space; else (one
        // endless token) at the limit.
        let at = Math.max(w.lastIndexOf(', '), w.lastIndexOf('; '), w.lastIndexOf(': ')) + 1;
        if (at < max / 3) at = w.lastIndexOf(' ');
        if (at < max / 3) at = max;
        pieces.push(rest.slice(0, at).trim());
        rest = rest.slice(at).trim();
      }
      if (rest) pieces.push(rest);
    }
  }
  // Short neighbours share one utterance, so a list is not a string of stops.
  const chunks: string[] = [];
  for (const p of pieces) {
    const last = chunks.length - 1;
    if (last >= 0 && chunks[last].length + 1 + p.length <= max) chunks[last] += ` ${p}`;
    else chunks.push(p);
  }
  return chunks;
}

const [reading, setReading] = createSignal<string | null>(null);
/** The key of the reply being read, or null. */
export const readingKey = reading;
let run = 0;

/** The system voice is there (a webview without it gets no menu item). */
export function readAloudSupported(): boolean {
  return typeof window !== 'undefined' && 'speechSynthesis' in window && typeof SpeechSynthesisUtterance === 'function';
}

let watch: ReturnType<typeof setInterval> | undefined;
function endWatch() { if (watch !== undefined) { clearInterval(watch); watch = undefined; } }

/** The view that started the read, as the opaque token it passed to `readAloud`. */
let owner: object | undefined;

/** Stop whatever is being read. With `by`, only when that view started it: a
 *  view going away (the quick-chat popover closing, a page staying mounted
 *  behind another) must not cut off a read another view started. */
export function stopReading(by?: object) {
  if (reading() == null) return;
  if (by !== undefined && owner !== by) return;
  run++;
  endWatch();
  setReading(null);
  try { window.speechSynthesis.cancel(); } catch { /* nothing was speaking */ }
}

/** Read `markdown` aloud as `key` (a reply), replacing any earlier read; `by`
 *  names the view that asked (see `stopReading`).
 *  False when there was nothing to say or no voice. */
export function readAloud(key: string, markdown: string, by?: object): boolean {
  if (!readAloudSupported()) return false;
  const chunks = speechChunks(speakableText(markdown));
  if (!chunks.length) { flashFooter('Nothing to read in that reply', 2000); return false; }
  const synth = window.speechSynthesis;
  const mine = ++run;
  endWatch();
  // WebKit speaks what is queued right after a cancel() but never says it
  // ended (WebKit 238189), so a new read waits a moment after a cancel, and
  // the end is also read from the voice itself (below).
  const cancelling = !!(synth.speaking || synth.pending);
  if (cancelling) synth.cancel();
  owner = by;
  setReading(key);
  const done = () => { if (mine === run) { endWatch(); setReading(null); } };
  const begin = () => {
    if (mine !== run) return; // stopped, or another reply took over, meanwhile
    chunks.forEach((text, i) => {
      const u = new SpeechSynthesisUtterance(text);
      u.lang = navigator.language || 'en-US';
      if (i === chunks.length - 1) u.onend = done;
      u.onerror = e => {
        // A cancel (Stop, or the next read) is not a failure.
        if (mine !== run || e.error === 'canceled' || e.error === 'interrupted') return;
        done();
        synth.cancel();
        flashFooter('× Couldn’t read it aloud', 3000);
      };
      synth.speak(u);
    });
    // The voice gone quiet for three looks in a row (1.2 s) is the end, event or not.
    let idle = 0;
    watch = setInterval(() => {
      if (mine !== run) { endWatch(); return; }
      idle = synth.speaking || synth.pending ? 0 : idle + 1;
      if (idle >= 3) done();
    }, 400);
  };
  if (cancelling) setTimeout(begin, 80); else begin();
  return true;
}
