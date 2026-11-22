/** Canvas bridge v1 (Canvas C1, 2026-10-04): the typed, versioned message
 *  contract between Hollow and a sandboxed artifact frame.
 *
 *  A frame runs in an opaque origin (`sandbox="allow-scripts"`, no
 *  same-origin), so postMessage is its only channel to the host. In Canvas and
 *  Interactive modes the artifact's own scripts can post anything the host
 *  runtime (`components/sandbox-doc.js`) can, so every message is untrusted:
 *  the host accepts one only if it comes from that exact frame, is at most
 *  `MAX_MESSAGE_BYTES` once serialised, carries `__hc: 1`, names a kind this
 *  version supports, and has exactly that kind's fields within their limits.
 *
 *  Child → host (v1): `size`, `error`, `navigate` (a vault target), `open` (a
 *  web or mail link, which the host still gates: `openFrameLink`), and since
 *  Canvas C3 wave 2 `state` (a canvas's small UI state, saved with it; only
 *  for the frame's own canvas: `acceptFrameMessage`'s `canvasId`), and
 *  since Canvas C4 A2 `draft` (an ask-canvas's answers, again only for the
 *  frame's own canvas). A draft only FILLS the composer: the person presses
 *  Enter, in every permission mode, and nothing here or in the host ever
 *  sends on a canvas's behalf. Whether a valid draft is taken is
 *  `draftRefusal`'s call (an ask-canvas, not locked, its current revision,
 *  a composer to fill, the frame focused, once a second). Since 10-11 B1:
 *  `blocked` (the frame's first CSP refusal of a kind, which the host shows
 *  as a permission line the person may grant) and `source` (⌥⌘U typed
 *  inside the frame: toggle the source view). Since repo ideas 2026-10-27
 *  #1: `design`, the frame's one computed-style report (contrast against the
 *  real ground, rendered text size, text over a texture) for the host's
 *  design check, only as the reply to the host's `design-check` (its
 *  `ticket`, which is not called a nonce: that word must never appear in
 *  the runtime, artifactSandbox.test.ts), within `LIMITS.designItems`
 *  items. It is advisory text under
 *  the canvas and nothing else: a canvas script can post one too, so it is
 *  bounded like any other message and shown only as text.
 *  Host → child (v1): `theme` (live tokens and colour scheme), `init`
 *  (once per load: canvas id, kit version, locale, reduced motion, saved
 *  state; `initMessage`), since Canvas C4 A1 `drafted`, the host's answer
 *  to a `draft` (`draftedMessage`), so an ask-canvas can tell the person
 *  what happened to their answers, and since A2 `answered` (the canvas is
 *  locked: the person sent its answer; `answeredMessage`). Pinned views can
 *  also post `live-data`: declared read-only scalar values, never a query or
 *  command channel. See `canvas/liveModel.ts` and docs/CANVAS.md.
 *
 *  Reserved, not accepted yet: host → child `motion` (a live reduced-motion
 *  change), declared below so a later slice extends this file instead of
 *  inventing a second channel. The ask kit (`src/canvas/kit/ask/`) builds
 *  the draft within `draftBytes` and `draftSummaryChars`.
 *
 *  Refused messages (Canvas C6 P1): the frame still hears nothing more
 *  than before, but the host logs each refusal of its own frame's message
 *  (`bridgeRefusal`, `refusalLimiter`; `canvas/bridgeRefusals.ts`) to a
 *  bounded app-data file (`canvas_bridge_log.rs`): kind, size, reason, canvas
 *  id and conversation, never the payload.
 *
 *  Pure: no DOM, no store, so it is unit-tested directly. */

import { linkKind, type LinkKind } from './linkGuard';

export const BRIDGE_VERSION = 1 as const;

/** Whole-message cap, measured as UTF-8 JSON. */
export const MAX_MESSAGE_BYTES = 64 * 1024;

