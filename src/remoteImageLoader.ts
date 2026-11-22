import { createEffect, createSignal, onCleanup, type Accessor } from 'solid-js';
import { api } from './api';
import { classifyImageUrl } from './remoteImages';

/**
 * The runtime half of click-to-load remote images (merged plan slice 1,
 * reviews A·S3 / B·F13; the pure rewrite is `remoteImages.ts`). The main
 * window's CSP has no remote `img-src`, so every remote image comes through
 * the backend's `fetch_remote_image` as a `data:` URL and is cached here for
 * the session: once you load an image it stays loaded across re-renders and
 * in every other view that shows it.
 */

/** Bounded so a long session of loaded images can't hold unbounded memory. */
const MAX_CACHED = 64;
const loaded = new Map<string, string>();
const inflight = new Map<string, Promise<string>>();
const [version, setVersion] = createSignal(0);

/** Bumps when an image finishes loading; renderers read it to re-render. */
export const remoteImagesVersion: Accessor<number> = version;

/** The `data:` URL for a remote image the user already loaded, if any. */
export function loadedRemoteImage(url: string): string | undefined {
  return loaded.get(url);
}

function fetchThroughBackend(url: string, cache: Map<string, string>): Promise<string> {
  const hit = cache.get(url);
  if (hit) return Promise.resolve(hit);
  const key = `${cache === loaded ? 'img' : 'icon'}:${url}`;
  let pending = inflight.get(key);
  if (!pending) {
    pending = api.fetchRemoteImage(url)
      .then(data => {
        if (!/^data:image\//i.test(data)) throw new Error('That is not an image.');
        cache.delete(url);
        cache.set(url, data);
        while (cache.size > MAX_CACHED) cache.delete(cache.keys().next().value!);
        return data;
      })
      .finally(() => inflight.delete(key));
    inflight.set(key, pending);
  }
  return pending;
}

/** Load a remote image the user asked for, then re-render the views showing it. */
export async function loadRemoteImage(url: string): Promise<string> {
  const data = await fetchThroughBackend(url, loaded);
  setVersion(v => v + 1);
  return data;
}

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * Delegated click handler for a rendered-markdown root: a click on a
 * `.md-remote-img` placeholder loads that image. Returns true when it handled
 * the click (the caller should do nothing else with it).
 */
export function handleRemoteImageClick(e: MouseEvent): boolean {
  const btn = (e.target as HTMLElement | null)?.closest?.('button.md-remote-img') as HTMLButtonElement | null;
  if (!btn) return false;
  // A placeholder inside a link must not also follow the link.
  e.preventDefault();
  e.stopPropagation();
  const url = btn.dataset.imageUrl;
  if (!url || btn.disabled) return true;
  const label = btn.querySelector('.md-remote-img-label');
  const host = classifyImageUrl(url);
  const hostName = host.kind === 'remote' ? host.host : 'the web';
  btn.disabled = true;
  if (label) label.textContent = `Loading image from ${hostName}…`;
  loadRemoteImage(url).catch(err => {
    // The view re-renders on success; on failure the placeholder stays and
    // says why, and a second click retries.
    btn.disabled = false;
    btn.title = errText(err);
    if (label) label.textContent = `Couldn’t load image from ${hostName} — Retry`;
  });
  return true;
}

const iconCache = new Map<string, string>();
/** Icons that failed once this session; not retried on every mount. */
const iconFailed = new Set<string>();

/**
 * The app's own icon art — connector favicons, GitHub owner avatars — which
 * loads without a click because Hollow chose the URL, not the content. It
 * still comes through the backend, since the CSP allows no remote images.
 * Returns null while loading or after a failure, so callers show their
 * fallback.
 */
export function createIconSrc(src: Accessor<string | null | undefined>): Accessor<string | null> {
  const [resolved, setResolved] = createSignal<string | null>(null);
  createEffect(() => {
    const url = src();
    if (!url) { setResolved(null); return; }
    const kind = classifyImageUrl(url);
    if (kind.kind === 'local') { setResolved(url); return; }
    if (kind.kind !== 'remote' || iconFailed.has(kind.url)) { setResolved(null); return; }
    const hit = iconCache.get(kind.url);
    if (hit) { setResolved(hit); return; }
    setResolved(null);
    let live = true;
    onCleanup(() => { live = false; });
    fetchThroughBackend(kind.url, iconCache).then(
      data => { if (live) setResolved(data); },
      () => { iconFailed.add(kind.url); },
    );
  });
  return resolved;
}
