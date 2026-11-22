import { For, Match, Show, Switch, createEffect, createMemo, createSignal, lazy, on, onCleanup, onMount } from 'solid-js';
import { Icon } from '../icons';
import { ResizeHandle } from './ResizeHandle';
import { PreviewPlayerControls } from './PreviewPlayerControls';
import { ChatPreview } from './ChatPreview';
import { PreviewVisual } from './PreviewVisual';
import { observePaneWidth } from '../paneScale';
import { canPreviewBack, canPreviewForward, closePreview, closePreviewDocument, openNewTab, openPreview, previewBack, previewDocuments, previewDocumentKey, previewForward, previewTarget, previewWebLoad, selectPreviewWeb, takePreviewWebLoad } from '../store/preview';
import { giveFocusBack } from '../focusReturn';
import { chatVisuals, persistSidePaneWidth, setSidePaneWidth, sidePaneWidth, SIDE_PANE_CHAT_MIN, SIDE_PANE_MIN } from '../store/chat';
import { activeBrowserTab, browserTabs, closeBrowserTab, newBrowserTab, openBrowserUrl, setActiveBrowserTab } from '../store/browser';
import { tabLabel } from '../browserUrl';
import { chatArtifactKind } from '../chatArtifactKind';
import { fileTabTitle, type PreviewFileFacts } from '../previewDetails';
import { chatAgentSessionKey } from '../store/chatRuntime';
import { projectScripts } from '../store/projectScripts';
import { showConfirm } from '../store/overlays';
import '../styles/preview-pane.css';
// The web mode carries the native page area and its tab strip; it loads
// with the first web page opened here.
const BrowserPane = lazy(() => import('./BrowserPane').then(m => ({ default: m.BrowserPane })));
// A chat's Subagents tab (COO rebuild L3) loads when one is first opened.
const SubagentsPanel = lazy(() => import('./SubagentsPanel').then(m => ({ default: m.SubagentsPanel })));
// A build workspace's Changes (COO rebuild L4: it was a workspace panel
// section) loads when first opened.
// A chat's terminals (parity 7.2) load when a Terminal tab first opens.
const TerminalPanel = lazy(() => import('./TerminalPanel').then(m => ({ default: m.TerminalPanel })));
const PrPanel = lazy(() => import('./PrPanel').then(m => ({ default: m.PrPanel })));
const DiffPanel = lazy(() => import('./DiffPanel').then(m => ({ default: m.DiffPanel })));
const WorkspacePreview = lazy(() => import('./WorkspacePreview').then(m => ({ default: m.WorkspacePreview })));
// A project script's output (4.5) loads when a script first runs.
const ScriptOutput = lazy(() => import('./ScriptOutput').then(m => ({ default: m.ScriptOutput })));
// A New tab's Tools (overnight 11-12 C5) load with the first New tab.
const PanelNewTab = lazy(() => import('./PanelNewTab').then(m => ({ default: m.PanelNewTab })));

/**
 * The Preview pane (run 15 P1): the app's one right-hand pane, mounted by
 * the shell beside every page and rendered only while something is
 * previewed (store/preview.ts). It shows a file by type (ChatPreview), a
 * web page (BrowserPane, its web mode) or a conversation's canvas
 * (PreviewVisual). Back and forward walk what was opened; × or Esc hide it.
 * Its width is the old side panel's (`sidePaneWidth`), so a dragged width
 * carries over.
 *
 * One header row (UI clutter pass, 2026-11-06): back and forward, the tabs,
 * New tab (a browser-style +, overnight 11-12 C5: its page lists the Tools,
 * Files, Browser, Subagents and Changes), the shown file's own controls (ChatPreview fills the
 * `preview-pane-tools` slot) and close. A narrow pane wraps the controls to a
 * second row. The tab names the item; its kind and full path (a file's size
 * and time too) are the tab's tooltip, so no kind word or path line repeats
 * it. A web page's address bar is the second row.
 */
