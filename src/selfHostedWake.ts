/**
 * Self-hosted models Q6: waking a chat's self-hosted model, and the words of
 * the start chip above its composer (`components/SelfHostedStatus.tsx`).
 * A model on Cloud Run sleeps after about 10 idle minutes, takes 4 to 6 to
 * start, and costs money while up, so only the person wakes it: a keystroke
 * or a send in a chat on it (at most once per 2 minutes per entry, `nudge`;
 * typing a `/` command doesn't count), or the chip's Start now (`startNow`).
 * Opening, focusing, scrolling or reading a chat never does: mounting the
 * chip only reads the backend's last wake (`watch`, from memory). State is
 * kept per entry, so two chats on one model share one start. The backend
 * holds the same 2-minute rule and joins a wake that is running
 * (`self_hosted_wake.rs`); every answer says how long ago its wake started,
 * and the times here are anchored to that start.
 *
 * `createWakes` and `createStatusView` take the wake and status calls and
 * the clock, so the node vitest suite runs them without Tauri or a DOM;
 * `store/selfHostedWake.ts` is the app's instance.
 */
import { createEffect, createMemo, createSignal, on, onCleanup, type Accessor } from 'solid-js';
import type { SelfHostedWake, SelfHostedWakeStatus } from './generated/ipc';
import { formatMs } from './selfHostedModels';

/** One wake per entry this often, at most, from keystrokes and sends. */
export const WAKE_EVERY_MS = 2 * 60_000;
/** An idle model sleeps after about this long, so Ready (and an error)
 *  lasts this long. */
export const IDLE_MS = 10 * 60_000;
/** A wake shows "Checking" this long before it says "starting" (an awake
 *  model answers in about 40 ms). */
export const SLOW_PING_MS = 3_000;
/** Past this, a start is taking longer than usual (Q0's took 272 s). */
export const LONG_START_MS = 6 * 60_000;
/** A start this window didn't send (found by `watch`) is read again this
 *  often; after a failed read, twice as long each time, up to `MAX_POLL_MS`. */
export const POLL_MS = 2_000;
export const MAX_POLL_MS = 30_000;
/** A start found by a read is shown at most this long after it began: the
 *  backend gives up on a wake at 16 minutes. */
export const ADOPTED_LIMIT_MS = 17 * 60_000;
/** After a refusal (the entry is missing or off), keystrokes ask again only
 *  this long after it; Start now asks at once. */
export const REFUSAL_PAUSE_MS = 10_000;

/** One entry's wake, as the chip reads it. */
export interface EntryWake {
  /** When the wake in flight started, or null. */
  sending: number | null;
  /** When the last wake started: the 2-minute throttle counts from here. */
  askedAt: number | null;
  /** The last answer. `refused`: the backend refused the call (no wake). */
  last: { at: number; ok: boolean; ms: number; error: string | null; refused?: boolean } | null;
}

export const NO_WAKE: EntryWake = { sending: null, askedAt: null, last: null };

export type WakeChipKind = 'idle' | 'checking' | 'starting' | 'ready' | 'error';

export interface WakeChip {
  kind: WakeChipKind;
  /** The state in words (the live region). */
  text: string;
  /** While starting: the time so far, as text. */
  elapsed?: string;
  /** Ready: how long the model took. Error: when Start now works again. */
  note?: string;
  /** Start now: shown, shown but waiting out the 2 minutes, or hidden. */
  startNow: 'show' | 'wait' | 'hide';
  /** The chip's tooltip. */
  title: string;
}

/** "0 s", "42 s", "4 min 5 s": whole seconds, for a counter. */
export function elapsedText(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(seconds / 60);
  if (!minutes) return `${seconds} s`;
  return seconds % 60 ? `${minutes} min ${seconds % 60} s` : `${minutes} min`;
}

/** Whether typing `draft` in the composer wakes the model: not while it is
 *  a `/` command (`/model`, `/new`…), which Hollow may handle itself (sending
 *  one to the agent still wakes it), nor an emptied box. */
export function typingWakes(draft: string): boolean {
  const text = draft.trimStart();
  return text.length > 0 && !text.startsWith('/');
}

const IDLE_TITLE = 'Hollow starts it when you type or send a message here, or press Start now; never on its own. A start after about 10 idle minutes takes 4 to 6 minutes, and a message sent meanwhile waits for it.';

