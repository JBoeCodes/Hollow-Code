import { createSignal, createEffect, on, onMount, onCleanup, For, Show } from 'solid-js';
import { Select } from './Select';
import { SourceCoverage } from './SourceCoverage';
import { api, type DirectoryConnection } from '../api';
import type { ConnectedFile, ConnectedFileState, ConnectedIndexStatus } from '../types';
import { refreshWorkspaces, selectWorkspace } from '../store/workspaces';
import { canLeaveConversation, openSettingsFromOutside } from '../store';
import { latestRequest } from '../latestRequest';
import { FILE_STATE_OPTIONS, connectedFileTag, findLine, listCountText } from '../connectedFiles';
import '../styles/directory-setup.css';
import { ErrorText } from './ui';
import { sourceStateLine } from '../sourceAuthority';
import { SourceStateText } from './WorkspaceSources';

/** Files per page of `connected_files` (next-level P07). */
const PAGE = 100;

type Listing = { files: ConnectedFile[]; direct: ConnectedFile[]; total: number; omitted: number; query: string };

/**
 * Library → Connected projects. The inventory is paged from the backend
 * (`connected_files`), never held whole here: a 50,000-file project costs one
 * page. Only the latest request may answer (`latestRequest`), so a slow page
 * for an older query never replaces a newer one. `initialFile` opens a file
 * directly (the command palette's project lookup), scrolled to the line that
 * best matches `find`.
 */
