import { Button, EmptyState, Menu, PageHeader, ErrorText, Skeleton, Tabs, type TabItem } from '../components/ui';
import { LibraryResults } from '../components/LibraryResults';
import { RecordingsView } from '../components/RecordingsView';
import { NOTE_TEMPLATES, addView, collectionLabel, meaningCountText, meaningInCollection, readViews, removeView, templateBody, type LibraryView } from '../libraryViews';
import type { BacklinkSource } from '../types';
import { Select } from '../components/Select';
import { Icon } from '../icons';
import { ICON_MEANING } from '../iconMeaning';
import { ConnectedProjects } from '../components/ConnectedProjects';
import { SOURCE_COLLECTIONS, inSourceCollection, projectRecordWorkspace } from '../sourceCatalog';
import { ProjectDetail } from './tasks/ProjectDetail';
// Records' Decisions (COO rebuild L4: they were on a workspace's Overview).
const RecordDecisions = lazy(() => import('../components/RecordDecisions').then(m => ({ default: m.RecordDecisions })));
import { groupLibrary, libraryTitle, matchesLibraryQuery, wikilinkStep, type LibraryGroup } from '../libraryGroups';
import { flashWithUndo } from '../store/snackbars';
import { workspaces, workspaceId, refreshWorkspaces, selectWorkspace } from '../store/workspaces';
import { libraryCollectionRequest, setLibraryCollectionRequest, openSettingsFromOutside, chatBusy, enableApprovalsResource, reviewQueueFailed, reviewQueueItems } from '../store';
import { learningsWaiting } from '../learningBulk';
import { openAdd, openImportSource } from '../modals/addState';
import { For, Show, Suspense, createEffect, createMemo, createResource, createSignal, lazy, on, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { WorkFileEditor, hasUnsavedDraft } from '../components/WorkFileEditor';
import { HistoryRecoveryPanel } from '../components/HistoryRecoveryPanel';
import { semanticSearchEnabled, noteIndex, refreshNoteIndex, openVaultNoteRequest, setOpenVaultNoteRequest, newNoteRequest, setNewNoteRequest, newNoteTemplateRequest, setNewNoteTemplateRequest, openVaultHistoryRequest, setOpenVaultHistoryRequest, flashFooter, vaultRoot, showConfirm, showPrompt } from '../store';
import { ActionMenu } from '../components/ActionMenu';
import { SemanticSearchLink } from '../components/SemanticSearchLink';
import { canManageNote, movedNotePath, noteFolder, renamedNotePath } from '../libraryNoteManage';
import { isWorkspaceVaultItem } from '../workspaceContent';
import { ArtifactSessions } from '../components/ArtifactSessions';
import type { SearchResult } from '../types';
import { beginSearch, searchEmptyText, visibleResults, type SearchState } from '../searchState';
import { latestRequest } from '../latestRequest';
import '../styles/workbench.css';
import '../styles/library.css';
import { bindLibraryViewReset, isLibraryDetail, libraryMode, openLibraryTab, resetResultsType, setLibraryMode, type LibraryMode } from '../store/libraryMode';
import { isWorkspaceSnapshot, snapshotParts, sourceStateLine, trackerMirrorOf, vaultFileKind, vaultRelativePath } from '../sourceAuthority';
import { SourceActions, SourceStateText } from '../components/WorkspaceSources';
import { RecoverableView } from '../components/RecoverableView';
// Conversations (S4, 2026-11-03): Conversation history, the old hidden page,
// loaded when first shown.
const ConversationHistory = lazy(() => import('../components/ConversationHistory').then(m => ({ default: m.ConversationHistory })));
const Learnings = lazy(() => import('../components/Learnings').then(m => ({ default: m.Learnings })));
// Skills (overnight 11-12 L2): Settings' skills catalog, the same view in its
// own chunk (Settings, Connectors keeps only a link row here since Settings PR 4).
const SkillsView = lazy(() => import('./Skills').then(m => ({ default: m.SkillsView })));
// All (overnight 11-12 L3): one searchable list of everything Library's
// readers load, its own chunk.
const LibraryList = lazy(() => import('../components/LibraryList').then(m => ({ default: m.LibraryList })));

export function Library() {
  // Library views (next-level P06): All (overnight 11-12 L3: one searchable
  // list over the readers below, the default), Notes (saved material), Results across
  // workspaces, Recordings (in Plaud; these three open from All since L4), Connected projects, Conversations
  // (Conversation history, its own page until S4), Skills and History
  // (Settings' catalog and History & recovery, overnight 11-12 L2), Learnings. (Boards went with
  // whiteboards on 2026-10-07; an unsupported saved view opens Results.)
  // The view lives in store/libraryMode.ts (consolidation lane B), so `g x`,
  // the palette and workspace Results can open Results → Canvases.
  type Mode=LibraryMode;
  const mode=libraryMode;
  // A tab starts clean: no open file, no file history, no New note form (P1-19).
  // Conversations is a step of its own for Back and Forward (S4): it opens
  // through its page id, `chat`, and leaving it for another tab is a step too.
  // The palette and links take the same path (openLibraryTab, #1311 re-review).
  const pickMode=(m:Mode)=>{void openLibraryTab(m);};
  // Learnings says how many wait (J7 review), from the review queue snapshot:
  // the tab keeps its name and adds the count, as a description too. One
  // array with a getter, so a new count never re-creates the tab buttons.
  enableApprovalsResource();
  const learningCount=()=>learningsWaiting(reviewQueueItems.latest,reviewQueueFailed());
  // All opens the strip (overnight 11-12 L3); Skills, History and Learnings
  // close it (overnight 11-12 L2). Notes, Results and Recordings left it
  // (overnight 11-12 L4): they open from All (Browse folders, Outputs → Grid,
  // Meetings → Recordings), All stays selected while one is open, and each
  // shows Back to All.
  const viewTabs:TabItem<Mode>[]=[{value:'all',label:'All',icon:'list'},
    {value:'connected',label:'Connected projects',icon:'folder'},{value:'conversations',label:'Conversations',icon:'message'},
    {value:'skills',label:'Skills',icon:ICON_MEANING.skill},{value:'history',label:'History',icon:'history'},
    {value:'learnings',label:'Learnings',icon:'book',accessibleName:'Learnings',get count(){return learningCount();},get hint(){const n=learningCount();return n?`${n} pending`:undefined;}}];
  const stripValue=():Mode=>isLibraryDetail(mode())?'all':mode();
  const connected=()=>mode()==='connected';
  const setConnected=(v:boolean)=>setLibraryMode(v?'connected':'notes');
  // Leaving Library: Results starts with every type next time.
  onCleanup(resetResultsType);
  const [views,setViews]=createSignal<LibraryView[]>(readViews((()=>{try{return localStorage.getItem('hollow.library.views');}catch{return null;}})()));
  const saveViews=(v:LibraryView[])=>{setViews(v);try{localStorage.setItem('hollow.library.views',JSON.stringify(v));}catch{/* session only */}};
  const [template,setTemplate]=createSignal('blank');
  const [connectedProject,setConnectedProject]=createSignal('');
  // `connected-file:{"path","find"}` (JSON): open one connected file (P07 palette lookup).
  const [connectedFile,setConnectedFile]=createSignal<{path:string;find?:string}|null>(null);
  const [query,setQuery]=createSignal('');const [folder,setFolder]=createSignal('shared');
  const [path,setPath]=createSignal('');const [creating,setCreating]=createSignal(false);
  const [name,setName]=createSignal('');const [error,setError]=createSignal('');
  const [pending,setPending]=createSignal(false);const [history,setHistory]=createSignal(false);
  const [run,setRun]=createSignal<string|null>(null);
  onCleanup(bindLibraryViewReset(()=>{setConnectedFile(null);setPath('');setHistory(false);setCreating(false);}));
  const [target,setTarget]=createSignal(workspaceId()??'');
  const [attached,setAttached]=createSignal('');
  // Bumped when a reviewed refresh replaces the open snapshot, so the reader re-reads it (plan L3 X5).
  const [readerVersion,setReaderVersion]=createSignal(0);
  // The open file's text as loaded, to tell a tracker mirror (plan L3 X3, B3).
  const [readerText,setReaderText]=createSignal('');
  // A project record (workspaces/<id>/PROJECT.md) opens as the project detail (B3); Show the file opens the editor.
  const [showRecordFile,setShowRecordFile]=createSignal(false);
  const [recordEditing,setRecordEditing]=createSignal(false);
  createEffect(on(path,()=>{setReaderText('');setShowRecordFile(false);setRecordEditing(false);},{defer:true}));
  const notes=createMemo(()=>noteIndex().filter(n=>isWorkspaceVaultItem({path:n.path})));
  const folders=createMemo(()=>Array.from(new Set(notes().map(n=>n.path.split('/')[0]))).sort());
  const collection=()=>SOURCE_COLLECTIONS.find(c=>c.id===folder()) as {id:string;label:string;description:string;tab?:Mode;tabLabel?:string}|undefined;
  const filtered=createMemo(()=>notes().filter(n=>inSourceCollection(n.path,folder())&&matchesLibraryQuery(n,query())));
  // Grouped by folder and titled from the notes' own headings, so a
  // citation file reads as its meeting or email, not `P046be4a221cb`.
  const groups=createMemo(()=>groupLibrary(filtered()));
  const [toggled,setToggled]=createSignal<Record<string,boolean>>({});
  const [expanded,setExpanded]=createSignal<Record<string,boolean>>({});
  const isOpen=(g:LibraryGroup)=>!!query().trim()||(toggled()[g.key]??!g.folded);
  const GROUP_PREVIEW=12;
  // Search by meaning (Settings, Search by meaning): the on-device index,
  // debounced, replacing the name filter while it has a query. Its hits stay
  // inside the chosen collection or folder, and the count names it (P1-19);
  // a narrower collection asks the index for more hits to filter.
  const [byMeaning,setByMeaning]=createSignal(false);
  // Pending is never "no results" (audit H09): the previous hits stay while
  // a newer request runs, and only the latest request may answer.
  const [meaning,setMeaning]=createSignal<SearchState<SearchResult>>({status:'idle'});
  const meaningOn=()=>byMeaning()&&semanticSearchEnabled()&&query().trim().length>=3;
  // What a saved view records: By meaning only where it is on, available and has a query it runs for.
  const savedMeaning=()=>byMeaning()&&semanticSearchEnabled()&&query().trim().length>=3;
  // A memo, so switching between two narrow collections doesn't search again.
  const meaningLimit=createMemo(()=>folder()==='all'?40:120);
  const order=latestRequest();
  let meaningTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(()=>{clearTimeout(meaningTimer);order.cancel();});
  createEffect(()=>{
    const q=query().trim();const on=byMeaning()&&semanticSearchEnabled();
    clearTimeout(meaningTimer);
    if(!on||q.length<3){order.cancel();setMeaning({status:'idle'});return;}
    const ticket=order.start();setMeaning(prev=>beginSearch(prev,q));
    const limit=meaningLimit();
    meaningTimer=setTimeout(()=>{api.semanticSearch(q,limit).then(r=>{if(order.isLatest(ticket))setMeaning({status:'done',query:q,results:r.filter(h=>isWorkspaceVaultItem({path:h.path}))});}).catch(e=>{if(order.isLatest(ticket))setMeaning({status:'error',query:q,message:String(e)});});},300);
  });
  const meaningHits=()=>meaningOn()?meaningInCollection(visibleResults(meaning())??[],folder()):null;
  const emptyText=()=>meaningOn()?searchEmptyText(meaning(),'meaning'):query()?searchEmptyText({status:'done',query:query(),results:[]},'name'):null;
  const byPath=createMemo(()=>new Map(notes().map(n=>[n.path,n])));
  const meaningShown=createMemo(()=>meaningHits()?.map(h=>{const n=byPath().get(h.path);const basename=n?.basename??(h.path.split('/').pop()??h.path).replace(/\.md$/i,'');return {path:h.path,title:libraryTitle({basename,title:n?.title}),detail:h.snippet||n?.summary||''};})??null);
  const shownCount=()=>meaningShown()?.length??filtered().length;
  onMount(()=>{void refreshWorkspaces();refreshNoteIndex().catch(e=>setError(String(e)));});
  createEffect(()=>{folder();setExpanded({});});
  createEffect(()=>{const c=libraryCollectionRequest();if(c){if(c.startsWith('connected-file:')){try{const f=JSON.parse(c.slice(15));if(typeof f?.path==='string'){setConnected(true);setConnectedProject('');setConnectedFile({path:f.path,find:typeof f.find==='string'?f.find:undefined});}}catch{/* malformed request: ignore */}}else if(c.startsWith('connected:')){setConnected(true);setConnectedFile(null);setConnectedProject(c.slice(10));}else{setConnected(false);setFolder(c);}setLibraryCollectionRequest(null);}});
  createEffect(()=>{path();setAttached('');});
  createEffect(()=>{const p=openVaultNoteRequest();if(p){setConnected(false);setPath(p);setHistory(false);setOpenVaultNoteRequest(null);}});
  createEffect(()=>{if(newNoteRequest()){setTemplate(newNoteTemplateRequest()??'blank');setNewNoteTemplateRequest(null);startNote();setNewNoteRequest(0);}});
  createEffect(()=>{const r=openVaultHistoryRequest();if(r){setConnected(false);setPath(r.path??'');setRun(r.runId??null);setHistory(true);setOpenVaultHistoryRequest(null);}});
  // New note opens its form in Notes with the name field focused (P1-19).
  let noteNameField: HTMLInputElement | undefined;
  function startNote(title=''){setConnected(false);setPath('');setName(title);setCreating(true);queueMicrotask(()=>noteNameField?.focus());}
  // Show file history opens in Notes from any tab; the same item hides it.
  const historyShown=()=>history()&&mode()==='notes';
  function toggleHistory(){if(historyShown()){setHistory(false);return;}setCreating(false);if(mode()!=='notes'){setLibraryMode('notes');setConnectedFile(null);setPath('');}setRun(null);setHistory(true);}
  async function createNote(){
    const title=name().trim();if(!title||pending())return;
    if(/[\/\\\n\r]/.test(title)){setError('Use a note name without slashes or line breaks.');return;}
    setPending(true);setError('');
    try{const p=`library/${title.replace(/\.md$/i,'')}.md`;const today=new Date();const d=`${today.getFullYear()}-${String(today.getMonth()+1).padStart(2,'0')}-${String(today.getDate()).padStart(2,'0')}`;await api.vaultCreateNote(p,templateBody(template(),title.replace(/\.md$/i,''),d));await refreshNoteIndex();setPath(p);setCreating(false);setName('');setHistory(false);}
    catch(e){setError(String(e));}finally{setPending(false);}
  }
  // A [[wikilink]] in the reader or editor (P0-26) opens its note; a link to
  // no note offers Create note, with the form filled in.
  function openWikilink(link:string){
    if(!link.trim())return;
    const step=wikilinkStep(link,notes());
    if(step.kind==='open'){setPath(step.path);setHistory(false);return;}
    flashWithUndo({message:`No note is named “${step.title}”.`,kind:'status',ttlMs:7000,actionLabel:'Create note',undo:()=>{setHistory(false);startNote(step.title);}});
  }
  async function attachToWork() {
    const source=path(), id=target();if(!source||!id||pending())return;
    setPending(true);setError('');
    try {await api.attachWorkspaceSource(id,source);if(path()===source&&target()===id)setAttached(id);}
    catch(e){setError(String(e));}finally{setPending(false);}
  }
  // Rename, move and trash (P1-18): writable vault notes only (libraryNoteManage.ts), from the reader's File
  // actions and each row's ⋯. Never while the note has an unsaved draft (open or kept from earlier), which is
  // kept under its old path.
  const [readerDirty,setReaderDirty]=createSignal(false);
  const noteTitle=(p:string)=>{const n=byPath().get(p);return n?libraryTitle(n):(p.split('/').pop()??p).replace(/\.md$/i,'');};
  const draftOpen=(p:string)=>{if((p===path()&&readerDirty())||hasUnsavedDraft(p)){flashFooter(`“${noteTitle(p)}” has an unsaved draft. Open it, then save or discard the draft first.`);return true;}return false;};
  async function relocate(p:string,to:string,done:string){
    try{await api.vaultMoveNote(p,to);}catch(e){setError(String(e));return;}
    await refreshNoteIndex().catch(()=>{});
    if(path()===p){setPath(to);setHistory(false);}
    flashFooter(done);
  }
  async function renameNote(p:string){
    if(draftOpen(p))return;setError('');
    // A rename breaks [[links]] by the old name; a move keeps the name, so they keep resolving.
    const links=await api.vaultBacklinks(p).catch(()=>[] as BacklinkSource[]);
    const name=await showPrompt({title:`Rename “${noteTitle(p)}”`,body:links.length?`${links.length} ${links.length===1?'note links':'notes link'} to this note by name. Those links stop working after a rename, unless only the letter case changes.`:undefined,confirmLabel:'Rename',input:{label:'File name',value:(p.split('/').pop()??'').replace(/\.md$/i,'')}});
    if(name===null)return;
    const to=renamedNotePath(p,name);
    if('error' in to){setError(to.error);return;}
    if(to.path!==p)await relocate(p,to.path,`Renamed to ${to.path.split('/').pop()}.`);
  }
  async function moveNote(p:string){
    if(draftOpen(p))return;setError('');
    const folder=await showPrompt({title:`Move “${noteTitle(p)}” to a folder`,body:'A folder in the vault, such as library/projects; type / for the vault’s top level. Hollow makes the folder if it doesn’t exist. Links by name keep working.',confirmLabel:'Move',input:{label:'Folder',value:noteFolder(p)}});
    if(folder===null)return;
    const to=movedNotePath(p,folder,noteIndex().map(n=>n.path));
    if('error' in to){setError(to.error);return;}
    if(to.path!==p)await relocate(p,to.path,`Moved to ${noteFolder(to.path)?`${noteFolder(to.path)}/`:'the vault’s top level'}.`);
  }
  async function trashNote(p:string){
    if(draftOpen(p))return;setError('');
    const title=noteTitle(p);
    if(!await showConfirm({title:`Move “${title}” to the trash?`,body:'It goes to the vault’s .trash folder. You can undo right after.',confirmLabel:'Move to trash',danger:true}))return;
    let trashed:string;
    try{trashed=await api.vaultDeleteNote(p);}catch(e){setError(String(e));return;}
    const wasOpen=path()===p;
    if(wasOpen){setPath('');setHistory(false);}
    await refreshNoteIndex().catch(()=>{});
    // Undo reopens the note only where it was open and nothing else has been opened since.
    flashWithUndo({message:`Moved “${title}” to the trash.`,kind:'status',ttlMs:8000,actionLabel:'Undo',undo:()=>{void api.vaultMoveNote(trashed,p).then(async()=>{await refreshNoteIndex().catch(()=>{});if(wasOpen&&!path()&&!history()&&!connected())setPath(p);flashFooter(`Restored “${title}”.`);}).catch(e=>setError(`Couldn’t restore “${title}”: ${String(e)}`));}});
  }
  const noteActions=(p:string,inFileActions=false)=>canManageNote(p)?[{label:'Rename…',action:()=>void renameNote(p),separatorBefore:inFileActions},{label:'Move to folder…',action:()=>void moveNote(p)},{label:'Move to trash',action:()=>void trashNote(p),danger:true}]:[];
  const item=(n:{path:string;title:string;detail:string})=><div class="library-row"><button class="library-item" classList={{selected:path()===n.path}} aria-current={path()===n.path?'true':undefined} title={n.path} onClick={()=>{setPath(n.path);setHistory(false);}}><span class="studio-library-file-icon" aria-hidden="true"><Icon name="fileText" size={18}/></span><strong>{n.title}</strong><Show when={n.detail}><small>{n.detail}</small></Show></button><Show when={canManageNote(n.path)}><ActionMenu label={`More actions for ${n.title}`} items={noteActions(n.path)}/></Show></div>;
  return <div class="work-page library-page" classList={{'library-has-file':!!path()||history()}}>
    <PageHeader title="Library" actions={<><Show when={mode()!=='all'}><Button variant={mode()==='notes'?'primary':'ghost'} size="sm" onClick={()=>startNote()}>New note</Button></Show><Menu label="Library actions" items={[{label:'Add a task, note, link, file or recording…',action:()=>openAdd()},{label:'Import a source',action:openImportSource},{label:'Manage connections',action:()=>{openSettingsFromOutside('connectors');}},{label:historyShown()?'Hide file history':'Show file history',action:toggleHistory}]}/></>}/>
    <Tabs label="Library view" class="library-tabs" value={stripValue()} onChange={pickMode} items={viewTabs}/>
    {/* All's views (L4): the way back, above the view. */}
    <Show when={isLibraryDetail(mode())}><button type="button" class="btn small ghost library-back-all" aria-label="Back to All" onClick={()=>pickMode('all')}><span aria-hidden="true">← </span>All</button></Show>
    <Show when={error()}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
    <Show when={creating()}><div class="work-create"><label>Note name<input ref={el=>{noteNameField=el;}} aria-label="Note name" value={name()} onInput={e=>setName(e.currentTarget.value)} onKeyDown={e=>{if(e.key==='Enter')void createNote();}}/></label><div class="field"><span>Start from</span><Select size="sm" label="Note template" value={template()} searchable={false} onChange={setTemplate} options={NOTE_TEMPLATES.map(t=>({value:t.id,label:t.label}))}/></div><div class="work-actions"><button class="btn primary" disabled={pending()||!name().trim()} onClick={createNote}>Create note</button><button class="btn ghost" onClick={()=>setCreating(false)}>Cancel</button></div><p class="work-muted">Saved as a plain Markdown file in library/.</p></div></Show>
    <Show when={mode()==='all'}><RecoverableView name="All" fallback={<Skeleton label="Opening All" rows={6}/>}><LibraryList onNewNote={()=>startNote()} onBrowse={()=>pickMode('notes')} onGrid={()=>pickMode('results')} onRecordings={()=>pickMode('recordings')}/></RecoverableView></Show>
    <Show when={mode()==='results'}><LibraryResults/></Show>
    <Show when={mode()==='recordings'}><RecordingsView/></Show>
    <Show when={mode()==='learnings'}><RecoverableView name="Learnings" fallback={<Skeleton label="Opening Learnings" rows={3}/>}><Learnings/></RecoverableView></Show>
    <Show when={mode()==='skills'}><RecoverableView name="Skills" fallback={<Skeleton label="Opening Skills" rows={4}/>}><SkillsView/></RecoverableView></Show>
    {/* The whole vault's History & recovery, as Settings → Advanced shows it; Show file history in Notes keeps the per-note one. */}
    <Show when={mode()==='history'}><RecoverableView name="History" fallback={<Skeleton label="Opening History" rows={3}/>}><HistoryRecoveryPanel/></RecoverableView></Show>
    <Show when={mode()==='conversations'}><RecoverableView name="Conversation history" fallback={<Skeleton rows={6} label="Opening Conversation history"/>}><ConversationHistory/></RecoverableView></Show>
    <Show when={mode()==='notes'||mode()==='connected'}>
    <Show when={!connected()} fallback={<ConnectedProjects initialProject={connectedProject()} initialFile={connectedFile()} onSelection={setPath} />}><div class="library-layout"><aside class="library-list" aria-label="Library files"><input class="library-search" type="search" aria-label="Find a file" placeholder={byMeaning()&&semanticSearchEnabled()?'Describe what you’re looking for…':'Find a file…'} value={query()} onInput={e=>setQuery(e.currentTarget.value)}/><Show when={semanticSearchEnabled()}><button class="btn small ghost library-meaning" aria-pressed={byMeaning()} title="Find notes by what they mean, using the on-device index" onClick={()=>setByMeaning(v=>!v)}>By meaning</button></Show><SemanticSearchLink/><Select class="library-folder" label="Library folder" value={folder()} onChange={setFolder} options={[...SOURCE_COLLECTIONS.map(c=>({value:c.id,label:c.label,group:'Collections'})),...folders().filter(f=>!SOURCE_COLLECTIONS.some(c=>c.id===f)).map(f=>({value:f,label:`${f}/`,group:'Folders'}))]}/><p class="work-muted library-description">{collection()?.description}<Show when={collection()?.tab}>{t=><> <button type="button" class="btn-link" onClick={()=>pickMode(t())}>Open {collection()!.tabLabel}</button></>}</Show></p><Show when={folder()==='records'}><Suspense><RecordDecisions/></Suspense></Show>
      <div class="library-utility"><div class="library-count" aria-live="polite">{meaningOn()&&meaning().status==='pending'?(shownCount()?`Updating… ${meaningCountText(shownCount(),folder())}`:'Searching by meaning…'):meaningOn()?meaningCountText(shownCount(),folder()):`${shownCount()} ${shownCount()===1?'file':'files'}${groups().length<2?'':` in ${groups().length} folders`}`}</div><button type="button" class="btn small ghost library-view-save" onClick={()=>{const label=query().trim()?`${collectionLabel(folder())}: ${query().trim()}${savedMeaning()?', by meaning':''}`:collectionLabel(folder());saveViews(addView(views(),{name:label,folder:folder(),query:query(),byMeaning:savedMeaning()}));}}><Icon name="pin" size={11}/>{query().trim()?'Save this view':'Pin this collection'}</button></div>
      <Show when={views().length}><div class="library-views" role="group" aria-label="Saved views"><For each={views()}>{v=><span class="library-view-chip" classList={{active:v.folder===folder()&&v.query===query().trim()&&v.byMeaning===savedMeaning()}}><button type="button" onClick={()=>{setFolder(v.folder);setQuery(v.query);setByMeaning(v.byMeaning);}}>{v.name}</button><button type="button" class="library-view-remove" aria-label={`Remove saved view ${v.name}`} onClick={()=>saveViews(removeView(views(),v.id))}><Icon name="x" size={10}/></button></span>}</For></div></Show>
      <div class="library-results" classList={{updating:meaningOn()&&meaning().status==='pending'}} aria-busy={meaningOn()&&meaning().status==='pending'}><Show when={shownCount()} fallback={<EmptyState class="library-empty" icon="book" title={<span role={emptyText()?.role}>{emptyText()?.title??'No files yet'}</span>} actions={<><Button onClick={()=>{setQuery('');setFolder('all');}}>Browse all files</Button><Button variant="ghost" onClick={()=>startNote()}>Write a note</Button></>}>{emptyText()?.detail??'Import a source or write a note. Files agents make in workspaces show up here too.'}</EmptyState>}><Show when={meaningShown()} fallback={<For each={groups()}>{g=><section class="library-group" aria-label={g.label}><h2 class="library-group-head"><button aria-expanded={isOpen(g)} onClick={()=>setToggled(t=>({...t,[g.key]:!isOpen(g)}))}><Icon name="chevronRight" size={14}/><span>{g.label}</span><small>{g.entries.length}</small></button></h2><Show when={isOpen(g)}><div class="library-group-items"><For each={expanded()[g.key]||query().trim()?g.entries:g.entries.slice(0,GROUP_PREVIEW)}>{n=>item(n)}</For></div><Show when={!expanded()[g.key]&&!query().trim()&&g.entries.length>GROUP_PREVIEW}><button class="btn small ghost library-group-more" onClick={()=>setExpanded(e=>({...e,[g.key]:true}))}>Show all {g.entries.length}</button></Show></Show></section>}</For>}>{hits=><div class="library-group-items"><For each={hits()}>{n=>item(n)}</For></div>}</Show></Show></div></aside>
      <section class="library-reader" aria-label="File reader"><button class="btn small library-back" onClick={()=>{setPath('');setHistory(false);}}>← All files</button><Show when={history()} fallback={<Show when={path()} fallback={<EmptyState icon="fileText" title="No file open">Choose a note to read or edit. New reference notes live in library/.</EmptyState>}><Show when={projectRecordWorkspace(path())&&!showRecordFile()} fallback={<><Show when={projectRecordWorkspace(path())}><button type="button" class="btn small ghost library-record-toggle" onClick={()=>setShowRecordFile(false)}>Show the project</button></Show><ReaderSource path={path()} tracked={trackerMirrorOf(readerText())} target={target()} onTarget={setTarget} pending={pending()} attached={attached()} onAttach={()=>void attachToWork()} onOpen={p=>{setPath(p);setHistory(false);}} onReplaced={()=>setReaderVersion(v=>v+1)}/><Show when={readerVersion()+1} keyed>{_version=><WorkFileEditor path={path()} actions={noteActions(path(),true)} askInCurrentConversation outline readOnly={isWorkspaceSnapshot(path())} onWikilinkClick={openWikilink} onLoaded={setReaderText} onSaved={()=>void refreshNoteIndex()} onEditorState={(_editing,dirty)=>setReaderDirty(dirty)}/>}</Show></>}><div class="library-project-record"><Show when={projectRecordWorkspace(path())} keyed>{ws=><ProjectDetail workspaceId={ws} onEditing={setRecordEditing}/>}</Show><button type="button" class="btn small ghost library-record-toggle" title={recordEditing()?'Your unsaved portfolio edits wait here until you come back':undefined} onClick={()=>setShowRecordFile(true)}>Show the file</button></div></Show><Show when={path()} keyed>{p=><Backlinks path={p} titleOf={x=>byPath().get(x)?.title} onOpen={x=>{setPath(x);setHistory(false);}}/>}</Show><Show when={path()} keyed>{p=><ArtifactSessions path={p}/>}</Show></Show>}><HistoryRecoveryPanel notePath={path()||undefined} runId={run()??undefined}/></Show></section>
    </div></Show></Show>
  </div>;
}

/** Notes that link to this one (P06/P08): read-only backlinks, each opens,
 *  named by the note's title as the list names it (P1-18). */
function Backlinks(props:{path:string;titleOf:(path:string)=>string|undefined;onOpen:(path:string)=>void}){
  const [links]=createResource(()=>props.path,p=>api.vaultBacklinks(p).catch(()=>[] as BacklinkSource[]));
  return <Show when={(links.latest??[]).length}><section class="library-backlinks" aria-label="Linked from"><h3>Linked from <span class="ui-section-count">{links.latest!.length}</span></h3><ul><For each={links.latest}>{b=><li><button class="btn-link" title={b.path} onClick={()=>props.onOpen(b.path)}>{libraryTitle({basename:b.basename,title:props.titleOf(b.path)})}</button><Show when={b.matches[0]}>{m=><small>{m().text.slice(0,140)}</small>}</Show></li>}</For></ul></section></Show>;
}

/**
 * What the open file is and what can be done with it (plan L3 X3/X4,
 * 2026-10-14; review U library). A snapshot says so first, with Open
 * original and Check for changes. Any other file says whether it is a note
 * or a Result, then offers to copy it into the chosen workspace as a
 * snapshot ("Use in this workspace"); when that workspace already holds a
 * snapshot of it, View snapshot opens that instead of attaching it again.
 */
function ReaderSource(props:{path:string;tracked:string|null;target:string;onTarget:(id:string)=>void;pending:boolean;attached:string;onAttach:()=>void;onOpen:(path:string)=>void;onReplaced:()=>void}){
  const snap=()=>snapshotParts(props.path);
  const [root]=createResource(()=>vaultRoot());
  // The workspace this file is a snapshot in, or the chosen destination; re-read after an attach.
  const [detail,{refetch}]=createResource(()=>snap()?.workspaceId??(props.target?`${props.target}`:null),id=>api.readWorkspace(id).catch(()=>null));
  createEffect(on(()=>props.attached,a=>{if(a)void refetch();},{defer:true}));
  const titleOf=(id:string)=>workspaces().find(w=>w.id===id)?.title??'that workspace';
  const source=()=>{const s=snap();return s?detail.latest?.sources.find(x=>x.path===s.path)??null:null;};
  const existing=()=>{
    const d=detail.latest;if(snap()||!d||d.workspace.id!==props.target)return null;
    const name=props.path.split('/').pop();
    return d.sources.find(x=>vaultRelativePath(x.original,root.latest)===props.path||(props.attached===props.target&&x.path===`sources/${name}`))??null;
  };
  const state=()=>{const s=source();if(snap())return sourceStateLine({kind:'snapshot',freshness:s?.freshness??null,captured:s?.captured??null,original:s?.original??null});
    const kind=vaultFileKind(props.path);return kind==='output'?sourceStateLine({kind:'output',workspace:titleOf(props.path.split('/')[1])}):(props.tracked?sourceStateLine({kind:'tracked',service:props.tracked==='linear'?'Linear':props.tracked}):sourceStateLine({kind:'note'}));};
  return <div class="library-reader-source">
    <SourceStateText state={state()}/>
    <Show when={snap()} fallback={<div class="work-source-actions">
      <label>Workspace<Select label="Destination workspace" placeholder="Choose workspace…" value={props.target} onChange={props.onTarget} emptyText="No workspace matches" options={workspaces().map(w=>({value:w.id,label:w.title}))}/></label>
      <Show when={existing()} fallback={<button class="btn small primary" disabled={!props.target||props.pending} title="Copy this file into the workspace’s sources as a snapshot; this file stays here" onClick={props.onAttach}>{props.target&&props.target===workspaceId()?'Use in this workspace':props.target?`Use in ${titleOf(props.target)}`:'Use in this workspace'}</button>}>{s=>
        <><button class="btn small" onClick={()=>props.onOpen(`${detail.latest!.workspace.path}/${s().path}`)}>View snapshot</button><span class="work-muted">Already in {titleOf(props.target)} as a snapshot.</span></>}
      </Show>
      <Show when={props.attached&&props.attached===props.target}><span role="status">Snapshot added to {titleOf(props.attached)}.</span><button class="btn small" disabled={chatBusy()} onClick={()=>selectWorkspace(props.attached)}>Open workspace</button></Show>
    </div>}>{s=>
      <Show when={source()}>{c=><SourceActions workspaceId={s().workspaceId} path={c().path} name={c().path.replace(/^sources\//,'')} original={c().original} freshness={c().freshness} refreshable={!!c().original} onReplaced={()=>{void refetch();props.onReplaced();}}/>}</Show>
    }</Show>
  </div>;
}
