import { describe, expect, it } from 'vitest';
import { CHAT_LIMIT_MAX, chatLimitDrafts, parseChatLimit, serialSaver } from './chatLimits';
import type { ChatLimits } from './types';

describe('Chats at once ceilings (2026-11-07)', () => {
  it('lets 60 run across Hollow and 25 in one workspace, as the backend does', () => {
    // `chat_limits.rs` MAX_AT_ONCE and MAX_PER_WORKSPACE: keep the two in step.
    expect(CHAT_LIMIT_MAX).toEqual({ chatsAtOnce: 60, chatsPerWorkspace: 25, subagentsPerChat: 20 });
    expect(parseChatLimit('60', CHAT_LIMIT_MAX.chatsAtOnce)).toEqual({ value: 60 });
    expect(parseChatLimit('25', CHAT_LIMIT_MAX.chatsPerWorkspace)).toEqual({ value: 25 });
    // The old ceilings (10 and 5) are ordinary values now.
    expect(parseChatLimit('11', CHAT_LIMIT_MAX.chatsAtOnce)).toEqual({ value: 11 });
    expect(parseChatLimit('6', CHAT_LIMIT_MAX.chatsPerWorkspace)).toEqual({ value: 6 });
  });

  it('refuses one past each ceiling and names the new range', () => {
    expect(parseChatLimit('61', CHAT_LIMIT_MAX.chatsAtOnce)).toEqual({ error: 'Enter a whole number from 1 to 60.' });
    expect(parseChatLimit('26', CHAT_LIMIT_MAX.chatsPerWorkspace)).toEqual({ error: 'Enter a whole number from 1 to 25.' });
  });
});

describe('Subagents per chat (2026-11-12)', () => {
  it('takes 1 to 20, as the backend does, and says why otherwise', () => {
    // `chat_limits.rs` MAX_SUBAGENTS_PER_CHAT: keep the two in step.
    expect(CHAT_LIMIT_MAX.subagentsPerChat).toBe(20);
    expect(parseChatLimit('1', CHAT_LIMIT_MAX.subagentsPerChat)).toEqual({ value: 1 });
    expect(parseChatLimit('20', CHAT_LIMIT_MAX.subagentsPerChat)).toEqual({ value: 20 });
    expect(parseChatLimit('21', CHAT_LIMIT_MAX.subagentsPerChat)).toEqual({ error: 'Enter a whole number from 1 to 20.' });
    expect(parseChatLimit('0', CHAT_LIMIT_MAX.subagentsPerChat)).toEqual({ error: 'Enter a whole number from 1 to 20.' });
  });

  it('a change to it sends the other two limits unchanged', async () => {
    const sent: ChatLimits[] = [];
    const saver = serialSaver<ChatLimits>(
      async next => { sent.push(next); return next; },
      async () => ({ chatsAtOnce: 20, chatsPerWorkspace: 10, subagentsPerChat: 6 }),
      () => {},
      () => {},
    );
    saver.set({ chatsAtOnce: 20, chatsPerWorkspace: 10, subagentsPerChat: 6 });
    await saver.save({ subagentsPerChat: 2 });
    expect(sent).toEqual([{ chatsAtOnce: 20, chatsPerWorkspace: 10, subagentsPerChat: 2 }]);
  });

  it('an untouched input follows the backend and a pending edit keeps its draft', () => {
    const d = chatLimitDrafts();
    expect(d.settle({}, { chatsAtOnce: 10, chatsPerWorkspace: 5, subagentsPerChat: 6 }))
      .toEqual({ chatsAtOnce: 10, chatsPerWorkspace: 5, subagentsPerChat: 6 });
    const carried = d.edit('subagentsPerChat', 2);
    expect(d.shown('subagentsPerChat', 6)).toBe(2);
    expect(d.settle(carried, { chatsAtOnce: 10, chatsPerWorkspace: 5, subagentsPerChat: 2 }).subagentsPerChat).toBe(2);
  });
});

