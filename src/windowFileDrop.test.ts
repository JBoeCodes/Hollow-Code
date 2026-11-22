import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { QUICK_CHATS_ID } from './workspaceTypes';

// The store's chat and workspace pieces the drop routing reads, as plain
// values a test sets.
const s = vi.hoisted(() => ({
  page: 'today', keepPage: false, version: 1, cwd: '/v/workspaces/q3' as string | null,
  onScreen: true, workspaceId: null as string | null, detail: null as null | { workspace: { id: string; title: string; kind?: string; cwd?: string } },
  focusOk: true, quickOk: true, focusCalls: 0, quickCalls: 0, focusRequests: 0, showsAfterOpen: true, switchTo: null as string | null,
  attached: [] as { name: string; known: boolean }[], footers: [] as string[],
}));
vi.mock('./store', () => ({
  chatComposerOnScreen: () => s.onScreen,
  chatConversationVersion: () => s.version,
  chatCwd: () => s.cwd,
  activePage: () => s.page,
  conversationHistoryShown: () => false,
  // A page with unsaved edits that the person keeps answers no.
  setActivePage: async (p: string) => { if (!s.keepPage) s.page = p; },
  attachChatImage: (file: File, _queue: boolean, known: boolean) => { s.attached.push({ name: file.name, known }); return true; },
  flashFooter: (text: string) => { s.footers.push(text); },
  requestChatFocus: () => { s.focusRequests++; },
}));
vi.mock('./store/workspaces', () => ({
  workspaceId: () => s.workspaceId,
  workspaceDetail: () => s.detail,
  // Opening shows the chat's message box, unless a test says what else happens.
  focusWorkspaceConversation: async () => { s.focusCalls++; if (s.focusOk && s.showsAfterOpen) s.onScreen = true; if (s.switchTo) s.workspaceId = s.switchTo; return s.focusOk; },
  startQuickChatForFiles: async () => { s.quickCalls++; if (s.quickOk) { s.workspaceId = QUICK_CHATS_ID; s.version++; if (s.showsAfterOpen) s.onScreen = true; } return s.quickOk; },
}));

import { FOLDER_REFUSED } from './composerDrop';
import { dropTargetLabel, routeDroppedFiles, routeDroppedItems } from './windowFileDrop';

const pdf = { file: new File(['x'], 'report.pdf', { type: 'application/pdf' }), known: true };
const csv = { file: new File(['a,b'], 'data.csv', { type: 'text/csv' }), known: false };

beforeEach(() => {
  Object.assign(s, { page: 'today', keepPage: false, version: 1, cwd: '/v/workspaces/q3', onScreen: true, workspaceId: null, detail: null, focusOk: true, quickOk: true, focusCalls: 0, quickCalls: 0, focusRequests: 0, showsAfterOpen: true, switchTo: null, attached: [], footers: [] });
});

