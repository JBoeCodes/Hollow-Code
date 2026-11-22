import { projectNameProblem } from '../projectCreation';
import { For, Show, createSignal, createEffect, on, onCleanup } from 'solid-js';
import { open } from '@tauri-apps/plugin-dialog';
import { api } from '../api';
import { WORKSPACE_STARTERS, type WorkspaceStarter } from '../studio';
import { type RepoCandidate, type WorkWorkspace, repoDetail } from '../workspaceTypes';
import { activePage, shownPage, chatAgentSessionKey, pendingChatSend, setPendingChatSend, chatBusy, chatCwd, chatSessionLoading, canLeaveConversation } from '../store';
import { projectWorkspaces, workspaceId, workspaceLoading, workspaceError, setWorkspaceWarning, refreshWorkspaces, openProjectAgent } from '../store/workspaces';
import { Select } from './Select';
import { Icon } from '../icons';
import { ErrorText } from './ui';
import { Portal } from 'solid-js/web';
import { holdFocus } from '../focusHold';

// A completed creation survives leaving/remounting the page until the person
// opens it successfully or explicitly dismisses it. Never create twice on retry.
type CreatedWorkspace = {workspace: WorkWorkspace; goal: string; warning: string};
const [created,setCreated]=createSignal<CreatedWorkspace | null>(null);
const [creatingWorkspace,setCreatingWorkspace]=createSignal(false);
type FailedCreation = {title: string; goal: string; kind: 'general'|'build'; repo: string; branch: string; base: string; error: string};
const [failedCreation,setFailedCreation]=createSignal<FailedCreation | null>(null);


