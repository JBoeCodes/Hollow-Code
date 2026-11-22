import { beforeEach, describe, expect, it, vi } from 'vitest';
const state = vi.hoisted(() => ({ bars: [] as { id: string; undo?: () => void }[], performed: [] as string[], leaving: new Set<string>() }));
vi.mock('./snackbars', () => ({ snackbars: () => state.bars, leavingSnackbars: () => state.leaving, performUndo: async (id: string) => { state.performed.push(id); } }));
import { organizationUndoReady, rememberOrganizationUndo, undoLastOrganization } from './organizationUndo';

beforeEach(() => { state.bars = []; state.performed = []; state.leaving = new Set(); rememberOrganizationUndo(undefined); });
describe('organization undo', () => {
  it('is ready only while its notice shows', () => {
    expect(organizationUndoReady()).toBe(false);
    rememberOrganizationUndo('s_1');
    expect(organizationUndoReady()).toBe(false);
    state.bars = [{ id: 's_1', undo: () => {} }];
    expect(organizationUndoReady()).toBe(true);
    state.bars = [];
    expect(organizationUndoReady()).toBe(false);
    expect(undoLastOrganization()).toBe(false);
  });
  it('is not ready while its notice plays its exit', () => {
    state.bars = [{ id: 's_3', undo: () => {} }];
    rememberOrganizationUndo('s_3');
    state.leaving = new Set(['s_3']);
    expect(organizationUndoReady()).toBe(false);
    expect(undoLastOrganization()).toBe(false);
  });
  it('runs the newest notice once', () => {
    state.bars = [{ id: 's_2', undo: () => {} }];
    rememberOrganizationUndo('s_2');
    expect(undoLastOrganization()).toBe(true);
    expect(state.performed).toEqual(['s_2']);
    expect(undoLastOrganization()).toBe(false);
  });
});
