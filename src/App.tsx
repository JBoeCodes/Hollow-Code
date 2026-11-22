import { ConfirmLoading, ConfirmUnavailable } from './components/ConfirmFallback';
import { modalOpen } from './modalGuard';
import { createHistoryWheel } from './historySwipe';
import { swipeNav } from './store/prefs';
import { windowAtMost } from './windowLayout';
import { listenAppMenu, runMenuAction } from './appMenu';
import { api } from './api';
import { ErrorBoundary, Show, Suspense, lazy, onCleanup, onMount } from 'solid-js';
import './styles/shell.css';
import './styles/motion.css';
import './store/studio';
// Hollow app tools: keep the rail and lists current with agents' changes.
import './appBridge';
import { Rail } from './components/Rail';
import { TopBar } from './components/TopBar';
import { Work } from './pages/Work';
import { RecoverableView } from './components/RecoverableView';
import { previewShown } from './store/preview';
import { TaskModal } from './modals/TaskModal';
import { addOpen } from './modals/addState';
import { preloadTaskEditor } from './modals/TaskModal';
import { OnceOpened, preloadWhenIdle } from './components/OnceOpened';
import { conflictCopiesOpen } from './store/conflictCopies';
import { syncClashesOpen } from './store/syncClashes';
import { attachCanvasTarget } from './store/canvasAttach';
import { canvasExportTarget } from './store/canvasExport';
import { canvasLookTarget } from './store/canvasLookTarget';
import { iconEditor } from './store/customIcons';
import { prepPackOpen } from './store/prepPackOpen';
import { ContextMenu } from './components/ContextMenu';
import { SnackbarStack } from './components/SnackbarStack';
import { WindowFileDrop } from './components/WindowFileDrop';
import { NotificationToastStack } from './components/NotificationToastStack';
import {
  activePage, setSidebarCollapsed, sidebarCollapsed,
  closeContextMenu, navBack, navForward,
  warmUpAllInBackground,
  toggleShortcutsOverlay, setWindowAway, commandPaletteOpen, shortcutsOverlayOpen,
} from './store';
import { confirmDialog } from './store/runtime';
import { projectDestinationTarget, contextMenu } from './store/overlays';
const ProjectDestination = lazy(() => import('./components/ProjectDestination'));
import { runProjectScript, scriptForKeystroke } from './store/projectScripts';
import { acceptShortcutsView, activeChord, hasChordModifier, recordingShortcut, windowCommandFor, type ShortcutsView } from './shortcuts';
import { listen as listenTauri } from './nativeEvents';
import { listen } from './domLifecycle';
import { linkToTakeOver } from './linkGuard';
import { cancelLeaderOnMove, leaderKeydown } from './store/leader';
import { listKeydown } from './store/listKeys';
import { WhichKey } from './components/WhichKey';
import { followLink } from './store/browser';
import './store/fonts';
import './store/viewPrefs';
import { startChatEfforts } from './store/chatRuntime';

// Work renders immediately. Editors, history, Tasks and Settings load on demand.
const ExperimentalVisuals = lazy(() => import('./components/ExperimentalVisuals'));
const ThreadNotificationCoordinator = lazy(() => import('./components/ThreadNotificationCoordinator'));
const LOADERS: Record<string, () => Promise<{ default: any }>> = {
  review: () => import('./pages/Review').then(m => ({ default: m.Review })),
  tasks: () => import('./pages/Tasks').then(m => ({ default: m.Tasks })),
  vault: () => import('./pages/Library').then(m => ({ default: m.Library })),
  workflows: () => import('./pages/Workflows').then(m => ({ default: m.Workflows })),
  settings: () => import('./pages/Settings').then(m => ({ default: m.Settings })),
};

const track = (id: string) => LOADERS[id];

