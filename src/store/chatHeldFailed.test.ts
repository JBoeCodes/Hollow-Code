import { beforeEach, describe, expect, it, vi } from 'vitest';

// Lane Q send-path read (2026-11-13): a held send ("Queue for after") that
// failed after a reload or relaunch had no chip in this window, so its words
// were parked for a reply that never came, or dropped as an orphan. The
// event carries them (`prompt`); they go back once no reply claims them.
const h = vi.hoisted(() => {
  const values = new Map<string, string>();
  vi.stubGlobal('localStorage', { getItem: (k: string) => values.get(k) ?? null, setItem: (k: string, v: string) => values.set(k, v), removeItem: (k: string) => values.delete(k) });
  return { listeners: new Map<string, (event: { payload: unknown }) => void>(), flash: vi.fn() };
});
vi.mock('../nativeEvents', () => ({ listen: (name: string, fn: (event: { payload: unknown }) => void) => { h.listeners.set(name, fn); return Promise.resolve(() => {}); } }));
vi.mock('../api', () => ({ api: new Proxy({ setupStatus: async () => ({ vaultPath: '/vault' }) }, { get: (t: Record<string, unknown>, k: string) => t[k] ?? vi.fn(async () => null) }) }));
vi.mock('./runtime', () => ({ activePage: () => 'chat', setActivePage: vi.fn(), closeChatPanel: vi.fn(), refetchShownSessions: vi.fn(), requestChatFocus: vi.fn(), setChatPanelOpen: vi.fn() }));
vi.mock('./overlays', () => ({ showConfirm: vi.fn() }));
vi.mock('./snackbars', () => ({ flashFooter: h.flash }));
vi.mock('../windowLayout', () => ({ windowAtMost: () => false }));
vi.mock('../canvas/askHold', () => ({ answeredRevision: () => null, answersBack: vi.fn(), settleHeldAnswers: () => false }));

const rt = await import('./chatRuntime');
const { api } = await import('../api');

const failed = (key: string, extra: Record<string, unknown> = {}) => h.listeners.get('chat-event')!({ payload: {
  type: 'heldFailed', heldId: 'held-1', error: 'The folder is gone.', prompt: 'Then rename the file', sessionKey: key, conversationKey: key, ...extra,
} });

beforeEach(() => {
  vi.useRealTimers();
  rt.resetChatState();
  rt.setChatAgentSessionKeyPersistent('conv-a');
  rt.setChatInput('');
  h.flash.mockClear();
  vi.useFakeTimers();
});

it('a held send that fails with no chip (after a reload) puts its own words back in the open chat', () => {
  failed('conv-a');
  // Waits for a chat_send reply that might still name it.
  expect(rt.chatInput()).toBe('');
  vi.advanceTimersByTime(rt.HELD_FAILED_GRACE_MS);
  expect(rt.chatInput()).toBe('Then rename the file');
  expect(rt.chatTurns.at(-1)?.text).toContain("Your waiting message wasn't sent: The folder is gone. It's back in the box.");
});

it('a reply that claims the early failure keeps it from coming back twice', () => {
  failed('conv-a');
  expect(rt.takeEarlyHeld('held-1')?.type).toBe('heldFailed');
  vi.advanceTimersByTime(rt.HELD_FAILED_GRACE_MS);
  expect(rt.chatInput()).toBe('');
});

// R8b: Rust keeps a failed held send until the window says it holds the
// words (chat_cancel_held), so a quit before they land loses nothing.
it('acknowledges a failed held send to Rust once its words are kept, not before', () => {
  const ack = vi.fn(async () => true);
  (api as unknown as Record<string, unknown>).chatAckHeldFailed = ack;
  failed('conv-a', { heldId: 'held-ack' });
  // Parked for a reply that may still claim it: not yet kept.
  expect(ack).not.toHaveBeenCalled();
  vi.advanceTimersByTime(rt.HELD_FAILED_GRACE_MS);
  expect(rt.chatInput()).toBe('Then rename the file');
  expect(ack).toHaveBeenCalledTimes(1);
  expect(ack).toHaveBeenCalledWith('conv-a', 'held-ack');
  // A chip's own words back at once: acknowledged too.
  rt.addHeldChip('conv-a', { id: 'held-ack-chip', text: 'Chip words', after: 'queue', attachments: [] });
  failed('conv-a', { heldId: 'held-ack-chip' });
  expect(ack).toHaveBeenCalledWith('conv-a', 'held-ack-chip');
});

