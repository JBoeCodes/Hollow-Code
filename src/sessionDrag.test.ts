import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  SESSION_DRAG_MIME, dispatchSessionDrop, encodeSessionDrag, isSessionDrag, parseSessionDrag,
  registerSessionDropHandler, type SessionDragPayload,
} from './sessionDrag';

describe('the conversation drag contract', () => {
  const off: (() => void)[] = [];
  afterEach(() => { while (off.length) off.pop()!(); });

  it('round-trips a session payload under its MIME type', () => {
    expect(SESSION_DRAG_MIME).toBe('application/x-hollow-session');
    const payload: SessionDragPayload = { kind: 'session', key: 's-20260930-abcd0001' };
    expect(parseSessionDrag(encodeSessionDrag(payload))).toEqual(payload);
    // Another kind keeps its own fields.
    const external = { kind: 'external-session', source: 'codex', id: 'rollout-1' };
    expect(parseSessionDrag(encodeSessionDrag(external))).toEqual(external);
  });

  it('refuses anything malformed', () => {
    for (const bad of [null, undefined, '', 'not json', '[]', '"s-1"', '{}', '{"kind":3}', '{"kind":""}',
      '{"kind":"session"}', '{"kind":"session","key":"../etc"}', '{"kind":"session","key":"S-UPPER"}', '{"kind":"session","key":42}']) {
      expect(parseSessionDrag(bad), String(bad)).toBeNull();
    }
  });

  it('recognizes a conversation drag by its types alone', () => {
    expect(isSessionDrag(['text/plain', SESSION_DRAG_MIME])).toBe(true);
    expect(isSessionDrag(['text/plain', 'Files'])).toBe(false);
    expect(isSessionDrag(null)).toBe(false);
  });

  it('dispatches a drop to the handler registered for its kind', async () => {
    const session = vi.fn(async () => {});
    const external = vi.fn(async () => {});
    off.push(registerSessionDropHandler<SessionDragPayload>('session', session));
    off.push(registerSessionDropHandler('external-session', external));
    expect(await dispatchSessionDrop(encodeSessionDrag({ kind: 'session', key: 's-1' }), 'beta-2')).toBe(true);
    expect(session).toHaveBeenCalledWith({ kind: 'session', key: 's-1' }, 'beta-2');
    expect(await dispatchSessionDrop(encodeSessionDrag({ kind: 'external-session', source: 'claude', id: 'x' }), 'quick-chats')).toBe(true);
    expect(external).toHaveBeenCalledWith({ kind: 'external-session', source: 'claude', id: 'x' }, 'quick-chats');
    // Unknown kind, malformed data or no target: nothing runs.
    expect(await dispatchSessionDrop(encodeSessionDrag({ kind: 'nope' }), 'beta-2')).toBe(false);
    expect(await dispatchSessionDrop('garbage', 'beta-2')).toBe(false);
    expect(await dispatchSessionDrop(encodeSessionDrag({ kind: 'session', key: 's-1' }), '')).toBe(false);
    expect(session).toHaveBeenCalledTimes(1);
  });

  it('unregisters only its own handler and surfaces handler failures', async () => {
    const first = vi.fn(async () => {});
    const second = vi.fn(async () => { throw new Error('refused'); });
    const offFirst = registerSessionDropHandler('session', first);
    off.push(registerSessionDropHandler('session', second));
    offFirst();
    await expect(dispatchSessionDrop(encodeSessionDrag({ kind: 'session', key: 's-2' }), 'a')).rejects.toThrow('refused');
    expect(first).not.toHaveBeenCalled();
  });
});
