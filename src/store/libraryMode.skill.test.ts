import { beforeEach, describe, expect, it, vi } from 'vitest';

// The palette's skill row (#1317 re-review): its "open this skill" request is
// set only once the view change is accepted, so a refused one leaves nothing
// for a later plain Library, Skills to pick up.
const rt = vi.hoisted(() => {
  vi.stubGlobal('localStorage', { getItem: () => null, setItem: () => {}, removeItem: () => {} });
  return { page: 'vault', request: vi.fn(), goToPage: vi.fn() };
});
vi.mock('./runtime', () => ({
  activePage: () => rt.page, setActivePage: vi.fn(), setOpenVaultNoteRequest: vi.fn(), requestOpenSkill: rt.request,
}));
vi.mock('./workspaces', () => ({ goToPage: rt.goToPage }));
vi.mock('../viewTransitions', () => ({ flushViewTransition: vi.fn() }));

import { openLibrarySkill, libraryMode } from './libraryMode';
import { registerNavigationGuard } from './navigationGuard';

beforeEach(() => { rt.request.mockClear(); rt.goToPage.mockClear(); rt.page = 'vault'; });

describe('openLibrarySkill', () => {
  it('names the skill once the view change is accepted', async () => {
    await openLibrarySkill('release-notes');
    expect(libraryMode()).toBe('skills');
    expect(rt.request).toHaveBeenCalledWith('release-notes');
  });

  it('names nothing when an unsaved draft is kept (Keep editing)', async () => {
    const off = registerNavigationGuard(async () => false);
    await openLibrarySkill('release-notes');
    off();
    expect(rt.request).not.toHaveBeenCalled();
  });

  it('from another page, names the skill only when the page jump is applied', async () => {
    rt.page = 'workflows';
    await openLibrarySkill('release-notes');
    expect(rt.request).not.toHaveBeenCalled();
    const { onNavigate } = rt.goToPage.mock.calls[0][1] as { onNavigate: () => void };
    onNavigate();
    expect(rt.request).toHaveBeenCalledWith('release-notes');
  });
});
