/**
 * Things the agent shows (```html / ```svg previews, canvases), listed in the
 * side panel's Visuals tab. They render inline everywhere (Canvas C3, wave 2
 * C1) and open there on Expand. Whiteboards were removed on 2026-10-07: an
 * old board fence is a notice and a code block now, not a visual. Pure and
 * dependency-light so the node vitest suite can load it.
 */
import { parseSegments, type Segment } from './chatSegments';

export type ChatVisualKind = 'html' | 'canvas';

export interface ChatVisual {
  /** Stable across reloads: kind + a hash of the fenced source. */
  key: string;
  kind: ChatVisualKind;
  lang: string;
  code: string;
  title: string;
}

/** The turn fields this module reads (a structural subset of ChatTurn). */
export interface VisualTurn {
  role: string;
  text: string;
  partial?: boolean;
}

function hash(text: string): string {
  // djb2, enough to tell two fences in one conversation apart.
  let h = 5381;
  for (let i = 0; i < text.length; i++) h = ((h << 5) + h + text.charCodeAt(i)) | 0;
  return (h >>> 0).toString(36);
}

function decode(text: string): string {
  return text.replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/\s+/g, ' ').trim();
}

export function htmlVisualTitle(code: string, lang: string): string {
  const title = /<title[^>]*>([^<]{1,160})<\/title>/i.exec(code)?.[1]
    ?? /<h1[^>]*>([^<]{1,160})<\/h1>/i.exec(code)?.[1];
  const clean = title ? decode(title) : '';
  if (clean) return clean.slice(0, 120);
  return lang.toLowerCase() === 'svg' ? 'SVG image' : 'HTML preview';
}

export const CANVAS_HEAD = /^[\t\n\f\r ]*<hc-canvas([\t\n\f\r ][^>]*)?>/i;
export const CANVAS_END = /<\/hc-canvas[\t\n\f\r ]*>[\t\n\f\r ]*$/i;
export const rootAttr = (head: string, name: string) =>
  new RegExp(`[\\t\\n\\f\\r ]${name}[\\t\\n\\f\\r ]*=[\\t\\n\\f\\r ]*(?:"([^"]*)"|'([^']*)'|([^\\s>]+))`, 'i').exec(head);

/** A finished canvas's title, by a light look at its root: an id in the
 *  canvas id rule, a title, and the end tag last. Cheap on purpose, so the
 *  App chunk needn't carry the full parser (`canvas/fence.ts`); the canvas
 *  itself re-validates when it renders, and a broken one shows in place. */
export function finishedCanvasTitle(code: string): string | null {
  const head = CANVAS_HEAD.exec(code)?.[1];
  if (head === undefined || !CANVAS_END.test(code)) return null;
  const pick = (m: RegExpExecArray | null) => (m ? (m[1] ?? m[2] ?? m[3] ?? '') : null);
  const id = pick(rootAttr(head, 'id'));
  const title = pick(rootAttr(head, 'title'));
  if (!id || !/^[a-z0-9-]{1,64}$/.test(id) || title === null) return null;
  const clean = decode(title).slice(0, 180);
  return clean || null;
}

/** The Visuals key of a canvas fence body (Expand selects it). */
export const canvasVisualKey = (code: string) => `canvas-${hash(code)}`;

/** A visual for an html/canvas segment, or null for prose and code. */
export function visualFromSegment(seg: Segment): ChatVisual | null {
  if (seg.kind === 'html') {
    return { key: `html-${hash(seg.code)}`, kind: 'html', lang: seg.lang, code: seg.code, title: htmlVisualTitle(seg.code, seg.lang) };
  }
  if (seg.kind === 'canvas' && seg.closed) {
    const title = finishedCanvasTitle(seg.code);
    if (title) return { key: canvasVisualKey(seg.code), kind: 'canvas', lang: seg.lang, code: seg.code, title };
  }
  return null;
}

// The transcript store changes on every streamed delta, so the side panel's
// list is recomputed often. Parsing is cached per settled turn text.
const cache = new Map<string, ChatVisual[]>();
const CACHE_CAP = 400;

export function visualsInText(text: string): ChatVisual[] {
  if (!text.includes('```')) return [];
  const hit = cache.get(text);
  if (hit) return hit;
  const found: ChatVisual[] = [];
  for (const seg of parseSegments(text)) {
    const v = visualFromSegment(seg);
    if (v) found.push(v);
  }
  if (cache.size >= CACHE_CAP) cache.clear();
  cache.set(text, found);
  return found;
}

/** Every settled visual in the conversation, oldest first, one per key
 *  (a repeated fence keeps its latest position). */
export function collectVisuals(turns: readonly VisualTurn[]): ChatVisual[] {
  const byKey = new Map<string, ChatVisual>();
  for (const t of turns) {
    if (t.role !== 'assistant' || t.partial) continue;
    for (const v of visualsInText(t.text)) {
      byKey.delete(v.key);
      byKey.set(v.key, v);
    }
  }
  return [...byKey.values()];
}

/** The visual the panel should show: the selected one while it is still in
 *  the conversation, else the newest. */
export function pickVisual(visuals: readonly ChatVisual[], key: string | null): ChatVisual | null {
  return (key ? visuals.find(v => v.key === key) : undefined) ?? visuals[visuals.length - 1] ?? null;
}
