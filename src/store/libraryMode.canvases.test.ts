import { beforeEach, describe, expect, it, vi } from 'vitest';

// Library's Results filter and All's filter live in libraryMode.ts (overnight
// 11-12 L4). Canvases is Results with the type filter on `canvases`; leaving
// Library resets both filters; an accepted open publishes the workspace
// filter and bumps the open counter so a link opened while Canvases is shown
// still applies.
const rt = vi.hoisted(() => {
  vi.stubGlobal('localStorage', { getItem: () => null, setItem: () => {}, removeItem: () => {} });
  return { page: 'vault' };
});
vi.mock('./runtime', () => ({
  activePage: () => rt.page, setActivePage: vi.fn(), setOpenVaultNoteRequest: vi.fn(), requestOpenSkill: vi.fn(),
}));
vi.mock('./workspaces', () => ({ goToPage: vi.fn() }));
vi.mock('../viewTransitions', () => ({ flushViewTransition: vi.fn() }));

import {
  canvasOpenRequest, canvasStartWorkspace, closeLibraryCanvases, libraryAllFilter, libraryCanvasesShown, libraryMode,
  openLibraryCanvases, resetResultsType, resultsType, setLibraryAllFilter, setLibraryMode, setResultsType,
} from './libraryMode';
import { takeGalleryCanvas } from '../canvas/galleryRequest';
import { registerNavigationGuard } from './navigationGuard';

beforeEach(() => {
  rt.page = 'vault';
  setLibraryMode('all');
  resetResultsType();
  takeGalleryCanvas();
});

describe('Canvases view', () => {
  it('is shown only on the Library page, in Results, with the canvases type', () => {
    expect(libraryCanvasesShown()).toBe(false);
    setLibraryMode('results');
    setResultsType('files');
    expect(libraryCanvasesShown()).toBe(false);
    setResultsType('canvases');
    expect(libraryCanvasesShown()).toBe(true);
    rt.page = 'today';
    expect(libraryCanvasesShown()).toBe(false);
    rt.page = 'vault';
    setLibraryMode('notes');
    expect(libraryCanvasesShown()).toBe(false);
  });

  it('opening it shows Results on canvases, filtered to the workspace, and bumps the request', async () => {
    const before = canvasOpenRequest();
    await openLibraryCanvases('ws-1');
    expect(libraryMode()).toBe('results');
    expect(resultsType()).toBe('canvases');
    expect(canvasStartWorkspace()).toBe('ws-1');
    expect(canvasOpenRequest()).toBe(before + 1);
    await openLibraryCanvases();
    expect(canvasStartWorkspace()).toBe('');
    expect(canvasOpenRequest()).toBe(before + 2);
  });

  it('publishes a Compare request only when the open is accepted', async () => {
    const request = { workspaceId: 'ws-1', id: 'c1', compare: true, back: null };
    const off = registerNavigationGuard(async () => false);
    await openLibraryCanvases('ws-1', request);
    off();
    expect(takeGalleryCanvas()).toBeNull();
    expect(resultsType()).toBe('all');
    await openLibraryCanvases('ws-1', request);
    expect(takeGalleryCanvas()).toEqual(request);
    expect(takeGalleryCanvas()).toBeNull();
  });

  it('an ordinary open clears a Compare request left by an earlier one', async () => {
    await openLibraryCanvases('ws-1', { workspaceId: 'ws-1', id: 'c1', compare: true, back: null });
    await openLibraryCanvases();
    expect(takeGalleryCanvas()).toBeNull();
  });
});

describe('closeLibraryCanvases', () => {
  it('clears the canvases filter, but leaves any other type alone', () => {
    setResultsType('canvases');
    closeLibraryCanvases();
    expect(resultsType()).toBe('all');
    setResultsType('files');
    closeLibraryCanvases();
    expect(resultsType()).toBe('files');
  });
});

describe('leaving Library', () => {
  it('resets Results to every type and All to Recent', () => {
    setResultsType('files');
    setLibraryAllFilter('meetings');
    expect(libraryAllFilter()).not.toBe('recent');
    resetResultsType();
    expect(resultsType()).toBe('all');
    expect(libraryAllFilter()).toBe('recent');
  });
});