const Review = lazy(track('review'));
const Tasks = lazy(track('tasks'));
const Vault = lazy(track('vault'));
const Workflows = lazy(track('workflows'));
const Settings = lazy(track('settings'));
// The result of an install from Hollow (Build PR B5); lazy, so its code and
// CSS stay out of the main bundle.
// Running is a view of Work (components/YourWork.tsx). Today, Weekly review,
// the workspace map, Runbooks, Tonight and Last night were cut (overnight
// 11-08 S3 and S4). The canvas gallery is Library → Results → Canvases.
// Sheets and overlays that aren't on screen at launch (next-level P21): their
// open state lives in small modules, the views load on first open, and every
// chunk is fetched once the shell is idle so the first ⌘K is instant.
const PALETTE = () => import('./components/CommandPalette');
const ADD = () => import('./modals/AddModal');
const SHORTCUTS = () => import('./components/ShortcutsOverlay');
// Overnight P3: the confirm dialog mounts at the first confirm (preloaded
// when idle), out of the App chunk.
const CONFIRM = () => import('./components/ConfirmDialog');
const ConfirmDialog = lazy(() => CONFIRM().then(m => ({ default: m.ConfirmDialog })));
// Conflict copies (overnight A10): its own chunk, loaded on first open only.
const COPIES = () => import('./components/ConflictCopiesDialog');
// Context sync clash cards (CX6c): loaded on first open only.
const SyncClashesDialog = lazy(() => import('./components/SyncClashesDialog').then(m => ({ default: m.SyncClashesDialog })));
const ExportCanvasDialog = lazy(() => import('./components/ExportCanvas'));
const CanvasLookDialog = lazy(() => import('./components/CanvasLookDialog'));
const CustomIconEditor = lazy(() => import('./components/CustomIconEditor'));
// Prep packs (overnight 10-11 A4): loaded on first open only.
const PREP = () => import('./components/PrepPackDialog');
const CommandPalette = lazy(() => PALETTE().then(m => ({ default: m.CommandPalette })));
const AddModal = lazy(() => ADD().then(m => ({ default: m.AddModal })));
const ShortcutsOverlay = lazy(() => SHORTCUTS().then(m => ({ default: m.ShortcutsOverlay })));
const ConflictCopiesDialog = lazy(() => COPIES().then(m => ({ default: m.ConflictCopiesDialog })));
// A saved canvas's Attach to issue… (A19): loaded on first open only.
const AttachCanvasDialog = lazy(() => import('./components/AttachCanvasDialog').then(m => ({ default: m.AttachCanvasDialog })));
// The Preview pane (run 15 P1): the one right-hand pane, loaded when the
// person first opens a file or link, rendered only while it shows one.
const PreviewPane = lazy(() => import('./components/PreviewPane').then(m => ({ default: m.PreviewPane })));
// The side panel's card (overnight 11-12 C4): ⌘\ with no tab chosen.
const PrepPackDialog = lazy(() => PREP().then(m => ({ default: m.PrepPackDialog })));
const SelfInstallNotice = lazy(() => import('./components/SelfInstallNotice').then(m => ({ default: m.SelfInstallNotice })));

// Modal-grade surfaces: anything that owns the keyboard while it's open.
// No global shortcut may fire behind one — navigating out from under a form
// you're filling in is the shape of a lost edit. The keymap card is
// deliberately absent: it holds nothing, and `?` has to stay a toggle.
// (MODAL_SEL / modalOpen live in modalGuard.ts, shared with the native menu.)