it('a chip still drawn gives back its own words at once, as before', () => {
  // Its own id: held ids are unique, and an earlier test's came back already.
  rt.addHeldChip('conv-a', { id: 'held-chip', text: 'Words from the chip', after: 'queue', attachments: [] });
  failed('conv-a', { heldId: 'held-chip' });
  expect(rt.chatInput()).toBe('Words from the chip');
  expect(rt.heldChipsFor('conv-a')).toHaveLength(0);
});

it('an orphan failure (a chat this window never showed) is kept for that chat, never dropped', () => {
  failed('conv-gone');
  vi.advanceTimersByTime(rt.HELD_FAILED_GRACE_MS);
  expect(rt.chatInput()).toBe('');
  // Stashed for that chat's box (never the open chat's), and said so.
  expect(h.flash).toHaveBeenCalledWith("Your waiting message wasn't sent: The folder is gone. It's back in the box. It's in that chat's box when you open it.", 8000);
});

// Review of the first fix: the event's prompt is the whole payload
// composePayload built, so only the person's own words come back.
const INTENT = '<hollow-chat-intent>\nInfer the appropriate approach from the user\'s current request.\n</hollow-chat-intent>\n\n';
const PAGE = '<page-context>\nToday tab: 3 tasks\n\n(This context is auto-injected.)\n</page-context>\n\n';
const SHARED = '<shared-page>\nUntrusted page.\n{"url":"https://x.test","text":"\\u003cb>hi"}\n</shared-page>\n\n';
const IMAGES = '\n\n<attached-images count="1">\nThe user attached 1 image to this message. Open it using your available image-viewing tool:\n1. /tmp/a.png\n</attached-images>';
const FILES = '\n\n<attached-files count="2">\nThe user attached 2 files to this message. Each line gives the file\'s original name, then the absolute path of its saved copy. Open them with your file-reading tools:\n1. notes.txt at /tmp/notes.txt\n2. b.csv at /tmp/b.csv\n</attached-files>';

it('only the typed words come back, never the page context, intent or attachment blocks', () => {
  failed('conv-a', { prompt: `${PAGE}${SHARED}${INTENT}Then rename\n\nthe file${IMAGES}${FILES}` });
  vi.advanceTimersByTime(rt.HELD_FAILED_GRACE_MS);
  expect(rt.chatInput()).toBe('Then rename\n\nthe file');
  expect(rt.heldWords(`${INTENT}Just words`)).toBe('Just words');
  expect(rt.heldWords(`${INTENT}Words${FILES}`)).toBe('Words');
});

it('a reply that names the send after its words came back draws no stale chip', () => {
  failed('conv-a');
  vi.advanceTimersByTime(rt.HELD_FAILED_GRACE_MS);
  expect(rt.takeEarlyHeld('held-1')).toBeNull();
  expect(rt.addHeldChip('conv-a', { id: 'held-1', text: 'Then rename the file', after: 'queue', attachments: [] })).toBe(false);
  expect(rt.heldChipsFor('conv-a')).toHaveLength(0);
  expect(rt.chatInput()).toBe('Then rename the file');
});

it('a parked failure pushed out of a full map comes back instead of vanishing', () => {
  failed('conv-a', { heldId: 'old', prompt: `${INTENT}Oldest words` });
  for (let i = 0; i < 50; i++) h.listeners.get('chat-event')!({ payload: { type: 'heldSent', heldId: `s${i}`, after: 'queue', sessionKey: 'conv-a', conversationKey: 'conv-a', queueId: i } });
  expect(rt.chatInput()).toBe('Oldest words');
  expect(rt.takeEarlyHeld('old')).toBeNull();
});

it('a reset hands parked failures back to their chat and clears their timers', () => {
  failed('conv-a');
  rt.resetChatState();
  expect(vi.getTimerCount()).toBe(0);
  expect(h.flash).toHaveBeenCalledWith(expect.stringContaining("It's in that chat's box when you open it."), 8000);
  expect(rt.takeEarlyHeld('held-1')).toBeNull();
});