export const LIMITS = {
  /** Reported content height in CSS px. */
  maxHeight: 100_000,
  errorChars: 500,
  navigateChars: 1024,
  hrefChars: 2048,
  /** `state` (and `init`'s copy of it), as compact UTF-8 JSON: the store's
   *  cap (`workspace_canvases::MAX_STATE_BYTES`). */
  stateBytes: 16 * 1024,
  /** How deeply a state value may nest. */
  stateDepth: 16,
  /** C4 `draft`: its answers as compact UTF-8 JSON, far below the message
   *  cap, and its readable summary in UTF-16 units. */
  draftBytes: 16 * 1024,
  draftSummaryChars: 500,
  /** C4 A2 host → child `answered`: the answer lines a locked frame shows. */
  answeredChars: 2000,
  canvasIdChars: 64,
  /** Host → child theme. */
  themeVars: 48,
  themeValueChars: 400,
  /** Host → child init. */
  localeChars: 35,
  kitChars: 32,
  /** Ideas 2026-10-27 #1 `design`: the items one report carries, each
   *  item's text excerpt (UTF-16 units), the elements one pass may look at,
   *  and the count of further matches it may claim. */
  designItems: 24,
  designTextChars: 80,
  designScan: 1500,
  designMore: 100_000,
  /** Ideas 2026-10-27 #4: the frame's reported width, and the largest
   *  coordinate or size of an unresolved text's box (CSS px). */
  designWidth: 8000,
  designCoord: 100_000,
} as const;

/** A canvas's UI state: a plain JSON object (the runtime keeps one key per
 *  component or script value), or null for none. */
export type CanvasState = { [key: string]: JsonValue } | null;
export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };

/** The canvas id rule (`fence.ts` CANVAS_ID; `canvas_fence` in Rust). */
export const CANVAS_ID = /^[a-z0-9-]{1,64}$/;

/** An ask field's name, what the agent reads its answer under (the ask
 *  kit's `FIELD_NAME`, `kit/ask/model.ts`; a test pins them equal). */
export const DRAFT_FIELD_NAME = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$/;

/** One answer, in the ask kit's shapes (A1): text, a number, a list of
 *  texts (multi, rank), a row → option map (matrix), or null (unanswered). */
export type DraftValue = null | string | number | string[] | { [row: string]: string };
export type DraftAnswers = { [name: string]: DraftValue };

export type ChildMessage =
  | { kind: 'size'; h: number }
  | { kind: 'settled'; h: number }
  | { kind: 'error'; message: string }
  | { kind: 'navigate'; target: string }
  | { kind: 'open'; href: string; link: LinkKind }
  | BlockedMessage
  | { kind: 'source' }
  | StateMessage
  | DraftMessage
  | AskInvalidMessage
  | DesignMessage;

/** Ideas 2026-10-27 #1: one element the frame's design pass measured.
 *  `contrast`: its text against its real ground (`ratio`, 1–21) below the
 *  WCAG minimum it `need`s (4.5, or 3 for large text); `small-text`: its
 *  rendered font size in CSS px; `texture-text`: text over a gradient,
 *  image, canvas or texture, whose contrast the frame doesn't guess. `text`
 *  is an excerpt of the element's own words. */
export type DesignItem =
  | { rule: 'contrast'; text: string; ratio: number; need: 3 | 4.5 }
  | { rule: 'small-text'; text: string; size: number }
  | { rule: 'texture-text'; text: string; need: 3 | 4.5; box: TextBox | null };
/** Ideas 2026-10-27 #4: an unresolved text's own lines, `[x, y, w, h]` in
 *  whole document CSS px (null: another element's text shares it, or it lies
 *  outside the document). */
export type TextBox = [number, number, number, number];
/** The reply to `design-check`: at most `LIMITS.designItems` items, the
 *  elements looked at (`scanned`), the matches past the cap (`more`),
 *  whether the pass looked at every text element (`complete`: no cap
 *  reached, none skipped mid-animation, nothing thrown), and since repo
 *  ideas 2026-10-27 #4 how the frame drew: its `width` (CSS px) and whether
 *  `texture` was allowed, so Measure on the picture can draw it again. A
 *  `texture-text` item carries the box of its own text lines (or null) and
 *  the ratio it `need`s. */
export interface DesignMessage { kind: 'design'; ticket: string; items: DesignItem[]; scanned: number; more: number; complete: boolean; width: number; texture: boolean }
/** Host → child: measure this canvas once (its reply is `design`). The
 *  `ticket` has the collect nonce's form (`COLLECT_NONCE`). */