export function App() {
  // App shortcuts: the table is `COMMANDS` + `shortcutFor` in
  // src/shortcuts.ts (page digits from navigation.ts). These are the
  // defaults; Settings, Keyboard shortcuts changes them or turns them all
  // off, saved by the backend (shortcuts.rs), which also sets the menu bar.
  //   Cmd+1-4, Cmd+,   pages in rail order; Settings
  //   Cmd+N / Cmd+⇧N   new conversation (frontier F4) / new note
  //   Cmd+K, Cmd+⇧P    command palette
  //   Cmd+B / Cmd+\    sidebar / side panel (frontier F6)
  //   Cmd+[ / Cmd+]    nav history back / forward
  //   Cmd+⇧[ / Cmd+⇧]  previous / next conversation (frontier F6)
  //   Cmd+⇧C           back to the workspace conversation
  //   ?                keymap card (bare key — typing guard below)
  //   Esc              stops a running turn — ChatBody owns it (escStop.ts)
  // None of them fire while a modal-grade surface is open (MODAL_SEL).
  onMount(() => onCleanup(listenAppMenu()));
  // Stop Running Turn, Back and Forward grey out in the menu bar (P1-49).
  onMount(() => {
    let off: (() => void) | undefined; let disposed = false;
    void import('./menuState').then(m => { if (!disposed) off = m.syncMenuState(); });
    onCleanup(() => { disposed = true; off?.(); });
  });
  // The shared actions' dispatcher (keys, menu, `g`; plan completion L4).
  onMount(() => preloadWhenIdle([PALETTE, ADD, SHORTCUTS, CONFIRM, () => import('./components/ActivityDock'), preloadTaskEditor, () => import('./actionRun')]));
  // Each chat's own reasoning effort lives in Rust: move the old localStorage
  // values over once and read them into the window's cache.
  onMount(() => { void startChatEfforts(); });
  // The saved shortcut settings, and later changes (an agent's, or Settings').
  onMount(() => {
    api.getShortcutSettings().then(acceptShortcutsView).catch(() => { /* defaults */ });
    let off: (() => void) | undefined; let disposed = false;
    listenTauri<ShortcutsView>('shortcuts-changed', e => acceptShortcutsView(e.payload))
      .then(u => { if (disposed) u(); else off = u; }).catch(() => { /* not in tauri */ });
    onCleanup(() => { disposed = true; off?.(); });
  });
  onMount(() => {
    // Match the named root container, including its scaled em. Only entering
    // compact collapses the rail; resizing within it preserves a manual open.
    let compact = false;
    const collapseForCompactWindow = () => {
      const next = windowAtMost('compact');
      if (next && !compact) setSidebarCollapsed(true);
      compact = next;
    };
    const observer = new ResizeObserver(collapseForCompactWindow);
    observer.observe(document.documentElement);
    const preference = new MutationObserver(collapseForCompactWindow);
    preference.observe(document.documentElement, { attributes: true, attributeFilter: ['data-text-scale'] });
    collapseForCompactWindow();
    onCleanup(() => { observer.disconnect(); preference.disconnect(); });
    const handleKeydown = (e: KeyboardEvent) => {
      // An open menu (a sidebar row's ⋯ or right-click) takes this Escape
      // first: its own listener closes it, and the sidebar stays (overnight
      // 11-04, lane S).
      if (e.key === 'Escape' && !e.defaultPrevented && windowAtMost('compact') && !sidebarCollapsed() && !modalOpen() && !contextMenu()) {
        setSidebarCollapsed(true);
        e.preventDefault();
        return;
      }
      // `?` → the keymap card. The only bare-key global shortcut, so it
      // carries its own typing guard (the canonical idiom from the
      // pages' j/k listeners): never fire while a field, composer, or
      // modal has the keystroke. `?` is Shift+/ on US layouts — check
      // e.key, never gate on shiftKey.
      if (e.key === '?' && activeChord('shortcuts') === '?' && !recordingShortcut() && !e.metaKey && !e.ctrlKey && !e.altKey && !e.repeat) {
        const t = e.target as HTMLElement | null;
        const typing = !!t && (
          t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' ||
          t.isContentEditable || !!t.closest('.modal')
        );
        // The target check alone isn't enough: a modal whose focus fell
        // back to <body> (click on dead sheet chrome) would pass it, and
        // the card would stack over the modal — then one Escape would
        // close both. Bail while ANY modal-grade surface is open.
        const blocked = typing || modalOpen();
        if (!blocked) {
          e.preventDefault();
          toggleShortcutsOverlay();
          return;
        }
      }

      // ⌘ alone on a Mac: ⌃ keeps its Cocoa text keys (five asks A4).
      if (!hasChordModifier(e)) return;

      // A modal owns the keyboard while it's open: page jumps, the palette,
      // the sidebar, and nav history all stay put behind it. Bailing here
      // rather than allowlisting keeps Escape (not a chord), Cmd+Enter, and
      // Cmd+R working — this handler never claimed them, and returning
      // without preventDefault leaves them to the modal and the webview.
      if (modalOpen()) return;
      // A surface already took the key (the browser panel's ⌘[ steps its
      // page, P1-47): Hollow doesn't run it too.
      if (e.defaultPrevented) return;

      // One keymap (shortcuts.ts): the palette, the keymap card and the
      // native menu read the same table, and a command runs what its menu
      // item runs (appMenu.ts). A ⌥ chord only for a command with no menu
      // item, and never an in-surface key's (P1-49).
      const command = windowCommandFor(e);
      if (!command) {
        // A project script's ⇧⌘ chord (4.5): only where no Hollow command holds the key.
        const hit = scriptForKeystroke(e);
        if (hit) { e.preventDefault(); void runProjectScript(hit.workspaceId, hit.script); }
        return;
      }
      e.preventDefault();
      // A held dictate key repeats; only its first press counts (the
      // release ends a hold, store/dictation.ts).
      if (command.id === 'dictate' && e.repeat) return;
      runMenuAction(command.id);
    };
    listen(document, 'keydown', handleKeydown);
    // The go-to leader (five asks A4): `g` then a key. Window capture, ahead
    // of every page's own letters, so the key after `g` is never a list's.
    listen(window, 'keydown', e => { leaderKeydown(e); }, true);
    listen(window, 'pointerdown', cancelLeaderOnMove, true);
    listen(window, 'focusin', cancelLeaderOnMove, true);
    listen(window, 'blur', cancelLeaderOnMove);
    // j / k / x / e / r / . in lists marked data-key-list (five asks A4).
    listen(document, 'keydown', e => { listKeydown(e); });

    // Mouse aux buttons — 3 is "back" on most mice, 4 is "forward".
    // Bind to `auxclick` only: it fires once per non-primary press, so we
    // navigate exactly once. (Binding to mousedown + mouseup as well made a
    // single press fire 2–3 times.) Some setups never pass these through at
    // all — the sidebar's back/forward buttons are the fallback in that case.
    const handleAux = (e: MouseEvent) => {
      if (modalOpen()) return;
      if (e.button === 3) { e.preventDefault(); navBack(); }
      else if (e.button === 4) { e.preventDefault(); navForward(); }
    };
    listen(document, 'auxclick', handleAux);
    // Two-finger trackpad swipe, the same steps (historySwipe.ts; Settings, Keyboard shortcuts).
    listen(document, 'wheel', createHistoryWheel({ enabled: swipeNav, blocked: () => modalOpen() || recordingShortcut(), back: navBack, forward: navForward }), { passive: true });

    // Suppress the native (browser/Tauri) right-click menu app-wide so the
    // bespoke ContextMenu is the only one that ever appears — EXCEPT inside
    // text fields, where users expect native copy/paste/spellcheck, and on
    // selected text no component handled (Copy, Look Up; P1-42): the click
    // falls on the highlighted text itself, not just near it.
    const handleContextMenu = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      const inEditable = !!target?.closest?.('input, textarea, [contenteditable="true"]');
      const selection = getSelection();
      const onSelection = !e.defaultPrevented && !!selection?.rangeCount && !selection.isCollapsed
        && [...selection.getRangeAt(0).getClientRects()].some(r => e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom);
      if (!inEditable && !onSelection) e.preventDefault();
    };
    listen(document, 'contextmenu', handleContextMenu);

    // Close any open context menu when the page changes.
    const handleDocumentClick = () => closeContextMenu();
    listen(document, 'click', handleDocumentClick);

    // A web or mail link nothing else claimed (a note, rendered markdown)
    // must not navigate this window away. On `window`, so it runs after
    // every component's handler (ChatMessage's included) and only takes
    // clicks they left alone. main_nav.rs blocks whatever slips past.
    const handleLinkClick = (e: MouseEvent) => {
      const link = linkToTakeOver(e);
      if (!link) return;
      e.preventDefault();
      followLink(link.href, link.kind, e);
    };
    listen(window, 'click', handleLinkClick);

    // The hearth banks — when the window loses focus the slab quietly
    // rests: the ambient breathing animations hold still (paused, not
    // removed, so phase is preserved and nothing jumps on return).
    // Honest motion design: no ambient animation in an unfocused
    // window. Class-toggle only — an earlier draft also dimmed via a
    // filter on .app, but a whole-window filter layer is a real
    // compositing cost in WKWebView.
    //
    // Focus moving into an in-app iframe (the Inbox email reader, HTML
    // artifacts) blurs the window too, and that is active reading, not
    // away. `document.hasFocus()` is the discriminator for it, which
    // `document.activeElement` could not be: activeElement reflects the
    // DOM focus tree, so it stays on the <iframe> after a later Cmd+Tab
    // and the old guard returned early forever — the app's only
    // animation-pause mechanism silently never fired once you had
    // opened an email. hasFocus() instead stays true while the focused
    // area is a descendant browsing context of this document, and goes
    // false on real system focus loss.
    //
    // visibilitychange is only a partial backup: on macOS another app
    // being frontmost does not change visibilityState, so it covers
    // minimize and Space switches but not the common case.
    // One detector, two consumers: the CSS pause switch and the WebGL loop.
    const setAway = (away: boolean) => {
      document.body.classList.toggle('is-away', away);
      setWindowAway(away);
    };
    let awayPoll: ReturnType<typeof setInterval> | null = null;
    const stopPoll = () => {
      if (awayPoll) { clearInterval(awayPoll); awayPoll = null; }
    };
    const bank = () => {
      if (document.hasFocus()) return;
      setAway(true);
      // Coming back to a window whose focus sits inside an iframe may
      // never fire `focus` on the parent, so poll until focus returns.
      // This runs only while banked, which is exactly when the app
      // should be doing nothing anyway.
      if (!awayPoll) {
        awayPoll = setInterval(() => {
          if (document.hasFocus()) { setAway(false); stopPoll(); }
        }, 1000);
      }
    };
    // 0ms for the normal case, 400ms in case WKWebView reports focus stale.
    const handleBlur = () => { setTimeout(bank, 0); setTimeout(bank, 400); };
    const handleFocus = () => { setAway(false); stopPoll(); };
    const handleVisibilityChange = () => {
      if (document.visibilityState !== 'visible') setAway(true);
      else if (document.hasFocus()) { setAway(false); stopPoll(); }
    };
    listen(window, 'blur', handleBlur);
    listen(window, 'focus', handleFocus);
    listen(document, 'visibilitychange', handleVisibilityChange);
    onCleanup(stopPoll);

    // Warm only provider discovery; source collections remain lazy.
    warmUpAllInBackground();
  });

  return (
    <>
      <div class="app hollow-shell">
        <Rail />
        <ErrorBoundary fallback={null}><Suspense fallback={null}><ExperimentalVisuals /></Suspense></ErrorBoundary>
        <ErrorBoundary fallback={null}><Suspense fallback={null}><ThreadNotificationCoordinator /></Suspense></ErrorBoundary>
        <main class="canvas">
          {/* The ambient layer (overnight 10-11 B1): decoration behind every page. */}
          <div class="ambient-layer" aria-hidden="true" />
          <TopBar />
          {/* The page and, beside it under the top bar, the Preview pane (run 15
              P1): the shell's one right-hand slot. Under the top bar, not
              beside the whole column, so the bar never moves when it opens. */}
          <div class="canvas-split">
          <div class="page" classList={{ active: true }}>
            {/* Work is eager and must never sit inside a Suspense boundary
                (pages/Work.tsx: its section reads would swap the page out on
                every pick); it keeps only the error boundary. Once opened it
                stays mounted behind the other pages (overnight 11-04 U4), so
                its conversation keeps its scroll, draft and running turn and
                comes back from its sidebar row; the wrapper hides it, its
                recovery included, and Work hides its own top-bar controls,
                focus, arrivals and clocks while another page shows. */}
            <OnceOpened when={activePage() === 'today'}><div class="page-retained" hidden={activePage() !== 'today'}><RecoverableView name="Work" suspends={false}><Work /></RecoverableView></div></OnceOpened>
            {/* One boundary PER PAGE, inside the Show. A page-local resource must
                never be able to unmount its siblings: the single app-wide boundary
                meant an in-flight resource on the page you left kept its counter
                raised (read() increments with no onCleanup, and only completeLoad
                decrements), so an abandoned fetch could blank the page you arrived
                at. Inside the Show, not around it — Show only creates its children
                when true, so the lazy chunk still suspends within its own page and a
                disposed page takes its counter with it. */}
            <Show when={activePage() === 'review'}><RecoverableView name="Needs you" fallback={<PageLoading />}><Review /></RecoverableView></Show>
            <Show when={activePage() === 'tasks'}><RecoverableView name="Tasks" fallback={<PageLoading />}><Tasks /></RecoverableView></Show>
            <Show when={activePage() === 'vault'}><RecoverableView name="Library" fallback={<PageLoading />}><Vault /></RecoverableView></Show>
            <Show when={activePage() === 'workflows'}><RecoverableView name="Automations" fallback={<PageLoading />}><Workflows /></RecoverableView></Show>
            <Show when={activePage() === 'settings'}><RecoverableView name="Settings" fallback={<PageLoading />}><Settings /></RecoverableView></Show>
          </div>
          {/* Beside every page, outside any Suspense (its file reads must never
              swap a page out), with its own compact recovery. */}
          <Show when={previewShown()}><RecoverableView name="Preview" compact suspends={false}><PreviewPane /></RecoverableView></Show>
          </div>
        </main>
      </div>
      <TaskModal />
      <OnceOpened when={addOpen()}><AddModal /></OnceOpened>
      <ContextMenu />
      {/* Its chunk failing to load must not leave a confirm (and the keys that
          wait on it) hanging: cancel it and say so (P3 review). */}
      <OnceOpened when={confirmDialog() !== null}>
        <ErrorBoundary fallback={() => <ConfirmUnavailable />}>
          <Suspense fallback={<ConfirmLoading />}><ConfirmDialog /></Suspense>
        </ErrorBoundary>
      </OnceOpened>
      <OnceOpened when={commandPaletteOpen()}><CommandPalette /></OnceOpened>
      <Show keyed when={projectDestinationTarget()}><Suspense><ProjectDestination /></Suspense></Show>
      <OnceOpened when={shortcutsOverlayOpen()}><ShortcutsOverlay /></OnceOpened>
      <WhichKey />
      <OnceOpened when={conflictCopiesOpen()}><ConflictCopiesDialog /></OnceOpened>
      <OnceOpened when={syncClashesOpen()}><SyncClashesDialog /></OnceOpened>
      <OnceOpened when={!!attachCanvasTarget()}><AttachCanvasDialog /></OnceOpened>
      <Show when={canvasExportTarget()}><ExportCanvasDialog /></Show>
      <Show when={canvasLookTarget()}><Suspense><CanvasLookDialog /></Suspense></Show>
      <Show keyed when={iconEditor()}><Suspense><CustomIconEditor /></Suspense></Show>
      <OnceOpened when={prepPackOpen()}><PrepPackDialog /></OnceOpened>
      <SnackbarStack />
      {/* A file dropped anywhere on the window attaches to the chat (2026-11-04). */}
      <WindowFileDrop />
      <NotificationToastStack />
      <Suspense fallback={null}><SelfInstallNotice /></Suspense>
    </>
  );
}

/** A lazy page's first load: the shape of a page, shimmering (overnight
 *  10-11 B1), instead of a spinner in an empty column. */
function PageLoading() {
  return <div class="page-loading page-skeleton" role="status" aria-busy="true" aria-label="Loading page">
    <span class="ui-skeleton-bar page-skeleton-title" aria-hidden="true" />
    <span class="ui-skeleton-bar page-skeleton-bar" aria-hidden="true" />
    <span class="ui-skeleton-bar page-skeleton-row" aria-hidden="true" />
    <span class="ui-skeleton-bar page-skeleton-row" aria-hidden="true" />
    <span class="ui-skeleton-bar page-skeleton-row" aria-hidden="true" />
    <span class="ui-skeleton-bar page-skeleton-row" aria-hidden="true" />
    <span class="ui-skeleton-bar page-skeleton-row" aria-hidden="true" />
  </div>;
}
