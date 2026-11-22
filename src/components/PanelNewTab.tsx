import { For, Show, batch } from 'solid-js';
import { Icon, type IconName } from '../icons';
import { DRAFT_TAB_OWNER, canReopenClosedView, closePreviewDocument, closedViewLabel, newTabLookupStillValid, previewIntentNow, reopenClosedView, replacePreviewDocument, selectPreviewWeb } from '../store/preview';
import { newBrowserTab } from '../store/browser';
import { chatAgentSessionKey, vaultRoot } from '../store/chatRuntime';
import { workspaceDetail } from '../store/workspaces';
import { isQuickChats } from '../workspaceTypes';
import { inVault, pinnedNotes } from '../projectFiles';
import { panelTools, type PanelTool } from '../sidePanelModel';
import '../styles/side-panel.css';

const ICONS: Record<PanelTool['id'], IconName> = { files: 'fileText', browser: 'globe', subagents: 'bot', terminal: 'terminal', changes: 'gitBranch' };

/**
 * A New tab in the Preview pane (overnight 11-12 C5, a browser's new tab
 * page): the Tools grid. Picking one opens it in this tab: Files (the open
 * project's WORKSPACE.md; the Files picker pins the other notes), Browser
 * (the start page with its address bar), Subagents (the owning chat's)
 * or, for a build workspace, Changes. Its tools act for the chat that
 * owns the tab (`chat`), never for whichever chat is on screen later.
 */
export function PanelNewTab(props: { tabKey: string; chat: string }) {
  const detail = () => workspaceDetail();
  // The owning chat is the one on screen (a draft owner: a chat not sent yet).
  const ownerShown = () => props.chat === DRAFT_TAB_OWNER ? !chatAgentSessionKey() : chatAgentSessionKey() === props.chat;
  const tools = () => {
    const w = detail()?.workspace;
    return panelTools({ project: !!w && !isQuickChats(w), chat: props.chat !== DRAFT_TAB_OWNER && ownerShown(), build: !!w?.build?.exists });
  };
  const open = async (tool: PanelTool) => {
    if (tool.unavailable || !ownerShown()) return;
    const d = detail();
    if (tool.id === 'browser') {
      batch(() => { closePreviewDocument(props.tabKey); newBrowserTab(); selectPreviewWeb(''); });
    } else if (tool.id === 'subagents') {
      replacePreviewDocument(props.tabKey, { kind: 'subagents', parent: props.chat });
    } else if (tool.id === 'terminal') {
      replacePreviewDocument(props.tabKey, { kind: 'terminal', chat: props.chat });
    } else if (tool.id === 'changes') {
      if (d) replacePreviewDocument(props.tabKey, { kind: 'changes', workspaceId: d.workspace.id });
    } else if (d) {
      const stamp = previewIntentNow();
      const root = await vaultRoot();
      // Closed, another tab selected, the pane closed or another chat on screen during the lookup: nothing opens.
      if (!root || workspaceDetail()?.workspace.id !== d.workspace.id || !newTabLookupStillValid(props.tabKey, props.chat, stamp) || !ownerShown()) return;
      const first = pinnedNotes(d.workspace, d.files)[0] ?? `${d.workspace.path}/WORKSPACE.md`;
      replacePreviewDocument(props.tabKey, { kind: 'file', path: inVault(root, first) });
    }
  };
  return (
    <div class="panel-newtab">
      <section aria-labelledby="panel-newtab-tools">
        <h3 id="panel-newtab-tools" class="panel-newtab-head">Tools</h3>
        <ul class="panel-newtab-tools">
          <For each={tools()}>{tool => (
            <li>
              <button type="button" class="panel-newtab-tool" data-tool={tool.id} aria-disabled={tool.unavailable ? 'true' : undefined}
                title={tool.unavailable ?? tool.detail} onClick={() => void open(tool)}>
                <Icon name={ICONS[tool.id]} size={15} />
                <span class="panel-newtab-tool-label">{tool.label}</span>
                <span class="panel-newtab-tool-detail">{tool.unavailable ?? tool.detail}</span>
              </button>
            </li>
          )}</For>
        </ul>
      </section>
      {/* The visible route to Reopen closed view (also a palette row and a View menu item; no chord). */}
      <Show when={canReopenClosedView()}>
        <section aria-labelledby="panel-newtab-closed">
          <h3 id="panel-newtab-closed" class="panel-newtab-head">Closed</h3>
          <ul class="panel-newtab-tools">
            <li>
              <button type="button" class="panel-newtab-tool" data-tool="reopen"
                onClick={() => batch(() => { reopenClosedView(); closePreviewDocument(props.tabKey); })}>
                <Icon name="refresh" size={15} />
                <span class="panel-newtab-tool-label">Reopen closed tab</span>
                <span class="panel-newtab-tool-detail">{closedViewLabel()}</span>
              </button>
            </li>
          </ul>
        </section>
      </Show>
    </div>
  );
}
