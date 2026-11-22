import { createEffect, createSignal, type Resource, type ResourceFetcherInfo } from 'solid-js';

/**
 * A {@link preserveDuringRefetch} read. `refreshFailed` is true while the
 * resource's last read failed but an earlier value is still on screen, so a
 * surface can say it is showing what was there before (X21); it clears when
 * a later read succeeds. `components/RefreshNotice.tsx` shows it.
 */
export type StableResource<T> = Resource<T> & { readonly refreshFailed: boolean };

/**
 * Keep the last resolved value visible while a Solid resource refetches.
 *
 * A resource read under the app-level Suspense boundary can otherwise replace
 * the entire active page with the fallback during a live vault refresh. The
 * `latest` accessor is explicitly non-suspending; initial loads still use the
 * normal accessor so first-open loading behavior is unchanged.
 */
export function preserveDuringRefetch<T>(resource: Resource<T>): StableResource<T> {
  // Never throws (plan completion L5, found by the `unavailable` fixture): an
  // errored Solid resource throws on read, and with no ErrorBoundary that
  // left Tasks on "Loading page" forever with an uncaught error when the task
  // list couldn't be read. Once errored this returns the last good value (or
  // `undefined` if there never was one). A failed refresh is not silent:
  // `refreshFailed` says the value on screen is the last good one (X21), and
  // Solid keeps `error` set through a Retry until a read succeeds.
  let lastGood: T | undefined;
  const stable = (() => {
    if (resource.state === 'errored') return lastGood;
    const value = resource.state === 'refreshing' ? resource.latest : resource();
    if (value !== undefined) lastGood = value;
    return value;
  }) as StableResource<T>;
  Object.defineProperties(stable, {
    state: { get: () => resource.state },
    loading: { get: () => resource.loading },
    error: { get: () => resource.error },
    latest: { get: () => resource.latest },
    refreshFailed: { get: () => resource.error !== undefined && resource.error !== null && lastGood !== undefined },
  });
  return stable;
}

/**
 * True only for a genuine first load — when there is no previous value to keep
 * on screen and a fallback is therefore honest.
 *
 * Use this instead of `resource.loading` to decide whether to show a placeholder
 * next to a `preserveDuringRefetch` read: `loading` is also true while
 * refreshing, so it would blank a row that still has perfectly good data behind
 * it, which is the whole thing the wrapper exists to prevent.
 */
export function isFirstLoad(resource: Resource<unknown>): boolean {
  return resource.state === 'pending' || resource.state === 'unresolved';
}

/**
 * The resource's value, or `undefined` once it has errored (B·F08).
 *
 * Reading an errored Solid resource — `r()` or `r.latest` — throws and replaces
 * its view with the surrounding error boundary. Read through this and show
 * `r.error` locally instead, preserving the view's other blocks.
 */
export function unlessErrored<T>(resource: { (): T | undefined; error: unknown }): T | undefined {
  return resource.error ? undefined : resource();
}

/** {@link unlessErrored} for the non-suspending `latest` value. */
export function latestUnlessErrored<T>(resource: { latest: T | undefined; error: unknown }): T | undefined {
  return resource.error ? undefined : resource.latest;
}

/**
 * A resource's latest value and loading flag, read without ever suspending.
 *
 * For a view that draws its own loading, empty and failure states but now sits
 * inside a page's Suspense boundary (a former overlay that became a page view,
 * consolidation lane B): reading `latest` before the first value arrives would
 * swap the whole view for the page's fallback, so a hung read would hide
 * everything else the view knows. The value is mirrored by an effect (effects
 * never suspend); the first frame sees `undefined` with `loading` true.
 */
export function unsuspended<T>(resource: Resource<T>): { readonly latest: T | undefined; readonly loading: boolean } {
  const [value, setValue] = createSignal<T | undefined>(undefined, { equals: false });
  createEffect(() => {
    if (resource.state === 'ready' || resource.state === 'refreshing') setValue(() => resource.latest);
  });
  return {
    get latest() { return value(); },
    get loading() { return resource.loading; },
  };
}

/**
 * A resource fetcher that a save can answer (H6 follow-up, 2026-10-15).
 *
 * Solid applies only a resource's newest load, but `mutate` is not a load: a
 * re-read already in flight when the person's save answers (an agent changed
 * a setting meanwhile, `store/settingsRevision.ts`) lands after the save and
 * puts the older value back. Save with `refetch(saved)` instead of
 * `mutate(saved)`: this fetcher answers that refetch with the saved value at
 * once, which makes it the newest load, so Solid drops every read still in
 * flight (its own latest-load rule, the same answer as `latestRequest.ts`).
 * A plain `refetch()` (`refetching` true) or a key change (false) reads.
 * Accepted: an agent write that lands after the save, whose re-read started
 * before the save's reply, is dropped too and shows the person's value until
 * the next read (no revision on the value to tell them apart).
 */
export function savedOrRead<T extends object>(read: () => Promise<T>) {
  return (_key: unknown, info: ResourceFetcherInfo<T, T>): T | Promise<T> =>
    typeof info.refetching === 'object' ? info.refetching : read();
}
