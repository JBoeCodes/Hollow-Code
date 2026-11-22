import { describe, expect, it } from 'vitest';
import { canPinWorkspace, dropPosition, pinAnnouncement, pinOrder, pinStep } from './workspacePins';
import type { WorkWorkspace } from './workspaceTypes';

const w = (id: string, pinned?: number, status = 'active', kind?: 'quick' | 'main'): WorkWorkspace =>
  ({ id, title: id.toUpperCase(), status, path: `workspaces/${id}`, cwd: id, modified: '', ...(pinned != null ? { pinned } : {}), ...(kind ? { kind } : {}) });

describe('pinned workspaces (2026-10-21)', () => {
  it('orders pins by rank, then id, and never lists an archived or built-in pin', () => {
    expect(pinOrder([w('c', 2), w('a'), w('b', 1), w('d', 2), w('x', 1, 'Archived'), w('quick-chats', 1, 'active', 'quick')])).toEqual(['b', 'c', 'd']);
    expect(pinOrder([w('a'), w('b')])).toEqual([]);
  });
  it('can pin any workspace but archived and built-in ones', () => {
    expect(canPinWorkspace(w('a'))).toBe(true);
    expect(canPinWorkspace(w('a', undefined, 'done'))).toBe(true);
    expect(canPinWorkspace(w('a', undefined, ' archived '))).toBe(false);
    expect(canPinWorkspace(w('main-agent', undefined, 'active', 'main'))).toBe(false);
  });
  it('places a dropped pin before or after the header it lands on', () => {
    const pins = ['a', 'b', 'c'];
    expect(dropPosition(pins, 'c', 'a', false)).toBe(1);
    expect(dropPosition(pins, 'a', 'c', true)).toBe(3);
    expect(dropPosition(pins, 'a', 'b', true)).toBe(2);
    expect(dropPosition(pins, 'c', 'b', false)).toBe(2);
    // Where it already is, onto itself, or onto a row outside Pinned: nothing.
    expect(dropPosition(pins, 'b', 'a', true)).toBeNull();
    expect(dropPosition(pins, 'b', 'c', false)).toBeNull();
    expect(dropPosition(pins, 'b', 'b', true)).toBeNull();
    expect(dropPosition(pins, 'b', 'z', true)).toBeNull();
  });
  it('says what changed', () => {
    expect(pinAnnouncement('pin', 'Hollow')).toBe('Pinned Hollow');
    expect(pinAnnouncement('unpin', 'Hollow')).toBe('Unpinned Hollow');
    expect(pinAnnouncement('move', 'Hollow', 2, 3)).toBe('Moved Hollow to 2 of 3 in Pinned');
  });
});

describe('pinStep (ENG-116)', () => {
  const pins = ['a', 'hidden', 'b', 'c'];
  it('steps over a pin the sidebar does not show', () => {
    // b up lands before a (place 1), not before the hidden pin (a no-op on screen).
    expect(pinStep(pins, ['hidden'], 'b', -1)).toBe(1);
    // a down lands after b (place 3).
    expect(pinStep(pins, ['hidden'], 'a', 1)).toBe(3);
  });
  it('matches the plain step with nothing hidden, and stops at either end', () => {
    expect(pinStep(['a', 'b', 'c'], [], 'b', -1)).toBe(1);
    expect(pinStep(['a', 'b', 'c'], [], 'b', 1)).toBe(3);
    expect(pinStep(['a', 'hidden', 'b'], ['hidden'], 'a', -1)).toBeNull();
    expect(pinStep(['a', 'b', 'hidden'], ['hidden'], 'b', 1)).toBeNull();
  });
  it('a hidden pin steps among all the pins', () => {
    expect(pinStep(pins, ['hidden'], 'hidden', -1)).toBe(1);
  });
});