export function ConnectedProjects(props: { initialProject?: string; initialFile?: { path: string; find?: string } | null; onSelection: (path:string)=>void }) {
  const [connection,setConnection]=createSignal<DirectoryConnection>();
  const [contentIndex,setContentIndex]=createSignal<ConnectedIndexStatus|null>(null);
  const [query,setQuery]=createSignal('');const [project,setProject]=createSignal(props.initialProject??'');
  const [fileState,setFileState]=createSignal<''|ConnectedFileState>('');
  // A folder connecting didn't make a workspace for gets one now; a Notion
  // project's workspace of the same name opens instead (overnight 10-11 Y1).
  const [openError,setOpenError]=createSignal('');
  const openProject=async()=>{setOpenError('');try{const id=await api.openProjectWorkspace(project());await refreshWorkspaces();await selectWorkspace(id);}catch(e){setOpenError(String(e));}};
  const [path,setPath]=createSignal('');const [text,setText]=createSignal('');const [find,setFind]=createSignal('');
  const [error,setError]=createSignal('');const [busy,setBusy]=createSignal(false);
  const [listing,setListing]=createSignal<Listing|null>(null);const [loading,setLoading]=createSignal(false);
  const order=latestRequest();const reads=latestRequest();let alive=true;let timer: ReturnType<typeof setTimeout>|undefined;
  let mark: HTMLElement|undefined;
  createEffect(()=>props.onSelection(path()));
  createEffect(()=>{setProject(props.initialProject??'');setPath('');});
  createEffect(()=>{const f=props.initialFile;if(f)void read(f.path,f.find??'');});
  async function list(append=false) {
    const q=query().trim();const offset=append?(listing()?.files.length??0):0;
    const ticket=order.start();setLoading(true);
    try {
      const r=await api.connectedFiles({query:q||undefined,projectId:project()||undefined,fileState:fileState()||undefined,offset,limit:PAGE});
      if(!alive||!order.isLatest(ticket))return;
      setListing(prev=>append&&prev?{...prev,files:[...prev.files,...r.files],total:r.total}:{files:r.files,direct:r.direct,total:r.total,omitted:r.omitted,query:q});
    } catch(e){if(alive&&order.isLatest(ticket))setError(String(e));}
    finally{if(alive&&order.isLatest(ticket))setLoading(false);}
  }
  // Re-list on a new query, project or state (debounced for typing), and
  // when the inventory itself changed (`changedAt`), not on every minute's
  // refresh, so "Show more" pages survive the poll.
  createEffect(on([query,project,fileState],()=>{clearTimeout(timer);timer=setTimeout(()=>void list(),150);},{defer:true}));
  createEffect(on(()=>connection()?`${connection()!.projectsPath}|${connection()!.coverage?.changedAt??''}`:'',key=>{if(key)void list();}));
  async function reload(refresh=false) {
    setBusy(true);setError('');
    try {const c=await (refresh?api.refreshDirectories():api.directoriesStatus());if(alive)setConnection(c);if(refresh)await refreshWorkspaces();}
    catch(e){if(alive)setError(String(e));}finally{if(alive)setBusy(false);}
    // Repo features R14b: "Contents indexed: N of M" (best effort).
    api.connectedIndexStatus().then(s=>{if(alive)setContentIndex(s);}).catch(()=>{});
  }
  onMount(()=>{void reload();const poll=setInterval(()=>void reload(),60_000);onCleanup(()=>clearInterval(poll));});
  onCleanup(()=>{alive=false;reads.cancel();order.cancel();clearTimeout(timer);});
  async function read(p:string, near='') {
    const token=reads.start();setPath(p);setText('');setFind(near);setError('');
    try {const content=await api.readConnectedFile(p);if(reads.isLatest(token)){setText(content);queueMicrotask(()=>mark?.scrollIntoView({block:'center'}));}}catch(e){if(reads.isLatest(token))setError(String(e));}
  }
  const located=()=>findLine(text(),find());
  /** The open file's project, for "Live file from <project>" (plan L3 X3). */
  const projectTitle=()=>{const l=listing();const f=[...(l?.files??[]),...(l?.direct??[])].find(x=>x.path===path());const all=connection()?.projects??[];const id=f?.projectId??project();return (all.find(p=>p.id===id)??all.find(p=>path().startsWith(`${p.path}/`)))?.title??null;};
  const row=(f:ConnectedFile,direct=false)=><button class="library-item library-row" classList={{selected:path()===f.path}} title={f.relative} onClick={()=>read(f.path)}><strong>{f.relative.split('/').pop()}</strong><small>{f.relative.split('/').slice(0,-1).join('/')}</small><Show when={connectedFileTag(f)} fallback={<Show when={direct}><span class="library-tag" title="Not in the listed inventory (its project is over its share); found by a direct lookup">found directly</span></Show>}>{tag=><span class="library-tag" title={tag().hint}>{tag().label}</span>}</Show></button>;
  return <section class="connected-files" aria-label="Connected project files">
    <SourceCoverage connection={connection()} contentIndex={contentIndex()} busy={busy()} onChange={setConnection} onRefresh={()=>void reload(true)} onShowState={s=>{setQuery('');setFileState(s);}}/>
    <Show when={error()}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
    <Show when={openError()}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
    <div class="library-layout"><aside class="library-list" aria-label="Connected files"><input type="search" aria-label="Find connected file" placeholder="Find a project file by name or path…" value={query()} onInput={e=>setQuery(e.currentTarget.value)}/><Select label="Connected project" value={project()} onChange={v=>{setProject(v);setPath('');}} emptyText="No project matches" options={[{value:'',label:'All connected projects'},...(connection()?.projects??[]).map(p=>({value:p.id,label:p.title}))]}/>
      <Select label="File state" value={fileState()} onChange={v=>setFileState(v as ''|ConnectedFileState)} options={FILE_STATE_OPTIONS.map(o=>({value:o.value,label:o.label}))}/>
      <Show when={project()}><button class="btn small" disabled={!canLeaveConversation()} onClick={()=>void openProject()}>Open project workspace</button></Show>
      <div class="library-count" aria-live="polite">{loading()&&!listing()?'Loading files…':listCountText(listing()?.files.length??0,listing()?.total??0,listing()?.direct.length??0,listing()?.query??'')}</div>
      <div class="library-results" classList={{updating:loading()}} aria-busy={loading()}>
        <For each={listing()?.files??[]} fallback={<Show when={listing()&&!listing()!.direct.length}><p class="work-muted">{listing()!.query||fileState()?`Nothing listed matches.${listing()!.omitted?` ${listing()!.omitted.toLocaleString('en-US')} files aren’t listed; a name search looks in them too.`:''}`:<>No connected files yet. Put repositories or project folders in your Hollow folder’s Projects folder; <button type="button" class="btn-link" onClick={()=>{openSettingsFromOutside('vault');}}>Settings → Sources → Hollow folder</button> shows where it is.</>}</p></Show>}>{f=>row(f)}</For>
        <Show when={listing()?.direct.length}><p class="work-muted connected-direct-head">Found directly, outside the listed files</p><For each={listing()!.direct}>{f=>row(f,true)}</For></Show>
      </div>
      <Show when={(listing()?.files.length??0)<(listing()?.total??0)}><button class="btn small" disabled={loading()} onClick={()=>void list(true)}>Show more</button></Show>
    </aside><section class="library-reader" aria-label="Connected file reader"><button class="btn small library-back" onClick={()=>setPath('')}>← Connected files</button><Show when={path()} fallback={<div class="work-empty"><h2>Project files, in place.</h2><p>Select a file to read its current contents. Repositories and existing folder organization are preserved.</p></div>}><SourceStateText state={sourceStateLine({kind:'live',project:projectTitle()})}/><button class="btn small" onClick={()=>api.chatPreviewOpen(path(),true).catch(e=>setError(String(e)))}>Show original file</button><p class="work-muted" style={{'overflow-wrap':'anywhere'}}>{path()} · read only<Show when={located()}>{l=><> · line {l().number}</>}</Show></p><Show when={located()} fallback={<pre class="connected-file-text">{text()}</pre>}>{l=><pre class="connected-file-text">{l().before}<mark ref={mark} class="connected-file-match">{l().line}</mark>{l().after}</pre>}</Show></Show></section></div>
  </section>;
}
