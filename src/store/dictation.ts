// Push-to-talk (overnight 10-11 B7): one dictation at a time, typing into a
// text field (the composer the mic belongs to, or the focused field for the
// ⌘⇧D command). The words arrive from `src-tauri/src/dictation.rs` as
// `dictation-event`s; this module writes them into the field the way typing
// would (an `input` event, so the composer's own state follows) and never
// sends anything. Pure parts in `../dictation.ts`.
import { createSignal } from 'solid-js';
import { listen } from '../nativeEvents';
import { api } from '../api';
import { errMsg } from '../utils';
import { activeChord } from '../shortcuts';
import { MOCK_BUILD, isBrowserMockMode } from '../mockMode';
import { applyTranscript, beginAnchor, endsHold, isRepeat, lastKeyOf, type Anchor, type DictationEvent, type DictationStatus } from '../dictation';
import { flashFooter, flashWithUndo } from './runtime';
import { modalOpen } from '../modalGuard';
import { fieldElement, isRichField, richFieldAt, visibleRichField, type DictationField } from '../composerField';

export type DictationPhase = 'idle' | 'starting' | 'listening' | 'stopping';

const [phase, setPhase] = createSignal<DictationPhase>('idle');
const [target, setTarget] = createSignal<DictationField | null>(null);
const [status, setStatus] = createSignal<DictationStatus | null>(null);
export const dictationPhase = phase;
export const dictationTarget = target;
export const dictationStatus = status;

let session: number | null = null;
let anchor: Anchor | null = null;
/** When the shortcut started this session, for hold-and-release. */
let pressedAt: number | null = null;
/** The field stopped taking words (the person edited around them). */
let frozen = false;
/** The last time the shortcut ran, to tell a held key's repeats from a tap. */
let lastShortcutAt: number | null = null;
let mockTimer: ReturnType<typeof setInterval> | null = null;
/** Counts starts, so a start cancelled while it awaited can tell. */
let starts = 0;

/** Read the status (the mic's tooltip, and whether it is offered). */
export async function refreshDictationStatus(): Promise<DictationStatus | null> {
  try {
    const s = await api.dictationStatus();
    setStatus(s);
    return s;
  } catch {
    return null;
  }
}

function write(text: string) {
  const el = target();
  if (!el || !anchor || frozen) return;
  // The box went away (another conversation opened): stop listening.
  if (!el.isConnected) { void stopDictation(); return; }
  const prev = anchor;
  const next = applyTranscript(prev, el.value, text);
  // The person edited around the words: stop listening rather than drop
  // every later word in silence (P1-55).
  if (!next) {
    frozen = true;
    if (phase() === 'listening') { flashFooter('Dictation stopped: you edited the message.'); void stopDictation(); }
    return;
  }
  anchor = next.anchor;
  // Replace the dictated part as typing would: through the editing command,
  // so ⌘Z undoes it and the text typed before stays undoable; it fires the
  // field's own input event. A field that refuses it gets the value.
  const start = prev.before.length;
  if (isRichField(el)) {
    el.replaceText(start, start + prev.inserted.length, next.anchor.inserted);
    el.setSelectionRange(next.caret, next.caret);
    return;
  }
  const focused = document.activeElement === el;
  if (!focused) el.focus();
  el.setSelectionRange(start, start + prev.inserted.length);
  const typed = document.execCommand?.('insertText', false, next.anchor.inserted) ?? false;
  if (!typed || el.value !== next.value) {
    el.value = next.value;
    el.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: null }));
  }
  el.setSelectionRange(next.caret, next.caret);
}

function finish() {
  if (mockTimer) { clearInterval(mockTimer); mockTimer = null; }
  session = null;
  anchor = null;
  pressedAt = null;
  lastShortcutAt = null;
  frozen = false;
  setPhase('idle');
  setTarget(null);
}

/** Fold one event from the backend (exported for tests). */
export function onDictationEvent(evt: DictationEvent) {
  if (session == null || evt.session !== session) return;
  switch (evt.kind) {
    case 'partial': write(evt.text); break;
    case 'final': write(evt.text); break;
    case 'error': if (evt.text) dictationRefused(`Dictation: ${evt.text}`); break;
    case 'ended': finish(); break;
  }
}

listen<DictationEvent>('dictation-event', e => onDictationEvent(e.payload)).catch(() => { /* not in tauri */ });

/** Start dictating into `el`. Asks macOS first when it hasn't been asked;
 *  refuses with the reason when on-device recognition isn't there. */
export async function startDictation(el: DictationField, byKey = false): Promise<boolean> {
  if (phase() !== 'idle') return false;
  setPhase('starting');
  setTarget(el);
  // A click or Esc while starting cancels (P1-55): every await below checks
  // that this start is still the current one.
  const token = ++starts;
  try {
    let s = await api.dictationStatus();
    if (token !== starts) return false;
    if (s.state === 'needsPermission') {
      flashFooter('Allow speech recognition and the microphone in the macOS prompt.');
      s = await api.dictationAuthorize();
      if (token !== starts) return false;
    }
    setStatus(s);
    if (s.state !== 'ready') {
      dictationRefused(s.reason ?? 'Dictation is unavailable.');
      finish();
      return false;
    }
    anchor = beginAnchor(el.value, el.selectionStart ?? el.value.length, el.selectionEnd ?? el.value.length);
    frozen = false;
    const id = await api.dictationStart();
    // Cancelled while the session opened: stop it, never use it.
    if (token !== starts) { void api.dictationStop(id).catch(() => {}); return false; }
    session = id;
    pressedAt = byKey ? Date.now() : null;
    setPhase('listening');
    el.focus();
    if (MOCK_BUILD && isBrowserMockMode()) simulate(session);
    return true;
  } catch (error) {
    if (token !== starts) return false;
    flashFooter(`Dictation: ${errMsg(error)}`);
    finish();
    return false;
  }
}

