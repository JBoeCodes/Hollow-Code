import { createEffect, createSignal, on, type Accessor } from 'solid-js';

/** The detail's local toggle, with explicit repeated deep-link requests. */
export function canvasCompareMode(request: Accessor<number>, consumed: () => void) {
  const [comparing, setComparing] = createSignal(false);
  createEffect(on(request, value => {
    if (value) { setComparing(true); consumed(); }
  }));
  return [comparing, setComparing] as const;
}
