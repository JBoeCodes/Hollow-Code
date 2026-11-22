import { afterEach, describe, expect, it, vi } from 'vitest';
import { createRoot, createSignal } from 'solid-js';
import type { SelfHostedWake, SelfHostedWakeStatus } from './generated/ipc';
import { contextWindow } from './chatUsage';
import { selfHostedEntryId } from './selfHostedModels';
import {
  ADOPTED_LIMIT_MS, IDLE_MS, MAX_POLL_MS, NO_WAKE, POLL_MS, REFUSAL_PAUSE_MS, WAKE_EVERY_MS, createStatusView, createWakes, elapsedText,
  typingWakes, wakeChip, wakeOk, type EntryWake,
} from './selfHostedWake';
import API_TS from './api.ts?raw';
import CHAT_BODY from './components/ChatBody.tsx?raw';
import USAGE_MENU from './components/ChatUsageMenu.tsx?raw';
import CHIP from './components/SelfHostedStatus.tsx?raw';
import STORE from './store/selfHostedWake.ts?raw';
import WAKE from './selfHostedWake.ts?raw';
import CSS from './styles/self-hosted-status.css?raw';

// Effects need Solid's reactive (browser) runtime, as exit.test.ts loads it;
// the node suite has no DOM, so the chip is driven through its view
// (`createStatusView`) and the markup is read as source.
// @ts-expect-error Solid's browser runtime has the public API but no subpath declaration.
vi.mock('solid-js', () => import('solid-js/dist/solid.js'));

afterEach(() => { vi.useRealTimers(); });

const NAME = 'Qwen3.8 27B';
const T = 1_700_000_000_000;

function answer(fields: Partial<SelfHostedWake> = {}): SelfHostedWake {
  return { status: 200, ms: 272_000, error: null, shared: false, startedMsAgo: 272_000, ...fields };
}

const flush = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };

/** Wake and status calls the test settles by hand, and a clock it moves.
 *  `status` returning 'fail' rejects the read. */
function harness(status: (id: string) => SelfHostedWakeStatus | null | 'fail' = () => null) {
  let now = T;
  const calls: { id: string; settle: (r: SelfHostedWake) => void; fail: (e: unknown) => void }[] = [];
  const reads: string[] = [];
  const wakes = createWakes({
    wake: id => new Promise((settle, fail) => { calls.push({ id, settle, fail }); }),
    status: id => { reads.push(id); const s = status(id); return s === 'fail' ? Promise.reject(new Error('read failed')) : Promise.resolve(s); },
    now: () => now,
  });
  const tick = async (ms: number) => { now += ms; await flush(); };
  const chip = (id = 'qwen') => wakeChip(NAME, wakes.state(id), now);
  return { wakes, calls, reads, tick, chip, now: () => now };
}

describe('the chat model id (Q6)', () => {
  it('names the entry only for a self-hosted model id', () => {
    expect(selfHostedEntryId('self-hosted:qwen')).toBe('qwen');
    expect(selfHostedEntryId('self-hosted:local-vllm-2')).toBe('local-vllm-2');
    for (const model of ['', 'qwen', 'claude-opus-5-5', 'self-hosted:', 'self-hosted:Qwen', 'self-hosted:-x', 'self-hosted:a b', `self-hosted:${'a'.repeat(33)}`]) {
      expect(selfHostedEntryId(model), model).toBeNull();
    }
  });

  it('typing a / command or emptying the box never wakes; any other text does', () => {
    for (const draft of ['/model', '  /new', '/', '/clear now', '', '   ']) expect(typingWakes(draft), draft).toBe(false);
    for (const draft of ['h', 'what is /model?', ' x']) expect(typingWakes(draft), draft).toBe(true);
  });
});

