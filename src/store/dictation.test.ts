import { beforeEach, describe, expect, it, vi } from 'vitest';

// P1-55: dictation stops when it can't help, and says why.
const api = vi.hoisted(() => ({
  dictationStatus: vi.fn(), dictationAuthorize: vi.fn(), dictationStart: vi.fn(), dictationStop: vi.fn(),
}));
const flash = vi.hoisted(() => vi.fn());
vi.mock('../api', () => ({ api }));
vi.mock('./runtime', () => ({ flashFooter: flash }));
vi.mock('../nativeEvents', () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock('../mockMode', () => ({ MOCK_BUILD: false, isBrowserMockMode: () => false }));

import { dictationPhase, dictationSent, onDictationEvent, startDictation, toggleDictation } from './dictation';
import type { RichComposerField } from '../composerField';

const field = (value = '') => ({ value, selectionStart: value.length, selectionEnd: value.length, isConnected: true, focus() {}, setSelectionRange() {}, dispatchEvent() { return true; } }) as unknown as HTMLTextAreaElement;
const flush = () => new Promise(r => setTimeout(r, 0));

beforeEach(async () => {
  vi.clearAllMocks();
  api.dictationStatus.mockResolvedValue({ state: 'ready', locale: 'en-US', reason: null });
  api.dictationStart.mockResolvedValue(7);
  api.dictationStop.mockResolvedValue(null);
  // Leave every test idle.
  dictationSent(null);
  await flush();
});

describe('dictation store (P1-55)', () => {
  it('dictates through a rich editor transaction and ignores late words after send', async () => {
    const el = field('Before after') as unknown as RichComposerField;
    el.element = {} as HTMLElement;
    el.selectionStart = el.selectionEnd = 7;
    el.replaceText = vi.fn((from, to, text) => { el.value = el.value.slice(0, from) + text + el.value.slice(to); });
    expect(await startDictation(el)).toBe(true);
    onDictationEvent({ session: 7, kind: 'partial', text: 'hello' });
    expect(el.value).toBe('Before hello after');
    expect(el.replaceText).toHaveBeenCalledWith(7, 7, 'hello ');
    onDictationEvent({ session: 7, kind: 'final', text: 'hello there' });
    expect(el.value).toBe('Before hello there after');
    dictationSent(el);
    onDictationEvent({ session: 7, kind: 'partial', text: 'late words' });
    expect(el.value).toBe('Before hello there after');
  });
  it('stops and says so when the person edits the field under it', async () => {
    const el = field('Hi ');
    expect(await startDictation(el)).toBe(true);
    (el as { value: string }).value = 'Something else entirely';
    onDictationEvent({ session: 7, kind: 'partial', text: 'hello' });
    await flush();
    expect(api.dictationStop).toHaveBeenCalledWith(7);
    expect(flash).toHaveBeenCalledWith('Dictation stopped: you edited the message.');
  });

  it('a send while listening returns to idle and stops the microphone', async () => {
    const el = field();
    await startDictation(el);
    expect(dictationPhase()).toBe('listening');
    dictationSent(el);
    expect(dictationPhase()).toBe('idle');
    expect(api.dictationStop).toHaveBeenCalledWith(7);
  });

  it('a click while starting cancels, and the late session is stopped, not used', async () => {
    let ready!: (s: unknown) => void;
    api.dictationStatus.mockReturnValueOnce(new Promise(r => { ready = r; }));
    const el = field();
    toggleDictation(el);
    expect(dictationPhase()).toBe('starting');
    toggleDictation(el);
    expect(dictationPhase()).toBe('idle');
    ready({ state: 'ready', locale: 'en-US', reason: null });
    await flush(); await flush();
    expect(dictationPhase()).toBe('idle');
    expect(api.dictationStart).not.toHaveBeenCalled();
  });

  it('a cancel while the session opens stops that late session and never uses it', async () => {
    let open!: (id: number) => void;
    api.dictationStart.mockReturnValueOnce(new Promise(r => { open = r; }));
    const el = field();
    const started = startDictation(el);
    await flush();
    expect(dictationPhase()).toBe('starting');
    toggleDictation(el);
    expect(dictationPhase()).toBe('idle');
    open(9);
    expect(await started).toBe(false);
    expect(api.dictationStop).toHaveBeenCalledWith(9);
    onDictationEvent({ session: 9, kind: 'partial', text: 'late' });
    expect((el as { value: string }).value).toBe('');
    expect(dictationPhase()).toBe('idle');
  });
});
