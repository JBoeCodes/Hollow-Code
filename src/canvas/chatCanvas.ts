/**
 * Chat canvases (Canvas C3, wave 2 C1): what a `hollow-canvas` fence shows in
 * a reply, and how it meets the canvas store. Pure apart from the injected
 * store calls, so the node Vitest suite covers it; `components/ChatCanvas.tsx`
 * renders it.
 *
 * While the reply streams a fence is `pending` (nothing extra is shown), then
 * `drawing` once the root start tag has closed (a skeleton with the title, at
 * the height the root asks for), then `ready` when the fence closes. A broken
 * fence (`invalid`), or a canvas whose frame reports a runtime error, shows
 * its source and "Ask to fix", which drafts a fix request in the composer and
 * never sends it.
 *
 * Saving: a fence that becomes ready during a live turn is recorded once
 * (`resolveCanvas` with `live`). Re-rendering a transcript or reopening a
 * conversation only reads. The store's `retired` outcome and a
 * `CANVAS_RETIRED:` refusal both mean "render the fence itself, don't save".
 *
 * Revisions and state (wave 2 C2): `canvasHeads` holds the newest this
 * window knows of each saved canvas (current revision, CAS token, state,
 * kept revisions), so an older message learns that a later one revised its
 * canvas, and state saves use the latest token. `saveCanvasStateCas` saves
 * the state a canvas frame posted; `canvasStateSaver` debounces it.
 */

import { canvasFrameBody, parseCanvasFence, withoutTypingFence, type CanvasErrorCode, type CanvasParse, type CanvasRoot } from './fence';
import { unclosedCause, unclosedReason } from './strayTags';
import type { Canvas, CanvasRecorded, CanvasRevision, CanvasStateSaved } from '../generated/ipc';
import type { ArtifactMode } from '../components/htmlSandbox';
import { canvasState, type CanvasState } from '../canvasBridge';

/** The skeleton's height when the root names none (CSS px). */
export const DEFAULT_CANVAS_HEIGHT = 240;

export type ChatCanvasView =
  | { state: 'pending' }
  | { state: 'drawing'; root: CanvasRoot; reserve: number }
  | { state: 'ready'; root: CanvasRoot; inner: string }
  | { state: 'invalid'; code: CanvasErrorCode; reason: string; root: CanvasRoot | null };

/** The id and title as far as they parse, for a broken canvas's fix request. */
function rootHint(code: string): CanvasRoot | null {
  const head = /^\s*<hc-canvas\b([^>]*)>?/i.exec(code)?.[1] ?? '';
  const attr = (name: string) => new RegExp(`(?:^|\\s)${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)'|([^\\s>]+))`, 'i').exec(head);
  const pick = (m: RegExpExecArray | null) => (m ? (m[1] ?? m[2] ?? m[3] ?? '') : '');
  const id = pick(attr('id')).slice(0, 64);
  const title = pick(attr('title')).replace(/\s+/g, ' ').trim().slice(0, 180);
  return id || title ? { id, title, ask: false, height: null } : null;
}

/**
 * The body and fence state a canvas segment is read with. `closed`: its
 * closing fence line arrived; `settled`: the reply is no longer streaming.
 * A fence still open in a settled reply (the person pressed Stop, the agent
 * failed, or the provider dropped the closing fence) will never close, so it
 * reads as ended: a truncated canvas is `invalid` and shows its source and
 * Ask to fix instead of a skeleton forever. A half-typed closing fence line
 * is dropped first, so a whole canvas still renders (and saves) as itself.
 */
export function settleCanvasBody(code: string, closed: boolean, settled: boolean): { code: string; closed: boolean } {
  if (closed || !settled) return { code, closed };
  return { code: withoutTypingFence(code), closed: true };
}

/** What a canvas segment shows. `closed`: its closing fence line arrived.
 *  A canvas that never reaches its `</hc-canvas>` names what hid it, on
 *  which line, and the fix (overnight A2, `strayTags.ts`). */
export function chatCanvasView(code: string, closed: boolean): ChatCanvasView {
  const p: CanvasParse = parseCanvasFence(code, closed);
  switch (p.state) {
    case 'pending': return p;
    case 'drawing': return { state: 'drawing', root: p.root, reserve: p.root.height ?? DEFAULT_CANVAS_HEIGHT };
    case 'ready': return p;
    case 'invalid': {
      const cause = p.code === 'unclosed' && closed ? unclosedCause(code) : null;
      return { ...p, reason: cause ? unclosedReason(cause) : p.reason, root: rootHint(code) };
    }
  }
}

/** A short, single-line excerpt of an error for a fix request. */
function excerpt(text: string, max = 300): string {
  const one = text.replace(/\s+/g, ' ').trim();
  return one.length > max ? `${one.slice(0, max - 1)}…` : one;
}