describe('the start chip (Q6)', () => {
  it('is idle before any wake, never claiming the model is starting', () => {
    expect(wakeChip(NAME, NO_WAKE, T)).toMatchObject({ kind: 'idle', text: 'Qwen3.8 27B sleeps when idle', startNow: 'show' });
    expect(wakeChip(NAME, NO_WAKE, T).elapsed).toBeUndefined();
  });

  it('checks for 3 s, then says it is starting, about 4 minutes, with the time so far as text', () => {
    const wake: EntryWake = { sending: T, askedAt: T, last: null };
    expect(wakeChip(NAME, wake, T + 2_900)).toMatchObject({ kind: 'checking', text: 'Checking Qwen3.8 27B…', startNow: 'hide' });
    expect(wakeChip(NAME, wake, T + 2_900).elapsed).toBeUndefined();
    expect(wakeChip(NAME, wake, T + 72_000)).toEqual({
      kind: 'starting', text: 'Qwen3.8 27B is starting, about 4 minutes', elapsed: '1 min 12 s so far',
      startNow: 'hide', title: 'A message you send now waits for it, with no error.',
    });
    expect(wakeChip(NAME, wake, T + 6 * 60_000 + 5_000).text).toBe('Qwen3.8 27B is still starting, usually 4 to 6 minutes');
    // An old error doesn't skip the grace either.
    const after = { ...wake, last: { at: T - 5_000, ok: false, ms: 1, error: 'down' } };
    expect(wakeChip(NAME, after, T + 1_000).kind).toBe('checking');
  });

  it('says the model is ready, by name, until it would sleep again', () => {
    const wake: EntryWake = { sending: null, askedAt: T, last: { at: T + 272_000, ok: true, ms: 272_000, error: null } };
    expect(wakeChip(NAME, wake, T + 273_000)).toMatchObject({ kind: 'ready', text: 'Qwen3.8 27B is ready', note: 'Answered in 4 min 32 s', startNow: 'hide' });
    expect(wakeChip(NAME, wake, T + 272_000 + IDLE_MS).kind).toBe('idle');
    // Another ping of a ready model stays Ready unless it turns slow.
    const ping = { ...wake, sending: T + 300_000 };
    expect(wakeChip(NAME, ping, T + 300_500).kind).toBe('ready');
    expect(wakeChip(NAME, ping, T + 304_000).kind).toBe('starting');
  });

  it('shows the error in plain words, with Start now once 2 minutes have passed, until it is old', () => {
    const wake: EntryWake = { sending: null, askedAt: T, last: { at: T + 1_000, ok: false, ms: 1_000, error: 'The health check (/health) answered 503.' } };
    expect(wakeChip(NAME, wake, T + 30_000)).toMatchObject({
      kind: 'error', text: "Qwen3.8 27B didn't start. The health check (/health) answered 503.",
      note: 'Start now works again in 1 min 30 s', startNow: 'wait',
    });
    const later = wakeChip(NAME, wake, T + WAKE_EVERY_MS);
    expect(later).toMatchObject({ kind: 'error', startNow: 'show' });
    expect(later.note).toBeUndefined();
    expect(wakeChip(NAME, wake, T + 1_000 + IDLE_MS).kind).toBe('idle');
    // A refusal woke nothing: Start now at once.
    const refused: EntryWake = { sending: null, askedAt: null, last: { at: T, ok: false, ms: 0, error: 'Qwen3.8 27B is switched off.', refused: true } };
    expect(wakeChip(NAME, refused, T + 1_000)).toMatchObject({ kind: 'error', startNow: 'show' });
  });

  it('counts in whole seconds and minutes', () => {
    expect(elapsedText(-5)).toBe('0 s');
    expect(elapsedText(42_900)).toBe('42 s');
    expect(elapsedText(240_000)).toBe('4 min');
    expect(elapsedText(245_000)).toBe('4 min 5 s');
  });
});