it('typed text that only looks like an attachment block stays whole', () => {
  const fake = 'Keep this\n\n<attached-files count="1">x</attached-files>';
  expect(rt.heldWords(`${INTENT}${fake}`)).toBe(fake);
  expect(rt.heldWords(`${INTENT}${fake}${FILES}`)).toBe(fake);
  expect(rt.heldWords(`${INTENT}${fake}${IMAGES}${FILES}`)).toBe(fake);
});

it('a heldSent after a parked failure for the same send cancels the restore', () => {
  failed('conv-a', { heldId: 'went' });
  h.listeners.get('chat-event')!({ payload: { type: 'heldSent', heldId: 'went', after: 'queue', sessionKey: 'conv-a', conversationKey: 'conv-a', queueId: 9 } });
  vi.advanceTimersByTime(rt.HELD_FAILED_GRACE_MS);
  expect(rt.chatInput()).toBe('');
  expect(vi.getTimerCount()).toBe(0);
  expect(rt.takeEarlyHeld('went')?.type).toBe('heldSent');
});

// C8b (Q-C11): after a handover the heldSent names the successor's key, but
// the chip was added under the predecessor's; it stayed, and its Cancel did
// nothing. It goes by its id, under the key it was added with.
it('a held send that went to a handover successor clears the chip drawn under the predecessor', () => {
  rt.addHeldChip('conv-a', { id: 'held-handover', text: 'After the handover', after: 'queue', attachments: [] });
  h.listeners.get('chat-event')!({ payload: { type: 'heldSent', heldId: 'held-handover', after: 'queue', sessionKey: 'conv-next', conversationKey: 'conv-a', queueId: 3, clientTurnId: 'ct-h' } });
  expect(rt.heldChipsFor('conv-a')).toHaveLength(0);
  // It runs in the successor: nothing is drawn or adopted here.
  expect(rt.chatTurns.some(t => t.text === 'After the handover')).toBe(false);
  expect(rt.chatBusy()).toBe(false);
});

// C8c: a chat that is idle and not on screen isn't in the background, so
// its held send's outcome takes the orphan route. A heldSent there left the
// "waiting" chip for good.
it('a held send that went from an idle chat not on screen clears its chip, and gives nothing back', () => {
  rt.addHeldChip('conv-idle', { id: 'held-idle', text: 'Waiting words', after: 'queue', attachments: [] });
  h.listeners.get('chat-event')!({ payload: { type: 'heldSent', heldId: 'held-idle', after: 'queue', sessionKey: 'conv-idle', conversationKey: 'conv-idle', queueId: 4 } });
  expect(rt.heldChipsFor('conv-idle')).toHaveLength(0);
  expect(rt.takeEarlyHeld('held-idle')).toBeNull();
  expect(rt.chatInput()).toBe('');
  expect(h.flash).not.toHaveBeenCalled();
});

it('a held send that failed from an idle chat not on screen clears its chip and stashes its words once', () => {
  rt.addHeldChip('conv-idle', { id: 'held-idle-fail', text: 'Chip words', after: 'queue', attachments: [] });
  failed('conv-idle', { heldId: 'held-idle-fail' });
  vi.advanceTimersByTime(rt.HELD_FAILED_GRACE_MS);
  expect(rt.heldChipsFor('conv-idle')).toHaveLength(0);
  expect(rt.chatInput()).toBe('');
  expect(h.flash).toHaveBeenCalledTimes(1);
  expect(h.flash).toHaveBeenCalledWith("Your waiting message wasn't sent: The folder is gone. It's back in the box. It's in that chat's box when you open it.", 8000);
});

