import { describe, expect, it } from 'vitest';
import { ROUTE_BY_ID } from './routeLedger';
import topBar from './components/TopBar.tsx?raw';
import rail from './components/Rail.tsx?raw';
import bell from './components/NotificationBell.tsx?raw';
import work from './pages/Work.tsx?raw';
import workbenchCss from './styles/workbench.css?raw';
import preview from './components/ChatPreview.tsx?raw';
import dock from './components/ActivityDock.tsx?raw';
import running from './components/MissionControl.tsx?raw';
import foundations from './styles/foundations.css?raw';

// The top bar and workspace buttons (asks 10-11 A3): global controls in one
// group, the workspace's own state with its name, the workspace panel button
// at the far right (overnight 11-04 U4; run 15 P1's Chat · Workspace switch,
// before that the side-panel toggle), per-file actions with the file.
// Nothing removed: moved.
describe('the top bar layout (asks 10-11 A3)', () => {
  it('keeps global controls in one group, including the notification bell', () => {
    const group = topBar.slice(topBar.indexOf('class="top-bar-global"'), topBar.indexOf('</div>', topBar.indexOf('class="top-bar-global"')));
    const order = ['<NotificationBell />', '<MainAgentOrb />'].map(n => group.indexOf(n));
    // The bell owns Needs you; the old standalone waiting button stays absent.
    expect(topBar).not.toContain('WaitingOnYou');
    expect(order.every(i => i >= 0)).toBe(true);
    // Ask ⌄ beside the orb was removed on 2026-10-21: it only repeated places
    // with their own visible routes (Work's Ask field, the orb, New).
    expect(topBar).not.toContain('AskButton');
    expect([...order].sort((a, b) => a - b)).toEqual(order);
    // The side-panel slot is the last thing on the right.
    expect(topBar.indexOf('class="top-bar-pane-slot"')).toBeGreaterThan(topBar.indexOf('<MainAgentOrb />'));
    // The workspace's own state sits on the left, after the breadcrumb.
    expect(topBar.indexOf('ref={setTopBarSlot}')).toBeGreaterThan(topBar.indexOf('class="top-bar-crumbs"'));
    expect(topBar.indexOf('ref={setTopBarSlot}')).toBeLessThan(topBar.indexOf('class="top-bar-right"'));
  });

  it('creation and search retain visible homes after leaving the top bar', () => {
    for (const id of ['create.conversation', 'create.workspace', 'tasks.add']) {
      const route = ROUTE_BY_ID[id];
      expect(route.path, id).not.toContain('Top bar');
      expect(route.walk.length, id).toBeGreaterThan(0);
    }
    expect(rail).toContain('aria-label="New project"');
    // L1 (11-12): the sidebar's New chat row went and Search chats came
    // back to the top bar as one icon beside the sidebar's toggle; it opens
    // the sidebar's search. Quick chats (2026-11-14): New chat's home is the
    // sidebar's Quick chats + again.
    expect(rail).not.toContain('aria-label="New chat"');
    expect(ROUTE_BY_ID['create.quick-chat'].path).toContain('Sidebar → Quick chats');
    expect(ROUTE_BY_ID['find.search'].path).toContain('Top bar → Search chats');
    expect(topBar).toContain('aria-label="Search chats"');
    expect(topBar).toContain('requestSearchChats(e.currentTarget)');
    expect(rail).toContain('createEffect(on(searchChatsRequest,');
    expect(topBar).not.toContain('<NewMenu');
    expect(topBar).not.toContain('openCommandPalette');
  });

  it('the sidebar is the return route, and Running keeps background operation details', () => {
    expect(ROUTE_BY_ID['workspace.back-to-conversation'].path).toContain('Sidebar');
    expect(topBar).not.toContain('focusWorkspaceConversation');
    expect(topBar).not.toContain('ActivityDock');
    expect(running).toContain('<ActivityDock />');
    expect(dock).toContain('aria-label="Background tasks"');
    expect(dock).toContain('clearCompletedTasks');
    expect(dock).toContain('{t().detail}');
    expect(dock).toContain('{t().result}');
  });

  it('Project files sits at the far right, only while the Work page shows (COO rebuild L4)', () => {
    // It took the place of the workspace panel button (overnight 11-04 U4),
    // at the edge where the side-panel toggle was.
    // The persistent card needs no second top-bar list toggle.
    expect(work).toContain('<Show when={shown()&&detail()&&topBarPaneSlot()}>{slot=><Portal mount={slot()}>{filesButton()}</Portal>}</Show>');
    expect(work).not.toContain('work-card-toggle');
    expect(work).toContain('aria-label="Project files" aria-pressed={filesShown()}');
    expect(work).toContain('const filesShown = previewShown;');
    expect(work).toContain('togglePreview(chatAgentSessionKey())');
    // The workspace's own top-bar controls leave with the page as well.
    expect(work).toContain('<Show when={shown()&&detail()&&topBarSlot()}>{slot=><Portal mount={slot()}><div class="work-actions work-topbar-actions">');
    // The workspace panel and its sections are gone: no section row, no
    // covering panel, nothing but the conversation on the desk.
    for (const gone of ['WorkspaceSectionRow', 'work-workspace', 'data-panel=', 'Workspace panel', 'SidePane', 'work-center-switch']) expect(work).not.toContain(gone);
    expect(workbenchCss).not.toContain('data-panel=');
  });

  it('a file: Preview / Source is a real switch; rare actions sit in its own menu', () => {
    expect(preview).toContain('<div class="ui-segmented chat-preview-toggle" role="group" aria-label="Show the file as">');
    expect(preview).toContain("aria-pressed={view() === 'preview' && !notesOn()}");
    expect(preview).not.toContain('role="tablist"');
    expect(preview).toContain('<ActionMenu label="File actions"');
    for (const item of ["label: 'Open in default app'", "label: 'Reveal in Finder'", "label: 'Copy path'"]) expect(preview).toContain(item);
    expect(preview).toContain('<Icon name="wand" size={13} />');
  });

  it('the bell carries waiting coverage without a sidebar refresh owner', () => {
    expect(bell).toContain("reviewRowLabel(badge(), 'Needs you')");
    expect(rail).not.toContain('refreshWaitingOnYou');
    expect(rail).toContain('title={key() ? `${name()} (${key()})` : name()}');
  });

  // 10-11 fix: the top bar vanished in the app. The ambient layer overflows
  // .canvas by design, and an `overflow: hidden` box is still a scroll
  // container, so a focus or scrollIntoView scrolled the column (top bar
  // included) off the window. The measured check is
  // tests/smoke/top-bar-visible.spec.ts; this keeps the gate honest.
  it('the page column clips instead of hiding, so it can never be scrolled', () => {
    const canvas = foundations.match(/^\.canvas \{.*$/m)?.[0] ?? '';
    expect(canvas).toMatch(/overflow: clip;/);
    expect(canvas.lastIndexOf('overflow: clip')).toBeGreaterThan(canvas.lastIndexOf('overflow: hidden'));
  });
});
