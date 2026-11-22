import { describe, expect, it } from 'vitest';
// `?raw` reads App.tsx as text without executing it, so this stays a pure
// node-environment test that never pulls in Solid or Tauri. Typed by
// vite/client; there is no @types/node in this project.
import APP_TSX from './App.tsx?raw';
import APP_BRIDGE_TS from './appBridge.ts?raw';
import RAIL_TSX from './components/Rail.tsx?raw';
import WORKSPACE_MENU_TS from './workspaceMenu.ts?raw';
import CONVERSATION_ACTIONS_TS from './conversationActions.ts?raw';
import SETTINGS_TSX from './pages/Settings.tsx?raw';
import WORKSPACES_TS from './store/workspaces.ts?raw';
import ARTIFACT_SESSIONS_TS from './store/artifactSessions.ts?raw';
import ADD_MODAL_TSX from './modals/AddModal.tsx?raw';
import ADD_STATE_TS from './modals/addState.ts?raw';
import MAIN_TSX from './main.tsx?raw';
import LIBRARY_TSX from './pages/Library.tsx?raw';
import ACTION_RUN_TS from './actionRun.ts?raw';
import LIB_RS from '../src-tauri/src/lib.rs?raw';
import TAURI_CONF from '../src-tauri/tauri.conf.json?raw';
import APP_ACTIONS_RS from '../src-tauri/src/app_actions.rs?raw';
import {
  NAV_PAGES, PAGE_KEY, PAGE_ORDER, PAGE_NAV_BY_ID,
  RAIL_WORKSPACE, RAIL_KNOWLEDGE, RAIL_PRIMARY, RAIL_BOTTOM, SETTINGS_PAGE, currentPage,
} from './navigation';
import { GO_TARGETS, NAVIGATE_PAGES } from './keyboardLeader';
import { NAVIGATE_ALIASES, navigateTarget, visiblePath } from './appBridgeCore';

// P1-05: the rail's rows come from NAV_PAGES (one id, one icon), and an
// agent can open Work's views and use the names the person sees.
describe('Work: one id, one icon, views an agent can open (P1-05)', () => {
  it('renders the rail rows from the non-hidden NAV_PAGES, in order, with their icons', () => {
    expect([...RAIL_PRIMARY, ...RAIL_BOTTOM].map(p => [p.id, p.icon])).toEqual(NAV_PAGES.filter(p => !p.hidden && p.id !== 'review').map(p => [p.id, p.icon]));
    expect(RAIL_PRIMARY.map(p => p.icon)).toEqual(['folder', 'checkSquare', 'book', 'zap']);
    expect(RAIL_TSX).toContain('<For each={RAIL_PRIMARY}>');
    expect(RAIL_TSX, 'goToPage resets Work; the rail has no special case').not.toContain('resetWorkView');
  });
  it('takes visible names as aliases and answers with the visible path', () => {
    expect(navigateTarget('your-work')).toBe('home');
    expect(navigateTarget('library')).toBe('library');
    expect(NAVIGATE_ALIASES).toEqual({ 'your-work': 'home' });
    expect(() => navigateTarget('toString')).toThrow('navigate: unknown page toString.');
    // Today, Weekly review and the workspace map were cut (overnight 11-08 S3 and S4).
    expect(visiblePath('mission-control')).toBe('Projects → Running');
    expect(visiblePath('history')).toBe('Library → Conversations');
    expect(visiblePath('home')).toBe('Projects → Home');
    expect(visiblePath('tasks', 'tasks/a.md')).toBe('Tasks → tasks/a.md');
  });
});

/**
 * Structural guards over the shell registry. `navigation.ts` is the single
 * source of truth for rail order, Cmd+digit assignment, palette rows and
 * breadcrumbs, so drift here shows up as a dead keystroke or a route with no
 * branch behind it — both silent failures today.
 *
 * The digit map is asserted as an explicit snapshot, so a renumber is a
 * conscious edit. Since frontier F6 the digits are contiguous in rail order.
 */