describe('a file dropped anywhere on the window (2026-11-04)', () => {
  it('attaches to the chat on screen, with the drop’s own file flag', async () => {
    expect(dropTargetLabel()).toBe('Drop to attach to this chat');
    await routeDroppedFiles([pdf, csv], []);
    expect(s.attached).toEqual([{ name: 'report.pdf', known: true }, { name: 'data.csv', known: false }]);
    expect(s.focusCalls + s.quickCalls).toBe(0);
    expect(s.focusRequests).toBe(1);
  });

  it('brings back a project’s chat hidden behind another page, then attaches there', async () => {
    Object.assign(s, { onScreen: false, workspaceId: 'q3', detail: { workspace: { id: 'q3', title: 'Q3 review', cwd: '/v/workspaces/q3' } } });
    expect(dropTargetLabel()).toBe('Drop to attach to the chat in Q3 review');
    await routeDroppedFiles([pdf], []);
    expect(s.focusCalls).toBe(1);
    expect(s.quickCalls).toBe(0);
    expect(s.attached).toEqual([{ name: 'report.pdf', known: true }]);
  });

  it('names an open quick chat as your open chat', () => {
    Object.assign(s, { onScreen: false, workspaceId: 'quick-chats', detail: { workspace: { id: 'quick-chats', title: 'Chats', kind: 'quick' } } });
    expect(dropTargetLabel()).toBe('Drop to attach to your open chat');
  });

  it('opens a new quick chat where none is open (Home), and the files wait for it', async () => {
    s.onScreen = false;
    expect(dropTargetLabel()).toBe('Drop to attach to a new quick chat');
    await routeDroppedFiles([csv], []);
    expect(s.quickCalls).toBe(1);
    expect(s.attached).toEqual([{ name: 'data.csv', known: false }]);
  });

  it('attaches nothing, and says so, when no chat could open', async () => {
    Object.assign(s, { onScreen: false, quickOk: false });
    await routeDroppedFiles([pdf, csv], []);
    expect(s.attached).toEqual([]);
    expect(s.footers.at(-1)).toBe('× The files weren’t attached: no chat could open. Drop them again.');
  });

  it('refuses folders in words and still attaches the files beside them', async () => {
    await routeDroppedFiles([pdf], ['Designs']);
    expect(s.footers).toContain(FOLDER_REFUSED);
    expect(s.attached).toEqual([{ name: 'report.pdf', known: true }]);
  });

  it('never attaches to another project picked while the first one opened', async () => {
    Object.assign(s, { onScreen: false, workspaceId: 'q3', detail: { workspace: { id: 'q3', title: 'Q3 review', cwd: '/v/workspaces/q3' } }, switchTo: 'other' });
    await routeDroppedFiles([pdf], []);
    expect(s.attached).toEqual([]);
    expect(s.footers.at(-1)).toBe('× The file wasn’t attached: its chat didn’t show. Drop it again.');
  });

  describe('a message box that never shows', () => {
    beforeEach(() => { vi.useFakeTimers({ toFake: ['setTimeout', 'Date'] }); });
    afterEach(() => { vi.useRealTimers(); });
    it('takes nothing: a page kept for its unsaved edits, say', async () => {
      Object.assign(s, { onScreen: false, workspaceId: 'q3', detail: { workspace: { id: 'q3', title: 'Q3 review', cwd: '/v/workspaces/q3' } }, showsAfterOpen: false });
      const routed = routeDroppedFiles([pdf], []);
      await vi.advanceTimersByTimeAsync(10_200);
      await routed;
      expect(s.attached).toEqual([]);
      expect(s.footers.at(-1)).toBe('× The file wasn’t attached: its chat didn’t show. Drop it again.');
    });
  });

  it('splits what the window read as the message box splits its drops', async () => {
    // The entry API's word decides: a folder is refused, an empty file with
    // no extension is still a file when it says so.
    const empty = new File([], 'LICENSE');
    await routeDroppedItems([
      { file: pdf.file, folder: false, name: 'report.pdf' },
      { file: new File([], 'Designs'), folder: true, name: 'Designs' },
      { file: empty, folder: false, name: 'LICENSE' },
    ], { version: s.version, workspace: null, onScreen: true });
    expect(s.footers).toContain(FOLDER_REFUSED);
    expect(s.attached).toEqual([{ name: 'report.pdf', known: true }, { name: 'LICENSE', known: true }]);
  });

  it('attaches nothing when the chat changed between the drop and its routing', async () => {
    const origin = { version: s.version, workspace: null, onScreen: true };
    s.version++;
    await routeDroppedFiles([pdf], [], origin);
    expect(s.attached).toEqual([]);
    expect(s.footers.at(-1)).toBe('× The file wasn’t attached: the chat changed first. Drop it again.');
  });

  it('asks the page first, and a page kept for its unsaved edits takes no drop', async () => {
    Object.assign(s, { page: 'workflows', keepPage: true, onScreen: false, workspaceId: 'q3', detail: { workspace: { id: 'q3', title: 'Q3 review', cwd: '/v/workspaces/q3' } } });
    await routeDroppedFiles([pdf], []);
    expect(s.focusCalls).toBe(0);
    expect(s.attached).toEqual([]);
    expect(s.footers.at(-1)).toBe('× The file wasn’t attached: you stayed on this page. Drop it again.');
  });

  it('never reopens another conversation for a drop: its chat must be the one held', async () => {
    Object.assign(s, { onScreen: false, workspaceId: 'q3', detail: { workspace: { id: 'q3', title: 'Q3 review', cwd: '/v/workspaces/q3' } }, cwd: '/v/workspaces/other' });
    await routeDroppedFiles([pdf], []);
    expect(s.focusCalls).toBe(0);
    expect(s.attached).toEqual([]);
    expect(s.footers.at(-1)).toBe('× The file wasn’t attached: its chat isn’t open; open it first. Drop it again.');
  });

  it('does nothing more for a drop of folders alone', async () => {
    s.onScreen = false;
    await routeDroppedFiles([], ['Designs']);
    expect(s.quickCalls + s.focusCalls).toBe(0);
    expect(s.attached).toEqual([]);
  });
});