export interface DesignCheckMessage { __hc: typeof BRIDGE_VERSION; kind: 'design-check'; ticket: string }

/** 10-11 B1: what a refused load wanted. `scripts` is a locked document's
 *  own scripts; `images` an `img-src` refusal; `network` any other remote
 *  load (fetch, fonts, media, styles, a remote script). */
export type BlockedWant = 'images' | 'network' | 'scripts';
export const BLOCKED_WANTS: readonly BlockedWant[] = ['images', 'network', 'scripts'];
/** 10-11 B1: the frame's first CSP refusal of a kind. `host` is the refused
 *  URL's host name ('' for `scripts`); it is the frame's own report, so the
 *  host only ever shows it as text. */
export interface BlockedMessage { kind: 'blocked'; want: BlockedWant; host: string }
/** A host name as `blocked` carries it: lowercase DNS labels (underscores
 *  allowed, no trailing dot) or an IP, at most 253 characters. */
const BLOCKED_HOST = /^[a-z0-9_]([a-z0-9_.-]{0,251}[a-z0-9_])?$|^\[[0-9a-f:.]{2,45}\]$/;

/** C3: small UI state saved with a canvas so it reopens as left. `state` is
 *  a fresh JSON copy, never the posted object. */
export interface StateMessage { kind: 'state'; canvasId: string; state: CanvasState }
/** C4: an ask-canvas's answers, a fresh copy. Question cards (2026-10-14):
 *  only ever the reply to the host's `collect` (its `nonce`), which Hollow's
 *  own Send button posts; the host then sends them as the person's message. */
export interface DraftMessage { kind: 'draft'; canvasId: string; answers: DraftAnswers; summary: string; nonce: string }
/** The reply to `collect` when `count` answers need attention; the frame
 *  shows which. */
export interface AskInvalidMessage { kind: 'ask-invalid'; nonce: string; count: number }
/** Host → child: Hollow's Send was pressed; hand over the answers. */
export interface CollectMessage { __hc: typeof BRIDGE_VERSION; kind: 'collect'; nonce: string }
/** A `collect` nonce: what the host makes (`newCollectNonce`). */
export const COLLECT_NONCE = /^[a-z0-9]{16,40}$/;
/** Why the host didn't take a draft (C4 A2), or `confirm`: not yet, the
 *  person is choosing whether to replace what the composer already holds
 *  (a later `drafted` settles it).
 *  - `invalid`: not a valid draft for this frame's canvas;
 *  - `not-ask`: the canvas is not an ask-canvas;
 *  - `locked`: its answer was already sent;
 *  - `not-current`: an earlier revision, or a canvas that isn't saved;
 *  - `unavailable`: this view can't send to a conversation;
 *  - `unrequested`: not the reply to a `collect` Hollow's Send just posted
 *    (a canvas script can never send on its own);
 *  - `busy`: a send is still going, or the conversation is still opening
 *    (while it answers, the answers are queued for after the turn, P1-33);
 *  - `not-sent`: taken, but not sent; the canvas keeps them and offers
 *    Send now and Take them back (P1-33; the message box before);
 *  - `send-failed`: taken, but the send threw: Send again;
 *  - `cancelled`: the person kept what the composer held (a curated
 *    panel's Refresh, which still only fills the composer). */
export type DraftRefusal = 'invalid' | 'not-ask' | 'locked' | 'not-current' | 'unavailable' | 'focus' | 'unrequested' | 'busy' | 'confirm' | 'cancelled' | 'not-sent' | 'send-failed';
/** C4 A2: the canvas is locked; `summary` is the answer the person sent. */
export interface AnsweredMessage { __hc: typeof BRIDGE_VERSION; kind: 'answered'; summary: string }

export type ColorScheme = 'light' | 'dark';
export type ThemeMessage = { __hc: typeof BRIDGE_VERSION; kind: 'theme'; scheme: ColorScheme; vars: Record<string, string> };
/** C3: posted once per frame load. `canvasId` is null for a frame that is not
 *  a saved canvas (an html fence, a brief), which can then save no state. */
export interface InitMessage {
  __hc: typeof BRIDGE_VERSION; kind: 'init';
  canvasId: string | null; kit: string; locale: string; reducedMotion: boolean; state: CanvasState;
}

