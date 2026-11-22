/**
 * Chat message segmentation — pure, dependency-free so it can be unit-tested.
 *
 * Lives outside `components/` on purpose: `ChatMessage.tsx` transitively pulls
 * in DOMPurify and `store.ts` (via HtmlArtifact), neither of which survives the
 * node-environment vitest suite, so nothing importable from there can be
 * covered by a test.
 */

import { CANVAS_FENCE } from './canvas/names';

/** The fence language of a whiteboard, removed on 2026-10-07 (diagrams are
 *  `hc-flow` canvases now). An old one in a conversation shows as its JSON
 *  source under a one-line notice; nothing is lost and nothing is drawn. */
export const RETIRED_BOARD_FENCE = 'hollow-whiteboard';
export const RETIRED_BOARD_NOTE = "*Whiteboards were removed from Hollow. This is the board's source.*";

/** The fence language of a question-card request (Paperclip PR 13). */
export const QUESTION_FENCE = 'hollow-questions';
/** The fence language of the suggested next message (src/shipActions.ts). */
export const NEXT_FENCE = 'hollow-next';
/** Fences Hollow reads instead of showing. */
const HIDDEN_FENCES: readonly string[] = [QUESTION_FENCE, NEXT_FENCE];

export type Segment =
  | { kind: 'md'; text: string }
  | { kind: 'code'; lang: string; code: string }
  | { kind: 'html'; lang: string; code: string }
  /** A `hollow-canvas` fence (Canvas C3). `closed`: its closing fence line
   *  has arrived, which `parseCanvasFence` needs to tell drawing from ready. */
  | { kind: 'canvas'; lang: string; code: string; closed: boolean };

/**
 * Split an assistant/system message into ordered segments so each renders
 * with the right surface: prose → MarkdownView, a ```html/```svg fence →
 * HtmlArtifact (sandboxed iframe), an old hollow-whiteboard → a notice and its
 * JSON as a code block,
 * hollow-canvas → a canvas (skeleton while it streams, then the sandboxed
 * frame; `ChatCanvas`), any other ``` fence → CodeBlock (label +
 * copy). Markdown is split only at fences, never at blank lines.
 *
 * A fence has to own its line, but leading whitespace is allowed, so a block
 * indented under a list item is extracted too and splits the list around it.
 * Long-standing behavior, and now pinned by test — the docblock this replaces
 * claimed the opposite.
 */
export function parseSegments(text: string): Segment[] {
  const lines = (text ?? '').split('\n');
  const segs: Segment[] = [];
  let buf: string[] = [];
  const flush = () => {
    if (buf.length) {
      const t = buf.join('\n');
      if (t.trim()) segs.push({ kind: 'md', text: t });
      buf = [];
    }
  };
  let i = 0;
  while (i < lines.length) {
    const open = /^[ \t]*(`{3,}|~{3,})[ \t]*([^`]*?)[ \t]*$/.exec(lines[i]);
    if (open) {
      const fenceChar = open[1][0];
      const fenceLen = open[1].length;
      const info = open[2].trim();
      const lang = info.split(/\s/)[0].toLowerCase();
      // Reserved Hollow fences require a bare language, matching the shared
      // hidden-fence contract. Attributes are ordinary visible source.
      const bare = info.toLowerCase() === lang;
      const closeRe = new RegExp('^[ \\t]*' + (fenceChar === '`' ? '`' : '~') + '{' + fenceLen + ',}[ \\t]*\\r?$');
      const body: string[] = [];
      let j = i + 1;
      let closed = false;
      while (j < lines.length) {
        if (closeRe.test(lines[j])) { closed = true; break; }
        body.push(lines[j]);
        j++;
      }
      flush();
      const code = body.join('\n');
      // Antigravity's question-card request (Paperclip PR 13) and the
      // suggested next message: the card and the composer render them, so
      // the raw block never shows in the transcript. While streaming, an
      // empty open fence at the very end whose language is still being typed
      // ("hollow-ne") is held back too, so it doesn't flash as a code block.
      const typing = !closed && j === i + 1 && lang.startsWith('hollow-') && HIDDEN_FENCES.some(f => f.startsWith(lang));
      if (bare && (HIDDEN_FENCES.includes(lang) || typing)) {
        i = closed ? j + 1 : j;
        continue;
      }
      // A canvas fence whose language is still being typed ("hollow-canv")
      // is a pending canvas, so it never flashes as a code block either.
      const typingCanvas = !closed && j === i + 1 && lang.startsWith('hollow-c') && CANVAS_FENCE.startsWith(lang);
      if (!bare && lang.startsWith('hollow-')) {
        segs.push({ kind: 'code', lang, code });
      } else if (lang === CANVAS_FENCE || typingCanvas) {
        segs.push({ kind: 'canvas', lang: CANVAS_FENCE, code, closed });
      } else if (lang === RETIRED_BOARD_FENCE) {
        segs.push({ kind: 'md', text: RETIRED_BOARD_NOTE }, { kind: 'code', lang: 'json', code });
      } else if (lang === 'html' || lang === 'svg') {
        segs.push({ kind: 'html', lang, code });
      } else {
        segs.push({ kind: 'code', lang, code });
      }
      i = closed ? j + 1 : j;
    } else {
      buf.push(lines[i]);
      i++;
    }
  }
  flush();
  return segs;
}

