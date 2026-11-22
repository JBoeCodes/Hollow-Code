// Undo snackbars and the footer flash (flashFooter / flashRetry) that routes
// into them. No store imports.
// Moved verbatim out of store/runtime.ts (plan L3 X19, 2026-10-14); runtime.ts
// re-exports every symbol, so `store/runtime` imports keep working.
import { createSignal, untrack } from 'solid-js';
import { plainError, retryNotice } from '../plainErrors';
import { errMsg } from '../utils';
import { noticeTimer, trimNotices, type NoticeTimer } from './noticeTimers';
import { recordAppMessage } from './appMessages';
import { motionReduced } from '../motion';

// ─────────────────────────────────────────────────────────────
// UNDO SNACKBARS — for reversible mutations (archive, mark read,
// status/priority change, note move). Each snackbar shows a short
// message + an Undo button that calls back into the originator.
// They auto-dismiss after a few seconds; clicking Undo dismisses
// immediately. Stacked at the bottom of the screen above the footer.
// ─────────────────────────────────────────────────────────────

export interface Snackbar {
  id: string;
  message: string;
  /** The raw error behind a plain-language message (overnight C7), shown
   *  under "Details". */
  detail?: string;
  /** When set, an Undo button is shown that calls this and dismisses. */
  undo?: () => void | Promise<void>;
  /** The action button's label instead of Undo (for example Open). */
  actionLabel?: string;
  /** Actions leave a sticky receipt visible (for example Reveal log). */
  actions?: { label: string; run: () => void | Promise<void> }[];
  /** Runs on explicit dismissal, never transient-cap eviction. */
  onDismiss?: () => void;
  /** How long until auto-dismiss (ms). Defaults to 6000 — long enough
   *  that the user can react but short enough to feel transient. 0 is sticky. */
  ttlMs?: number;
  /** Track which kind of action this represents (helps when grouping
   *  duplicates or applying styling). Free-form. */
  kind?: 'archive' | 'markread' | 'status' | 'priority' | 'move' | 'delete' | string;
}

export const [snackbars, setSnackbars] = createSignal<Snackbar[]>([]);
const clocks = new Map<string, NoticeTimer>();

/// Show a snackbar with an optional Undo action. Returns the snackbar's
/// id so callers can dismiss it programmatically (e.g. after a follow-up
/// reconcile renders the undo moot).
export function flashWithUndo(opts: Omit<Snackbar, 'id'>): string {
  const id = `s_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`;
  const ttl = opts.ttlMs ?? 6000;
  // Store the effective ttl on the bar so the SnackbarStack can render
  // its circular dismiss ring with the correct duration. Without this
  // the component would default to a guess.
  const bar: Snackbar = { ...opts, id, ttlMs: ttl };
  recordAppMessage(bar.message, bar.kind === 'error', bar.detail);
  setSnackbars([bar, ...untrack(snackbars)]);
  clocks.set(id, noticeTimer(ttl, () => dismissSnackbar(id)));
  trimSnackbars(id);
  return id;
}

// Preserve the incoming row and every interaction-held action. If all older
// rows are held, allow temporary overflow until a hold is released.
function trimSnackbars(incoming?: string) {
  const active = untrack(snackbars).filter(b => (b.ttlMs ?? 6000) > 0 && !untrack(leavingSnackbars).has(b.id));
  trimNotices(active.reverse().map(b => b.id), clocks, id => {
    forgetTimer(id); setSnackbars(rows => rows.filter(b => b.id !== id));
  }, incoming);
}

// A bar with keyboard focus or the pointer on it waits (overnight B8): its
// Undo must stay reachable by Tab, which can take longer than the ttl.
export const [heldSnackbars, setHeldSnackbars] = createSignal<ReadonlySet<string>>(new Set());

export function holdSnackbar(id: string, reason = 'interaction') {
  const clock = clocks.get(id);
  if (!clock) return; // Leaving and evicted rows have already disposed theirs.
  clock.hold(reason);
  setHeldSnackbars(prev => new Set(prev).add(id));
}