export const CHILD_KINDS = ['size', 'settled', 'error', 'navigate', 'open', 'blocked', 'source', 'state', 'draft', 'ask-invalid', 'design'] as const;
/** Child kinds declared for a later slice and refused as `unsupported`
 *  until then. None since C4 A2 took `draft`. */
export const RESERVED_CHILD_KINDS: readonly string[] = [];

export type Rejection =
  | 'source' | 'origin' | 'not-object' | 'too-large' | 'version'
  | 'unknown-kind' | 'unsupported' | 'invalid'
  /** A `state` or `draft` for a canvas that is not this frame's (or a frame with none). */
  | 'canvas';
export type Parsed<T> = { ok: true; message: T } | { ok: false; reason: Rejection };

const reject = (reason: Rejection): { ok: false; reason: Rejection } => ({ ok: false, reason });

/** UTF-8 byte length of the message as JSON, or null when it can't be
 *  serialised (cycles, BigInt). Structured clone already copied it; this only
 *  measures what the host would have to look at. */
export function messageBytes(data: unknown): number | null {
  try {
    const json = JSON.stringify(data);
    return json === undefined ? null : new TextEncoder().encode(json).length;
  } catch {
    return null;
  }
}

const CONTROL = /[\u0000-\u001f\u007f]/;

function isPlainObject(v: unknown): v is Record<string, unknown> {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return false;
  const proto = Object.getPrototypeOf(v);
  return proto === Object.prototype || proto === null;
}

/** A fresh, plain JSON copy of `v` when it is JSON all the way down (null,
 *  booleans, finite numbers, strings, arrays and plain objects, at most
 *  `depth` levels), else undefined. Structured clone can deliver Dates, Maps,
 *  typed arrays and the like; none of them is state. */
function jsonCopy(v: unknown, depth: number): JsonValue | undefined {
  if (v === null || typeof v === 'boolean' || typeof v === 'string') return v;
  if (typeof v === 'number') return Number.isFinite(v) ? v : undefined;
  if (depth <= 0) return undefined;
  if (Array.isArray(v)) {
    const out: JsonValue[] = [];
    for (const item of v) {
      const c = jsonCopy(item, depth - 1);
      if (c === undefined) return undefined;
      out.push(c);
    }
    return out;
  }
  if (!isPlainObject(v)) return undefined;
  const out: { [key: string]: JsonValue } = {};
  for (const key of Object.keys(v)) {
    const c = jsonCopy(v[key], depth - 1);
    if (c === undefined) return undefined;
    // Own data property even for "__proto__": never the prototype setter.
    Object.defineProperty(out, key, { value: c, enumerable: true, writable: true, configurable: true });
  }
  return out;
}

/** A canvas state as the bridge and the store accept it: a plain JSON object
 *  (or null) of at most `LIMITS.stateBytes` compact UTF-8 bytes, copied.
 *  Undefined when it is anything else. */
export function canvasState(v: unknown): CanvasState | undefined {
  if (v === null) return null;
  if (!isPlainObject(v)) return undefined;
  const copy = jsonCopy(v, LIMITS.stateDepth);
  if (copy === undefined) return undefined;
  const bytes = messageBytes(copy);
  return bytes !== null && bytes <= LIMITS.stateBytes ? (copy as CanvasState) : undefined;
}

/** A draft's answers as the bridge accepts them: a plain object of field
 *  names to the ask kit's value shapes (`DraftValue`), nothing deeper, at
 *  most `LIMITS.draftBytes` compact UTF-8 bytes, copied as own data
 *  properties (a "__proto__" row is data). Undefined when it is anything else. */
export function draftAnswers(v: unknown): DraftAnswers | undefined {
  if (!isPlainObject(v)) return undefined;
  const own = (o: object, key: string, value: unknown) => Object.defineProperty(o, key, { value, enumerable: true, writable: true, configurable: true });
  const out: DraftAnswers = {};
  for (const name of Object.keys(v)) {
    if (!DRAFT_FIELD_NAME.test(name)) return undefined;
    const x = v[name];
    let copy: DraftValue;
    if (x === null || typeof x === 'string') copy = x;
    else if (typeof x === 'number') { if (!Number.isFinite(x)) return undefined; copy = x; }
    else if (Array.isArray(x)) { if (!x.every(item => typeof item === 'string')) return undefined; copy = [...x] as string[]; }
    else if (isPlainObject(x)) {
      const rows: { [row: string]: string } = {};
      for (const row of Object.keys(x)) {
        if (typeof x[row] !== 'string') return undefined;
        own(rows, row, x[row]);
      }
      copy = rows;
    } else return undefined;
    own(out, name, copy);
  }
  const bytes = messageBytes(out);
  return bytes !== null && bytes <= LIMITS.draftBytes ? out : undefined;
}

