import { createEffect, createRoot, createSignal } from 'solid-js';
import { api } from '../api';
import { customThemes } from './theme';
import { workspaces } from './workspaces';
import type { Look } from '../canvasLook';
type Entry = { look: Look | null; missing: boolean };
const cache = new Map<string, Entry>();
const [version, setVersion] = createSignal(0);
let listed: unknown;
createRoot(() => createEffect(() => {
  const themes = customThemes.latest;
  if (themes === listed) return;
  listed = themes;
  const old = [...cache];
  cache.clear();
  for (const [name, entry] of old) {
    if (themes?.some(t => t.name === name)) readLook(name, entry.look);
    else cache.set(name, { look: null, missing: true });
  }
  setVersion(n => n + 1);
}));

function readLook(name: string, look: Look | null = null): Entry {
  const pending = { look, missing: false };
  cache.set(name, pending);
  void Promise.all([api.readCustomTheme(name), import('../canvasLook')]).then(([theme, { lookPairs }]) => {
    if (cache.get(name) !== pending) return;
    pending.look = lookPairs(theme);
    setVersion(n => n + 1);
  }).catch(() => {
    if (cache.get(name) !== pending) return;
    pending.look = null;
    pending.missing = true;
    setVersion(n => n + 1);
  });
  return pending;
}

/** Shared by mounted frames and the picker, without suspending the composer. */
export function namedLook(name?: string | null): Entry | null {
  version();
  if (!name) return null;
  return cache.get(name) ?? readLook(name);
}
export function lookFor(workspaceId?: string | null): Look | null {
  if (!workspaceId) return null;
  return namedLook(workspaces().find(w => w.id === workspaceId)?.canvasTheme)?.look ?? null;
}
