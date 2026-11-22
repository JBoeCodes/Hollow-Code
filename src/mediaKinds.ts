import { convertFileSrc } from '@tauri-apps/api/core';
import { isBrowserMockMode } from './mockMode';
import { isVideoPath } from './chatFileLinks';

export { VIDEO_EXTS, isVideoPath } from './chatFileLinks';

/** Whether QuickTime Player opens this file (the backend checks again). */
export function opensInQuickTime(p: string): boolean {
  return isVideoPath(p);
}

/** The URL a `<video>` loads a granted token from: the `hollow-media`
 *  scheme in Tauri, and a path the Playwright smoke routes on the mock. */
export function mediaSrc(token: string): string {
  if (isBrowserMockMode()) return `/__media__/${token}`;
  return convertFileSrc(token, 'hollow-media');
}

/** The stable refusal codes `media_grant` answers with. */
export type MediaRefusal = 'OUTSIDE_ROOTS' | 'NOT_VIDEO' | 'UNSUPPORTED_CONTAINER' | 'MISSING' | 'HIDDEN';

const REFUSALS: readonly MediaRefusal[] = ['OUTSIDE_ROOTS', 'NOT_VIDEO', 'UNSUPPORTED_CONTAINER', 'MISSING', 'HIDDEN'];

/** The refusal code in a `media_grant` error, or null for anything else. */
export function refusalOf(err: unknown): MediaRefusal | null {
  const text = err instanceof Error ? err.message : String(err ?? '');
  return REFUSALS.find(code => text === code) ?? null;
}

/** What the player says when a file can't play: refusals from the grant,
 *  then load failures classified by granting again (the page can't see a
 *  media response's status). */
export type MediaProblem = MediaRefusal | 'changed' | 'format' | 'failed';

export function problemCopy(p: MediaProblem): string {
  switch (p) {
    case 'MISSING': return 'File not found';
    case 'OUTSIDE_ROOTS': return 'Outside Hollow’s folders';
    case 'HIDDEN':
    case 'NOT_VIDEO':
    case 'UNSUPPORTED_CONTAINER': return 'This file can’t play inside Hollow';
    case 'changed': return 'Still being written. Try again when the render finishes.';
    case 'format': return 'This video’s format doesn’t play inside Hollow';
    case 'failed': return 'Couldn’t load this video';
  }
}

/** Whether a problem keeps the Open and Reveal actions (a missing file has
 *  nothing to open). */
export function problemHasActions(p: MediaProblem): boolean {
  return p !== 'MISSING';
}

/** A first grant compared with a second one, after the player failed. */
export function classifyFailure(
  first: { size: number; modifiedMs: number | null },
  again: { size: number; modifiedMs: number | null } | MediaRefusal,
  notSupported: boolean,
  retried: boolean,
): MediaProblem | 'retry' {
  if (typeof again === 'string') return again;
  const changed = again.size !== first.size || again.modifiedMs !== first.modifiedMs;
  if (changed) return retried ? 'changed' : 'retry';
  return notSupported ? 'format' : 'failed';
}

// ── One budget for every <video> in the app ─────────────────────────────

/** Mounted `<video>` elements app-wide at most. */
export const MEDIA_BUDGET = 4;

export interface MediaSlot {
  /** `inline` players may be evicted; explicit opens never are. */
  explicit: boolean;
  /** Paused (or not started). A playing player is never evicted. */
  paused: () => boolean;
  /** When it was last on screen (higher is more recent). */
  lastVisible: () => number;
  /** Unmount back to the card. */
  evict: () => void;
}

const slots = new Set<MediaSlot>();

/**
 * Take a mount slot. An explicit open (side pane, Results, a clicked card)
 * always gets one, evicting the least recently visible paused inline player
 * if the budget is full. An automatic inline mount gets one only if free.
 * Returns whether the slot was taken.
 */
export function takeMediaSlot(slot: MediaSlot): boolean {
  if (slots.has(slot)) return true;
  if (slots.size < MEDIA_BUDGET) {
    slots.add(slot);
    return true;
  }
  if (!slot.explicit) return false;
  const victim = [...slots]
    .filter(s => !s.explicit && s.paused())
    .sort((a, b) => a.lastVisible() - b.lastVisible())[0];
  // Take the slot first, so a waiting inline player can't grab the one the
  // victim frees.
  slots.add(slot);
  if (victim) {
    slots.delete(victim);
    victim.evict();
  }
  return true;
}

export function releaseMediaSlot(slot: MediaSlot): void {
  if (!slots.delete(slot)) return;
  if (slots.size < MEDIA_BUDGET) for (const fn of [...waiters]) fn();
}

/** How many players are mounted (tests). */
export function mountedMediaCount(): number {
  return slots.size;
}

const waiters = new Set<() => void>();

/** Be told when a slot frees, so a waiting inline player can try again. */
export function onMediaSlotFreed(fn: () => void): () => void {
  waiters.add(fn);
  return () => waiters.delete(fn);
}

// ── One player plays at a time ──────────────────────────────────────────

const players = new Set<HTMLVideoElement>();

export function registerPlayer(v: HTMLVideoElement): () => void {
  players.add(v);
  const onPlay = () => {
    for (const other of players) if (other !== v && !other.paused) other.pause();
  };
  v.addEventListener('play', onPlay);
  return () => {
    v.removeEventListener('play', onPlay);
    players.delete(v);
  };
}