/** A draft's summary: the readable `label: value` lines, at most
 *  `LIMITS.draftSummaryChars`, no control characters but newlines and tabs. */
const DRAFT_SUMMARY_CONTROL = /[\u0000-\u0008\u000b-\u001f\u007f]/;

function exactKeys(d: Record<string, unknown>, keys: string[]): boolean {
  const own = Object.keys(d);
  return own.length === keys.length && keys.every(k => Object.prototype.hasOwnProperty.call(d, k));
}

/** Validate a child → host message's shape and limits. Source checks are
 *  `acceptFrameMessage`'s job; this judges only the data. */
export function parseChildMessage(data: unknown): Parsed<ChildMessage> {
  if (!isPlainObject(data)) return reject('not-object');
  const bytes = messageBytes(data);
  if (bytes === null || bytes > MAX_MESSAGE_BYTES) return reject('too-large');
  if (data.__hc !== BRIDGE_VERSION) return reject('version');
  const kind = data.kind;
  if ((RESERVED_CHILD_KINDS as readonly unknown[]).includes(kind)) return reject('unsupported');
  if (!(CHILD_KINDS as readonly unknown[]).includes(kind)) return reject('unknown-kind');
  switch (kind) {
    case 'size': case 'settled': {
      const h = data.h;
      if (!exactKeys(data, ['__hc', 'kind', 'h']) || typeof h !== 'number' || !Number.isFinite(h) || h < 0 || h > LIMITS.maxHeight) return reject('invalid');
      return { ok: true, message: { kind, h } };
    }
    case 'error': {
      const m = data.message;
      if (!exactKeys(data, ['__hc', 'kind', 'message']) || typeof m !== 'string' || m.length > LIMITS.errorChars) return reject('invalid');
      return { ok: true, message: { kind, message: m } };
    }
    case 'navigate': {
      const t = data.target;
      if (!exactKeys(data, ['__hc', 'kind', 'target']) || typeof t !== 'string') return reject('invalid');
      const target = t.trim();
      if (!target || target.length > LIMITS.navigateChars || CONTROL.test(target)) return reject('invalid');
      return { ok: true, message: { kind, target } };
    }
    case 'open': {
      const raw = data.href;
      if (!exactKeys(data, ['__hc', 'kind', 'href']) || typeof raw !== 'string') return reject('invalid');
      const href = raw.trim();
      const link = linkKind(href);
      if (!link || href.length > LIMITS.hrefChars || CONTROL.test(href) || /\s/.test(href)) return reject('invalid');
      return { ok: true, message: { kind, href, link } };
    }
    case 'blocked': {
      const want = data.want;
      const h = data.host;
      if (!exactKeys(data, ['__hc', 'kind', 'want', 'host']) || !(BLOCKED_WANTS as readonly unknown[]).includes(want) || typeof h !== 'string') return reject('invalid');
      if (want === 'scripts' ? h !== '' : !BLOCKED_HOST.test(h)) return reject('invalid');
      return { ok: true, message: { kind, want: want as BlockedWant, host: h } };
    }
    case 'source': {
      if (!exactKeys(data, ['__hc', 'kind'])) return reject('invalid');
      return { ok: true, message: { kind } };
    }
    case 'state': {
      const id = data.canvasId;
      if (!exactKeys(data, ['__hc', 'kind', 'canvasId', 'state']) || typeof id !== 'string' || !CANVAS_ID.test(id)) return reject('invalid');
      const state = canvasState(data.state);
      if (state === undefined) return reject('invalid');
      return { ok: true, message: { kind, canvasId: id, state } };
    }
    case 'draft': {
      const id = data.canvasId;
      const summary = data.summary;
      const nonce = data.nonce;
      if (!exactKeys(data, ['__hc', 'kind', 'canvasId', 'answers', 'summary', 'nonce']) || typeof id !== 'string' || !CANVAS_ID.test(id)) return reject('invalid');
      if (typeof nonce !== 'string' || !COLLECT_NONCE.test(nonce)) return reject('invalid');
      if (typeof summary !== 'string' || !summary.trim() || summary.length > LIMITS.draftSummaryChars || DRAFT_SUMMARY_CONTROL.test(summary)) return reject('invalid');
      const answers = draftAnswers(data.answers);
      if (answers === undefined) return reject('invalid');
      return { ok: true, message: { kind, canvasId: id, answers, summary, nonce } };
    }
    case 'ask-invalid': {
      const nonce = data.nonce;
      const count = data.count;
      if (!exactKeys(data, ['__hc', 'kind', 'nonce', 'count']) || typeof nonce !== 'string' || !COLLECT_NONCE.test(nonce)) return reject('invalid');
      if (typeof count !== 'number' || !Number.isInteger(count) || count < 1 || count > 1000) return reject('invalid');
      return { ok: true, message: { kind, nonce, count } };
    }
    case 'design': {
      const { ticket, items, scanned, more, complete, width, texture } = data;
      if (!exactKeys(data, ['__hc', 'kind', 'ticket', 'items', 'scanned', 'more', 'complete', 'width', 'texture']) || typeof ticket !== 'string' || !COLLECT_NONCE.test(ticket)) return reject('invalid');
      const count = (n: unknown, max: number): n is number => typeof n === 'number' && Number.isInteger(n) && n >= 0 && n <= max;
      if (!count(scanned, LIMITS.designScan) || !count(more, LIMITS.designMore) || typeof complete !== 'boolean' || !Array.isArray(items) || items.length > LIMITS.designItems) return reject('invalid');
      if (!count(width, LIMITS.designWidth) || width < 1 || typeof texture !== 'boolean') return reject('invalid');
      const copied: DesignItem[] = [];
      for (const item of items) {
        const one = designItem(item);
        if (!one) return reject('invalid');
        copied.push(one);
      }
      return { ok: true, message: { kind, ticket, items: copied, scanned, more, complete, width, texture } };
    }
  }
  return reject('unknown-kind');
}