/**
 * The fix request "Ask to fix" drafts in the composer (never sent: the
 * person presses Enter). Quotes the error; names the canvas when its id and
 * title are known, so the agent re-emits the same id as a new revision.
 */
export function canvasFixRequest(root: Pick<CanvasRoot, 'id' | 'title'> | null, error: string): string {
  const quoted = JSON.stringify(excerpt(error));
  const which = root?.title && root.id
    ? `The canvas "${excerpt(root.title, 120)}" (id ${root.id})`
    : root?.id ? `The canvas ${root.id}` : 'The hollow-canvas fence in your last reply';
  const same = root?.id ? ' with the same id' : '';
  return `${which} is broken: ${quoted}. Please send a corrected hollow-canvas fence${same}.`;
}

/** Bounded plain-text correction draft; reserve the same-id instruction. */
export function canvasLayoutRequest(root: Pick<CanvasRoot, 'id' | 'title'>, flowLabel: string, findings: readonly { text: string }[]): string {
  const plain = (s: string, max: number) => excerpt(s.replace(/[<>]/g, ''), max);
  const ending = '\nPlease send a corrected hollow-canvas fence with the same id; shorter labels, groups ([Name] … []) or direction="right" usually fix them.';
  const heading = `The canvas "${plain(root.title, 120)}" (id ${plain(root.id, 64)}) has ${findings.length} layout findings in its diagram "${plain(flowLabel, 120)}":\n`;
  const rows = findings.map((f, i) => `${i + 1}. ${plain(f.text, 300)}`).join('\n');
  return heading + excerpt(rows, 2000 - heading.length - ending.length) + ending;
}

/** The mode a canvas opens in: always Canvas (scripts, no network). The
 *  per-artifact switch stays; Interactive is still the person's red choice. */
export const CANVAS_FENCE_MODE: ArtifactMode = 'canvas';

/** The mode an ```html / ```svg fence opens in (plan §1): Canvas in quick and
 *  workspace chats, Offline in any other conversation. */
export function htmlFenceMode(inWorkspace: boolean): ArtifactMode {
  return inWorkspace ? 'canvas' : 'offline';
}

/** Which canvas store a conversation uses: its workspace (Quick chats
 *  included), else the conversation's own app-data store. */
export type ChatCanvasScope = { workspaceId: string | null; sessionKey: string | null };

export function hasStore(scope: ChatCanvasScope | null): scope is ChatCanvasScope {
  return !!scope && (!!scope.workspaceId || !!scope.sessionKey);
}

/** The store calls this module makes (a subset of `api`). */
export interface CanvasStoreCalls {
  readCanvas(scope: ChatCanvasScope, id: string): Promise<Canvas | null>;
  recordCanvas(scope: ChatCanvasScope, body: string, expectedSha256: string | null, opts?: { turnId?: string | null; kit?: string | null }): Promise<CanvasRecorded>;
}

/** What a ready canvas renders and says about its saved copy. */
export interface ResolvedCanvas {
  /** The fence body to render: the store's current revision when this fence
   *  is it, else the transcript's own body. */
  body: string;
  /** This fence's revision in the store, when it is saved there. */
  revision: number | null;
  /** The store's current revision, when it has the canvas. */
  latest: number | null;
  saved: boolean;
  /** One short line when the canvas is not (or no longer) saved. */
  note: string | null;
  /** The store's record of the canvas as this call saw it, when it has one. */
  canvas: Canvas | null;
}