/** Resume only after every interaction hold clears, with at least 3 s left. */
export function releaseSnackbar(id: string, reason = 'interaction') {
  const clock = clocks.get(id);
  clock?.release(reason);
  if (clock?.held()) return;
  clearHeld(id);
  trimSnackbars();
}

function forgetTimer(id: string) {
  clocks.get(id)?.dispose(); clocks.delete(id);
  clearHeld(id);
}
function clearHeld(id: string) {
  setHeldSnackbars(prev => { const next = new Set(prev); next.delete(id); return next; });
}

// Bars currently playing their exit animation. Kept beside the list (not
// as a flag on the bar) so flipping it never replaces the bar object —
// Solid's <For> is referentially keyed, and a replaced object would
// recreate the row and replay its entrance.
export const [leavingSnackbars, setLeavingSnackbars] = createSignal<ReadonlySet<string>>(new Set());
const SNACKBAR_EXIT_MS = 170;

export function dismissSnackbar(id: string) {
  const bar = snackbars().find(b => b.id === id);
  if (!bar) return;
  if (leavingSnackbars().has(id)) return; // exit already playing
  forgetTimer(id);
  bar.onDismiss?.();
  if (motionReduced()) {
    setSnackbars(prev => prev.filter(b => b.id !== id));
    return;
  }
  // Mark it leaving (CSS plays snackbar-out), then actually remove.
  setLeavingSnackbars(prev => new Set(prev).add(id));
  setTimeout(() => {
    setSnackbars(prev => prev.filter(b => b.id !== id));
    setLeavingSnackbars(prev => {
      const next = new Set(prev);
      next.delete(id);
      return next;
    });
  }, SNACKBAR_EXIT_MS);
}

/// Invoke the snackbar's undo callback and dismiss it. Wrapping the call
/// in trackTask gives undo failures a place to surface in the
/// ActivityDock without needing per-call error UI.
export async function performUndo(id: string) {
  const bar = snackbars().find(b => b.id === id);
  if (!bar?.undo) return;
  dismissSnackbar(id);
  await runAction((bar.actionLabel ?? 'Undo').toLowerCase(), bar.undo);
}

export async function performSnackbarAction(id: string, index: number) {
  const action = untrack(snackbars).find(b => b.id === id)?.actions?.[index];
  if (!action) return;
  await runAction(action.label, action.run);
}

async function runAction(label: string, run: () => void | Promise<void>) {
  try { await run(); }
  catch (error) { flashFooter(`× ${label} failed: ${errMsg(error)}`); }
}

// ── Transient status ─────────────────────────────────────────
// `footerStatus` used to be the whole channel, and NOTHING rendered it:
// 162 call sites, 64 of them `×`-prefixed failures, all writing into a
// signal with zero consumers. A failed reply to a real person was
// indistinguishable from a successful one.
//
// So `flashFooter` now routes into the snackbar stack, which is already
// mounted and already handles stacking, exits and reduced motion. Every
// existing call site starts working without being touched. App messages
// retains the session history; the unused footerStatus signal is retired.

/// Show a transient status message. Failures are styled as failures and
/// dwell longer, because the user has to read them to act on them.
export function flashFooter(msg: string, holdMs?: number) {
  // A raw OS, database or parser error reads as a sentence; the raw text
  // stays under Details (overnight C7). It is a failure either way.
  const plain = plainError(msg.replace(/^\s*×\s*/, ''));
  // Existing callers mark failures with a × prefix.
  const failed = /^\s*×/.test(msg) || plain.detail !== null;
  flashWithUndo({
    message: plain.text,
    ...(plain.detail ? { detail: plain.detail } : {}),
    kind: failed ? 'error' : 'status',
    // Failures get read time; confirmations get out of the way. A failure
    // stays at least 7 s, whatever the caller asked (P0-26: 73 callers asked
    // 1.5-4 s), so it can be read and its Details reached.
    ttlMs: failed ? Math.max(holdMs ?? 0, 7000) : holdMs ?? 3000,
  });
}

/// A failure the person can try again (overnight G2): plain language as in
/// flashFooter, plus a Retry button that runs `retry`.
export function flashRetry(msg: string, retry: () => void | Promise<void>): string {
  const notice = retryNotice(msg, retry);
  return flashWithUndo(notice);
}