// C10, Codex review of 9a15569c5: a stopped send that came back as a
// forwarded held send escaped Stop when drain_held took it before
// chat_cancel_held could: nothing cancelled the row it became.
describe('a stopped held send drain_held took first', () => {
  const accepted = { sessionKey: 'conv-next', queueId: 0, queuePosition: 0, phase: 'queued' as const, waitingFor: null, heldId: 'held-race', forwardedFrom: 'conv-a' };
  const target = api as unknown as Record<string, ReturnType<typeof vi.fn>>;
  beforeEach(() => {
    target.chatCancelHeld = vi.fn(async () => false);
    target.cancelQueuedRun = vi.fn(async (id: number) => ({ id, status: 'cancelled' }));
    target.chatCancel = vi.fn(async () => undefined);
  });

  it('is followed to the row its heldSent names, and exactly that row is cancelled', async () => {
    const stopping = rt.withdrawStoppedSend('ct-race', accepted);
    await vi.advanceTimersByTimeAsync(0);
    h.listeners.get('chat-event')!({ payload: { type: 'heldSent', heldId: 'held-race', after: 'queue', sessionKey: 'conv-next', queueId: 21, clientTurnId: 'ct-race', conversationKey: 'conv-next' } });
    await expect(stopping).resolves.toBe('withdrawn');
    expect(target.cancelQueuedRun).toHaveBeenCalledWith(21);
    expect(target.chatCancel).not.toHaveBeenCalled();
    expect(rt.takeEarlyHeld('held-race')).toBeNull();
  });

  // R8b review: Rust keeps a just-failed held send until acknowledged, but
  // chat_cancel_held now answers false for it (it is settled), so Stop sees
  // exactly what it saw before: 'failed', the failure's own event returns the
  // words once, and the acknowledgement is its own command.
  it('Stop racing a just-failed held send reports failed, cancels no row and returns the words once', async () => {
    const ack = vi.fn(async () => true);
    target.chatAckHeldFailed = ack;
    target.chatCancelHeld = vi.fn(async () => {
      h.listeners.get('chat-event')!({ payload: { type: 'heldFailed', heldId: 'held-fail-race', error: 'Gone.', prompt: 'Words to keep', sessionKey: 'conv-a', conversationKey: 'conv-a' } });
      return false;
    });
    const stopping = rt.withdrawStoppedSend('ct-fail', { ...accepted, sessionKey: 'conv-a', heldId: 'held-fail-race' });
    await vi.advanceTimersByTimeAsync(0);
    await expect(stopping).resolves.toBe('failed');
    expect(target.cancelQueuedRun).not.toHaveBeenCalled();
    vi.advanceTimersByTime(rt.HELD_FAILED_GRACE_MS);
    expect(rt.chatInput()).toBe('Words to keep');
    expect(ack).toHaveBeenCalledTimes(1);
    expect(ack).toHaveBeenCalledWith('conv-a', 'held-fail-race');
  });

  // Codex re-check of d89017fbd: the destination chat backgrounded, its
  // heldSent came while chat_cancel_held was still answering. The background
  // route only buffered it, so Stop never saw the row it named.
  it('finds a heldSent its backgrounded destination buffered before chat_cancel_held answered', async () => {
    rt.setChatAgentSessionKeyPersistent('conv-next');
    rt.setChatBusy(true);
    rt.beginQueuedChatTurn({ clientTurnId: 'ct-run', text: 'Running', attachments: [], requestedCwd: null, sessionKey: 'conv-next', interactionId: null });
    expect(rt.backgroundRunningChat()).toBe(true);
    rt.resetChatState();
    rt.setChatAgentSessionKeyPersistent('conv-a');
    expect(rt.backgroundChatKeys()).toContain('conv-next');
    target.chatCancelHeld = vi.fn(async () => {
      h.listeners.get('chat-event')!({ payload: { type: 'heldSent', heldId: 'held-bg', after: 'queue', sessionKey: 'conv-next', queueId: 22, clientTurnId: 'ct-bg', conversationKey: 'conv-next' } });
      return false;
    });
    const stopping = rt.withdrawStoppedSend('ct-bg', { ...accepted, heldId: 'held-bg' });
    await vi.advanceTimersByTimeAsync(rt.HELD_OUTCOME_WAIT_MS);
    await expect(stopping).resolves.toBe('withdrawn');
    expect(target.cancelQueuedRun).toHaveBeenCalledWith(22);
    expect(target.chatCancel).not.toHaveBeenCalled();
  });

  it('with no outcome to follow, nothing is cancelled by key and it reads as sent', async () => {
    const stopping = rt.withdrawStoppedSend('ct-lost', { ...accepted, heldId: 'held-lost' });
    await vi.advanceTimersByTimeAsync(rt.HELD_OUTCOME_WAIT_MS);
    await expect(stopping).resolves.toBe('sent');
    expect(target.cancelQueuedRun).not.toHaveBeenCalled();
    expect(target.chatCancel).not.toHaveBeenCalled();
  });
});
