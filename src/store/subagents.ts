// A chat's subagents, read for its Subagents chip, its tab and its
// transcript's subagent lines (COO rebuild L3): one shared read per chat
// (`session_subagents`, L2), polled every 3 s while one is active, else
// every 20 s, and again soon after any run-queue change (a start, a turn
// ending). Only while something on screen uses it and the window isn't
// hidden; the last view stays cached for the next one that does.
import { createEffect, createRoot, createSignal, on, onCleanup, type Accessor, type Setter } from 'solid-js';
import { api } from '../api';
import { runQueue } from './runQueue';
import { OLDER_BATCH, pollMs } from '../subagentsModel';
import type { SubagentsView } from '../generated/ipc';

interface Entry {
  view: Accessor<SubagentsView | null>;
  setView: Setter<SubagentsView | null>;
  error: Accessor<string | null>;
  setError: Setter<string | null>;
  users: number;
  seq: number;
  /** How many older finished subagents "Show older" has paged in. */
  older: Accessor<number>;
  setOlder: Setter<number>;
  timer?: ReturnType<typeof setTimeout>;
}
const entries = new Map<string, Entry>();

function entryOf(key: string): Entry {
  let e = entries.get(key);
  if (!e) {
    const [view, setView] = createSignal<SubagentsView | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [older, setOlder] = createSignal(0);
    e = { view, setView, error, setError, users: 0, seq: 0, older, setOlder };
    entries.set(key, e);
  }
  return e;
}

async function read(key: string): Promise<void> {
  const e = entryOf(key);
  if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
  const mine = ++e.seq;
  try {
    const next = await api.sessionSubagents(key, e.older());
    if (mine === e.seq) { e.setView(next); e.setError(null); }
  } catch (err) {
    if (mine === e.seq) e.setError(String(err));
  }
}

function schedule(key: string) {
  const e = entryOf(key);
  clearTimeout(e.timer);
  if (e.users <= 0) return;
  e.timer = setTimeout(() => void read(key).then(() => schedule(key)), pollMs(e.view()));
}

function acquire(key: string) {
  const e = entryOf(key);
  e.users++;
  if (e.users === 1) void read(key).then(() => schedule(key));
}

function release(key: string) {
  const e = entryOf(key);
  e.users = Math.max(0, e.users - 1);
  if (e.users === 0) clearTimeout(e.timer);
}

// The queue changes in bursts: every chat in use reads once, half a second
// after the last change.
let soon: ReturnType<typeof setTimeout> | undefined;
createRoot(() => createEffect(on(runQueue, () => {
  clearTimeout(soon);
  soon = setTimeout(() => { for (const [key, e] of entries) if (e.users > 0) void read(key); }, 500);
}, { defer: true })));

/** The subagents of `parent()` while the caller is mounted. */
export function createSubagentsRead(parent: Accessor<string | null | undefined>) {
  createEffect(on(parent, key => {
    if (!key) return;
    acquire(key);
    onCleanup(() => release(key));
  }));
  const view = () => { const key = parent(); return key ? entryOf(key).view() : null; };
  const error = () => { const key = parent(); return key ? entryOf(key).error() : null; };
  const refresh = () => { const key = parent(); return key ? read(key) : Promise.resolve(); };
  /** Show older: pages in the next batch of the chain's older finished subagents. */
  const showOlder = () => {
    const key = parent();
    if (!key) return Promise.resolve();
    const e = entryOf(key);
    e.setOlder(n => n + OLDER_BATCH);
    return read(key);
  };
  return { view, error, refresh, showOlder };
}

/** A clock for live elapsed: ticks every second while `live()` holds. */
export function createSecondClock(live: Accessor<boolean>): Accessor<number> {
  const [now, setNow] = createSignal(Date.now());
  const [visible, setVisible] = createSignal(typeof document === 'undefined' || document.visibilityState !== 'hidden');
  if (typeof document !== 'undefined') {
    const changed = () => setVisible(document.visibilityState !== 'hidden');
    document.addEventListener('visibilitychange', changed);
    onCleanup(() => document.removeEventListener('visibilitychange', changed));
  }
  createEffect(on(() => live() && visible(), ticking => {
    setNow(Date.now());
    if (!ticking) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    onCleanup(() => clearInterval(timer));
  }));
  return now;
}