export function NewWorkspaceForm(props: {request: {starter?: WorkspaceStarter}; onCreated: () => void}) {
  const [creating,setCreating]=createSignal(false);const [title,setTitle]=createSignal('');const [goal,setGoal]=createSignal('');
  let dialog: HTMLFormElement | undefined;
  let releaseFocus: (() => void) | undefined;
  createEffect(on(creating, shown => {
    if (!shown) { releaseFocus?.(); releaseFocus=undefined; return; }
    queueMicrotask(() => {
      if (!creating() || !dialog) return;
      releaseFocus?.();
      const restoreFocus=holdFocus(dialog);
      const root=document.getElementById('root');
      const wasInert=root?.inert ?? false;
      if(root)root.inert=true;
      releaseFocus=()=>{if(root)root.inert=wasInert;restoreFocus();};
      dialog.querySelector<HTMLInputElement>('[aria-label="Project name"]')?.focus();
    });
  }));
  onCleanup(() => releaseFocus?.());
  const [pending,setPending]=createSignal(false);
  // Build workspaces: a git worktree + branch of their own (build_workspaces.rs).
  const [kind,setKind]=createSignal<'general'|'build'>('general');
  const [repos,setRepos]=createSignal<RepoCandidate[]>([]);const [repo,setRepo]=createSignal('');const [reposLoading,setReposLoading]=createSignal(false);
  const [branch,setBranch]=createSignal('');const [base,setBase]=createSignal('');
  const [reposError,setReposError]=createSignal('');
  const [createError,setCreateError]=createSignal('');
  let ownedSend: { request: NonNullable<ReturnType<typeof pendingChatSend>>; receipt: CreatedWorkspace } | undefined;
  function cancelOwnedSend() {
    const owned=ownedSend; ownedSend=undefined;
    if(owned && pendingChatSend()===owned.request){setPendingChatSend(null);setCreated(owned.receipt);}
  }
  let mounted = true;
  onCleanup(()=>{ mounted = false; cancelOwnedSend(); });
  createEffect(()=>{
    const queued=pendingChatSend();
    if(ownedSend && queued===ownedSend.request && (activePage()!=='today' || workspaceLoading() || chatSessionLoading()
      || workspaceId()!==ownedSend.receipt.workspace.id || chatCwd()!==ownedSend.request.cwd || chatAgentSessionKey()!==ownedSend.request.sessionKey)) cancelOwnedSend();
  });
  createEffect(on(created, receipt=>{
    if(!receipt)return;
    setTitle(receipt.workspace.title);setGoal(receipt.goal);
    setKind(receipt.workspace.build?'build':'general');setRepo(receipt.workspace.build?.repo??'');
    setBranch(receipt.workspace.build?.branch??'');setBase(receipt.workspace.build?.base??'');
    setCreateError('Project created. Your goal is saved in its brief. Open it to continue.');
  }));
  createEffect(on(failedCreation, draft=>{
    if(!draft || created())return;
    setTitle(draft.title);setGoal(draft.goal);setKind(draft.kind);
    setRepo(draft.repo);setBranch(draft.branch);setBase(draft.base);setCreateError(draft.error);
  }));
  const blocker = () => pending() || creatingWorkspace() ? 'Creating or opening your project…'
    : !canLeaveConversation() ? 'Wait for the current message to finish sending.'
    : created() ? '' : projectNameProblem(title(), projectWorkspaces().map(w=>w.title)) ? projectNameProblem(title(), projectWorkspaces().map(w=>w.title))
    : new TextEncoder().encode(goal()).length > 12000 ? 'Keep project instructions under 12,000 bytes.'
    : WORKSPACE_STARTERS.some(st=>st.goal.trim()===goal().trim()) ? 'Replace the … first'
    : kind()==='build'&&!repo() ? 'Choose the repository to build in.' : '';
  async function loadRepos(){
    if(reposLoading())return;
    setReposLoading(true);setReposError('');
    try{setRepos(await api.buildRepoCandidates());}
    catch(e){setReposError(String(e));}
    finally{setReposLoading(false);}
  }
  async function chooseKind(next:'general'|'build'){
    setKind(next);
    // Never preselect: the first discovered repository is usually unrelated
    // (audit H05). The user chooses one, or picks a folder outside discovery.
    if(next==='build'&&!repos().length)await loadRepos();
  }
  async function pickRepo(){
    const path=await open({multiple:false,directory:true,title:'Choose a git repository'}).catch(()=>null);
    if(typeof path!=='string')return;
    if(!repos().some(r=>r.path===path))setRepos([{name:path.split('/').pop()||path,path},...repos()]);
    setRepo(path);
  }
  // Clone from an address (4.4): the clone lands in the Projects folder and is chosen as the repository.
  const [cloneUrl,setCloneUrl]=createSignal('');const [cloning,setCloning]=createSignal(false);const [cloneError,setCloneError]=createSignal('');
  async function cloneFromUrl(){
    const url=cloneUrl().trim();
    if(!url||cloning()||pending())return;
    setCloning(true);setCloneError('');
    try{
      const cloned=await api.cloneRepository(url);
      if(!mounted)return;
      await loadRepos();
      if(!repos().some(r=>r.path===cloned.path))setRepos([{name:cloned.name,path:cloned.path},...repos()]);
      setRepo(cloned.path);setCloneUrl('');
    }catch(e){if(mounted)setCloneError(String(e));}
    finally{if(mounted)setCloning(false);}
  }
  async function create(start = false) {
    if(blocker() || (start && !goal().trim()))return;
    const origin = { page: shownPage(), id: workspaceId(), cwd: chatCwd(), session: chatAgentSessionKey() };
    const stillHere = () => mounted && shownPage()===origin.page && workspaceId()===origin.id
      && chatCwd()===origin.cwd && chatAgentSessionKey()===origin.session;
    const submittedGoal = goal().trim() || title().trim();
    const submittedDraft = {title:title(),goal:goal(),kind:kind(),repo:repo(),branch:branch(),base:base()};
    const name=title().trim()||submittedGoal.split('\n')[0].slice(0,80);
    setPending(true);setCreatingWorkspace(true);setCreateError('');
    try {
      let receipt=created();
      if(!receipt){
        if(kind()==='build'){
          const made=await api.createBuildWorkspace({title:name,goal:submittedGoal,repo:repo(),branch:branch().trim()||undefined,base:base().trim()||undefined});
          receipt={workspace:made.workspace,goal:submittedGoal,warning:made.warnings.join(' ')};
        }else receipt={workspace:await api.createWorkspace(name,submittedGoal),goal:submittedGoal,warning:''};
        setCreated(receipt);
        setFailedCreation(null);
      }
      const {workspace:w,warning}=receipt;
      await refreshWorkspaces();
      if(!stillHere())return;
      const opened=await openProjectAgent(w.id,stillHere);
      if(!opened || !mounted || workspaceId()!==w.id || chatCwd()!==w.cwd || chatSessionLoading()){
        setCreateError('Project created. Your goal is saved in its brief. Try opening it again.');
        return;
      }
      if(start){
        if(chatBusy() || !chatAgentSessionKey() || pendingChatSend()){
          setCreateError(pendingChatSend() ? 'Project created. Another message is waiting to send. Try Open and start after it finishes.' : 'Project created. Wait for the conversation to be ready, then try Open and start.');
          return;
        }
        // A preset carries only this submitted goal, leaving any composer
        // draft and attachments alone. The new conversation owns its cwd/key.
        const request={text:receipt.goal,cwd:w.cwd,sessionKey:chatAgentSessionKey(),preset:true};
        ownedSend={request,receipt};setPendingChatSend(request);
      }
      setCreating(false);setCreated(null);setBranch('');setBase('');setTitle('');setGoal('');
      // After the open, which clears the banner; a failed open's error stays.
      // Build warnings are shown as written (A08).
      if(warning&&!workspaceError())setWorkspaceWarning(warning);
      props.onCreated();
    }catch(e){
      if(!created())setFailedCreation({...submittedDraft,error:String(e)});
      else if(mounted)setCreateError(String(e));
    }finally{setPending(false);setCreatingWorkspace(false);}
  }
  /** A "Start from" chip inside the form: fills only what the user hasn't
   *  written (an untouched starter template counts as unwritten) and keeps a
   *  chosen repository. */
  function applyStarter(st: WorkspaceStarter) {
    const pristine = (v: string, pick: (s: WorkspaceStarter) => string) => !v.trim() || WORKSPACE_STARTERS.some(x => pick(x) === v);
    if (pristine(goal(), x => x.goal)) setGoal(st.goal);
    if (pristine(title(), x => x.name)) setTitle(st.name);
    void chooseKind(st.kind);
  }
  function cancelCreate() { if(pending()||creatingWorkspace())return;setCreating(false);setCreated(null);setFailedCreation(null); }
  function startNew(starter?: WorkspaceStarter) { if(pending()||creatingWorkspace())return;setCreating(true);if(created()||failedCreation())return;setCreateError('');setRepo('');setTitle(starter?.name ?? '');setGoal(starter?.goal ?? '');void chooseKind(starter?.kind ?? 'general'); }

  createEffect(on(()=>props.request, request=>startNew(request.starter)));
  return <Show when={creating()}><Portal><div class="rbk-backdrop project-create-backdrop"><form ref={dialog} class="work-create project-create-dialog" role="dialog" aria-modal="true" aria-labelledby="project-create-title" onSubmit={e=>{e.preventDefault();void create();}} onKeyDown={e=>{
      if(e.defaultPrevented || e.isComposing)return;
      if(e.key==='Escape'){e.preventDefault();e.stopPropagation();cancelCreate();}
      if(e.key==='Enter'&&(e.metaKey||e.ctrlKey)&&!e.shiftKey&&!e.altKey){e.preventDefault();e.stopPropagation();void create();}
    }}><div class="work-create-head"><div><h2 id="project-create-title">New project</h2><p class="work-muted">A home for your chats, instructions and files.</p></div><button type="button" class="btn icon work-create-close" aria-label="Cancel new project" disabled={pending()} onClick={cancelCreate}><Icon name="x" size={16}/></button></div><div class="project-create-fields"><label>Name<input aria-label="Project name" disabled={pending()||!!created()} value={title()} onInput={e=>setTitle(e.currentTarget.value)} placeholder="e.g. Portal or Union Rec" maxLength={100} required/></label>
      <label><span>Instructions <span class="work-muted">optional</span></span><textarea aria-label="Project instructions" disabled={pending()||!!created()} value={goal()} onInput={e=>setGoal(e.currentTarget.value)} placeholder="What should your project agent know?" rows={3}/></label>
      <details class="project-create-options"><summary tabindex="0">Templates and location</summary>
      <div class="work-starters" role="group" aria-label="Start from"><For each={WORKSPACE_STARTERS.filter(st=>st.kind!=='build')}>{st=><button type="button" disabled={pending()||!!created()} aria-pressed={title()===st.name&&goal()===st.goal} onClick={()=>applyStarter(st)}><Icon name={st.icon} size={14}/>{st.title}</button>}</For></div>
      <p class="work-muted">Saved in your Hollow Library. For code work, choose a repository.</p>
      <div class="work-kind" role="radiogroup" aria-label="Project location"><button type="button" disabled={pending()||!!created()} role="radio" aria-checked={kind()==='general'} onClick={()=>void chooseKind('general')}>Hollow Library</button><button type="button" disabled={pending()||!!created()} role="radio" aria-checked={kind()==='build'} onClick={()=>void chooseKind('build')}><Icon name="terminal" size={14}/>Repository worktree</button></div>
      <Show when={kind()==='build'}><div class="work-build-fields"><p class="work-muted">Hollow makes a new git worktree and branch for this workspace, so its changes never touch your main checkout or another workspace. Conversations here run inside it.</p>
        <div class="field"><span id="build-repo-label">Repository</span><span class="work-build-repo">
          <Show when={repos().length} fallback={<span class="work-muted work-build-none">{reposLoading()?'Looking for repositories…':reposError()?'Repository discovery is unavailable.':'No repositories found in your Projects folder. Choose the repository folder instead.'}</span>}>
            <Select label="Repository" disabled={pending()||!!created()} placeholder="Choose a repository…" value={repo()} onChange={setRepo} emptyText="No repository matches"
              options={repos().map(r=>({value:r.path,label:r.name,detail:repoDetail(r),keywords:r.path}))}/>
          </Show>
          <button type="button" disabled={pending()||!!created()} class="btn small" classList={{primary:!reposLoading()&&!repos().length}} onClick={()=>void pickRepo()}>Choose folder…</button></span>
          <Show when={reposError()}><ErrorText class="work-error" as="div" text={reposError()}/><button type="button" class="btn small" disabled={reposLoading()||pending()||!!created()} onClick={()=>void loadRepos()}>Retry repositories</button></Show>
          <Show when={repo()}>{p=><small class="work-build-path" title={p()}>{p().replace(/^\/Users\/[^/]+/,'~')}</small>}</Show>
          <Show when={!repo()&&repos().length}><small class="work-muted">Pick the repository this workspace should change. Nothing is chosen for you.</small></Show></div>
        <div class="field work-clone"><label for="clone-address">Clone from an address <span class="work-muted">https:// or ssh, saved in your Projects folder</span></label><span class="work-build-repo"><input id="clone-address" aria-label="Clone address" disabled={pending()||cloning()||!!created()} value={cloneUrl()} onInput={e=>setCloneUrl(e.currentTarget.value)} onKeyDown={e=>{if(e.key==='Enter'&&!e.metaKey&&!e.ctrlKey){e.preventDefault();e.stopPropagation();void cloneFromUrl();}}} placeholder="https://github.com/owner/repo.git" spellcheck={false}/><button type="button" class="btn small" disabled={!cloneUrl().trim()||cloning()||pending()||!!created()} onClick={()=>void cloneFromUrl()}>{cloning()?'Cloning…':'Clone'}</button></span>
          <Show when={cloneError()}><ErrorText class="work-error" as="div" text={cloneError()}/></Show></div>
        <label>Branch <span class="work-muted">optional</span><input aria-label="Branch name" disabled={pending()||!!created()} value={branch()} onInput={e=>setBranch(e.currentTarget.value)} placeholder="hollow/<name>"/></label>
        <label>Base branch <span class="work-muted">optional</span><input aria-label="Base branch" disabled={pending()||!!created()} value={base()} onInput={e=>setBase(e.currentTarget.value)} placeholder="origin's default branch, freshly fetched"/></label></div></Show>
      </details>
      <Show when={createError()}><ErrorText class="work-error" as="div" text={createError()}/></Show></div>
      <footer class="project-create-footer"><p id="workspace-create-blocker" class="work-muted" role="status">{blocker()||(goal().trim() ? 'Create and start also sends your instructions as the first message.' : 'Its lead will be ready in the project sidebar.')}</p><div class="work-actions"><button type="button" class="btn ghost" disabled={pending()} onClick={cancelCreate}>Cancel</button><Show when={goal().trim()}><button type="button" class="btn" aria-describedby="workspace-create-blocker" disabled={!!blocker()} onClick={()=>void create(true)}>{created()?'Open and start':'Create and start'}</button></Show><button type="submit" class="btn primary" aria-describedby="workspace-create-blocker" disabled={!!blocker()}>{pending()?'Opening…':created()?'Open project':'Create project'}</button></div></footer></form></div></Portal></Show>;
}
