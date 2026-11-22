// A finished background tracker sync pass (moved here on 2026-11-10 from the
// removed Notion sync's pass reader). The Rust tracker sync emits
// `tracker-sync-updated` with `{ok}` after every pass, even a no-change one
// that writes no mirror files; `store/runtime.ts` keeps the latest so pages
// holding a sync result of their own can refresh. Pure, so the node vitest
// suite can load it.

/** One finished pass: when this window heard of it (ms) and whether it worked. */
export interface SyncPass { at: number; ok: boolean }

/** The event's payload read defensively: anything but `{ok: true}` is a failed pass. */
export function passFromEvent(payload: unknown, at: number): SyncPass {
  const ok = typeof payload === 'object' && payload !== null && (payload as { ok?: unknown }).ok === true;
  return { at, ok };
}