describe('waking (Q6)', () => {
  it('a keystroke wakes at most once per 2 minutes per entry', async () => {
    const h = harness();
    expect(h.wakes.nudge('qwen')).toBe(true);
    expect(h.wakes.state('qwen').sending).toBe(T);
    // More keystrokes while it starts send nothing.
    await h.tick(1_000);
    expect(h.wakes.nudge('qwen')).toBe(false);
    h.calls[0].settle(answer({ ms: 900, startedMsAgo: 1_000 }));
    await h.tick(0);
    expect(h.chip().kind).toBe('ready');
    await h.tick(WAKE_EVERY_MS - 2_000);
    expect(h.wakes.nudge('qwen')).toBe(false);
    expect(h.calls).toHaveLength(1);
    await h.tick(1_000);
    expect(h.wakes.nudge('qwen')).toBe(true);
    expect(h.calls.map(c => c.id)).toEqual(['qwen', 'qwen']);
    // Another entry has its own wake.
    expect(h.wakes.nudge('local')).toBe(true);
    expect(h.calls).toHaveLength(3);
  });

  it('a shared answer counts from its own wake’s start', async () => {
    const h = harness();
    h.wakes.startNow('qwen');
    await h.tick(1_000);
    // Another window's wake, started 100 s ago, answered 70 s ago.
    h.calls[0].settle(answer({ shared: true, startedMsAgo: 100_000, ms: 30_000 }));
    await h.tick(0);
    const start = h.now() - 100_000;
    expect(h.wakes.state('qwen')).toMatchObject({ sending: null, askedAt: start, last: { at: start + 30_000, ok: true } });
    // The 2 minutes run from that start, and so does Ready's 10.
    await h.tick(20_000);
    expect(h.wakes.nudge('qwen')).toBe(true);
    h.calls[1].settle(answer({ shared: true, status: 503, error: 'The health check (/health) answered 503.', ms: 5, startedMsAgo: 60_000 }));
    await h.tick(0);
    const failed = h.wakes.state('qwen');
    expect(failed.askedAt).toBe(h.now() - 60_000);
    expect(wakeChip(NAME, failed, h.now())).toMatchObject({ kind: 'error', note: 'Start now works again in 1 min' });
  });

  it('two chats on one model share one start, and Start now joins a running one', async () => {
    const h = harness();
    expect(h.wakes.nudge('qwen')).toBe(true);
    expect(h.wakes.startNow('qwen')).toBe(false);
    expect(h.calls).toHaveLength(1);
    h.calls[0].settle(answer());
    await h.tick(0);
    // Start now sends at once, inside the 2 minutes too (the backend then
    // shares its last answer).
    expect(h.wakes.startNow('qwen')).toBe(true);
    expect(h.calls).toHaveLength(2);
  });

  it('a refusal shows the backend’s words and doesn’t start the 2-minute wait', async () => {
    const h = harness();
    h.wakes.startNow('gone');
    h.calls[0].fail('There is no self-hosted model "gone". Add it in Settings → Models → Self-hosted models.');
    await h.tick(0);
    expect(wakeChip('gone', h.wakes.state('gone'), h.now())).toMatchObject({
      kind: 'error', startNow: 'show', text: 'gone didn\'t start. There is no self-hosted model "gone". Add it in Settings → Models → Self-hosted models.',
    });
    expect(h.wakes.state('gone').askedAt).toBeNull();
    // Keystrokes ask again after a short pause, not 2 minutes.
    expect(h.wakes.nudge('gone')).toBe(false);
    await h.tick(REFUSAL_PAUSE_MS);
    expect(h.wakes.nudge('gone')).toBe(true);
    expect(wakeOk(answer())).toBe(true);
    expect(wakeOk(answer({ status: 503 }))).toBe(false);
    expect(wakeOk(answer({ status: null, error: 'down' }))).toBe(false);
  });

  it('a watch takes a start already running (a reloaded window) and reads it until it answers, sending nothing', async () => {
    vi.useFakeTimers();
    let backend: SelfHostedWakeStatus | null = { startedMsAgo: 90_000, result: null };
    const h = harness(() => backend);
    const unwatch = h.wakes.watch('qwen');
    await h.tick(0);
    expect(h.chip()).toMatchObject({ kind: 'starting', elapsed: '1 min 30 s so far' });
    // Keystrokes join it.
    expect(h.wakes.nudge('qwen')).toBe(false);
    backend = { startedMsAgo: 92_000, result: answer({ ms: 91_000, startedMsAgo: 92_000 }) };
    await vi.advanceTimersByTimeAsync(POLL_MS);
    await flush();
    expect(h.reads).toHaveLength(2);
    expect(h.chip()).toMatchObject({ kind: 'ready', text: 'Qwen3.8 27B is ready' });
    expect(h.calls).toHaveLength(0);
    // Settings changed the entry: the backend forgot it, and so does the chip.
    backend = null;
    unwatch();
    h.wakes.watch('qwen')();
    await flush();
    expect(h.wakes.state('qwen')).toBe(NO_WAKE);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('a failed read keeps reading, slower each time, and a found start ends at its limit', async () => {
    vi.useFakeTimers();
    let backend: SelfHostedWakeStatus | null | 'fail' = { startedMsAgo: 60_000, result: null };
    const h = harness(() => backend);
    const unwatch = h.wakes.watch('qwen');
    await h.tick(0);
    expect(h.chip().kind).toBe('starting');
    expect(h.wakes.startNow('qwen')).toBe(false);
    backend = 'fail';
    await vi.advanceTimersByTimeAsync(POLL_MS);
    await flush();
    expect(h.reads).toHaveLength(2);
    // Re-armed after 4 s, then 8 s: never stuck without a read.
    expect(vi.getTimerCount()).toBe(1);
    await vi.advanceTimersByTimeAsync(POLL_MS * 2 - 1);
    expect(h.reads).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(1);
    await flush();
    expect(h.reads).toHaveLength(3);
    // Past the start's limit the next failed read ends it, and a keystroke wakes.
    await h.tick(ADOPTED_LIMIT_MS);
    await vi.advanceTimersByTimeAsync(MAX_POLL_MS);
    await flush();
    expect(h.wakes.state('qwen').sending).toBeNull();
    expect(vi.getTimerCount()).toBe(0);
    expect(h.chip().kind).toBe('idle');
    expect(h.wakes.nudge('qwen')).toBe(true);
    expect(h.calls).toHaveLength(1);
    unwatch();
  });

  it('a found start past its limit never blocks a wake, even with no read to end it', async () => {
    const h = harness(() => ({ startedMsAgo: 60_000, result: null }));
    h.wakes.watch('qwen')();
    await h.tick(0);
    expect(h.wakes.startNow('qwen')).toBe(false);
    await h.tick(ADOPTED_LIMIT_MS);
    expect(h.wakes.startNow('qwen')).toBe(true);
    expect(h.calls).toHaveLength(1);
  });

  it('Settings’ change starts the entry over: state, the 2-minute wait, and any answer still to come', async () => {
    const h = harness();
    h.wakes.nudge('qwen');
    h.wakes.forget('qwen');
    expect(h.wakes.state('qwen')).toBe(NO_WAKE);
    // The old call's answer is dropped; a keystroke wakes the entry as it now is.
    h.calls[0].settle(answer({ status: 503, error: 'old' }));
    await h.tick(0);
    expect(h.wakes.state('qwen')).toBe(NO_WAKE);
    expect(h.wakes.nudge('qwen')).toBe(true);
    expect(h.calls).toHaveLength(2);
    // Settings' save, switch and remove tell the store, which forgets and re-reads.
    for (const call of ["selfHostedChanged(entry.id, invoke<SelfHostedModels>('save_self_hosted_model'", "selfHostedChanged(id, invoke<SelfHostedModels>('remove_self_hosted_model'", "selfHostedChanged(id, invoke<SelfHostedModels>('set_self_hosted_model_enabled'"]) {
      expect(API_TS, call).toContain(call);
    }
    expect(STORE).toContain('onSelfHostedChange(id => {\n  loadSelfHostedEntries();\n  selfHostedWakes.forget(id);\n});');
  });
});

describe('the chip’s view (Q6)', () => {
  function mount(entry = 'qwen', found: SelfHostedWakeStatus | null = null) {
    vi.useFakeTimers();
    vi.setSystemTime(T);
    const wake = vi.fn((_id: string) => new Promise<SelfHostedWake>(() => {}));
    const status = vi.fn((_id: string) => Promise.resolve(found));
    const load = vi.fn();
    const clock = vi.fn(() => Date.now());
    const wakes = createWakes({ wake, status, now: () => Date.now() });
    const view = createRoot(dispose => {
      const [entryId, setEntryId] = createSignal(entry);
      return { dispose, setEntryId, chip: createStatusView({ entryId, name: () => NAME, wakes, load, now: clock }) };
    });
    return { ...view, wake, status, load, clock, wakes };
  }

  it('mounting or changing the entry reads, and never wakes', async () => {
    const v = mount();
    try {
      await flush();
      expect(v.load).toHaveBeenCalledTimes(1);
      expect(v.status).toHaveBeenCalledWith('qwen');
      expect(v.chip()).toMatchObject({ kind: 'idle', startNow: 'show' });
      v.setEntryId('local');
      await flush();
      expect(v.load).toHaveBeenCalledTimes(2);
      expect(v.status.mock.calls.map(c => c[0])).toEqual(['qwen', 'local']);
      expect(v.wake).not.toHaveBeenCalled();
    } finally { v.dispose(); }
  });

  it('unmounting stops the reads of a start it found', async () => {
    const v = mount('qwen', { startedMsAgo: 30_000, result: null });
    await flush();
    expect(v.chip()).toMatchObject({ kind: 'starting', elapsed: '30 s so far' });
    // The 1 s clock and the 2 s read.
    expect(vi.getTimerCount()).toBe(2);
    v.dispose();
    expect(vi.getTimerCount()).toBe(0);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(v.status).toHaveBeenCalledTimes(1);
  });

  it('ticks twice a minute at rest and each second while a time shows, and stops when unmounted', async () => {
    const v = mount();
    try {
      await flush();
      expect(vi.getTimerCount()).toBe(1);
      v.clock.mockClear();
      await vi.advanceTimersByTimeAsync(60_000);
      expect(v.clock).toHaveBeenCalledTimes(2);
      v.wakes.startNow('qwen');
      await flush();
      expect(v.chip()).toMatchObject({ kind: 'checking', startNow: 'hide' });
      expect(vi.getTimerCount()).toBe(1);
      v.clock.mockClear();
      await vi.advanceTimersByTimeAsync(10_000);
      expect(v.clock.mock.calls.length).toBeGreaterThanOrEqual(10);
      expect(v.chip()).toMatchObject({ kind: 'starting', elapsed: '10 s so far' });
      expect(v.wake).toHaveBeenCalledTimes(1);
    } finally { v.dispose(); }
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('the chip’s markup and the call sites (Q6)', () => {
  it('binds the view: state, the live words, the time, the note and Start now hidden, waiting or shown', () => {
    expect(CHIP).toContain('data-state={chip().kind}');
    expect(CHIP).toContain('<span class="self-hosted-status-text" role="status">{chip().text}</span>');
    expect(CHIP).toContain('<Show when={chip().elapsed}>');
    expect(CHIP).toContain('<Show when={chip().note}>');
    expect(CHIP).toContain("<Show when={chip().startNow !== 'hide'}>");
    expect(CHIP).toContain("disabled={chip().startNow === 'wait'}");
    expect(CHIP).toContain('>Start now</button>');
    expect(CHIP).toContain('type="button"');
    // Every capability is a click; the chip adds no key of its own.
    expect(CHIP).not.toMatch(/onKeyDown|addEventListener\('keydown'/);
    for (const text of [CHIP, STORE, WAKE, CSS]) expect(text).not.toMatch(/[–—]/);
  });

  it('only the composer’s input and send, and Start now, wake: opening, focusing or reading a chat never does', () => {
    const sources = import.meta.glob(['./**/*.ts', './**/*.tsx', '!./**/*.test.ts'], { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
    const sites = (re: RegExp) => Object.entries(sources).flatMap(([file, text]) => (text.match(re) ?? []).map(m => `${file} ${m}`)).sort();
    expect(sites(/selfHostedWakes\.(nudge|startNow)\([^)]*\)/g)).toEqual([
      './components/SelfHostedStatus.tsx selfHostedWakes.startNow(props.entryId)',
      './store/selfHostedWake.ts selfHostedWakes.nudge(id)',
    ]);
    expect(sites(/wakeFromComposer\([^)]*\)/g)).toEqual([
      './components/ChatBody.tsx wakeFromComposer()',
      './components/ChatBody.tsx wakeFromComposer(v)',
      './components/ChatBody.tsx wakeFromComposer(v)',
      './store/selfHostedWake.ts wakeFromComposer(draft?: string)',
    ]);
    expect(sites(/wakeSelfHostedModel|'wake_self_hosted_model'/g).map(s => s.split(' ')[0])).toEqual(['./api.ts', './api.ts', './mockApi.ts', './store/selfHostedWake.ts']);
    // Typing wakes from either editor's user-input callback; the send call is
    // in followSend (a local / command never reaches it).
    const input = CHAT_BODY.slice(CHAT_BODY.indexOf('onInput={e => {'), CHAT_BODY.indexOf('onKeyDown={onKeyDown}'));
    expect(input).toContain('wakeFromComposer(v);');
    const richInput = CHAT_BODY.slice(CHAT_BODY.indexOf('<ComposerMarkdown initialDoc='), CHAT_BODY.indexOf('}} composer={{'));
    expect(richInput).toContain('onChange={v => {');
    expect(richInput).toContain('wakeFromComposer(v);');
    expect(CHAT_BODY).toContain('followSend: () => { followSend(); wakeFromComposer(); },');
    expect(STORE).toContain('if (draft != null && !typingWakes(draft)) return;');
  });

  it('moves only with both motion gates open and while the window is here; the time so far is text', () => {
    const gate = CSS.indexOf('@media (prefers-reduced-motion: no-preference)');
    expect(gate).toBeGreaterThan(0);
    expect(CSS.slice(0, gate)).not.toMatch(/animation|transition/);
    expect(CSS.slice(gate)).toContain(':root:not([data-motion="reduced"]) .self-hosted-status[data-state="starting"] .self-hosted-status-dot { animation: status-pulse 1.6s var(--ease-soft) infinite; animation-play-state: var(--ambient-play, running); }');
    expect(CHIP).not.toMatch(/requestAnimationFrame|\.animate\(|transition/);
    expect(wakeChip(NAME, { sending: T, askedAt: T, last: null }, T + 5_000).elapsed).toBe('5 s so far');
  });
});

describe('the context ring (Q6)', () => {
  it('uses the entry’s window, not the 200K it infers for Claude', () => {
    expect(contextWindow('claude', 'self-hosted:qwen', 50_000, null)).toBe(200_000);
    expect(contextWindow('claude', 'self-hosted:qwen', 50_000, 131_072)).toBe(131_072);
    expect(USAGE_MENU.match(/u(sed)?\.window \?\? selfHostedChatWindow\(\)/g)).toHaveLength(2);
    expect(STORE).toContain('selfHostedEntry(id)?.contextWindow');
  });
});