/** One `design` item, copied, or null when it breaks its shape or limits. */
function designItem(v: unknown): DesignItem | null {
  if (!isPlainObject(v)) return null;
  const text = v.text;
  if (typeof text !== 'string' || !text.trim() || text.length > LIMITS.designTextChars || DRAFT_SUMMARY_CONTROL.test(text)) return null;
  const within = (n: unknown, lo: number, hi: number): n is number => typeof n === 'number' && Number.isFinite(n) && n >= lo && n <= hi;
  const { ratio, need, size } = v;
  switch (v.rule) {
    case 'contrast':
      if (!exactKeys(v, ['rule', 'text', 'ratio', 'need']) || !within(ratio, 1, 21) || (need !== 3 && need !== 4.5)) return null;
      return { rule: 'contrast', text, ratio, need };
    case 'small-text':
      if (!exactKeys(v, ['rule', 'text', 'size']) || !within(size, 0, 200)) return null;
      return { rule: 'small-text', text, size };
    case 'texture-text': {
      const box = v.box;
      const int = (n: unknown, lo: number): n is number => typeof n === 'number' && Number.isInteger(n) && n >= lo && n <= LIMITS.designCoord;
      const boxed = box === null || (Array.isArray(box) && box.length === 4 && int(box[0], 0) && int(box[1], 0) && int(box[2], 1) && int(box[3], 1)
        && box[0] + box[2] <= LIMITS.designCoord && box[1] + box[3] <= LIMITS.designCoord);
      if (!exactKeys(v, ['rule', 'text', 'need', 'box']) || (need !== 3 && need !== 4.5) || !boxed) return null;
      return { rule: 'texture-text', text, need, box: box === null ? null : [box[0], box[1], box[2], box[3]] };
    }
  }
  return null;
}

/** Ask the frame for its design report (ideas 2026-10-27 #1). */
export function designCheckMessage(ticket: string): DesignCheckMessage {
  return { __hc: BRIDGE_VERSION, kind: 'design-check', ticket };
}

