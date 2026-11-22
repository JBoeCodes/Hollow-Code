import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';

// Settings' link rows and page jumps ask about unsaved changes like a search
// page hit does (#1317 re-review).
const s = vi.hoisted(() => {
  vi.stubGlobal('localStorage', { getItem: () => null, setItem: () => {}, removeItem: () => {} });
  vi.stubGlobal('HTMLElement', class {});
  vi.stubGlobal('document', { activeElement: null, body: {}, querySelector: () => null, getElementById: () => null });
  return { dirty: [] as string[], open: vi.fn() };
});
vi.mock('../store', () => ({ openSettingsPageTarget: s.open, setSettingsSectionRequest: vi.fn(), settingsSectionRequest: () => null }));
vi.mock('./settingsDirty', () => ({ dirtyLabels: () => s.dirty, clearSavedCue: vi.fn() }));

import { createSettingsNavigation } from './controllers/settingsNavigation';

beforeEach(() => { s.open.mockClear(); s.dirty = []; });
const nav = () => createRoot(() => createSettingsNavigation());

describe('leaving Settings for a Library tab', () => {
  it('opens at once when nothing is unsaved', () => {
    nav().openPageTarget({ page: 'vault', tab: 'history' });
    expect(s.open).toHaveBeenCalledWith({ page: 'vault', tab: 'history' });
  });

  it('asks first when a form is unsaved, then opens on Leave and stays on Keep editing', () => {
    s.dirty = ['Disk space warning'];
    const n = nav();
    n.openPageTarget({ page: 'vault', tab: 'history' });
    expect(s.open).not.toHaveBeenCalled();
    expect(n.pendingSwitch()).toMatchObject({ to: 'Library', unsaved: ['Disk space warning'] });
    n.keepEditing();
    expect(n.pendingSwitch()).toBeNull();
    expect(s.open).not.toHaveBeenCalled();
    n.openPageTarget({ page: 'vault', tab: 'history' });
    n.leaveWithoutSaving();
    expect(s.open).toHaveBeenCalledWith({ page: 'vault', tab: 'history' });
  });

  it('a jump to a control that moved to a page asks too', () => {
    s.dirty = ['Disk space warning'];
    const n = nav();
    n.jumpTo('history');
    expect(s.open).not.toHaveBeenCalled();
    expect(n.pendingSwitch()?.unsaved).toEqual(['Disk space warning']);
    n.leaveWithoutSaving();
    expect(s.open).toHaveBeenCalledWith(expect.objectContaining({ page: 'vault', tab: 'history' }));
  });
});
