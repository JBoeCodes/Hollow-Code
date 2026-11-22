// "New since you were here" (parity 0.3): which turns of a conversation arrived since the person last
// had it open. A visit records how far they read (`seen`, a turn count) per conversation in
// localStorage, 200 conversations at most; opening it again shows a divider before turn `seen` when
// more turns exist. Turn counts, not times: a turn can't be unread by a clock skew, and an edit to
// an earlier turn is not new.

import { createSignal } from 'solid-js';

const KEY = 'hollow.chatVisits.v1';

/** Bumped on every recorded visit, so the sidebar's unread marks re-read it. */
export const [visitsVersion, setVisitsVersion] = createSignal(0);
/** Conversations remembered. */
export const VISITS_KEPT = 200;

type Visits = Record<string, { seen: number; at: number }>;

// The parsed blob, reused while the stored text is unchanged: every sidebar row asks (3.3).
let parsedFrom: string | null | undefined;
let parsed: Visits = {};

function read(): Visits {
  try {
    const text = localStorage.getItem(KEY);
    if (parsedFrom !== undefined && text === parsedFrom) return parsed;
    const raw: unknown = JSON.parse(text ?? '{}');
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {};
    const out: Visits = {};
    for (const [k, v] of Object.entries(raw as Record<string, unknown>)) {
      const e = v as { seen?: unknown; at?: unknown } | null;
      if (e && Number.isInteger(e.seen) && (e.seen as number) > 0 && typeof e.at === 'number') out[k] = { seen: e.seen as number, at: e.at };
    }
    parsedFrom = text; parsed = out;
    return out;
  } catch { return {}; }
}

/** How many turns the person had seen when they last left this conversation; null on a first visit. */
export function visitedSeen(key: string): number | null {
  return read()[key]?.seen ?? null;
}

/** Remember that `seen` turns of `key` were seen (never fewer than before), dropping the oldest visits past VISITS_KEPT. */
export function recordVisit(key: string, seen: number, at = Date.now()): void {
  if (!key || !Number.isInteger(seen) || seen <= 0) return;
  try {
    const visits = { ...read() };
    visits[key] = { seen: Math.max(seen, visits[key]?.seen ?? 0), at };
    const keys = Object.keys(visits);
    if (keys.length > VISITS_KEPT) for (const k of keys.sort((a, b) => visits[a].at - visits[b].at).slice(0, keys.length - VISITS_KEPT)) delete visits[k];
    localStorage.setItem(KEY, JSON.stringify(visits));
    setVisitsVersion(v => v + 1);
  } catch { /* the divider is a convenience: no storage, no divider */ }
}

/** The index of the first turn that is new since `seen` turns were read, or null (a first visit, or nothing new). */
export function watermarkAt(seen: number | null, total: number): number | null {
  return seen !== null && seen > 0 && seen < total ? seen : null;
}

/** The divider's words. */
export function watermarkText(count: number): string {
  return count === 1 ? '1 new message since you were here' : `${count} new messages since you were here`;
}