/** A start that hasn't opened its session yet: cancel it (P1-55). */
function cancelStart() {
  starts += 1;
  finish();
}

/** The composer sent its message (P1-55): stop at once, so no late word lands
 *  in the emptied box and the microphone goes off. `el` null stops any. */
export function dictationSent(el: DictationField | null) {
  if (phase() === 'idle' || (el && target() !== el)) return;
  if (phase() === 'starting') { cancelStart(); return; }
  const id = session;
  if (id != null) void api.dictationStop(id).catch(() => {});
  finish();
}

/** Which Privacy pane a refusal names, for "Open System Settings". */
const privacyPane = (reason: string): 'microphone' | 'speech' | null =>
  /Microphone/.test(reason) ? 'microphone' : /Speech Recognition/.test(reason) ? 'speech' : null;

/** A refusal in words; one that names a Privacy pane offers to open it (P1-55). */
function dictationRefused(reason: string) {
  const pane = privacyPane(reason);
  if (!pane) { flashFooter(reason); return; }
  flashWithUndo({ message: reason, kind: 'status', ttlMs: 10_000, actions: [{ label: 'Open System Settings', run: () => api.openDictationSettings(pane).catch(e => flashFooter(`× Couldn’t open System Settings: ${errMsg(e)}`)) }] });
}

/** Stop listening; the recogniser's last words still land, then it ends. */
export async function stopDictation(): Promise<void> {
  if (phase() !== 'listening' || session == null) return;
  const id = session;
  setPhase('stopping');
  try {
    await api.dictationStop(id);
  } catch (error) {
    flashFooter(`Dictation: ${errMsg(error)}`);
    finish();
    return;
  }
  // The mock has no event stream to end it.
  if (MOCK_BUILD && isBrowserMockMode()) { finish(); return; }
  // The final result normally ends it; never leave the mic looking live.
  setTimeout(() => { if (session === id) finish(); }, 4000);
}

/** The text field the ⌘⇧D command types into: the focused one, else the
 *  chat composer on screen (the Main agent page's included). Behind a dialog or a
 *  full-screen view only the focused field counts: a composer under it still
 *  has layout boxes, and words typed there can't be seen (cross-PR review S2). */
export function dictationField(): DictationField | null {
  const active = document.activeElement;
  const rich = richFieldAt(active);
  if (rich && !fieldElement(rich).closest('[hidden],[inert]')) return rich;
  if (active instanceof HTMLTextAreaElement && !active.readOnly && !active.disabled) return active;
  if (active instanceof HTMLInputElement && ['text', 'search', ''].includes(active.type) && !active.readOnly && !active.disabled) return active;
  if (modalOpen()) return null;
  const shown = (el: Element | null) => !!el && (el as HTMLElement).getClientRects().length > 0;
  return [...document.querySelectorAll<HTMLTextAreaElement>('textarea[aria-label="Chat message"]')].find(shown) ?? visibleRichField();
}

/** The ⌘⇧D command: a tap starts and a second tap stops; held, the release
 *  stops it (`onDictationKeyUp`). */
export function dictateFromShortcut() {
  const now = Date.now();
  const repeat = isRepeat(lastShortcutAt, now);
  lastShortcutAt = now;
  // The native menu's key equivalent repeats while held: not a second tap.
  if (repeat) return;
  if (phase() === 'listening') { void stopDictation(); return; }
  if (phase() !== 'idle') return;
  const el = dictationField();
  if (!el) { flashFooter('Click into a message box first, then dictate.'); return; }
  void startDictation(el, true);
}

/** Toggle from a mic button that belongs to `el`. */
export function toggleDictation(el: DictationField | null | undefined) {
  if (!el) return;
  // Listening into another box (or this one): any mic stops it.
  if (phase() === 'listening') {
    if (target() !== el) flashFooter('Stopped dictating into the other box.');
    void stopDictation();
    return;
  }
  // The macOS prompt can wait behind another window: a click cancels (P1-55).
  if (phase() === 'starting') { cancelStart(); return; }
  if (phase() === 'idle') void startDictation(el);
}

if (typeof document !== 'undefined') {
  document.addEventListener('keyup', e => {
    if (phase() !== 'listening') return;
    if (endsHold(e, lastKeyOf(activeChord('dictate')), pressedAt, Date.now())) void stopDictation();
  }, true);
  // Escape in the field being dictated into stops listening (the text
  // stays). Window capture, registered at launch, so it runs before the
  // chat's own Escape (which would stop a running turn).
  window.addEventListener('keydown', e => {
    if ((phase() === 'listening' || phase() === 'starting') && e.key === 'Escape' && target() && e.target === fieldElement(target()!)) {
      e.preventDefault();
      e.stopImmediatePropagation();
      if (phase() === 'starting') cancelStart(); else void stopDictation();
    }
  }, true);
  // Access changed in System Settings: read the status again on return (P1-55).
  window.addEventListener('focus', () => { const s = status()?.state; if (s && s !== 'ready') void refreshDictationStatus(); });
}

// The browser mock has no microphone: a few words arrive on a timer so the
// listening state can be seen and screenshot.
function simulate(id: number) {
  const words = 'Draft a short note to Pat about the Q3 pricing review and attach last week’s numbers'.split(' ');
  let n = 0;
  mockTimer = setInterval(() => {
    n += 1;
    onDictationEvent({ session: id, kind: 'partial', text: words.slice(0, n).join(' ') });
    if (n >= words.length && mockTimer) { clearInterval(mockTimer); mockTimer = null; }
  }, 180);
}
