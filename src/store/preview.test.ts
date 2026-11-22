import { beforeEach, describe, expect, it } from 'vitest';
import {
  canPreviewBack, canPreviewForward, closePreview, dropConversationPreviews, dropSubagentPreviews, subagentTabParents, openPreview, previewBack, previewForward,
  previewNonce, previewTarget, resetPreviewForTests, takePreviewWebLoad, targetOf, togglePreview, PREVIEW_HISTORY_CAP,
  composerPage,
  closePreviewDocument, previewDocuments, previewDocumentKey, selectPreviewWeb, PREVIEW_DOCUMENT_CAP,
  openNewTab, replacePreviewDocument, previewCardShown, newTabStillFor, newTabLookupStillValid, previewIntentNow, DRAFT_TAB_OWNER,
  canReopenClosedView, closedViewLabel, reopenClosedView, CLOSED_VIEW_CAP,
} from './preview';
import chatPreviewSource from '../components/ChatPreview.tsx?raw';
import chatRuntimeSource from './chatRuntime.ts?raw';
import previewPaneSource from '../components/PreviewPane.tsx?raw';

const flat = (text: string) => text.replace(/\s+/g, '');
/** A function's body in flattened source: from its signature to the next top-level `export`. */
function body(source: string, signature: string): string {
  const text = flat(source);
  const start = text.indexOf(flat(signature));
  if (start < 0) throw new Error(`missing ${signature}`);
  const next = text.indexOf('export', start + flat(signature).length);
  return text.slice(start, next < 0 ? undefined : next);
}