interface FrameLike { contentWindow: unknown; isConnected: boolean }
interface MessageLike { source: unknown; origin: string; data: unknown }

/** Accept a `message` event only from `frame` itself: the same window object,
 *  still connected, and the opaque origin every sandboxed frame has. A message
 *  from any other frame (another artifact, a nested frame) is not ours.
 *  `canvasId` is the saved canvas this frame shows (null or absent: none); a
 *  `state` or `draft` naming any other canvas is refused, so a frame can
 *  never write another canvas's state or answer for it. */
export function acceptFrameMessage(e: MessageLike, frame: FrameLike | undefined, canvasId: string | null = null): Parsed<ChildMessage> {
  if (!frame || !frame.isConnected || !frame.contentWindow || e.source !== frame.contentWindow) return reject('source');
  if (e.origin !== 'null') return reject('origin');
  const r = parseChildMessage(e.data);
  if (r.ok && (r.message.kind === 'state' || r.message.kind === 'draft') && (canvasId === null || r.message.canvasId !== canvasId)) return reject('canvas');
  return r;
}

const LOCALE = /^[A-Za-z0-9-]{2,35}$/;

/** The host → child init for one frame load. Anything the runtime would
 *  refuse is dropped here first: a state that is not a small plain JSON
 *  object becomes null (the canvas starts fresh), an odd locale becomes
 *  `en`, and a frame that is not a canvas (`canvasId` null) gets no state. */
export function initMessage(o: { canvasId: string | null; kit: string; locale: string; reducedMotion: boolean; state: unknown }): InitMessage {
  const canvasId = o.canvasId !== null && CANVAS_ID.test(o.canvasId) ? o.canvasId : null;
  const state = canvasId === null ? null : canvasState(o.state) ?? null;
  return {
    __hc: BRIDGE_VERSION, kind: 'init', canvasId,
    kit: o.kit.slice(0, LIMITS.kitChars),
    locale: LOCALE.test(o.locale) ? o.locale : 'en',
    reducedMotion: !!o.reducedMotion,
    state,
  };
}

/** Hollow's Send asks the frame for its answers. */
export function collectMessage(nonce: string): CollectMessage {
  return { __hc: BRIDGE_VERSION, kind: 'collect', nonce };
}

/** A fresh `collect` nonce (32 hex characters). */
export function newCollectNonce(): string {
  const b = new Uint8Array(16);
  crypto.getRandomValues(b);
  return [...b].map(x => x.toString(16).padStart(2, '0')).join('');
}

/** How long the frame has to answer a `collect` (ms). */
export const COLLECT_TIMEOUT_MS = 3000;

/** Tell a locked ask-canvas the answer the person sent (C4 A2). */
export function answeredMessage(summary: string): AnsweredMessage {
  return { __hc: BRIDGE_VERSION, kind: 'answered', summary: summary.slice(0, LIMITS.answeredChars) };
}

/** The least time between two drafts one frame may have taken (ms): about
 *  one press of Send, so a canvas script posting drafts in a loop gets one a
 *  second at most, and only while the person is using its frame. */
export const DRAFT_INTERVAL_MS = 1000;

/** What a frame's valid draft meets before the host sends it. */
export interface DraftGate {
  /** The frame is an ask-canvas (`<hc-canvas ask>`). */
  ask: boolean;
  /** Its answer was already sent (the transcript has it). */
  locked: boolean;
  /** The host view's own refusal (`not-current`, `unavailable`), or null. */
  host: DraftRefusal | null;
  /** It answers the `collect` Hollow's Send posted, within its time. */
  requested: boolean;
  /** A send from this frame is still going. */
  pending: boolean;
  now: number;
  /** When this frame's last draft was taken (ms), or null. */
  last: number | null;
}

/** Why the host refuses a valid draft, or null to take it. A locked canvas
 *  never drafts again, whatever else it tries. */
export function draftRefusal(g: DraftGate): DraftRefusal | null {
  if (!g.ask) return 'not-ask';
  if (g.locked) return 'locked';
  if (g.host) return g.host;
  if (!g.requested) return 'unrequested';
  if (g.pending || (g.last !== null && g.now - g.last < DRAFT_INTERVAL_MS)) return 'busy';
  return null;
}