describe('Chats at once (2026-10-20)', () => {
  it('takes only a whole number in range, and says why otherwise', () => {
    expect(parseChatLimit('3', 10)).toEqual({ value: 3 });
    expect(parseChatLimit(' 10 ', 10)).toEqual({ value: 10 });
    for (const bad of ['2.5', '0', '11', '', '  ', 'x', '-1']) {
      expect(parseChatLimit(bad, 10), bad).toEqual({ error: 'Enter a whole number from 1 to 10.' });
    }
    expect(parseChatLimit('6', 5)).toEqual({ error: 'Enter a whole number from 1 to 5.' });
  });

  it('a second change before the first reply builds on the first, never its stale value', async () => {
    const sent: ChatLimits[] = [];
    const replies: Array<() => void> = [];
    const saved: ChatLimits[] = [];
    const saver = serialSaver<ChatLimits>(
      next => new Promise(resolve => { sent.push(next); replies.push(() => resolve(next)); }),
      () => Promise.reject(new Error('no read expected')),
      s => saved.push(s),
      () => { throw new Error('no error expected'); },
    );
    saver.set({ chatsAtOnce: 10, chatsPerWorkspace: 5, subagentsPerChat: 6 });
    const first = saver.save({ chatsAtOnce: 2 });
    const second = saver.save({ chatsPerWorkspace: 1 });
    await Promise.resolve();
    expect(sent).toEqual([{ chatsAtOnce: 2, chatsPerWorkspace: 5, subagentsPerChat: 6 }]);
    replies[0]();
    await first;
    await new Promise(r => setTimeout(r, 0));
    expect(sent[1]).toEqual({ chatsAtOnce: 2, chatsPerWorkspace: 1, subagentsPerChat: 6 });
    replies[1]();
    await second;
    expect(saved.at(-1)).toEqual({ chatsAtOnce: 2, chatsPerWorkspace: 1, subagentsPerChat: 6 });
  });

  it('the read after a refused save runs in line, so a later save is never undone (delta review)', async () => {
    let backend: ChatLimits = { chatsAtOnce: 10, chatsPerWorkspace: 5, subagentsPerChat: 6 };
    const sent: ChatLimits[] = [];
    const shown: ChatLimits[] = [];
    let readGate: () => void = () => {};
    const saver = serialSaver<ChatLimits>(
      async next => {
        sent.push(next);
        if (next.chatsAtOnce === 7) throw new Error('refused');
        backend = next;
        return next;
      },
      // A slow recovery read: it answers with what the backend held when asked.
      () => { const answer = backend; return new Promise<ChatLimits>(resolve => { readGate = () => resolve(answer); }); },
      saved => shown.push(saved),
      () => {},
    );
    saver.set(backend);
    const a = saver.save({ chatsAtOnce: 7 });   // refused, then re-read
    const b = saver.save({ chatsAtOnce: 2 });   // made before the re-read answers
    await new Promise(r => setTimeout(r, 0));
    expect(sent).toEqual([{ chatsAtOnce: 7, chatsPerWorkspace: 5, subagentsPerChat: 6 }]);
    readGate();
    await a; await b;
    const c = saver.save({ chatsPerWorkspace: 1 });
    await c;
    expect(sent.at(-1)).toEqual({ chatsAtOnce: 2, chatsPerWorkspace: 1, subagentsPerChat: 6 });
    expect(shown.at(-1)).toEqual({ chatsAtOnce: 2, chatsPerWorkspace: 1, subagentsPerChat: 6 });
  });

  it('a refused save whose re-read fails keeps the acknowledged state', async () => {
    const sent: ChatLimits[] = [];
    const saver = serialSaver<ChatLimits>(
      async next => { sent.push(next); if (next.chatsAtOnce === 7) throw new Error('refused'); return next; },
      () => Promise.reject(new Error('offline')),
      () => {},
      () => {},
    );
    saver.set({ chatsAtOnce: 3, chatsPerWorkspace: 2, subagentsPerChat: 6 });
    await saver.save({ chatsAtOnce: 7 });
    await saver.save({ chatsPerWorkspace: 1 });
    expect(sent.at(-1)).toEqual({ chatsAtOnce: 3, chatsPerWorkspace: 1, subagentsPerChat: 6 });
  });

  it('a refused save leaves the acknowledged state for the next one', async () => {
    let fail = true;
    const sent: ChatLimits[] = [];
    const errors: unknown[] = [];
    const saver = serialSaver<ChatLimits>(
      async next => { sent.push(next); if (fail) throw new Error('Run 1 to 10 chats at once.'); return next; },
      () => Promise.reject(new Error('offline')),
      () => {},
      e => errors.push(e),
    );
    saver.set({ chatsAtOnce: 10, chatsPerWorkspace: 5, subagentsPerChat: 6 });
    await saver.save({ chatsAtOnce: 20 });
    fail = false;
    await saver.save({ chatsPerWorkspace: 2 });
    expect(errors).toHaveLength(1);
    expect(sent[1]).toEqual({ chatsAtOnce: 10, chatsPerWorkspace: 2, subagentsPerChat: 6 });
  });
});

