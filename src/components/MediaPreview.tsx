import { Show, createEffect, createSignal, on, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { flashFooter } from '../store';
import type { MediaGrant } from '../types';
import {
  classifyFailure, mediaSrc, onMediaSlotFreed, opensInQuickTime, problemCopy, problemHasActions,
  refusalOf, registerPlayer, releaseMediaSlot, takeMediaSlot,
  type MediaProblem, type MediaSlot,
} from '../mediaKinds';

/** How long a player waits for metadata before it checks what went wrong. */
const LOAD_TIMEOUT_MS = 8000;

function fmtSize(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/**
 * The one video player (inline video, 2026-10-17): the side pane (`pane`),
 * a Results file (`result`) and the strip under a reply (`inline`) all use
 * it. It asks `media_grant` for a token and plays it from the `hollow-media`
 * scheme (media.rs), with the native controls (play, scrub, volume,
 * fullscreen), no autoplay and no loop.
 *
 * - Every `<video>` takes a slot from one app-wide budget (mediaKinds.ts):
 *   an `inline` player mounts only on screen and when a slot is free, else it
 *   stays a card with a Play button; an explicit open (pane, result, a clicked
 *   card) always mounts. An `inline` player that leaves the screen while
 *   paused frees its decoder, grant and slot, and comes back where it was.
 * - Starting one player pauses the others.
 * - The page can't see a media response's status, so on `error` it asks
 *   for a grant again and compares with the grant that failed: a changed
 *   file gets one retry, then "still being written"; an unchanged one that
 *   WebKit can't decode is a format problem. No metadata after 8 s checks the
 *   same way, but an unchanged file without an error keeps loading and says
 *   so. It never stays a black box.
 * - The grant is released on unmount, on a path change and on `nonce`
 *   (the pane's Refresh). The token never leaves this component.
 */
export function MediaPreview(props: {
  path: string;
  name?: string;
  variant: 'inline' | 'pane' | 'result';
  sessionKey?: string | null;
  nonce?: number;
  onOpenPane?: () => void;
}) {
  const name = () => props.name ?? props.path.slice(props.path.lastIndexOf('/') + 1);
  const [grant, setGrant] = createSignal<MediaGrant | null>(null);
  const [problem, setProblem] = createSignal<MediaProblem | null>(null);
  const [mounted, setMounted] = createSignal(false);
  const [size, setSize] = createSignal<number | null>(null);
  const [slow, setSlow] = createSignal(false);
  /** The card's poster frame (inline only), asked for once on screen. */
  const [poster, setPoster] = createSignal<string | null>(null);
  let posterAsked = false;
  let lastPath = props.path;
  let lastSession = props.sessionKey;
  let posterGeneration = 0;

  let figure!: HTMLElement;
  let video: HTMLVideoElement | undefined;
  let explicit = props.variant !== 'inline';
  let onScreen = props.variant !== 'inline';
  let visibleAt = 0;
  let resumeAt = 0;
  let playWhenReady = false;
  let retried = false;
  /** The generation a check is running for (-1: none), so a check left
   *  over from an earlier file never blocks this one's. */
  let checking = -1;
  let generation = 0;
  let loadTimer: ReturnType<typeof setTimeout> | undefined;
  let waiting: (() => void) | null = null;

  const slot: MediaSlot = {
    get explicit() { return explicit; },
    paused: () => !video || video.paused,
    lastVisible: () => visibleAt,
    evict: () => unmount(true),
  };

  function stopWaiting() {
    waiting?.();
    waiting = null;
  }

  /** The card's poster, once per path, only when the card shows (waiting for
   *  a slot, or put back off screen or by an explicit open elsewhere). */
  function askPoster() {
    if (posterAsked || props.variant !== 'inline') return;
    posterAsked = true;
    const asked = props.path;
    const askedSession = props.sessionKey, stamp = posterGeneration;
    void api.mediaPoster(asked, askedSession).then(url => { if (stamp === posterGeneration && asked === props.path && askedSession === props.sessionKey) setPoster(url); }, () => {});
  }

  async function mount() {
    if (mounted() || problem()) return;
    if (!takeMediaSlot(slot)) {
      askPoster();
      // Full: try again when a player frees its slot.
      waiting ??= onMediaSlotFreed(() => { if (onScreen) void mount(); });
      return;
    }
    stopWaiting();
    setMounted(true);
    const mine = ++generation;
    try {
      const g = await api.mediaGrant(props.path, props.sessionKey);
      if (mine !== generation) { void api.mediaRelease(g.token); return; }
      setSize(g.size);
      setGrant(g);
    } catch (err) {
      if (mine !== generation) return;
      setProblem(refusalOf(err) ?? 'failed');
      unmount(false);
    }
  }

  /** Back to the card (or the problem panel): free the decoder, the grant
   *  and the slot. `keepTime` remembers where playback was. */
  function unmount(keepTime: boolean) {
    generation++;
    clearTimeout(loadTimer);
    setSlow(false);
    if (keepTime) askPoster();
    if (video) {
      if (keepTime) resumeAt = video.currentTime;
      video.pause();
      video.removeAttribute('src');
      video.load();
    }
    const g = grant();
    if (g) void api.mediaRelease(g.token);
    setGrant(null);
    if (mounted()) {
      setMounted(false);
      releaseMediaSlot(slot);
    }
  }

  /** Something failed (`error`), or metadata is late (`timeout`): grant
   *  again and compare with the grant that failed (see the component docs).
   *  One check at a time. A late load of an unchanged file isn't a failure:
   *  it keeps waiting and says it's slow (an iCloud file still downloading,
   *  a network volume, a busy read queue). */
  async function classify(why: 'error' | 'timeout') {
    const current = grant();
    const mine = generation;
    if (!current || checking === mine) return;
    checking = mine;
    clearTimeout(loadTimer);
    try {
      let again: MediaGrant;
      try {
        again = await api.mediaGrant(props.path, props.sessionKey);
      } catch (err) {
        if (mine !== generation) return;
        setProblem(refusalOf(err) ?? 'failed');
        unmount(false);
        return;
      }
      if (mine !== generation) { void api.mediaRelease(again.token); return; }
      // Read the element after the wait: it may have failed meanwhile (that
      // `error` call found this check running and returned).
      const errored = !!video?.error;
      const notSupported = video?.error?.code === 4; // MEDIA_ERR_SRC_NOT_SUPPORTED
      const verdict = classifyFailure(current, again, notSupported, retried);
      if (verdict === 'retry') {
        retried = true;
        void api.mediaRelease(current.token);
        setSize(again.size);
        setSlow(false);
        setGrant(again);
        return;
      }
      void api.mediaRelease(again.token);
      if (why === 'timeout' && !errored) {
        setSlow(true);
        loadTimer = setTimeout(() => { if (!video || video.readyState < 1) void classify('timeout'); }, LOAD_TIMEOUT_MS);
        return;
      }
      setProblem(verdict);
      unmount(false);
    } finally {
      if (checking === mine) checking = -1;
    }
  }

  // A new path or a Refresh starts over.
  createEffect(on(() => [props.path, props.nonce, props.sessionKey] as const, () => {
    stopWaiting();
    unmount(false);
    setProblem(null);
    retried = false;
    resumeAt = 0;
    if (props.path !== lastPath || props.sessionKey !== lastSession) {
      lastPath = props.path; lastSession = props.sessionKey; posterGeneration++;
      setPoster(null); posterAsked = false;
    }
    if (onScreen) void mount();
  }));

  // Each new source gets the metadata timeout.
  createEffect(on(grant, g => {
    clearTimeout(loadTimer);
    if (g) loadTimer = setTimeout(() => { if (!video || video.readyState < 1) void classify('timeout'); }, LOAD_TIMEOUT_MS);
  }));

  onMount(() => {
    if (props.variant !== 'inline') return;
    if (typeof IntersectionObserver === 'undefined') {
      onScreen = true;
      void mount();
      return;
    }
    const observer = new IntersectionObserver(entries => {
      const entry = entries[entries.length - 1];
      if (entry.isIntersecting) {
        onScreen = true;
        visibleAt = performance.now();
        if (!mounted() && !problem()) void mount();
      } else {
        onScreen = false;
        if (mounted() && (!video || video.paused)) unmount(true);
      }
    }, { rootMargin: '200px' });
    observer.observe(figure);
    onCleanup(() => observer.disconnect());
  });

  onCleanup(() => {
    posterGeneration++;
    stopWaiting();
    unmount(false);
  });

  function playFromCard() {
    explicit = true;
    playWhenReady = true;
    setProblem(null);
    void mount();
  }

  function retry() {
    stopWaiting();
    unmount(false);
    retried = false; playWhenReady = false; explicit = true;
    setProblem(null);
    void mount();
  }

  function onMetadata() {
    clearTimeout(loadTimer);
    setSlow(false);
    if (video && resumeAt > 0) video.currentTime = resumeAt;
    if (playWhenReady && video) {
      playWhenReady = false;
      void video.play().catch(() => { /* the user presses play */ });
    }
  }

  async function open(reveal: boolean) {
    try {
      const how = await api.chatPreviewOpen(props.path, reveal, !reveal && opensInQuickTime(props.path) ? 'quicktime' : undefined);
      if (how === 'revealed' && !reveal) flashFooter('Shown in Finder', 3000);
    } catch (err) {
      flashFooter(`× open failed: ${err instanceof Error ? err.message : String(err)}`, 2500);
    }
  }

  const openLabel = () => (opensInQuickTime(props.path) ? 'Open in QuickTime Player' : 'Open in default app');

  const actions = () => (
    <>
      <button type="button" class="btn small" onClick={() => void open(false)}>{openLabel()}</button>
      <button type="button" class="btn small" onClick={() => void open(true)}>Reveal in Finder</button>
    </>
  );

  return (
    <figure ref={figure} class={`media-preview media-preview-${props.variant}`} data-media-path={props.path}>
      <Show when={problem()} fallback={
        <Show when={mounted()} fallback={
          <div class="media-preview-card">
            <Show when={poster()}>{url => <img class="media-preview-poster" src={url()} alt="" />}</Show>
            <button type="button" class="media-preview-play" aria-label={`Play ${name()}`} onClick={playFromCard}>
              <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><polygon points="7 4 20 12 7 20 7 4" fill="currentColor" /></svg>
            </button>
          </div>
        }>
          <video
            ref={el => { video = el; onCleanup(registerPlayer(el)); onCleanup(() => { video = undefined; }); }}
            class="media-preview-video"
            controls
            preload="metadata"
            playsinline
            aria-label={`Video: ${name()}`}
            src={grant() ? `${mediaSrc(grant()!.token)}#t=0.001` : undefined}
            onLoadedMetadata={onMetadata}
            onError={() => void classify('error')}
          />
        </Show>
      }>
        {p => (
          <div class="media-preview-problem" role="status">
            <p>{problemCopy(p())}</p>
            <div class="media-preview-actions">
              <Show when={p() === 'changed' || p() === 'failed'}><button type="button" class="btn small" onClick={retry}>Try again</button></Show>
              <Show when={problemHasActions(p())}>{actions()}</Show>
              <Show when={props.variant === 'inline' && props.onOpenPane}>
                <button type="button" class="btn small" onClick={() => props.onOpenPane?.()}>Open in Preview pane</button>
              </Show>
            </div>
          </div>
        )}
      </Show>
      <figcaption class="media-preview-caption">
        <span class="media-preview-name" title={props.path}>{name()}</span>
        <Show when={size() !== null}><span class="media-preview-size">{fmtSize(size()!)}</span></Show>
        <Show when={slow() && !problem()}><span class="media-preview-slow" role="status">Still loading…</span></Show>
        <Show when={props.variant !== 'pane' && !problem()}>
          <span class="media-preview-actions">
            {actions()}
            <Show when={props.variant === 'inline' && props.onOpenPane}>
              <button type="button" class="btn small" onClick={() => props.onOpenPane?.()}>Open in Preview pane</button>
            </Show>
          </span>
        </Show>
      </figcaption>
    </figure>
  );
}