const THEME_VAR = /^--app-[a-z0-9-]{1,40}$/;

/** A theme value the child may set as a custom property: plain text with no
 *  declaration, block or markup breakouts. */
export function isSafeThemeValue(v: unknown): v is string {
  return typeof v === 'string' && v.length > 0 && v.length <= LIMITS.themeValueChars
    && !/[;{}<>\\]/.test(v) && !CONTROL.test(v);
}

/** Build a host → child theme message from `--app-*` pairs, dropping anything
 *  the child runtime would refuse. The runtime re-checks the same rules. */
export function themeMessage(scheme: ColorScheme, pairs: Array<[string, string]>): ThemeMessage {
  const vars: Record<string, string> = {};
  for (const [name, value] of pairs) {
    if (Object.keys(vars).length >= LIMITS.themeVars) break;
    if (THEME_VAR.test(name) && isSafeThemeValue(value)) vars[name] = value;
  }
  return { __hc: BRIDGE_VERSION, kind: 'theme', scheme, vars };
}

/** Canvas C6 P1: the refusals the host logs. Every `Rejection` but `source`
 *  and `origin` (another window's message, not this frame's) and every
 *  `DraftRefusal` but `confirm` and `cancelled` (the person choosing, not a
 *  refusal of the frame) and `not-sent` and `send-failed` (Hollow's own send
 *  failing after it took the draft). A `state` dropped because the person
 *  isn't using the frame is logged as `focus`. `canvas_bridge_log.rs` holds the same list. */
export const LOGGED_REASONS = ['not-object', 'too-large', 'version', 'unknown-kind', 'unsupported', 'invalid', 'canvas', 'not-ask', 'locked', 'not-current', 'unavailable', 'focus', 'unrequested', 'busy'] as const;
export type LoggedReason = typeof LOGGED_REASONS[number];
export const isLoggedReason = (reason: string): reason is LoggedReason => (LOGGED_REASONS as readonly string[]).includes(reason);

/** One refused message as the host logs it: never the payload, a summary,
 *  a link or any other text the canvas wrote. `kind` is a child kind or
 *  `other` (an unknown kind's own text is not kept); `bytes`, the message as
 *  UTF-8 JSON (null when it can't be serialised); `suppressed`, refusals the
 *  limit dropped since the last one logged. */
export interface BridgeRefusal {
  kind: (typeof CHILD_KINDS)[number] | 'other';
  bytes: number | null;
  reason: LoggedReason;
  canvasId: string | null;
  conversation: string | null;
  suppressed: number;
}

/** A conversation key (`agent_sessions::is_safe_session_key`). */
const SESSION_KEY = /^s-[a-z0-9-]{0,62}$/;

/** The record for a refused message `data`, or null when `reason` is not a
 *  refusal the host logs. Ids that break their rules are null. */
export function bridgeRefusal(
  data: unknown, reason: Rejection | DraftRefusal, canvasId: string | null, conversation: string | null, suppressed = 0,
): BridgeRefusal | null {
  if (!isLoggedReason(reason)) return null;
  const k = isPlainObject(data) ? data.kind : undefined;
  return {
    kind: (CHILD_KINDS as readonly unknown[]).includes(k) ? (k as (typeof CHILD_KINDS)[number]) : 'other',
    bytes: messageBytes(data),
    reason,
    canvasId: canvasId !== null && CANVAS_ID.test(canvasId) ? canvasId : null,
    conversation: conversation !== null && SESSION_KEY.test(conversation) ? conversation : null,
    suppressed,
  };
}

/** How many refusals the host logs: at most `perWindow` per `windowMs`
 *  across every frame in this window (the backend has its own limit too).
 *  `take(now)` is the count dropped since the last one taken, or null when
 *  this one is dropped (and counted). */
export interface RefusalLimiter { take(now: number): number | null }
export function refusalLimiter(perWindow = 30, windowMs = 60_000): RefusalLimiter {
  let start = -Infinity;
  let count = 0;
  let dropped = 0;
  return {
    take(now) {
      if (now - start >= windowMs || now < start) { start = now; count = 0; }
      if (count >= perWindow) { dropped++; return null; }
      count++;
      const d = dropped;
      dropped = 0;
      return d;
    },
  };
}