describe('Chats at once: recovery (2026-10-20 delta review)', () => {
  it('a callback that throws never stops the saves after it', async () => {
    const sent: ChatLimits[] = [];
    let throwOnSaved = true;
    const saver = serialSaver<ChatLimits, null>(
      async next => { sent.push(next); if (next.chatsAtOnce === 7) throw new Error('refused'); return next; },
      async () => ({ chatsAtOnce: 10, chatsPerWorkspace: 5, subagentsPerChat: 6 }),
      () => { if (throwOnSaved) { throwOnSaved = false; throw new Error('render failed'); } },
      () => { throw new Error('footer failed'); },
    );
    saver.set({ chatsAtOnce: 10, chatsPerWorkspace: 5, subagentsPerChat: 6 });
    await saver.save({ chatsAtOnce: 3 }).catch(() => {});   // onSaved throws
    await saver.save({ chatsAtOnce: 7 }).catch(() => {});   // onError throws
    await saver.save({ chatsPerWorkspace: 2 });
    expect(sent.at(-1)).toEqual({ chatsAtOnce: 10, chatsPerWorkspace: 2, subagentsPerChat: 6 });
    expect(sent).toHaveLength(3);
  });

  it('every acknowledged answer carries the tag of the change it settles', async () => {
    const answers: Array<[number, string | null, boolean]> = [];
    const saver = serialSaver<ChatLimits, string>(
      async next => { if (next.chatsAtOnce === 7) throw new Error('refused'); return next; },
      async () => ({ chatsAtOnce: 4, chatsPerWorkspace: 2, subagentsPerChat: 6 }),
      (saved, tag, refused) => answers.push([saved.chatsPerWorkspace, tag, refused]),
      () => {},
    );
    await saver.load();
    await saver.save({ chatsAtOnce: 7 }, 'a');
    await saver.save({ chatsAtOnce: 3 }, 'b');
    expect(answers).toEqual([[2, null, false], [2, 'a', true], [2, 'b', false]]);
  });

  it('an untouched field follows every acknowledged answer (another window lowered it)', () => {
    const d = chatLimitDrafts();
    expect(d.settle({}, { chatsAtOnce: 10, chatsPerWorkspace: 5, subagentsPerChat: 6 })).toEqual({ chatsAtOnce: 10, chatsPerWorkspace: 5, subagentsPerChat: 6 });
    const carried = d.edit('chatsAtOnce', 7);
    // Refused; the re-read shows the workspace limit lowered elsewhere to 2.
    expect(d.settle(carried, { chatsAtOnce: 10, chatsPerWorkspace: 2, subagentsPerChat: 6 })).toEqual({ chatsAtOnce: 10, chatsPerWorkspace: 2, subagentsPerChat: 6 });
    const next = d.edit('chatsAtOnce', 3);
    expect(d.settle(next, { chatsAtOnce: 3, chatsPerWorkspace: 2, subagentsPerChat: 6 })).toEqual({ chatsAtOnce: 3, chatsPerWorkspace: 2, subagentsPerChat: 6 });
  });

  it('a refusal restores a field only if no newer edit came after it, whatever the value', () => {
    const d = chatLimitDrafts();
    d.settle({}, { chatsAtOnce: 10, chatsPerWorkspace: 5, subagentsPerChat: 6 });
    const first = d.edit('chatsAtOnce', 7);   // refused later
    const second = d.edit('chatsAtOnce', 8);
    const third = d.edit('chatsAtOnce', 7);   // the same value as the refused one
    // The refusal's re-read: the newer 7 stays; the untouched field follows.
    expect(d.settle(first, { chatsAtOnce: 10, chatsPerWorkspace: 5, subagentsPerChat: 6 })).toEqual({ chatsPerWorkspace: 5, subagentsPerChat: 6 });
    expect(d.shown('chatsAtOnce', 10)).toBe(7);
    expect(d.settle(second, { chatsAtOnce: 8, chatsPerWorkspace: 5, subagentsPerChat: 6 })).toEqual({ chatsPerWorkspace: 5, subagentsPerChat: 6 });
    expect(d.settle(third, { chatsAtOnce: 7, chatsPerWorkspace: 5, subagentsPerChat: 6 })).toEqual({ chatsAtOnce: 7, chatsPerWorkspace: 5, subagentsPerChat: 6 });
    expect(d.shown('chatsAtOnce', 7)).toBe(7);
    // Typing not yet committed survives an answer; refused typing goes back
    // to the last submission (pending: its draft; settled: what is saved).
    d.typing('chatsAtOnce');
    expect(d.settle({}, { chatsAtOnce: 7, chatsPerWorkspace: 5, subagentsPerChat: 6 })).toEqual({ chatsPerWorkspace: 5, subagentsPerChat: 6 });
    d.abandon('chatsAtOnce');
    expect(d.shown('chatsAtOnce', 7)).toBe(7);
    const pending = d.edit('chatsAtOnce', 6);
    d.typing('chatsAtOnce');
    d.abandon('chatsAtOnce');
    expect(d.shown('chatsAtOnce', 7), 'its pending draft').toBe(6);
    expect(d.settle(pending, { chatsAtOnce: 6, chatsPerWorkspace: 5, subagentsPerChat: 6 })).toEqual({ chatsAtOnce: 6, chatsPerWorkspace: 5, subagentsPerChat: 6 });
    // Its own refusal, with nothing newer: back to what is saved.
    const lone = d.edit('chatsPerWorkspace', 4);
    expect(d.settle(lone, { chatsAtOnce: 6, chatsPerWorkspace: 5, subagentsPerChat: 6 })).toEqual({ chatsAtOnce: 6, chatsPerWorkspace: 5, subagentsPerChat: 6 });
  });
});
