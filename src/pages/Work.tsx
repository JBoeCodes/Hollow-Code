import { MoreButton } from '../components/MoreButton';
import { workspaceMenuItems } from '../workspaceMenu';
import type { WorkspaceStarter } from '../studio';
import { newWorkspaceRequest, setNewWorkspaceRequest } from '../store/workspaces';
import { Show, createSignal, createEffect, on, lazy, Suspense, onCleanup } from 'solid-js';
import { Portal } from 'solid-js/web';
import { topBarPaneSlot, topBarSlot } from '../store/topBar';
import { Icon } from '../icons';
import { ProjectIcon } from '../components/ProjectIcon';
import { isMainAgent, isQuickChats } from '../workspaceTypes';
import { activePage, conversationHistoryShown, chatBusy, chatCwd, canLeaveConversation, requestChatFocus } from '../store';
import { workspaceDetail, workspaceError, workspaceLoading, workspaceListLoaded, workspaceListError, setWorkspaceError, workspaceErrorPlain, refreshWorkspaces, restoreLastConversation, reopenWorkspaceConversation, workspaceId } from '../store/workspaces';
import { openPreview, closePreview, previewShown, togglePreview } from '../store/preview';
import { vaultRoot, chatAgentSessionKey } from '../store/chatRuntime';
import { sideChatOpen } from '../store/sideChat';
import { createPanelPresence } from '../store/panelAnimation';
import { inVault, pinnedNotes } from '../projectFiles';
import '../styles/workbench.css';
import { PageHeader, Skeleton, ErrorText } from '../components/ui';
import { SessionIndicator } from '../components/SessionIndicator';
import { workspaceRollup } from '../store/sessionStates';
import { SESSION_STATE_WORDS, type SessionRollup } from '../sessionIndicator';
import type { WorkspaceDetail } from '../workspaceTypes';
// Home and its destination tabs are needed only outside an open workspace.
const YourWork = lazy(() => import('../components/YourWork').then(m => ({ default: m.YourWork })));
const NewWorkspaceForm = lazy(()=>import('../components/NewWorkspaceForm').then(m=>({default:m.NewWorkspaceForm})));
const ChatBody = lazy(()=>import('../components/ChatBody').then(m=>({default:m.ChatBody})));
// 7.6: the side chat pane beside the conversation, loaded the first time it opens.
const SideChatPane = lazy(()=>import('../components/SideChatPane'));

/** The top bar's Project files (COO rebuild L4, 2026-11-10): the workspace
 *  panel and its sections are gone, and a project's notes open as files in
 *  the Preview pane's Files tab. It opens the project's WORKSPACE.md (its
 *  picker pins PLAN.md, SOURCES.md and AGENTS.md beside it) and, while the
 *  pane shows, hides it. */
async function openProjectFiles(d: WorkspaceDetail) {
  const root = await vaultRoot();
  if (!root || workspaceDetail()?.workspace.id !== d.workspace.id) return;
  const first = pinnedNotes(d.workspace, d.files)[0] ?? `${d.workspace.path}/WORKSPACE.md`;
  openPreview({ kind: 'file', path: inVault(root, first) });
}

