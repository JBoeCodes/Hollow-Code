import { Show, createEffect, createSignal, type JSX } from 'solid-js';

/**
 * Mounts its (lazy) children the first time `when` is true and keeps them
 * mounted after, so the surface's own exit animation and state still work
 * (next-level P21: sheets and overlays that aren't on screen at launch stay
 * out of the main bundle). The child reads the same open signal and renders
 * itself; this only decides when its code is first needed. No Suspense
 * boundary, so the surface's own resources never hold it offscreen.
 */
export function OnceOpened(props: { when: boolean; children: JSX.Element }) {
  const [seen, setSeen] = createSignal(false);
  createEffect(() => { if (props.when) setSeen(true); });
  return <Show when={seen() || props.when}>{props.children}</Show>;
}

/** Fetch lazy chunks once the shell is idle, so the first open is instant. */
export function preloadWhenIdle(loaders: (() => Promise<unknown>)[]) {
  const run = () => { for (const load of loaders) void load().catch(() => { /* loads again on first open */ }); };
  const idle = (window as Window & { requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => number }).requestIdleCallback;
  if (idle) idle(run, { timeout: 4000 }); else setTimeout(run, 1500);
}