/** The chip for entry `name` at `now`. */
export function wakeChip(name: string, wake: EntryWake, now: number): WakeChip {
  const { sending, askedAt, last } = wake;
  const fresh = !!last?.ok && now - last.at < IDLE_MS;
  if (sending != null && now - sending < SLOW_PING_MS) {
    // Every wake gets the grace: an awake model answers well inside it.
    if (fresh) return { kind: 'ready', text: `${name} is ready`, note: `Answered in ${formatMs(last!.ms)}`, startNow: 'hide', title: 'It sleeps again after about 10 idle minutes.' };
    return { kind: 'checking', text: `Checking ${name}…`, startNow: 'hide', title: 'Hollow is asking whether the model is up.' };
  }
  if (sending != null) {
    const long = now - sending >= LONG_START_MS;
    return {
      kind: 'starting',
      text: long ? `${name} is still starting, usually 4 to 6 minutes` : `${name} is starting, about 4 minutes`,
      elapsed: `${elapsedText(now - sending)} so far`,
      startNow: 'hide',
      title: 'A message you send now waits for it, with no error.',
    };
  }
  if (last && !last.ok && now - last.at < IDLE_MS) {
    const wait = last.refused || askedAt == null ? 0 : WAKE_EVERY_MS - (now - askedAt);
    return {
      kind: 'error',
      text: `${name} didn't start. ${last.error ?? 'The model gave no answer.'}`,
      note: wait > 0 ? `Start now works again in ${elapsedText(wait)}` : undefined,
      startNow: wait > 0 ? 'wait' : 'show',
      title: 'Hollow starts a model at most once every 2 minutes.',
    };
  }
  if (fresh) {
    return { kind: 'ready', text: `${name} is ready`, note: `Answered in ${formatMs(last!.ms)}`, startNow: 'hide', title: 'It sleeps again after about 10 idle minutes.' };
  }
  return { kind: 'idle', text: `${name} sleeps when idle`, startNow: 'show', title: IDLE_TITLE };
}

/** Whether a wake's answer means the model is up. */
export function wakeOk(result: SelfHostedWake): boolean {
  return result.error == null && result.status != null && result.status >= 200 && result.status < 300;
}

/** A backend answer as the chip keeps it, anchored to its wake's start. */
function outcome(result: SelfHostedWake, start: number, now: number): NonNullable<EntryWake['last']> {
  const ok = wakeOk(result);
  return { at: Math.min(now, start + result.ms), ok, ms: result.ms, error: ok ? null : result.error ?? `The health check answered ${result.status}.` };
}

export interface WakeDeps {
  /** The wake command (the app passes the API's; tests a stub). */
  wake: (id: string) => Promise<SelfHostedWake>;
  /** The backend's last wake of an entry, read from memory. */
  status: (id: string) => Promise<SelfHostedWakeStatus | null>;
  now: () => number;
}

export type Wakes = ReturnType<typeof createWakes>;