export async function sha256Hex(text: string): Promise<string> {
  const bytes = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
  return [...new Uint8Array(bytes)].map(b => b.toString(16).padStart(2, '0')).join('');
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

const unsaved = (body: string, note: string | null = null): ResolvedCanvas => ({ body, revision: null, latest: null, saved: false, note, canvas: null });

function fromStore(body: string, sha: string, canvas: Canvas | null): ResolvedCanvas {
  if (!canvas) return unsaved(body);
  if (canvas.fenceSha256 === sha) return { body: canvas.html, revision: canvas.revision, latest: canvas.revision, saved: true, note: null, canvas };
  const kept = canvas.revisions.find(r => r.fenceSha256 === sha);
  if (kept) return { body, revision: kept.revision, latest: canvas.revision, saved: true, note: null, canvas };
  return { body, revision: null, latest: canvas.revision, saved: false, note: null, canvas };
}

/** Live records in flight or done, by store and body: a frame that remounts
 *  when its turn settles reuses the first call instead of recording again. */
const recorded = new Map<string, Promise<ResolvedCanvas>>();
const RECORDED_CAP = 256;

/** For tests: forget every recorded body. */
export function forgetRecordedCanvases() { recorded.clear(); }

async function record(calls: CanvasStoreCalls, scope: ChatCanvasScope, id: string, body: string, sha: string, kit: string | null, turnId: string | null): Promise<ResolvedCanvas> {
  const attempt = async (): Promise<CanvasRecorded> => {
    const current = await calls.readCanvas(scope, id).catch(() => null);
    return calls.recordCanvas(scope, body, current?.sha256 ?? null, { kit, turnId });
  };
  let rec: CanvasRecorded;
  try {
    try { rec = await attempt(); }
    catch (e) {
      // Something else saved the canvas between the read and the save: read
      // again and try once more (a kept body succeeds whatever the token).
      if (!message(e).startsWith('CANVAS_CHANGED:')) throw e;
      rec = await attempt();
    }
  } catch (e) {
    const m = message(e);
    if (m.startsWith('CANVAS_RETIRED:')) return unsaved(body, 'Not saved: this conversation’s saved canvases were cleared.');
    return unsaved(body, `Not saved: ${excerpt(m.replace(/^CANVAS_[A-Z]+:\s*/, ''), 160)}`);
  }
  if (rec.outcome === 'retired') {
    return { body, revision: rec.matchedRevision, latest: rec.canvas.revision, saved: false, note: 'An older version that is no longer kept; not saved again.', canvas: rec.canvas };
  }
  return { ...fromStore(body, sha, rec.canvas), saved: true };
}

/**
 * Resolve a ready canvas against its store. `live`: the fence finished
 * during a turn running now, so it is recorded (once per store and body).
 * Otherwise this only reads: the store's current revision renders when this
 * fence is it; an earlier kept revision renders from the transcript with its
 * number; an unknown or unreadable canvas renders from the transcript.
 * `turnId`: the history run id of the reply the fence streamed in. The store
 * keeps it on the revision.
 */
export async function resolveCanvas(
  calls: CanvasStoreCalls, scope: ChatCanvasScope | null, id: string, body: string,
  opts: { live: boolean; kit?: string | null; turnId?: string | null; written?: string | null },
): Promise<ResolvedCanvas> {
  if (!hasStore(scope)) return unsaved(body);
  const sha = await sha256Hex(body);
  if (!opts.live) {
    const canvas = await calls.readCanvas(scope, id).catch(() => null);
    const found = fromStore(body, sha, canvas);
    // `written`: the fence as the agent wrote it, when `escapeRowTags`
    // changed it (overnight A2). A canvas saved before then holds those
    // bytes, so it still finds its revision, state and answer lock.
    if (found.saved || !opts.written || opts.written === body) return found;
    const old = fromStore(opts.written, await sha256Hex(opts.written), canvas);
    return old.saved ? { ...old, body } : found;
  }
  const key = `${scope.workspaceId ?? ''}\u0000${scope.sessionKey ?? ''}\u0000${sha}`;
  const done = recorded.get(key);
  if (done) return done;
  // The turn's end saves the fence as written (question cards, 2026-10-14,
  // `workspace_canvases::record_reply`). When the stray-tag repair changed
  // it and that save came first, use it rather than add a second revision.
  if (opts.written && opts.written !== body) {
    const canvas = await calls.readCanvas(scope, id).catch(() => null);
    const old = fromStore(opts.written, await sha256Hex(opts.written), canvas);
    if (old.saved) return { ...old, body };
  }
  if (recorded.size >= RECORDED_CAP) recorded.delete(recorded.keys().next().value as string);
  const p = record(calls, scope, id, body, sha, opts.kit ?? null, opts.turnId || null);
  recorded.set(key, p);
  return p;
}

/** The ready canvas's inner HTML from a stored or transcript body (the
 *  store re-validated it, so a parse failure here means the bytes changed
 *  underneath; fall back to the transcript's own inner). */
export function innerOf(body: string, fallback: string): string {
  const p = parseCanvasFence(body, true);
  return p.state === 'ready' ? canvasFrameBody(p) : fallback;
}

/* ── Revisions and state (wave 2 C2) ─────────────────────────────────── */

/** One saved canvas as this window last saw it. */
export interface CanvasHead {
  revision: number;
  /** The sidecar's SHA-256: the CAS token for the next write. */
  token: string;
  state: CanvasState;
  stateSavedAt: string | null;
  /** Every kept revision, oldest first (the store's list). */
  revisions: CanvasRevision[];
}

/** A canvas's key across a window: its store and id. */
export function canvasKey(scope: ChatCanvasScope, id: string): string {
  return `${scope.workspaceId ?? ''}\u0000${scope.sessionKey ?? ''}\u0000${id}`;
}

const savedAt = (at: string | null) => { const t = at ? Date.parse(at) : NaN; return Number.isNaN(t) ? -Infinity : t; };

export function headOf(canvas: Canvas): CanvasHead {
  return { revision: canvas.revision, token: canvas.sha256, state: canvasState(canvas.state) ?? null, stateSavedAt: canvas.stateSavedAt, revisions: canvas.revisions };
}

/** The newest known head per canvas. A note that is older than what is
 *  known (a lower revision, or the same revision with an earlier state save)
 *  is ignored, so a stale read or a cached record never rolls a head back. */
export class CanvasHeads {
  private heads = new Map<string, CanvasHead>();
  private listeners = new Set<() => void>();

  get(key: string): CanvasHead | undefined { return this.heads.get(key); }

  note(key: string, next: CanvasHead): void {
    const known = this.heads.get(key);
    if (known && (next.revision < known.revision
      || (next.revision === known.revision && savedAt(next.stateSavedAt) < savedAt(known.stateSavedAt)))) return;
    if (known && known.token === next.token && known.revision === next.revision) return;
    this.heads.set(key, next);
    if (this.heads.size > 512) this.heads.delete(this.heads.keys().next().value as string);
    this.listeners.forEach(fn => fn());
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  clear(): void { this.heads.clear(); }
}

export const canvasHeads = new CanvasHeads();

export interface CanvasStateCalls extends CanvasStoreCalls {
  saveCanvasState(scope: ChatCanvasScope, id: string, state: unknown, expectedSha256: string): Promise<CanvasStateSaved>;
}

const sameJson = (a: unknown, b: unknown) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

export type StateSaveOutcome = 'saved' | 'unchanged' | 'kept-newer' | 'failed';

/**
 * Save `state` (posted by the canvas's frame at `changedAt`, ms) with the
 * store's compare-and-swap. The token is the newest head's. On
 * `CANVAS_CHANGED:` the canvas is read again: if its state was saved after
 * this change (another window, the side panel's copy), that newer value is
 * kept and this one dropped; otherwise the save is retried once with the
 * fresh token. A state equal to the saved one writes nothing.
 */
export async function saveCanvasStateCas(
  calls: CanvasStateCalls, heads: CanvasHeads, scope: ChatCanvasScope, id: string, state: CanvasState, changedAt: number,
  expectedRevision?: number,
): Promise<StateSaveOutcome> {
  const key = canvasKey(scope, id);
  if (!heads.get(key)) {
    const c = await calls.readCanvas(scope, id).catch(() => null);
    if (c) heads.note(key, headOf(c));
  }
  for (let attempt = 0; attempt < 2; attempt++) {
    const head = heads.get(key);
    if (!head) return 'failed';
    if (expectedRevision !== undefined && head.revision !== expectedRevision) return 'kept-newer';
    if (sameJson(head.state, state)) return 'unchanged';
    try {
      const r = await calls.saveCanvasState(scope, id, state, head.token);
      heads.note(key, { ...head, token: r.sha256, state, stateSavedAt: r.stateSavedAt, revision: r.revision });
      return 'saved';
    } catch (e) {
      if (!message(e).startsWith('CANVAS_CHANGED:')) return 'failed';
      const c = await calls.readCanvas(scope, id).catch(() => null);
      if (!c) return 'failed';
      heads.note(key, headOf(c));
      if (savedAt(c.stateSavedAt) > changedAt) return 'kept-newer';
    }
  }
  return 'failed';
}

/** Debounce a canvas's state saves: the last state within `delayMs` is saved,
 *  one save at a time (each waits for the previous token). `flush` saves
 *  what is pending now (on unmount). */
export function canvasStateSaver(save: (state: CanvasState, changedAt: number, revision?: number) => Promise<unknown>, delayMs = 400,
  report: (status: 'pending' | 'saving' | 'failed' | 'saved') => void = () => {}) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let sequence = 0;
  let pending: { state: CanvasState; at: number; sequence: number; revision?: number } | null = null;
  let chain: Promise<unknown> = Promise.resolve();
  const flush = (): Promise<unknown> => {
    clearTimeout(timer);
    if (!pending) return chain;
    const job = pending;
    pending = null;
    chain = chain.then(async () => {
      if (job.sequence === sequence) report('saving');
      let failed = false;
      try { failed = await (job.revision === undefined ? save(job.state, job.at) : save(job.state, job.at, job.revision)) === 'failed'; }
      catch { failed = true; }
      // A late failed write must never replace an edit posted since it began.
      // Keep the original timestamp: retry still respects newer remote state.
      if (job.sequence === sequence) {
        if (failed && !pending) pending = job;
        report(failed ? 'failed' : 'saved');
      }
    });
    return chain;
  };
  return {
    push(state: CanvasState, at = Date.now(), revision?: number) {
      pending = { state, at, sequence: ++sequence, revision };
      report('pending');
      clearTimeout(timer);
      timer = setTimeout(() => { void flush(); }, delayMs);
    },
    flush,
  };
}

/** The version label: the shown revision, and the current one when they differ. */
export function versionLabel(shown: number, latest: number | null): string {
  return latest && latest !== shown ? `v${shown} of ${latest}` : `v${shown}`;
}