export function PreviewPane() {
  let pane: HTMLElement | undefined;
  let opener: Element | null = null;
  const [width, setWidth] = createSignal(0);
  // The file's controls go in the header row; the file's size and time come
  // back for its tab's tooltip.
  const [tools, setTools] = createSignal<HTMLElement>();
  const [facts, setFacts] = createSignal<PreviewFileFacts | null>(null);
  // An open, back or forward to a web page loads it in the web mode; an open
  // tab of that address is reused (store/browser.ts), so a second click on a
  // link brings its page back. ⌘\ reopening loads nothing: the tab shows as
  // it was left. An empty address is the start page.
  createEffect(on(previewWebLoad, () => {
    const url = takePreviewWebLoad();
    if (url) void openBrowserUrl(url, { newTab: true });
  }));
  // Opened by a click: focus moves into the pane (unless the person is
  // typing), and closing hands it back to where it was.
  onMount(() => {
    opener = document.activeElement;
    const typing = opener instanceof HTMLElement && (opener.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(opener.tagName));
    if (!typing) pane?.focus({ preventScroll: true });
  });
  // After the pane's DOM is gone (in a compact window the page it covered is
  // hidden until then), and only if focus fell to the page body or stayed in
  // the pane: never over a control that took it meanwhile.
  onCleanup(() => {
    const back = opener instanceof HTMLElement ? opener : null;
    giveFocusBack(back, '#preview-pane');
  });
  // The page beside it keeps SIDE_PANE_CHAT_MIN of the split's width.
  const maxWidth = () => {
    const split = pane?.parentElement?.clientWidth ?? window.innerWidth;
    return Math.max(SIDE_PANE_MIN, split - SIDE_PANE_CHAT_MIN);
  };
  const kind = () => previewTarget()?.kind;
  let tabStrip: HTMLDivElement | undefined;
  // A canvas tab is named by its canvas; what kind of item a tab holds is
  // the first line of its tooltip.
  const visualOf = (key: string) => chatVisuals().find(v => v.key === key);
  const tabs = createMemo(() => [
    ...previewDocuments().map(t => {
      if (t.kind === 'file') {
        const label = t.path.startsWith('data:') ? 'Image' : t.path.slice(t.path.lastIndexOf('/') + 1);
        return { id: `doc:${previewDocumentKey(t)}`, label, title: fileTabTitle(t.path, chatArtifactKind(label), facts()), icon: 'fileText' as const };
      }
      if (t.kind === 'subagents') return { id: `doc:${previewDocumentKey(t)}`, label: 'Subagents', title: 'Subagents\nWhat this chat started', icon: 'bot' as const };
      if (t.kind === 'newtab') return { id: `doc:${previewDocumentKey(t)}`, label: 'New tab', title: 'New tab\nOpen a tool here: Files, Browser or Subagents', icon: 'appWindow' as const };
      if (t.kind === 'terminal') return { id: `doc:${previewDocumentKey(t)}`, label: 'Terminal', title: 'Terminal\nThis chat’s shells, in its folder. Closing the tab ends them', icon: 'terminal' as const };
      if (t.kind === 'pr') return { id: `doc:${previewDocumentKey(t)}`, label: 'Pull request', title: `Pull request\nThis branch’s pull request, its checks and comments: ${t.cwd}`, icon: 'gitPullRequest' as const };
      if (t.kind === 'diff') return { id: `doc:${previewDocumentKey(t)}`, label: 'Diff', title: `Diff\nThis folder’s changes, by file or by turn: ${t.cwd}`, icon: 'gitBranch' as const };
      if (t.kind === 'changes') return { id: `doc:${previewDocumentKey(t)}`, label: 'Changes', title: 'Changes\nThis build workspace’s changed files, commit and publish', icon: 'gitBranch' as const };
      if (t.kind === 'script') return { id: `doc:${previewDocumentKey(t)}`, label: projectScripts(t.workspaceId).find(s => s.id === t.scriptId)?.name ?? 'Script', title: 'Script output\nWhat a project script printed, with Run and Stop', icon: 'terminal' as const };
      const v = visualOf(t.key);
      const what = v?.kind === 'canvas' ? 'Canvas' : v?.lang === 'svg' ? 'SVG' : 'HTML';
      return { id: `doc:${previewDocumentKey(t)}`, label: v?.title || 'Canvas', title: v ? `${what}\n${v.title}` : 'Canvas', icon: 'eye' as const };
    }),
    ...browserTabs().map(t => ({ id: `web:${t.id}`, label: tabLabel(t.url, t.title), title: `Web page\n${t.url}`, icon: 'globe' as const })),
    ...(kind() === 'url' && !activeBrowserTab() ? [{ id: 'new', label: 'New web tab', title: 'New web tab', icon: 'globe' as const }] : []),
  ]);
  const selected = () => {
    const t = previewTarget();
    return t?.kind === 'url' ? (activeBrowserTab() ? `web:${activeBrowserTab()}` : 'new') : t ? `doc:${previewDocumentKey(t)}` : '';
  };
  const pick = (id: string) => {
    if (id.startsWith('doc:')) {
      const t = previewDocuments().find(t => `doc:${previewDocumentKey(t)}` === id);
      if (t) openPreview(t);
    } else if (id.startsWith('web:')) {
      const t = browserTabs().find(t => `web:${t.id}` === id);
      if (t) { setActiveBrowserTab(t.id); selectPreviewWeb(t.url); }
    } else { newBrowserTab(); selectPreviewWeb(''); }
  };
  const focusTab = (id: string) => queueMicrotask(() => {
    const button = Array.from(tabStrip?.querySelectorAll<HTMLButtonElement>('[role="tab"]') ?? []).find(el => el.dataset.tab === id);
    button?.focus(); button?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  });
  const closeTab = (id: string) => {
    const list = tabs();
    const at = list.findIndex(t => t.id === id);
    const next = list[at + 1] ?? list[at - 1];
    const active = selected() === id;
    if (active && next) pick(next.id);
    if (id.startsWith('doc:')) closePreviewDocument(id.slice(4));
    else if (id.startsWith('web:')) void closeBrowserTab(id.slice(4));
    if (active && !next) closePreview();
    else focusTab(active ? next!.id : selected());
  };
  // A Terminal tab closing ends its shells (the store hears it): ask first when one is live.
  const requestClose = async (id: string) => {
    const doc = id.startsWith('doc:') ? previewDocuments().find(t => `doc:${previewDocumentKey(t)}` === id) : undefined;
    // The terminals store loads with the first Terminal tab, not with the pane.
    if (doc?.kind === 'terminal' && (await import('../store/terminals')).liveTerminalCount(doc.chat) > 0
      && !(await showConfirm({ title: 'Close the terminal?', body: 'This ends its shells and anything they started.', confirmLabel: 'Close terminal' }))) return;
    closeTab(id);
  };
  // The tab on show keeps its name (up to 360px; the pane names the item
  // nowhere else now) and scrolls into view when it changes; the others shrink.
  createEffect(on(selected, id => queueMicrotask(() => {
    Array.from(tabStrip?.querySelectorAll<HTMLElement>('[role="tab"]') ?? []).find(el => el.dataset.tab === id)?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  })));
  const tabKey = (e: KeyboardEvent) => {
    if (!(e.target instanceof HTMLElement) || e.target.getAttribute('role') !== 'tab') return;
    const id = e.target.dataset.tab;
    const list = tabs(); const at = list.findIndex(t => t.id === id);
    const to = e.key === 'Home' ? 0 : e.key === 'End' ? list.length - 1 : e.key === 'ArrowRight' ? (at + 1) % list.length : e.key === 'ArrowLeft' ? (at + list.length - 1) % list.length : -1;
    if (to < 0) return;
    // Manual activation: arrows only move focus; Enter/Space activate the
    // button. Traversing the strip never triggers native reads or history.
    e.preventDefault(); focusTab(list[to].id);
  };
  return (
    <aside id="preview-pane" ref={el => { pane = el; observePaneWidth(el, setWidth); }} class="preview-pane" classList={{ sized: sidePaneWidth() != null }}
      tabindex="-1" aria-label="Preview" style={{ ...(sidePaneWidth() != null ? { '--preview-pane-w': `${sidePaneWidth()}px` } : {}), ...(width() ? { '--preview-pane-now': `${Math.round(width())}px` } : {}) }}
      onKeyDown={e => { if (e.key === 'Escape' && !e.defaultPrevented) { e.stopPropagation(); closePreview(); } }}>
      <ResizeHandle edge="left" label="Resize preview" current={width} min={SIDE_PANE_MIN} max={maxWidth()}
        onResize={setSidePaneWidth} onCommit={persistSidePaneWidth} onReset={() => { setSidePaneWidth(null); persistSidePaneWidth(); }} />
      <div class="preview-pane-bar">
        <button type="button" class="chat-panel-iconbtn" aria-label="Back" title="Back" disabled={!canPreviewBack()} onClick={previewBack}><Icon name="arrowLeft" size={14} /></button>
        <button type="button" class="chat-panel-iconbtn" aria-label="Forward" title="Forward" disabled={!canPreviewForward()} onClick={previewForward}><Icon name="arrowRight" size={14} /></button>
        {/* New tab follows the last tab, apart from the file's controls. */}
        <div class="preview-pane-tabset">
          <div class="preview-tabs" ref={tabStrip} style={{ 'grid-template-columns': tabs().map(t => t.id === selected() ? 'max-content 24px' : 'minmax(80px, max-content) 24px').join(' ') }} onKeyDown={tabKey}>
            <div class="preview-tab-list" role="tablist" aria-label="Preview tabs">
              <For each={tabs().map(t => t.id)}>{id => {
                const t = () => tabs().find(t => t.id === id)!;
                return <button type="button" role="tab" class="preview-tab" data-tab={id} aria-selected={selected() === id} aria-controls="preview-tab-content" tabIndex={selected() === id ? 0 : -1} title={t()?.title} style={{ 'grid-column': tabs().findIndex(t => t.id === id) * 2 + 1, 'grid-row': 1 }} onClick={() => pick(id)}><Icon name={t()?.icon ?? 'fileText'} size={13}/><span>{t()?.label}</span></button>;
              }}</For>
            </div>
            <For each={tabs().map(t => t.id)}>{id => <button type="button" class="preview-tab-close" aria-label={`Close ${tabs().find(t => t.id === id)?.label}`} style={{ 'grid-column': tabs().findIndex(t => t.id === id) * 2 + 2, 'grid-row': 1 }} onClick={() => void requestClose(id)}><Icon name="x" size={12}/></button>}</For>
          </div>
          <button type="button" class="chat-panel-iconbtn" aria-label="New tab" title="New tab: Files, Browser, Subagents" onClick={() => openNewTab(chatAgentSessionKey())}><Icon name="plus" size={14} /></button>
        </div>
        {/* Mini player controls (parity 5.6): a direct child of the bar. */}
        <PreviewPlayerControls />
        {/* The file's controls and close wrap together in a narrow pane. */}
        <div class="preview-pane-end">
          <div class="preview-pane-tools" ref={setTools} />
          <button type="button" class="chat-panel-iconbtn preview-pane-close" aria-label="Close preview" title="Close preview (Esc)" onClick={closePreview}><Icon name="x" size={14} /></button>
        </div>
      </div>
      <div class="preview-pane-body" id="preview-tab-content" role="tabpanel" aria-label={tabs().find(t => t.id === selected())?.label ?? 'Preview'}>
        <Switch>
          <Match when={kind() === 'file'}><ChatPreview toolbar={tools()} onFacts={setFacts} /></Match>
          <Match when={kind() === 'url'}><BrowserPane unifiedTabs /></Match>
          <Match when={(() => { const t = previewTarget(); return t?.kind === 'visual' ? t.key : null; })()}>{key => <Show when={key()} keyed>{k => <PreviewVisual visualKey={k} />}</Show>}</Match>
          <Match when={(() => { const t = previewTarget(); return t?.kind === 'subagents' ? t.parent : null; })()}>{parent => <Show when={parent()} keyed>{p => <SubagentsPanel parent={p} />}</Show>}</Match>
          <Match when={(() => { const t = previewTarget(); return t?.kind === 'newtab' ? t : null; })()}>{t => <Show when={previewDocumentKey(t())} keyed>{k => <PanelNewTab tabKey={k} chat={t().chat} />}</Show>}</Match>
          <Match when={(() => { const t = previewTarget(); return t?.kind === 'terminal' ? t.chat : null; })()}>{chat => <Show when={chat()} keyed>{c => <TerminalPanel chat={c} />}</Show>}</Match>
          <Match when={(() => { const t = previewTarget(); return t?.kind === 'pr' ? t.cwd : null; })()}>{cwd => <Show when={cwd()} keyed>{c => <PrPanel cwd={c} />}</Show>}</Match>
          <Match when={(() => { const t = previewTarget(); return t?.kind === 'diff' ? t.cwd : null; })()}>{cwd => <Show when={cwd()} keyed>{c => <DiffPanel cwd={c} />}</Show>}</Match>
          <Match when={(() => { const t = previewTarget(); return t?.kind === 'changes' ? t : null; })()}>{t => <Show when={`${t().kind}:${t().workspaceId}`} keyed>{_k => <WorkspacePreview view={t().kind} workspaceId={t().workspaceId} />}</Show>}</Match>
          <Match when={(() => { const t = previewTarget(); return t?.kind === 'script' ? t : null; })()}>{t => <Show when={previewDocumentKey(t())} keyed>{_k => <ScriptOutput workspaceId={t().workspaceId} scriptId={t().scriptId} />}</Show>}</Match>
        </Switch>
      </div>
    </aside>
  );
}
