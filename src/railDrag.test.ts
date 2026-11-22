import { describe, expect, it } from 'vitest';
import { railDropPlan } from './railDrag';

const active = { pinned: false, settled: false, snoozedUntil: null };
describe('sidebar drop and menu parity', () => {
  it('pins an active or settled chat with the existing pin action', () => {
    expect(railDropPlan('pinned').action).toBe('pin');
    expect(railDropPlan('pinned', { ...active, settled: true }).action).toBe('pin');
  });
  it('returns pinned, snoozed and settled chats to active with the matching action', () => {
    expect(railDropPlan('active', { ...active, pinned: true }).action).toBe('unpin');
    expect(railDropPlan('active', { ...active, settled: true }).action).toBe('wake');
    expect(railDropPlan('active', { ...active, snoozedUntil: '2030-01-01T00:00:00Z' }).action).toBe('wake');
  });
  it('does not defeat pin or live-work settlement protection', () => {
    expect(railDropPlan('settled', { ...active, pinned: true })).toMatchObject({ action: null, label: 'Unpin first' });
    expect(railDropPlan('settled', active, true).action).toBeNull();
    expect(railDropPlan('settled', active).action).toBe('settle');
  });
  it('leaves a chat already in its destination unchanged', () => {
    expect(railDropPlan('active', active).action).toBeNull();
    expect(railDropPlan('pinned', { ...active, pinned: true }).action).toBeNull();
    expect(railDropPlan('settled', { ...active, settled: true }).action).toBeNull();
  });
});