describe('shell registry', () => {
  it('assigns every shortcut digit to exactly one page', () => {
    const keys = NAV_PAGES.flatMap(p => (p.shortcutKey ? [p.shortcutKey] : []));
    expect(new Set(keys).size).toBe(keys.length);
  });

  it('pins the current digit map so a renumber is deliberate', () => {
    expect(PAGE_KEY).toEqual({
      today: '1',
      tasks: '2',
      vault: '3',
      workflows: '4',
      review: '5',
      settings: ',',
    });
  });

  it('numbers pages contiguously from 1 and leaves retired pages unbound (frontier F6)', () => {
    const digits = Object.values(PAGE_KEY).filter(k => /^\d$/.test(k!)).sort();
    // Review took ⌘5 (P1-14), so the older digits keep their pages.
    expect(digits).toEqual(['1', '2', '3', '4', '5']);
    // Calendar is no longer a page, so it has no digit (overnight 11-08 S3).
    expect(Object.keys(PAGE_KEY)).not.toContain('calendar');
    expect(NAV_PAGES.some(page => String(page.id) === 'inbox')).toBe(false);
    expect(NAV_PAGES.some(page => String(page.id) === 'approvals')).toBe(false);
  });

  it('has a unique id per page, and PAGE_ORDER mirrors NAV_PAGES', () => {
    const ids = NAV_PAGES.map(p => p.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(PAGE_ORDER).toEqual(ids);
  });

  it('centers navigation on work and shared tools without legacy shortcuts', () => {
    expect(RAIL_WORKSPACE.map(page => page.id)).toEqual(['today', 'tasks', 'vault']);
    expect(RAIL_KNOWLEDGE.map(page => page.id)).toEqual(['workflows']);
    expect(PAGE_KEY.vault).toBe('3');
    expect(PAGE_NAV_BY_ID.chat.hidden).toBe(true);
  });

  it('redirects old links without loading retired pages', () => {
    expect(currentPage('projects')).toBe('today');
    // Stored page ids for the cut screens fall back to Work (overnight 11-08 S3).
    // Conversation history (`chat`) and the old ingest id still open Library.
    for (const p of ['calendar', 'people', 'graph', 'imports'] as const) expect(currentPage(p)).toBe('today');
    expect(currentPage('ingest')).toBe('vault');
    expect(currentPage('chat')).toBe('vault');
  });

  it('indexes every page in PAGE_NAV_BY_ID', () => {
    for (const page of NAV_PAGES) {
      expect(PAGE_NAV_BY_ID[page.id]).toBe(page);
    }
    expect(Object.keys(PAGE_NAV_BY_ID).length).toBe(NAV_PAGES.length);
  });

  it('partitions the rail into workspace + knowledge + pinned Settings', () => {
    const railed = [...RAIL_WORKSPACE, ...RAIL_KNOWLEDGE];
    const visible = NAV_PAGES.filter(p => !p.hidden && p.id !== SETTINGS_PAGE.id && p.id !== 'review');
    expect(railed.map(p => p.id).sort()).toEqual(visible.map(p => p.id).sort());
    expect(SETTINGS_PAGE.section).toBe('system');
  });

  it('gives every registered page a render branch in App.tsx', () => {
    for (const page of NAV_PAGES.filter(p=>!p.hidden)) {
      expect(
        APP_TSX.includes(`'${page.id}'`),
        `NAV_PAGES has "${page.id}" but App.tsx has no branch for it`,
      ).toBe(true);
    }
  });

  it('requires the cosmetic fields every chrome surface reads', () => {
    for (const page of NAV_PAGES) {
      for (const field of ['label', 'icon', 'ext', 'keywords'] as const) {
        expect(page[field], `${page.id}.${field} is empty`).toBeTruthy();
      }
    }
  });
});

/**
 * Hidden pages (merged plan slice 6, decision 7: "the hidden old Chat and
 * Imports pages go once confirmed unused"). Audited 2026-10-03: neither is
 * unused, so both routes stay. A hidden page either redirects through
 * `currentPage` (and then App.tsx must not render it) or still renders, and
 * then something a person or an agent can trigger has to open it. The pins
 * below name those entry points; delete a page only once they are gone.
 */
const opens = (source: string, page: string) =>
  new RegExp(`(?:setActivePage|goToPage)\\(\\s*'${page}'\\s*\\)`).test(source);

describe('hidden pages', () => {
  const hidden = NAV_PAGES.filter(p => p.hidden);
  const rendered = (id: string) => new RegExp(`activePage\\(\\)\\s*===\\s*'${id}'`).test(APP_TSX);

  it('renders exactly the hidden pages that do not redirect', () => {
    const live = hidden.filter(p => currentPage(p.id) === p.id).map(p => p.id);
    // None since S4: Conversation history (`chat`) is a Library tab.
    expect(live).toEqual([]);
    for (const id of live) expect(rendered(id), `${id} has no render branch in App.tsx`).toBe(true);
    for (const p of hidden.filter(p => currentPage(p.id) !== p.id)) {
      expect(rendered(p.id), `${p.id} redirects, so App.tsx must not render it`).toBe(false);
    }
  });

  it('keeps the agent navigate tool on the go-to targets (P0-06)', () => {
    const enumMatch = APP_ACTIONS_RS.match(/"ui\.navigate"[\s\S]*?Kind::OneOf\(&\[([^\]]*)\]/);
    expect(enumMatch, 'ui.navigate page enum not found in app_actions.rs').toBeTruthy();
    const toolPages = [...enumMatch![1].matchAll(/"([^"]+)"/g)].map(m => m[1]);
    // The Rust list is the shared list, its visible-name aliases (P1-05), then the three record targets.
    expect(toolPages).toEqual([...NAVIGATE_PAGES, ...Object.keys(NAVIGATE_ALIASES), 'workspace', 'session', 'browser']);
    // The shared list is every go-to target but the internal ones (From
    // other apps, the one extra page, was removed in run 15 P4).
    expect(NAVIGATE_PAGES).toEqual(GO_TARGETS.map(t => t.id).filter(id => id !== 'jump' && id !== 'quick-chat'));
    // Retired and renamed pages are offered nowhere.
    for (const old of ['calendar', 'projects', 'people', 'graph', 'ingest', 'imports', 'vault', 'workflows', 'today', 'weekly-review', 'agenda', 'commitments', 'inbox', 'map', 'runbooks', 'whats-new']) expect(toolPages).not.toContain(old);
  });

  it('refuses each retired page with the same words in Rust and in the bridge (P0-06)', () => {
    // rustfmt wraps a long tuple onto several lines. Whitespace between the
    // fields is the same row.
    const rows = [...APP_ACTIONS_RS.matchAll(/\(\s*"ui\.navigate",\s*"page",\s*"([^"]+)",\s*"([^"]+)",?\s*\)/g)];
    expect(rows.map(r => r[1]).sort()).toEqual(['agenda', 'calendar', 'chat', 'commitments', 'graph', 'imports', 'inbox', 'ingest', 'map', 'people', 'projects', 'review', 'runbooks', 'today', 'vault', 'weekly-review', 'whats-new', 'workflows']);
    for (const [, old, note] of rows) expect(() => navigateTarget(old)).toThrow(`navigate: ${old} ${note}.`);
  });

  it('still opens Conversation history (chat, Library → Conversations) from the rail, Settings, Library and agents', () => {
    // A sidebar project's or Chats' ⋯ → Open in Conversation history; its
    // older chats load in place (overnight 11-04 S4b).
    expect(WORKSPACE_MENU_TS).toContain("label: 'Open in Conversation history', action: () => act(m => m.openHistory(id))");
    expect(opens(CONVERSATION_ACTIONS_TS, 'chat'), "the sidebar menus' Open in Conversation history").toBe(true);
    expect(RAIL_TSX).not.toContain('older in Conversation history');
    // Settings' Conversation history button was cut with History & recovery's move to Library (Settings PR 4).
    expect(SETTINGS_TSX).not.toContain('Conversation history</button>');
    expect(opens(ARTIFACT_SESSIONS_TS, 'chat'), 'Library: sessions that touched a file').toBe(true);
    // navigate('session') / openSessionByKey for a conversation in no workspace (#318).
    expect(WORKSPACES_TS).toMatch(/function openOnChatPage[\s\S]*?goToPage\(\s*'chat'\s*\)/);
    // navigate('browser') no longer goes there (run 15 P1): the Preview pane
    // shows the page beside whatever page is open.
    expect(APP_BRIDGE_TS).toMatch(/case 'browser': \{[\s\S]*?openPreview\(\{ kind: 'url', url \}\)/);
  });

  it('removed From other apps (run 15 P4): no rail group, Settings section or Add entry opens it', () => {
    expect(RAIL_TSX).not.toContain('RailImports');
    expect(SETTINGS_TSX).not.toContain('ImportsPanel');
    expect(opens(ADD_MODAL_TSX, 'imports'), 'Add → conversation export').toBe(false);
    expect(APP_TSX).not.toMatch(/pages\/Imports/);
  });

  it('Import a source is the Add sheet set to a Library source (S5): no second sheet or window', () => {
    expect(ADD_STATE_TS).toContain("export function openImportSource() { openAdd('', 'source'); }");
    // Library's ⋯, the palette row and the system-wide key all open it.
    expect(LIBRARY_TSX).toContain("{label:'Import a source',action:openImportSource}");
    expect(ACTION_RUN_TS).toContain("'quick-ingest': () => openImportSource(),");
    // ⌘⌥⇧I and the tray's Import a source… bring the main window forward on it.
    expect(LIB_RS).toContain('Some("quick-ingest") => show_main_with(app, "quick-ingest"),');
    expect(LIB_RS).toContain('"ingest" => show_main_with(app, "quick-ingest"),');
    // The Import source sheet and the borderless Import window are gone.
    expect(APP_TSX).not.toMatch(/QuickIngestModal|quickIngestState/);
    expect(MAIN_TSX).not.toContain("'quick-ingest'");
    expect(TAURI_CONF).not.toContain('quick-ingest');
    expect(LIB_RS).not.toMatch(/get_webview_window\("quick-ingest"\)/);
    // Its Domain and Purpose live on in Add.
    for (const field of ['Domain (optional)', 'Purpose (optional)']) expect(ADD_MODAL_TSX).toContain(`<span>${field}</span>`);
  });

  it('Quick capture is the Add sheet set to a task (S5): no quick-capture window', () => {
    expect(ADD_STATE_TS).toContain("export function openQuickCapture() { openAdd('', 'task'); }");
    expect(ACTION_RUN_TS).toContain("'quick-capture': () => openQuickCapture(),");
    expect(LIB_RS).toContain('Some("quick-capture") => show_main_with(app, "quick-capture"),');
    expect(LIB_RS).toContain('"quick" => show_main_with(app, "quick-capture"),');
    expect(MAIN_TSX).not.toContain("'quick-capture'");
    expect(TAURI_CONF).not.toContain('quick-capture');
    expect(LIB_RS).not.toMatch(/get_webview_window\("quick-capture"\)|toggle_quick_capture_window/);
    // The window's parsed chips live on in Add's task preview.
    for (const field of ['durationMin', 'recurrence', 'priority', 'project', 'labels', 'dueTime']) expect(ADD_MODAL_TSX).toContain(`p().value.${field}`);
  });
});