export function Work() {
  const [creationRequest,setCreationRequest]=createSignal<{starter?: WorkspaceStarter} | null>(null);
  // Whether this page is on screen. It stays mounted behind the other pages
  // (overnight 11-04 U4, App.tsx), so its conversation keeps its scroll,
  // draft and running turn; hidden, it puts nothing in the top bar, takes no
  // focus and runs no page clock.
  const shown = () => activePage() === 'today';
  let root: HTMLDivElement | undefined;
  // Hidden behind another page: nothing in it keeps focus (WebKit leaves it
  // on an element that stops displaying, so Escape and typing would still
  // reach the hidden composer) or can take it, and a New project form left
  // open goes, as it did when the page unmounted.
  createEffect(on(shown, now=>{
    if(root) root.inert = !now;
    if(now) return;
    const focus = document.activeElement;
    if(focus instanceof HTMLElement && root?.contains(focus)) focus.blur();
    setCreationRequest(null);
  }));
  const detail=()=>workspaceDetail();
  // The side chat takes the desk's second column while it is open, in a project or Chats (never the
  // Main agent, whose page stays its own). Mounted for its exit duration after closing (the Panel
  // animation setting), so it slides out and then goes.
  const sideChatWanted=()=>sideChatOpen()&&!!detail()&&!isMainAgent(detail()!.workspace);
  const sidePresence=createPanelPresence(sideChatWanted);
  // The workspace being read is the one already on the desk.
  const rereadingShown=()=>!!detail()&&detail()!.workspace.id===workspaceId();
  createEffect(()=>{if(newWorkspaceRequest()&&shown()){startNew();setNewWorkspaceRequest(0);}});
  // Every time the page shows (its mount included), as every mount did: one
  // refresh, then one a minute while it shows. Hidden, no clock runs.
  createEffect(on(shown, now=>{
    if(!now) return;
    void refreshWorkspaces({ join: true });
    const refreshTimer=setInterval(()=>void refreshWorkspaces({ join: true }),60_000);
    onCleanup(()=>clearInterval(refreshTimer));
    // Launch reopens the last conversation (frontier F4); later shows reopen
    // the remembered workspace.
    void restoreLastConversation();
  }));
  function startNew(starter?: WorkspaceStarter) { setCreationRequest({starter}); }
  // A file (or Changes) on show in the Preview pane: the button hides it.
  const filesShown = previewShown;
  // Project files, at the top bar's far right edge where the workspace panel
  // button was (asks 10-11 A3). Not in Chats, which has no project notes.
  const filesButton = () => <Show when={detail()}>{d =>
    <button type="button" class="top-bar-iconbtn work-files-toggle" aria-label="Project files" aria-pressed={filesShown()}
      title={filesShown() ? 'Hide the Preview pane' : 'Project files: WORKSPACE.md, PLAN.md, SOURCES.md and AGENTS.md in the Preview pane'}
      onClick={()=>filesShown()?closePreview():isQuickChats(d().workspace)?togglePreview(chatAgentSessionKey()):void openProjectFiles(d())}><Icon name="panelRight" size={16}/></button>}</Show>;
  // A build workspace's branch opens its Changes (it was a workspace panel
  // section): changed files, diff, commit and publish.
  const branch = () => <Show when={detail()?.workspace.build}>{b=><button type="button" class="work-branch" classList={{gone:!b().exists}} disabled={!b().exists}
    aria-label={b().exists?`Changes on ${b().branch}`:`${b().branch}: worktree removed`}
    title={b().exists?`Changes: commit and publish\nWorktree: ${b().worktree}\nFrom ${b().base} in ${b().repo}`:`The worktree was removed; the branch ${b().branch} is still in ${b().repo}`}
    onClick={()=>{const id=detail()?.workspace.id;if(id)openPreview({kind:'changes',workspaceId:id});}}><Icon name="terminal" size={13}/>{b().branch}<Show when={!b().exists}> · worktree removed</Show></button>}</Show>;
  return <div ref={root} class="work-page" classList={{'work-active':!!detail()}}>
    <PageHeader class={`work-header${detail()&&topBarSlot()?' work-header-slim':''}`} title={<><Show when={detail()}>{d=><ProjectIcon workspaceId={d().workspace.id} size="card"/>}</Show>{detail()?.workspace.title??'Projects'}</>}
      actions={<Show when={!(detail()&&topBarSlot())}><div class="work-actions">
      {branch()}<Show when={detail()&&!isMainAgent(detail()!.workspace)}><WorkspaceStatus id={detail()!.workspace.id} cwd={detail()!.workspace.cwd}/></Show>{filesButton()}<Show when={detail()}>{d=><MoreButton class="work-more-btn" label={`Actions for ${d().workspace.title}`} items={() => workspaceMenuItems(d().workspace)}/>}</Show>

    </div></Show>}/>
      {/* An open workspace keeps its controls in the top bar (2026-09-29):
          the header line itself is then only for screen readers. Only while
          the page shows: a hidden Work page leaves the bar to the page on
          screen (overnight 11-04 U4). */}
      <Show when={shown()&&detail()&&topBarSlot()}>{slot=><Portal mount={slot()}><div class="work-actions work-topbar-actions">
      {branch()}<Show when={detail()&&!isMainAgent(detail()!.workspace)}><WorkspaceStatus id={detail()!.workspace.id} cwd={detail()!.workspace.cwd}/></Show><Show when={detail()}>{d=><MoreButton class="work-more-btn" label={`Actions for ${d().workspace.title}`} items={() => workspaceMenuItems(d().workspace)}/>}</Show>
    </div></Portal>}</Show>
      <Show when={shown()&&detail()&&topBarPaneSlot()}>{slot=><Portal mount={slot()}>{filesButton()}</Portal>}</Show>
    <Show when={workspaceError()}>{msg=><ErrorText class="work-error" as="div" plain={workspaceErrorPlain()} text={msg()}>{' '}<button class="btn small ghost" onClick={()=>{setWorkspaceError('');void refreshWorkspaces();}}>Dismiss</button></ErrorText>}</Show>
    <Show when={creationRequest()}><Suspense fallback={<Portal><div class="rbk-backdrop project-create-backdrop"><div class="work-create project-create-dialog" role="dialog" aria-modal="true" aria-label="New project"><Skeleton label="Loading project form" rows={4}/></div></div></Portal>}><NewWorkspaceForm request={creationRequest()!} onCreated={()=>requestChatFocus()}/></Suspense></Show>
    {/* A re-read of the workspace on screen (its chat's sidebar row picked
        again, from another page or this one) keeps the desk mounted, so the
        retained conversation keeps its scroll place (overnight 11-04 U4);
        opening another workspace still says so. */}
    <Show when={(!workspaceLoading()||rereadingShown())&&(workspaceListLoaded()||!!detail())} fallback={<div class="studio-loading" role="status"><span class="studio-loading-mark" aria-hidden="true"/><span>{workspaceLoading()?'Opening workspace…':workspaceListError()?"Couldn't load your workspaces.":'Loading your work…'}</span><Show when={workspaceListError()}><button class="btn" onClick={()=>void refreshWorkspaces()}>Retry loading workspaces</button></Show></div>}>
      {/* Home mounts only while it shows, as before: its reads and clocks
          never run behind another page. */}
      <Show when={detail()} fallback={<Show when={shown()}><YourWork onNew={startNew}/></Show>}>
        <div class="work-desk" classList={{'has-side-chat':sidePresence.mounted()}}>
          <section class="work-conversation chat-main" aria-label="Workspace conversation">
            {/* Library Conversations owns the shared chat while open, even
                for this workspace's cwd. Never mount two composers or image
                sinks; returning remounts this owner with the shared draft. */}
            <Show when={!conversationHistoryShown()&&chatCwd()===detail()?.workspace.cwd} fallback={<div class="work-empty"><p>The current conversation belongs to another context.</p><button class="btn" disabled={!canLeaveConversation()} onClick={()=>void reopenWorkspaceConversation()}>Open workspace conversation</button></div>}><Suspense fallback={<Skeleton class="work-skeleton" label="Loading conversation" rows={5} />}>
              <ChatBody workspace />
            </Suspense></Show>
          </section>
          <Show when={sidePresence.mounted()}><Suspense fallback={null}><SideChatPane closing={sidePresence.closing()}/></Suspense></Show>
        </div>
      </Show>
    </Show>
  </div>;
}


/** The workspace header's live state (asks 10-11 A2): its conversations
 *  rolled up in the session-indicator family ("Running", "2 running",
 *  "Waiting on you"), or the conversation on screen while its first turn
 *  runs before the list has it. Nothing when all are idle. Not on the Main
 *  agent, whose own header line says it (11-06). */
function WorkspaceStatus(props: { id: string; cwd: string }) {
  const state = (): SessionRollup | null => workspaceRollup(props.id)
    ?? (chatBusy() && chatCwd() === props.cwd ? { state: 'running', count: 1, label: SESSION_STATE_WORDS.running } : null);
  return <Show when={state()}>{r =>
    <span class="work-status" data-state={r().state} role="status" aria-label={r().label} title={r().label}>
      <SessionIndicator state={r().state} size={12} decorative/><span class="work-status-words" aria-hidden="true">{r().count > 1 ? `${r().count} ${SESSION_STATE_WORDS[r().state].toLowerCase()}` : SESSION_STATE_WORDS[r().state]}</span>
    </span>}
  </Show>;
}
