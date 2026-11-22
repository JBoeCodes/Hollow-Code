/**
 * Stable row identity for lists fed by the backend.
 *
 * Every list in the app reads from a `createResource` whose fetcher returns
 * freshly deserialized IPC objects. Solid's `<For>` (mapArray) keys items by
 * `===`, so a refetch that changes nothing still hands `<For>` an array of
 * all-new references and it destroys and recreates every row. The live vault
 * watcher refetches on every change burst, so the cost is paid continuously:
 * DOM is thrown away, hover and focus are lost mid-pointer, and CSS
 * transitions inside a rebuilt container (a `.meter-fill` in a task tray) snap
 * instead of animating.
 *
 * `keyedList` sits between the resource and the `<For>`: it remembers the
 * object it handed out for each key and returns that same object again when
 * the new payload is structurally identical, so `<For>` sees an unchanged
 * element and leaves its DOM alone. A genuinely changed item gets the new
 * object, so the row re-renders exactly when it should.
 *
 * ## Why a closure cache and not `reconcile` from solid-js/store
 *
 * `reconcile` is a store *mutator*: it only applies through `setStore`, which
 * means every keyed resource would have to become a `createStore`, every row
 * object would become a mutating proxy, and every `items()` read site would
 * change shape. It would also drag `solid-js/store` into the node-environment
 * vitest tier. This helper is a pure function of its input with no reactive
 * dependency at all, so it composes with any accessor (resource, memo, plain
 * getter), needs zero changes at the `<For>`, and is directly unit-testable.
 */

/** Recursion cap for the payload compare. See `samePayload`. */
const MAX_DEPTH = 8;

/**
 * Structural equality for JSON-shaped payloads, with an `===` fast path at
 * every level. Wire types are plain scalars, arrays, and objects, so nothing
 * deeper is handled on purpose.
 *
 * Past `MAX_DEPTH` the compare degrades to reference equality. That is the
 * safe direction: a false "changed" only costs one row rebuild, whereas a
 * false "unchanged" would pin stale data on screen.
 */
export function samePayload(a: unknown, b: unknown, depth = 0): boolean {
  if (a === b) return true;
  if (depth >= MAX_DEPTH) return false;
  if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return false;

  const aIsArray = Array.isArray(a);
  if (aIsArray !== Array.isArray(b)) return false;

  if (aIsArray) {
    const x = a as unknown[];
    const y = b as unknown[];
    if (x.length !== y.length) return false;
    for (let i = 0; i < x.length; i++) {
      if (!samePayload(x[i], y[i], depth + 1)) return false;
    }
    return true;
  }

  const x = a as Record<string, unknown>;
  const y = b as Record<string, unknown>;
  const keys = Object.keys(x);
  if (keys.length !== Object.keys(y).length) return false;
  for (const k of keys) {
    if (!Object.prototype.hasOwnProperty.call(y, k)) return false;
    if (!samePayload(x[k], y[k], depth + 1)) return false;
  }
  return true;
}

/**
 * Wrap a list accessor so unchanged items keep their object identity.
 *
 * ```ts
 * const items = keyedList(() => tasksData()?.items, t => t.url);
 * // <For each={items()}> now patches instead of rebuilding.
 * ```
 *
 * Create it once per component instance (not inside a memo or an effect) —
 * the cache lives in the returned closure. Reading it is idempotent, so it is
 * safe to call several times inside one tracked scope; repeated calls against
 * the same source array return the same output array without rewalking.
 *
 * `keyOf` must be unique per item. When two items in one pass produce the same
 * key, the first claims the cache slot and the rest pass through untouched, so
 * two rows never alias one object (which `<For>` cannot represent).
 */
export function keyedList<T>(source: () => T[] | undefined, keyOf: (item: T) => string): () => T[] {
  let cache = new Map<string, T>();
  let lastSource: T[] | undefined;
  let lastOut: T[] = [];
  let primed = false;

  return () => {
    const next = source();
    if (primed && next === lastSource) return lastOut;

    const items = next ?? [];
    const fresh = new Map<string, T>();
    const out: T[] = new Array(items.length);
    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      const key = keyOf(item);
      if (fresh.has(key)) {
        // Duplicate key in this pass: never hand out the cached reference
        // twice, or `<For>` sees one element in two slots.
        out[i] = item;
        continue;
      }
      const prev = cache.get(key);
      const kept = prev !== undefined && samePayload(prev, item) ? prev : item;
      fresh.set(key, kept);
      out[i] = kept;
    }

    // Swapping the map (rather than deleting) is what evicts vanished keys, so
    // the cache stays the size of the live list.
    cache = fresh;
    lastSource = next;
    lastOut = out;
    primed = true;
    return out;
  };
}

/**
 * `next`, with every item structurally equal to `prev`'s item of the same key
 * replaced by that earlier object; `prev` itself when nothing changed (same
 * keys in the same order, every item kept), so a signal set with the result
 * notifies no one (overnight 10-11/12 P4). For store signals fed by a re-read
 * that usually changes nothing (the workspace list, every workspace's
 * conversations). `keep` decides an item that isn't equal as a whole: return
 * the earlier object, a new one built from both, or `item`.
 */
export function keepUnchanged<T>(prev: readonly T[], next: T[], keyOf: (item: T) => string,
  keep: (old: T, item: T) => T = (old, item) => (samePayload(old, item) ? old : item)): T[] {
  const byKey = new Map<string, T>();
  for (const item of prev) { const k = keyOf(item); if (!byKey.has(k)) byKey.set(k, item); }
  const seen = new Set<string>();
  let same = prev.length === next.length;
  const out = next.map((item, i) => {
    const k = keyOf(item);
    const old = seen.has(k) ? undefined : byKey.get(k);
    seen.add(k);
    const kept = old === undefined ? item : keep(old, item);
    if (kept !== prev[i]) same = false;
    return kept;
  });
  return same ? (prev as T[]) : out;
}

/** [`keepUnchanged`] for a record keyed by id: `prev` when every entry is
 *  kept and no key came or went. */
export function keepUnchangedRecord<T>(prev: Readonly<Record<string, T>>, next: Record<string, T>,
  keep: (old: T, item: T) => T = (old, item) => (samePayload(old, item) ? old : item)): Record<string, T> {
  const keys = Object.keys(next);
  let same = keys.length === Object.keys(prev).length;
  const out: Record<string, T> = {};
  for (const k of keys) {
    const old = Object.prototype.hasOwnProperty.call(prev, k) ? prev[k] : undefined;
    out[k] = old === undefined ? next[k] : keep(old, next[k]);
    if (out[k] !== old) same = false;
  }
  return same ? (prev as Record<string, T>) : out;
}