// Run 15 P1 review: a conversation coming back from the background (not
// through resetChatState) also drops the previous conversation's visuals,
// and the pane hands focus back only after its DOM is gone (a compact
// window's page is hidden until then).
describe('preview pane wiring', () => {
  it('restoring a background conversation drops the old one’s visuals', () => {
    expect(body(chatRuntimeSource, 'export function restoreBackgroundChat(key: string)')).toContain('dropConversationPreviews();');
  });
  it('closing gives focus back after the pane is removed, unless focus went elsewhere', () => {
    expect(previewPaneSource).toContain("giveFocusBack(back, '#preview-pane')");
    expect(previewPaneSource).not.toMatch(/onCleanup\(\(\) => \{\s*if \(opener instanceof HTMLElement/);
  });
  it('the web mode loads what an open, back or forward asked for', () => {
    expect(previewPaneSource).toContain('takePreviewWebLoad()');
    expect(previewPaneSource).toContain('on(previewWebLoad');
  });
  it('Ask the agent to fix this file brings a composer on screen first, and queues nothing when that is refused', () => {
    const ask = body(chatPreviewSource, 'async function askToFix()');
    expect(ask).toContain('composerPage(shownPage(),');
    // An unsaved editor may ask before the page changes; Cancel leaves no draft queued.
    expect(ask).toContain('awaitsetActivePage(page);if(shownPage()!==page)return;');
    expect(ask.indexOf('awaitsetActivePage(page)')).toBeLessThan(ask.indexOf('askClaudeInChat('));
  });
});

// Run 15 P1: the right pane is a Preview pane that shows only what the person
// asked for. Nothing is previewed until something is opened; closing hides it
// and keeps its history; back and forward walk what was opened.
describe('preview pane state', () => {
  beforeEach(() => resetPreviewForTests());

  it('keeps one bounded document tab per path and retains the latest citation', () => {
    openPreview({ kind: 'file', path: '/v/a.ts', line: 2 });
    openPreview({ kind: 'file', path: '/v/a.ts', line: 8, end: 10 });
    expect(previewDocuments()).toEqual([{ kind: 'file', path: '/v/a.ts', line: 8, end: 10 }]);
    for (let i = 0; i < PREVIEW_DOCUMENT_CAP; i++) openPreview(`/v/${i}.pdf`);
    expect(previewDocuments()).toHaveLength(PREVIEW_DOCUMENT_CAP);
    // History outlives the tab cap, and going back restores the tab/citation.
    for (let i = 0; i < PREVIEW_DOCUMENT_CAP; i++) previewBack();
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/a.ts', line: 8, end: 10 });
    expect(previewDocuments()).toContainEqual(previewTarget());
  });

  it('closing an inactive document removes all of its history without moving the active one', () => {
    openPreview('/v/a.md'); openPreview('/v/b.pdf'); openPreview('/v/a.md'); openPreview('/v/c.md');
    closePreviewDocument(previewDocumentKey({ kind: 'file', path: '/v/a.md' }));
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/c.md' });
    previewBack();
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/b.pdf' });
    expect(canPreviewBack()).toBe(false);
  });

  it('closing the active document selects another; closing the last hides the pane', () => {
    openPreview('/v/a.md'); openPreview('/v/b.pdf');
    closePreviewDocument(previewDocumentKey({ kind: 'file', path: '/v/b.pdf' }));
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/a.md' });
    closePreviewDocument(previewDocumentKey({ kind: 'file', path: '/v/a.md' }));
    expect(previewTarget()).toBeNull();
    togglePreview(); expect(previewTarget()).toMatchObject({ kind: 'newtab' });
  });

  it('closing the last document never reloads a closed web tab from history', () => {
    openPreview('https://closed.example/');
    takePreviewWebLoad();
    openPreview('/v/last.pdf');
    closePreviewDocument(previewDocumentKey({ kind: 'file', path: '/v/last.pdf' }));
    expect(previewTarget()).toBeNull();
    expect(takePreviewWebLoad()).toBeNull();
  });

  it('selecting a live web tab records history without reloading it', () => {
    openPreview('/v/a.pdf');
    selectPreviewWeb('https://example.com/form');
    expect(takePreviewWebLoad()).toBeNull();
    expect(previewTarget()).toEqual({ kind: 'url', url: 'https://example.com/form' });
    previewBack();
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/a.pdf' });
    previewForward();
    expect(takePreviewWebLoad()).toBe('https://example.com/form');
  });

  it('conversation changes drop visual tabs and preserve app-wide files', () => {
    openPreview('/v/a.pdf'); openPreview({ kind: 'visual', key: 'old:0' });
    dropConversationPreviews();
    expect(previewDocuments()).toEqual([{ kind: 'file', path: '/v/a.pdf' }]);
  });

  it('another chat drops the Subagents tabs outside its chain and their history (L3)', () => {
    openPreview('/v/a.pdf');
    openPreview({ kind: 'subagents', parent: 's-a' });
    openPreview({ kind: 'subagents', parent: 's-a', open: 's-sub-1' });
    openPreview({ kind: 'subagents', parent: 's-b' });
    // One tab per chat, remembering the subagent it last showed.
    expect(previewDocuments()).toEqual([{ kind: 'file', path: '/v/a.pdf' }, { kind: 'subagents', parent: 's-a', open: 's-sub-1' }, { kind: 'subagents', parent: 's-b' }]);
    expect(subagentTabParents()).toEqual(['s-a', 's-b']);
    // s-a handed over to s-a2: the new conversation's chain holds s-a, so its tab stays.
    dropSubagentPreviews(new Set(['s-a2', 's-a']));
    expect(previewDocuments()).toEqual([{ kind: 'file', path: '/v/a.pdf' }, { kind: 'subagents', parent: 's-a', open: 's-sub-1' }]);
    // The shown tab went, so the pane hides; its history holds no s-b step.
    expect(previewTarget()).toBeNull();
    togglePreview();
    expect(previewTarget()).toEqual({ kind: 'subagents', parent: 's-a', open: 's-sub-1' });
    previewBack();
    expect(previewTarget()).toEqual({ kind: 'subagents', parent: 's-a' });
    dropSubagentPreviews(new Set());
    expect(previewDocuments()).toEqual([{ kind: 'file', path: '/v/a.pdf' }]);
    expect(previewTarget()).toBeNull();
    // A conversation change keeps dropping only visuals.
    openPreview({ kind: 'subagents', parent: 's-c' });
    dropConversationPreviews();
    expect(previewDocuments()).toContainEqual({ kind: 'subagents', parent: 's-c' });
  });

  it('is hidden until something is opened', () => {
    expect(previewTarget()).toBeNull();
    expect(canPreviewBack()).toBe(false);
    expect(canPreviewForward()).toBe(false);
  });

  it('reads a string as a web address, an image data URL or a file path', () => {
    expect(targetOf('https://example.com/a')).toEqual({ kind: 'url', url: 'https://example.com/a' });
    expect(targetOf('HTTP://localhost:3000')).toEqual({ kind: 'url', url: 'HTTP://localhost:3000' });
    expect(targetOf('data:image/png;base64,AAAA')).toEqual({ kind: 'file', path: 'data:image/png;base64,AAAA' });
    expect(targetOf('/Users/me/vault/notes/a.md')).toEqual({ kind: 'file', path: '/Users/me/vault/notes/a.md' });
    expect(targetOf('  ')).toBeNull();
    expect(targetOf('mailto:a@b.c')).toBeNull();
    expect(targetOf('relative/path.md')).toBeNull();
  });

  it('opens a file, a web page and a conversation visual', () => {
    openPreview('/v/a.md');
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/a.md' });
    openPreview({ kind: 'url', url: 'https://example.com' });
    expect(previewTarget()).toEqual({ kind: 'url', url: 'https://example.com' });
    openPreview({ kind: 'visual', key: 'turn-3:0' });
    expect(previewTarget()).toEqual({ kind: 'visual', key: 'turn-3:0' });
  });

  it('keeps cited lines with a file and ignores bad ones', () => {
    openPreview({ kind: 'file', path: '/v/a.ts', line: 12, end: 20 });
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/a.ts', line: 12, end: 20 });
    openPreview({ kind: 'file', path: '/v/b.ts', line: 0, end: 5 });
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/b.ts' });
    openPreview({ kind: 'file', path: '/v/c.ts', line: 9, end: 3 });
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/c.ts', line: 9 });
  });

  it('walks back and forward, and a new open drops the forward items', () => {
    openPreview('/v/a.md');
    openPreview('/v/b.md');
    openPreview('/v/c.md');
    previewBack();
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/b.md' });
    expect(canPreviewForward()).toBe(true);
    previewBack();
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/a.md' });
    expect(canPreviewBack()).toBe(false);
    previewBack();
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/a.md' });
    previewForward();
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/b.md' });
    openPreview('/v/d.md');
    expect(canPreviewForward()).toBe(false);
    previewBack();
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/b.md' });
  });

  it('opening what is already shown re-reads it instead of adding a step', () => {
    openPreview('/v/a.md');
    const before = previewNonce();
    openPreview('/v/a.md');
    expect(previewNonce()).toBe(before + 1);
    expect(canPreviewBack()).toBe(false);
  });

  it('closing hides it and keeps the history; toggling brings the last item back', () => {
    openPreview('/v/a.md');
    openPreview('/v/b.md');
    closePreview();
    expect(previewTarget()).toBeNull();
    togglePreview();
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/b.md' });
    previewBack();
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/a.md' });
    togglePreview();
    expect(previewTarget()).toBeNull();
  });

  it('the card starts visible and the first pane open shows Tools for this chat', () => {
    expect(previewCardShown()).toBe(true);
    togglePreview('s-current');
    expect(previewTarget()).toMatchObject({ kind: 'newtab', chat: 's-current' });
    expect(previewCardShown()).toBe(false);
    togglePreview();
    expect(previewTarget()).toBeNull();
    expect(previewCardShown()).toBe(true);
  });

  it('an unreadable string opens nothing and leaves the pane as it was', () => {
    openPreview('/v/a.md');
    openPreview('not a path');
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/a.md' });
  });

  it('another conversation drops the old one’s visuals and hides the pane if one was showing', () => {
    openPreview('/v/a.md');
    openPreview({ kind: 'visual', key: 'turn-1:0' });
    dropConversationPreviews();
    expect(previewTarget()).toBeNull();
    togglePreview();
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/a.md' });
    openPreview({ kind: 'visual', key: 'turn-2:0' });
    previewBack();
    dropConversationPreviews();
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/a.md' });
    expect(canPreviewForward()).toBe(false);
  });

  it('keeps at most the newest items', () => {
    for (let i = 0; i < PREVIEW_HISTORY_CAP + 5; i++) openPreview(`/v/${i}.md`);
    let steps = 0;
    while (canPreviewBack()) { previewBack(); steps++; }
    expect(steps).toBe(PREVIEW_HISTORY_CAP - 1);
    expect(previewTarget()).toEqual({ kind: 'file', path: '/v/5.md' });
  });

  // Run 15 P1 review: the web mode loads an address on each open of it (a
  // second click on the same link brings that page back even after the tab
  // went elsewhere), on back and forward, but not when ⌘\ reopens the pane,
  // which shows the tab as it was left.
  it('asks the web mode to load an address once per open, back or forward', () => {
    expect(takePreviewWebLoad()).toBeNull();
    openPreview('https://a.example/');
    expect(takePreviewWebLoad()).toBe('https://a.example/');
    expect(takePreviewWebLoad()).toBeNull();
    openPreview('https://a.example/');
    expect(takePreviewWebLoad()).toBe('https://a.example/');
    openPreview('/v/a.md');
    expect(takePreviewWebLoad()).toBeNull();
    previewBack();
    expect(takePreviewWebLoad()).toBe('https://a.example/');
    previewForward();
    expect(takePreviewWebLoad()).toBeNull();
  });

  it('⌘\\ reopening a web page loads nothing again', () => {
    openPreview('https://a.example/');
    expect(takePreviewWebLoad()).toBe('https://a.example/');
    togglePreview();
    togglePreview();
    expect(previewTarget()).toEqual({ kind: 'url', url: 'https://a.example/' });
    expect(takePreviewWebLoad()).toBeNull();
  });

  it('the start page (a saved page review) loads no address', () => {
    openPreview({ kind: 'url', url: '' });
    expect(previewTarget()).toEqual({ kind: 'url', url: '' });
    expect(takePreviewWebLoad()).toBeNull();
  });

  it('a file opened before the web mode took its address cancels that load', () => {
    openPreview('https://a.example/');
    openPreview('/v/a.md');
    expect(takePreviewWebLoad()).toBeNull();
  });

  // Run 15 P1 review: the pane opens beside any page, but only Conversation
  // history (`chat`, Library → Conversations since S4) and an open
  // workspace's page have a composer.
  it('a draft made in the pane goes to a page with a composer', () => {
    expect(composerPage('chat', false)).toBeNull();
    expect(composerPage('chat', true)).toBeNull();
    expect(composerPage('today', true)).toBeNull();
    expect(composerPage('vault', true)).toBe('today');
    expect(composerPage('tasks', false)).toBe('chat');
    expect(composerPage('today', false)).toBe('chat');
  });
});

// Overnight 11-12 C4 and C5: the side panel's card and browser-style New tabs.
describe('side panel card and New tab', () => {
  beforeEach(() => resetPreviewForTests());
  const tab = () => previewTarget();
  const keyOf = () => previewDocumentKey(tab() as Parameters<typeof previewDocumentKey>[0]);

  it('the card returns after any pane closes, including the last document', () => {
    expect(previewCardShown()).toBe(true);
    openPreview({ kind: 'subagents', parent: 's-a' });
    expect(previewCardShown()).toBe(false);
    expect(tab()).toEqual({ kind: 'subagents', parent: 's-a' });
    closePreview();
    expect(tab()).toBeNull();
    expect(previewCardShown()).toBe(true);
    togglePreview();
    expect(previewCardShown()).toBe(false);
    closePreviewDocument(keyOf());
    expect(previewCardShown()).toBe(true);
  });

  it('+ opens a New tab on the chat, selected, after the other tabs', () => {
    openPreview('/v/a.md');
    openNewTab('s-a');
    expect(tab()).toMatchObject({ kind: 'newtab', chat: 's-a' });
    expect(previewDocuments().map(t => t.kind)).toEqual(['file', 'newtab']);
    openNewTab('s-a');
    expect(previewDocuments().map(t => t.kind)).toEqual(['file', 'newtab', 'newtab']);
  });

  it('closing a New tab selects its neighbour; the last one hides the pane', () => {
    openPreview('/v/a.md');
    openNewTab('s-a');
    closePreviewDocument(keyOf());
    expect(tab()).toEqual({ kind: 'file', path: '/v/a.md' });
    closePreviewDocument(keyOf());
    openNewTab(null);
    closePreviewDocument(keyOf());
    expect(tab()).toBeNull();
    expect(previewDocuments()).toEqual([]);
  });

  it('a picked tool takes the New tab’s place, and Back walks what came before', () => {
    openPreview('/v/a.md');
    openNewTab('s-a');
    openPreview('/v/b.md');
    previewBack();
    expect(tab()).toMatchObject({ kind: 'newtab' });
    replacePreviewDocument(keyOf(), { kind: 'subagents', parent: 's-a' });
    expect(tab()).toEqual({ kind: 'subagents', parent: 's-a' });
    // In the New tab's place in the strip, not at its end.
    expect(previewDocuments().map(t => t.kind)).toEqual(['file', 'subagents', 'file']);
    previewBack();
    expect(tab()).toEqual({ kind: 'file', path: '/v/a.md' });
    previewForward();
    expect(tab()).toEqual({ kind: 'subagents', parent: 's-a' });
  });

  it('a tool already open in a tab is selected there and the New tab goes', () => {
    openPreview({ kind: 'subagents', parent: 's-a' });
    openNewTab('s-a');
    replacePreviewDocument(keyOf(), { kind: 'subagents', parent: 's-a' });
    expect(tab()).toEqual({ kind: 'subagents', parent: 's-a' });
    expect(previewDocuments()).toEqual([{ kind: 'subagents', parent: 's-a' }]);
  });

  it('a tool picked for a New tab closed meanwhile opens nothing (review)', () => {
    openPreview('/v/a.md');
    openNewTab('s-a');
    const key = keyOf();
    expect(newTabStillFor(key, 's-a')).toBe(true);
    expect(newTabStillFor(key, 's-b')).toBe(false);
    closePreviewDocument(key);
    expect(newTabStillFor(key, 's-a')).toBe(false);
    replacePreviewDocument(key, { kind: 'file', path: '/v/b.md' });
    expect(tab()).toEqual({ kind: 'file', path: '/v/a.md' });
    expect(previewDocuments()).toEqual([{ kind: 'file', path: '/v/a.md' }]);
  });

  it('a New tab tool lookup applies only if nothing newer happened (review, lane S on #1306)', () => {
    openPreview('/v/a.md');
    openNewTab('s-a');
    const key = keyOf();
    // Normal path: nothing between the stamp and the answer, so the first pinned note opens.
    let stamp = previewIntentNow();
    expect(newTabLookupStillValid(key, 's-a', stamp)).toBe(true);
    replacePreviewDocument(key, { kind: 'file', path: '/v/pinned.md' });
    expect(tab()).toEqual({ kind: 'file', path: '/v/pinned.md' });
    // Another tab selected during the lookup.
    openNewTab('s-a');
    const key2 = keyOf();
    stamp = previewIntentNow();
    openPreview('/v/a.md');
    expect(newTabLookupStillValid(key2, 's-a', stamp)).toBe(false);
    // The pane closed during the lookup.
    openPreview({ kind: 'newtab', id: 99, chat: 's-a' });
    const key3 = keyOf();
    stamp = previewIntentNow();
    closePreview();
    expect(newTabLookupStillValid(key3, 's-a', stamp)).toBe(false);
  });

  it('a New tab on an unsent chat has a draft owner, and the next chat drops it (review)', () => {
    openNewTab(null);
    expect(tab()).toMatchObject({ kind: 'newtab', chat: DRAFT_TAB_OWNER });
    expect(subagentTabParents()).toEqual([DRAFT_TAB_OWNER]);
    // The chat's first send (or any chat coming on screen) keeps only its own chain.
    dropSubagentPreviews(new Set(['s-new']));
    expect(previewDocuments()).toEqual([]);
    expect(tab()).toBeNull();
  });

  it('another chat drops the New tabs opened on the last one', () => {
    openNewTab('s-a');
    openPreview('/v/a.md');
    expect(subagentTabParents()).toEqual(['s-a']);
    dropSubagentPreviews(new Set(['s-b']));
    expect(previewDocuments()).toEqual([{ kind: 'file', path: '/v/a.md' }]);
    expect(tab()).toEqual({ kind: 'file', path: '/v/a.md' });
  });
});

describe('Reopen closed view (t3code parity 4.6)', () => {
  beforeEach(() => resetPreviewForTests());
  const file = (name: string) => ({ kind: 'file' as const, path: `/w/${name}` });
  it('a tab closed by hand comes back, newest first, and is selected', () => {
    openPreview(file('a.md')); openPreview(file('b.md'));
    expect(canReopenClosedView()).toBe(false);
    closePreviewDocument(previewDocumentKey(file('b.md')));
    closePreviewDocument(previewDocumentKey(file('a.md')));
    expect(canReopenClosedView()).toBe(true);
    expect(closedViewLabel()).toBe('a.md');
    expect(reopenClosedView()).toBe(true);
    expect(previewTarget()).toEqual(file('a.md'));
    expect(closedViewLabel()).toBe('b.md');
    expect(reopenClosedView()).toBe(true);
    expect(previewTarget()).toEqual(file('b.md'));
    expect(reopenClosedView()).toBe(false);
    expect(canReopenClosedView()).toBe(false);
  });
  it('a New tab is never kept, and hiding the pane is not a close', () => {
    openNewTab('chat-1');
    const tab = previewDocuments()[0];
    closePreviewDocument(previewDocumentKey(tab));
    expect(canReopenClosedView()).toBe(false);
    openPreview(file('a.md'));
    closePreview();
    expect(canReopenClosedView()).toBe(false);
  });
  it('a Subagents tab is not kept: it belongs to one chat', () => {
    openPreview({ kind: 'subagents', parent: 'chat-1' });
    closePreviewDocument(previewDocumentKey({ kind: 'subagents', parent: 'chat-1' }));
    expect(canReopenClosedView()).toBe(false);
  });
  it('keeps only the newest few, without duplicating a tab closed twice', () => {
    for (let i = 0; i < CLOSED_VIEW_CAP + 3; i++) { openPreview(file(`f${i}.md`)); closePreviewDocument(previewDocumentKey(file(`f${i}.md`))); }
    let n = 0; while (reopenClosedView() && n < 40) n++;
    expect(n).toBe(CLOSED_VIEW_CAP);
  });
});
