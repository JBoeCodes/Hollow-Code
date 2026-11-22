import { type Accessor, createEffect, createSignal, on } from 'solid-js';
import { motionReduced } from '../motion';

export type ExitPhase = 'open' | 'closing' | 'closed';

/// Graceful departures — mirror a boolean `open` source into a render
/// phase that holds the DOM for `ms` after close so a CSS exit
/// animation can play. Everything in Hollow enters with a spring;
/// without this, everything leaves by teleport.
///
/// Usage: render on `phase() !== 'closed'`, style the exit under an
/// `.is-closing` class (added when `phase() === 'closing'`). Exits
/// should be ~2x faster than entrances.
///
/// Races handled: reopening mid-exit cancels the hold (phase snaps back
/// to 'open'); closing while already closed is a no-op; reduced motion
/// unmounts immediately, no hold.
export function createExitPhase(open: Accessor<boolean>, ms = 150): Accessor<ExitPhase> {
  const [phase, setPhase] = createSignal<ExitPhase>(open() ? 'open' : 'closed');
  let timer: ReturnType<typeof setTimeout> | null = null;
  createEffect(on(open, (isOpen) => {
    if (timer) { clearTimeout(timer); timer = null; }
    if (isOpen) { setPhase('open'); return; }
    if (phase() === 'closed') return;
    if (motionReduced()) { setPhase('closed'); return; }
    setPhase('closing');
    timer = setTimeout(() => { setPhase('closed'); timer = null; }, ms);
  }, { defer: true }));
  return phase;
}