/** Do two segments carry identical content (and so render identically)? */
export function sameSeg(a: Segment, b: Segment): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === 'md') return b.kind === 'md' && a.text === b.text;
  if (a.kind === 'canvas' && b.kind === 'canvas' && a.closed !== b.closed) return false;
  return b.kind !== 'md' && a.lang === b.lang && a.code === b.code;
}

/**
 * Reference-stabilize a freshly parsed segment list against the previous one.
 *
 * Streaming text only ever grows at the tail, so every leading segment whose
 * content is byte-identical to last time is settled — it will not change again
 * this turn. Putting the previous object back into that slot lets Solid's
 * `<For>` (which keys by reference) keep those rows mounted instead of
 * disposing and rebuilding them, which is what stops the whole answer being
 * re-run through marked + DOMPurify, and the code highlighter, on every
 * streaming flush. Only the growing tail is rebuilt.
 *
 * Stops at the first difference deliberately: past that point indices no
 * longer line up (an arriving fence restructures the tail), and reusing across
 * a mismatch would pin stale text on screen. Mutates and returns `next`. Index
 * i only ever receives `prev[i]`, so the result can never hold a duplicate
 * reference — `<For>` requires its items to be unique.
 */
export function reuseSettledSegments(prev: readonly Segment[], next: Segment[]): Segment[] {
  const shared = Math.min(prev.length, next.length);
  for (let i = 0; i < shared; i++) {
    if (!sameSeg(prev[i], next[i])) break;
    next[i] = prev[i];
  }
  return next;
}

/**
 * The settle run's counterpart of `reuseSettledSegments`: when a turn
 * finishes every row is rebuilt once (so an html fence upgrades from source
 * to its live frame), except a finished canvas, whose row renders the same
 * whether or not the turn is streaming. Keeping that object keeps its frame
 * mounted instead of reloading it as the turn ends. Same-index only, so the
 * result never holds a duplicate reference. Mutates and returns `next`.
 */
export function reuseFinishedCanvases(prev: readonly Segment[], next: Segment[]): Segment[] {
  const shared = Math.min(prev.length, next.length);
  for (let i = 0; i < shared; i++) {
    const a = prev[i];
    if (a.kind === 'canvas' && a.closed && sameSeg(a, next[i])) next[i] = a;
  }
  return next;
}

/** The lines inside each suggested-next-message block of `text`, and the
 *  text without those blocks (closed, or left open at the end). */
export function splitNextFence(text: string): { text: string; blocks: string[][] } {
  return splitHiddenFence(text, NEXT_FENCE);
}

/** The lines inside each `lang` block of `text`, and the text without those
 *  blocks (closed, or left open at the end): the hidden-block convention
 *  `hollow-next` set (an assistant's suggestion) and an ask-canvas's answer
 *  (`hollow-canvas-answer`, in a user message; `canvas/askAnswer.ts`) share.
 *  Rust strips both from vault session records and the learning job's input
 *  (`next_step::strip`), by the same fence rules: both pass
 *  `canvas/hiddenFences.golden.json` (Canvas C6 P1). */
export function splitHiddenFence(text: string, lang: string): { text: string; blocks: string[][] } {
  if (!text.includes(lang)) return { text, blocks: [] };
  const lines = text.split('\n');
  const kept: string[] = [];
  const blocks: string[][] = [];
  for (let i = 0; i < lines.length; i++) {
    const open = /^[ \t]*(`{3,}|~{3,})[ \t]*([^`]*?)[ \t]*$/.exec(lines[i]);
    if (!open) { kept.push(lines[i]); continue; }
    const close = new RegExp('^[ \\t]*' + (open[1][0] === '`' ? '`' : '~') + '{' + open[1].length + ',}[ \\t]*\\r?$');
    const body: string[] = [];
    const start = i;
    while (i + 1 < lines.length && !close.test(lines[i + 1])) body.push(lines[++i]);
    if (i + 1 < lines.length) i++; // the closing fence
    // Any other fence is kept whole, so one that quotes a hidden block stays.
    if (open[2].trim().toLowerCase() === lang) blocks.push(body);
    else kept.push(...lines.slice(start, i + 1));
  }
  return { text: kept.join('\n').trimEnd(), blocks };
}
