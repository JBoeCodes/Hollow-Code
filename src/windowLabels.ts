// The saved automation-window labels (overnight Spare S10), once something
// read `automation_windows` (Settings, Background work windows, the Automations
// page when it opens, the Main agent's queue line): `describeSchedule` names
// edited hours and the person's own windows as saved, not as shipped. A
// signal, so a list already on screen re-renders when they arrive. Tiny on
// purpose: the main bundle's store writes it and lazy chunks read it.

import { createSignal } from 'solid-js';

const [saved, setSaved] = createSignal<Record<string, string>>({});

export function rememberWindowLabels(windows: readonly { name: string; label: string }[]) {
  const next = Object.fromEntries(windows.map(w => [w.name, w.label]));
  // Unchanged labels keep the old object, so nothing re-renders for nothing.
  setSaved(prev => (JSON.stringify(prev) === JSON.stringify(next) ? prev : next));
}

export function savedWindowLabel(name: string): string | undefined {
  const all = saved();
  return Object.prototype.hasOwnProperty.call(all, name) ? all[name] : undefined;
}
