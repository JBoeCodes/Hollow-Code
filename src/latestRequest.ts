/**
 * Request identity: only the latest request may answer (plan L3 X21a,
 * 2026-10-15). The one way an async view drops a stale response — rapid
 * typing, a re-selected row, a slow read overtaken by a newer one, or a
 * response that lands after the view unmounted. Before this each view kept
 * its own `const mine = ++gen; … if (mine !== gen) return` counter;
 * `scripts/check-latest-request.mjs` keeps the migrated views free of them.
 *
 *   const reads = latestRequest();
 *   async function load() {
 *     const ticket = reads.start();
 *     try { const v = await api.read(); if (reads.isLatest(ticket)) setValue(v); }
 *     catch (e) { if (reads.isLatest(ticket)) setError(String(e)); }
 *   }
 *   onCleanup(() => reads.cancel());
 *
 * Pure: no Solid, no DOM, so the node unit tests load it directly. Moved
 * here from `searchState.ts` (`requestOrder`, audit H09; `runLatest`, P07).
 */
export interface LatestRequest {
  /** Start a request: its ticket supersedes every earlier one. */
  start(): number;
  /** True while `ticket` is the newest request and nothing cancelled it. */
  isLatest(ticket: number): boolean;
  /** The current ticket, without superseding anything: work that belongs to
   *  whatever request is current (a save against the file that is open)
   *  checks it later with `isLatest`, and any newer start or cancel drops it. */
  peek(): number;
  /** Supersede everything in flight (the view unmounted, the input cleared). */
  cancel(): void;
}

export function latestRequest(): LatestRequest {
  let latest = 0;
  return {
    start: () => ++latest,
    isLatest: ticket => ticket === latest,
    peek: () => latest,
    cancel: () => { latest++; },
  };
}

/**
 * Run one request under `order` and hand `deliver` its outcome only if it
 * is still the latest (next-level P07): rapid typing fires many lookups, and
 * whichever answers last must not win unless it was asked last. Returns the
 * ticket, so a caller can tell its own request apart.
 */
export function runLatest<T>(order: LatestRequest, request: () => Promise<T>, deliver: (outcome: { ok: true; value: T } | { ok: false; error: unknown }) => void): number {
  const ticket = order.start();
  request().then(
    value => { if (order.isLatest(ticket)) deliver({ ok: true, value }); },
    error => { if (order.isLatest(ticket)) deliver({ ok: false, error }); },
  );
  return ticket;
}

/**
 * Whether what `read` names is still the same (lane S re-review of #1304):
 * captured now, checked after an await. A confirm dialog stays open while a
 * shortcut or an agent moves the chat on screen, and the answer must not land
 * on the chat it moved to. `read` lists identities, compared one by one with
 * `Object.is`. Pure, like the rest of this module.
 *
 *   const still = unchangedSince(() => [chatAgentSessionKey(), chatConversationVersion()]);
 *   if (!(await confirm()) || !still()) return;
 */
export function unchangedSince(read: () => readonly unknown[]): () => boolean {
  const at = read();
  return () => {
    const now = read();
    return now.length === at.length && now.every((v, i) => Object.is(v, at[i]));
  };
}