/** Per-entry wake state, shared by every chat on that entry. */
export function createWakes(deps: WakeDeps) {
  const [all, setAll] = createSignal<Readonly<Record<string, EntryWake>>>({});
  const state = (id: string): EntryWake => all()[id] ?? NO_WAKE;
  const put = (id: string, next: Partial<EntryWake>) => setAll(prev => ({ ...prev, [id]: { ...(prev[id] ?? NO_WAKE), ...next } }));
  /** Entries with a wake this window sent and still waits for. */
  const own = new Set<string>();
  /** Entries whose `sending` came from a read of the backend (`apply`). */
  const adopted = new Set<string>();
  /** Chips showing each entry; its start is read again only while one is. */
  const watchers = new Map<string, number>();
  const failures = new Map<string, number>();
  /** Bumped by `forget`, so an answer to an older call is dropped. */
  const generation = new Map<string, number>();
  const gen = (id: string) => generation.get(id) ?? 0;
  const polls = new Map<string, ReturnType<typeof setTimeout>>();
  const stopPoll = (id: string) => { clearTimeout(polls.get(id)); polls.delete(id); };
  const arm = (id: string, delay: number) => {
    stopPoll(id);
    if ((watchers.get(id) ?? 0) > 0) polls.set(id, setTimeout(() => read(id), delay));
  };
  /** A read-derived start past its limit is over, whatever the reads say. */
  const expire = (id: string) => {
    const sending = state(id).sending;
    if (!adopted.has(id) || sending == null || deps.now() - sending < ADOPTED_LIMIT_MS) return false;
    adopted.delete(id);
    stopPoll(id);
    put(id, { sending: null });
    return true;
  };

  function send(id: string): boolean {
    // A wake in flight is joined: the chip already shows it.
    if (state(id).sending != null && !expire(id)) return false;
    stopPoll(id);
    adopted.delete(id);
    own.add(id);
    const before = state(id).askedAt;
    const at = deps.now();
    const g = gen(id);
    put(id, { sending: at, askedAt: at });
    deps.wake(id).then(
      result => {
        if (gen(id) !== g) return;
        own.delete(id);
        const now = deps.now();
        const start = now - result.startedMsAgo;
        // Shared: another call's wake, so its start is the one that counts.
        put(id, { sending: null, askedAt: Math.min(at, start), last: outcome(result, start, now) });
      },
      err => {
        if (gen(id) !== g) return;
        own.delete(id);
        // A refusal (no such entry, switched off) woke nothing: the 2-minute
        // wait doesn't start.
        put(id, { sending: null, askedAt: before, last: { at: deps.now(), ok: false, ms: 0, error: err instanceof Error ? err.message : String(err), refused: true } });
      },
    );
    return true;
  }

  /** Take the backend's last wake of `id` (a start this window didn't see,
   *  after a reload; or none, after Settings changed the entry). */
  function apply(id: string, status: SelfHostedWakeStatus | null) {
    if (own.has(id)) return;
    const now = deps.now();
    adopted.delete(id);
    if (!status) {
      // Nothing there: never woken, or Settings changed the entry since. A
      // refusal is this window's own (it woke nothing) and stays.
      if (!state(id).last?.refused) drop(id);
      return;
    }
    const start = now - status.startedMsAgo;
    if (!status.result) {
      if (now - start >= ADOPTED_LIMIT_MS) { put(id, { sending: null }); return; }
      adopted.add(id);
      put(id, { sending: start, askedAt: start });
      arm(id, POLL_MS);
      return;
    }
    put(id, { sending: null, askedAt: start, last: outcome(status.result, start, now) });
  }

  const drop = (id: string) => setAll(prev => { const next = { ...prev }; delete next[id]; return next; });

  /** Read the backend's last wake of `id` (no call to the model). A failed
   *  read of a start found earlier is tried again, slower each time, until
   *  that start's limit, so the chip never stays on "starting". */
  function read(id: string): void {
    stopPoll(id);
    const g = gen(id);
    deps.status(id).then(
      status => { if (gen(id) !== g) return; failures.delete(id); apply(id, status); },
      () => {
        if (gen(id) !== g || !adopted.has(id) || expire(id)) return;
        const n = (failures.get(id) ?? 0) + 1;
        failures.set(id, n);
        arm(id, Math.min(MAX_POLL_MS, POLL_MS * 2 ** n));
      },
    );
  }

  return {
    state,
    /** A keystroke or a send in a chat on `id`: wakes it at most once per
     *  `WAKE_EVERY_MS`. True when a wake was sent. */
    nudge(id: string): boolean {
      const { askedAt, last } = state(id);
      const now = deps.now();
      if (askedAt != null && now - askedAt < WAKE_EVERY_MS) return false;
      if (last?.refused && now - last.at < REFUSAL_PAUSE_MS) return false;
      return send(id);
    },
    /** Start now: sends the wake at once (the backend still shares one
     *  started under 2 minutes ago). True when a wake was sent. */
    startNow: send,
    /** A chip shows `id`: read the backend's last wake now (no call to the
     *  model), and a start found there until it answers. The returned
     *  function ends the watch; the last one stops the reads. */
    watch(id: string): () => void {
      watchers.set(id, (watchers.get(id) ?? 0) + 1);
      read(id);
      return () => {
        const left = (watchers.get(id) ?? 1) - 1;
        if (left > 0) { watchers.set(id, left); return; }
        watchers.delete(id);
        stopPoll(id);
      };
    },
    /** Settings saved, switched or removed `id` (the backend forgot its last
     *  wake too): start over, and drop any answer still to come. */
    forget(id: string): void {
      generation.set(id, gen(id) + 1);
      stopPoll(id);
      own.delete(id);
      adopted.delete(id);
      failures.delete(id);
      drop(id);
    },
  };
}

/** The chip's state for the entry `entryId()` names: mounting (or a new
 *  entry) reads the saved entries (`load`) and watches the backend's last
 *  wake (`watch`, reads only), never a wake; unmounting ends the watch. The
 *  clock ticks each second while the chip shows a time or a wait, else twice
 *  a minute (Ready gives way after 10 minutes). */
export function createStatusView(o: { entryId: Accessor<string>; name: Accessor<string>; wakes: Wakes; load: () => void; now?: () => number }) {
  const clock = o.now ?? Date.now;
  createEffect(on(o.entryId, id => { o.load(); onCleanup(o.wakes.watch(id)); }));
  const [now, setNow] = createSignal(clock());
  const wake = () => o.wakes.state(o.entryId());
  const chip = createMemo(() => wakeChip(o.name(), wake(), now()));
  // A wake sent or answered shows at once, not at the next tick.
  createEffect(on(wake, () => setNow(clock()), { defer: true }));
  const ticking = createMemo(() => chip().kind === 'checking' || chip().kind === 'starting' || chip().startNow === 'wait');
  createEffect(() => {
    const timer = setInterval(() => setNow(clock()), ticking() ? 1000 : 30_000);
    onCleanup(() => clearInterval(timer));
  });
  return chip;
}
