import { QUICK_CHATS_ID, QUICK_CHATS_TITLE, isBuiltinWorkspace, type WorkWorkspace, type WorkspaceDetail } from './workspaceTypes';
import { CONTEXT_DOCS, contextHealthFromSizes } from './workspaceRecap';
import { readPlan } from './planModel';
import { sha256Hex, sourceLines } from './sectionMarkdown';
export function workspaceFixture(command: string, args: Record<string,unknown> | undefined, notes: Map<string,string>): { value: unknown } | null {
  if (command === 'read_agent_session') {
    const summary = (JSON.parse(localStorage.getItem('mockWorkspaceSessions') ?? '[]') as { key: string }[]).find(s => s.key === args?.key);
    if (summary) return { value: { summary, turns: [], eventsRetained: true } };
  }
  // Like the backend (2026-11-04), the built-in chats space reads as Chats
  // whatever a seeded title says ("Quick chats" in older fixtures and vaults).
  const read=():WorkWorkspace[]=>(JSON.parse(localStorage.getItem('mockWorkspaces')??'[]') as WorkWorkspace[]).map(w=>w.id===QUICK_CHATS_ID?{...w,title:QUICK_CHATS_TITLE}:w);
  const saveNotes=()=>localStorage.setItem('mockWorkspaceNotes',JSON.stringify(Object.fromEntries(notes)));
  const stored=JSON.parse(localStorage.getItem('mockWorkspaceNotes')??'{}') as Record<string,string>;
  for(const [path,text] of Object.entries(stored))if(!notes.has(path))notes.set(path,text);
  const detail=(id:string):WorkspaceDetail=>{
    const workspace=read().find(w=>w.id===id);if(!workspace)throw Error('Workspace not found');
    const sources=mockSources(workspace.path,notes);
    const reentry=JSON.parse(localStorage.getItem('mockWorkspaceReentry')??'{}')[id] ?? {asOf:new Date().toISOString(),nextStep:null,openItems:[],openItemCount:0,lastRun:null,gaps:[]};
    const sizes=CONTEXT_DOCS.flatMap(name=>{const t=notes.get(`${workspace.path}/${name}`);return t==null?[]:[{name,bytes:new TextEncoder().encode(t).length}];});
    const contextHealth=workspace.kind==='quick'?[]:contextHealthFromSizes(sizes);
    return {projectAgentKey:isBuiltinWorkspace(workspace)?undefined:`s-project-${sha256Hex(id).slice(0,48)}`,workspace,sources,sourcesOmitted:0,reentry,contextHealth,files:[...notes].filter(([p])=>p.startsWith(workspace.path+'/')&&!hiddenPath(p)).map(([path,text])=>({path,name:path.slice(workspace.path.length+1),kind:path.includes('/sources/')?'source':path.includes('/outputs/')?'output':path.includes('/sessions/')?'session':'context',size:text.length,modified:JSON.parse(localStorage.getItem('mockWorkspaceFileTimes')??'{}')[path]??null})),sessions:(JSON.parse(localStorage.getItem('mockWorkspaceSessions')??'[]') as any[]).filter(s=>s.cwd===workspace.cwd)};
  };
  // The Main agent's folder (its page opens it): the seeded one, else a fresh one.
  // Like the backend, ensure CREATES the built-in workspace when it is missing
  // (review F06): every fixture (empty, demo, large) can open the Main agent.
  if(command==='ensure_main_agent_workspace'){const list=JSON.parse(localStorage.getItem('mockWorkspaces')??'[]') as {id:string}[];const found=list.find(w=>w.id==='main');if(found)return {value:found};const main={id:'main',title:'COO',status:'active',path:'workspaces/main',cwd:'/Users/fixture/vault/workspaces/main',modified:new Date().toISOString()};localStorage.setItem('mockWorkspaces',JSON.stringify([...list,main]));return {value:main};}
  if(command==='create_main_standing_orders'){
    // Small fixture text; Rust tests pin the full seed and brief requirement.
    if(!read().some(w=>w.id==='main'))throw Error('Open the COO first.');
    const path='workspaces/main/STANDING_ORDERS.md';
    if(notes.has(path))return {value:false};
    notes.set(path,'# Standing orders\n\n## Watch\n\n- Chats it started.\n');
    saveNotes();
    return {value:true};
  }
  if(command==='read_agent_conversation'){const value=JSON.parse(localStorage.getItem('mockWorkspaceConversations')??'{}')[String(args?.key)];if(value)return {value};}
  if(command==='list_workspaces'){
    if(localStorage.getItem('mockWorkspaceListError'))throw new Error('Workspace folder is unavailable');
    return {value:read()};
  }
  // Quick chats (2026-09-30): created on first use, like the backend.
  const ensureQuick=()=>{
    const found=read().find(w=>w.id==='quick-chats');if(found)return found;
    const quick={id:'quick-chats',path:'workspaces/quick-chats',title:'Quick chats',status:'active',cwd:'/Users/fixture/vault/workspaces/quick-chats',modified:new Date().toISOString(),kind:'quick' as const};
    notes.set('workspaces/quick-chats/WORKSPACE.md','---\nhollow_workspace: 1\nkind: quick\ntitle: "Chats"\nstatus: active\n---\n\n# Chats\n');saveNotes();
    localStorage.setItem('mockWorkspaces',JSON.stringify([...read(),quick]));return quick;
  };
  if(command==='ensure_quick_chats_workspace')return {value:ensureQuick()};
  if(command==='move_agent_session'){
    const key=String(args?.key),to=String(args?.workspaceId);
    if(localStorage.getItem('mockMoveError'))throw localStorage.getItem('mockMoveError');
    const target=to==='quick-chats'?ensureQuick():read().find(w=>w.id===to);if(!target)throw `No workspace ${to}: not found`;
    const all=JSON.parse(localStorage.getItem('mockWorkspaceSessions')??'[]') as any[];const hit=all.find(s=>s.key===key);if(!hit)throw `no session ${key}`;
    const from=read().find(w=>w.cwd===hit.cwd)?.id??null;
    const moved=hit.cwd!==target.cwd;hit.cwd=target.cwd;if(moved){hit.harnessSessionId=null;hit.resumable='briefOnly';}
    localStorage.setItem('mockWorkspaceSessions',JSON.stringify(all));
    const conversations=JSON.parse(localStorage.getItem('mockWorkspaceConversations')??'{}');if(conversations[key]){conversations[key].summary={...conversations[key].summary,cwd:target.cwd,harnessSessionId:null,resumable:'briefOnly'};localStorage.setItem('mockWorkspaceConversations',JSON.stringify(conversations));}
    localStorage.setItem('mockLastMove',JSON.stringify({key,to}));
    return {value:{key,from,to,cwd:target.cwd,moved,record:moved?`${target.path}/sessions/${key}.md`:null,expiredCards:0,warnings:[]}};
  }
  if(command==='workspace_icons')return {value:read().slice(0,1).map((w:{id:string})=>({workspaceId:w.id,imageUrl:MOCK_APP_ICON,owner:'example-user',avatarUrl:null}))};
  if(command==='create_workspace'){
    const id=`work-${Date.now()}-${read().length}`,path=`workspaces/${id}`;
    const workspace={id,path,title:String(args?.title),status:'active',cwd:`/Users/fixture/vault/${path}`,modified:new Date().toISOString()};
    notes.set(`${path}/WORKSPACE.md`,`---\nhollow_workspace: 1\ntitle: ${JSON.stringify(args?.title)}\nstatus: active\n---\n\n# ${args?.title}\n\n## Outcome\n\n${args?.goal}\n\n## Current context\n\nKeep relevant facts here.\n\n## Next step\n\nStart with the request.\n`);
    notes.set(`${path}/PLAN.md`,'# Plan\n\n- [ ] Produce and check the result\n');notes.set(`${path}/SOURCES.md`,'# Sources\n');
    localStorage.setItem('mockWorkspaces',JSON.stringify([...read(),workspace]));saveNotes();return {value:workspace};
  }
  if(command==='quick_chat_scratch'){const key=String(args?.key);return {value:{path:`/Users/fixture/vault/workspaces/quick-chats/scratch/${key}`,relative:`workspaces/quick-chats/scratch/${key}`,files:Number(localStorage.getItem('mockScratchFiles')??0),bytes:0}};}
  if(command==='adopt_chat_scratch'){if(localStorage.getItem('mockAdoptError'))throw localStorage.getItem('mockAdoptError');const moved=Number(localStorage.getItem('mockScratchFiles')??0);localStorage.setItem('mockLastAdopt',JSON.stringify({key:args?.key,to:args?.workspaceId}));return {value:{moved,kept:0,into:`sources/chat-${String(args?.key)}`}};}
  if(command==='clone_repository'){
    if(localStorage.getItem('mockCloneError'))throw localStorage.getItem('mockCloneError');
    const url=String(args?.url),name=String(args?.name||url.replace(/\/+$/,'').split(/[/:]/).pop()?.replace(/\.git$/,'')||'repo');
    localStorage.setItem('mockLastClone',url);return {value:{name,path:`/Users/fixture/Projects/${name}`}};
  }
  if(command==='list_project_scripts'){return {value:(JSON.parse(localStorage.getItem('mockProjectScripts')??'{}')[String(args?.workspaceId)]??[])};}
  if(command==='save_project_scripts'){
    const all=JSON.parse(localStorage.getItem('mockProjectScripts')??'{}');
    const list=((args?.scripts??[]) as {id?:string;name:string;command:string;chord?:string}[]).map((s,i)=>({id:s.id||`s-mock${Date.now().toString(16)}${i}`,name:s.name.trim(),command:s.command.trim(),chord:s.chord||undefined,serviceName:`script-${s.name.trim().toLowerCase().replace(/[^a-z0-9]+/g,'-').replace(/^-|-$/g,'')}`}));
    if(list.some(s=>!s.name))throw 'A script\'s name is 1 to 40 characters, on one line';
    all[String(args?.workspaceId)]=list;localStorage.setItem('mockProjectScripts',JSON.stringify(all));return {value:list};
  }
  if(command==='run_project_script'){
    const list=(JSON.parse(localStorage.getItem('mockProjectScripts')??'{}')[String(args?.workspaceId)]??[]) as {id:string;serviceName:string;command:string}[];
    const hit=list.find(s=>s.id===String(args?.scriptId));if(!hit)throw 'That script no longer exists';
    localStorage.setItem('mockLastScriptRun',hit.id);
    return {value:{scope:'/Users/fixture/vault',name:hit.serviceName,command:hit.command,cwd:'/Users/fixture/vault',port:null,kind:'other',status:'running',exitCode:null,pid:1,startedAt:new Date().toISOString(),log:''}};

  }
  if(command==='build_repo_candidates'&&localStorage.getItem('mockRepoCandidatesError'))throw new Error(localStorage.getItem('mockRepoCandidatesError')!);
  if(command==='build_repo_candidates')return {value:[{name:'hollow',path:'/Users/fixture/Projects/hollow',parent:'',branch:'main',worktree:false,remote:'github.com/fixture/hollow'},{name:'hollow',path:'/Users/fixture/Projects/work/hollow',parent:'work',branch:'feature/menus',worktree:true,remote:'github.com/fixture/hollow'},{name:'site',path:'/Users/fixture/Projects/site',parent:'',branch:'main',worktree:false,remote:null}]};
  if(command==='create_build_workspace'){
    const input=(args?.input??{}) as {title:string;goal:string;repo:string;branch?:string;base?:string};
    const id=`work-${Date.now()}-${read().length}`,path=`workspaces/${id}`;
    const branch=input.branch||`hollow/${input.title.toLowerCase().replace(/[^a-z0-9]+/g,'-').replace(/^-|-$/g,'')||'work'}`;
    const worktree=`/Users/fixture/Hollow/Worktrees/${input.repo.split('/').pop()}/${branch.split('/').pop()}`;
    const workspace={id,path,title:input.title,status:'active',cwd:worktree,modified:new Date().toISOString(),build:{repo:input.repo,worktree,branch,base:input.base||'origin/main',exists:true}};
    notes.set(`${path}/WORKSPACE.md`,`---\nhollow_workspace: 1\ntitle: ${JSON.stringify(input.title)}\nstatus: active\n---\n\n# ${input.title}\n\n## Outcome\n\n${input.goal}\n`);
    notes.set(`${path}/PLAN.md`,'# Plan\n');notes.set(`${path}/SOURCES.md`,'# Sources\n');
    localStorage.setItem('mockWorkspaces',JSON.stringify([...read(),workspace]));saveNotes();
    return {value:{workspace,warnings:localStorage.getItem('mockBuildFetchFails')?['Could not fetch origin (Connection refused (os error 61)); the branch starts from what this Mac last saw.']:[]}};
  }
  if(command==='remove_build_worktree'){
    const id=String(args?.id);
    if(!args?.force&&localStorage.getItem('mockWorktreeUnsaved'))throw 'WORKTREE_UNSAVED: {"uncommitted":2,"unpushed":1}';
    const list=read().map(w=>w.id===id&&(w as any).build?{...w,cwd:`/Users/fixture/vault/${w.path}`,build:{...(w as any).build,exists:false}}:w);
    localStorage.setItem('mockWorkspaces',JSON.stringify(list));return {value:undefined};
  }
  // The Plan view's row toggle (Canvas C5a P3), with plan_rows.rs's rules: the file's revision, then the row's line identity, then a real row; the same state writes nothing.
  if(command==='toggle_plan_row'){
    const w=detail(String(args?.workspaceId)).workspace,path=`${w.path}/PLAN.md`,text=notes.get(path);
    if(text==null)throw 'This workspace has no PLAN.md';
    if(sha256Hex(text)!==args?.expectedRevision)throw 'PLAN_CHANGED: PLAN.md changed since it was read; nothing was ticked. Read it again.';
    const n=Number(args?.line),at=sourceLines(text)[n-1];
    if(!at||sha256Hex(at.text)!==args?.lineSha256)throw `PLAN_CHANGED: line ${n} of PLAN.md is no longer the row you ticked; nothing was ticked. Read it again.`;
    const row=readPlan(text).sections.flatMap(s=>s.rows).find(r=>r.id.line===n);
    if(!row)throw `line ${n} of PLAN.md is not a checklist row; nothing was ticked`;
    const done=Boolean(args?.done);
    if(row.done===done)return {value:{outcome:'unchanged',revision:String(args?.expectedRevision),line:n,lineSha256:String(args?.lineSha256),done}};
    const raw=text.split('\n');raw[n-1]=raw[n-1].replace(/^(\uFEFF?[ \t]*(?:[-*+]|\d{1,9}[.)])[ \t]+\[)[ xX]\]/,`$1${done?'x':' '}]`);
    const next=raw.join('\n');notes.set(path,next);saveNotes();
    const times=JSON.parse(localStorage.getItem('mockWorkspaceFileTimes')??'{}');times[path]=new Date().toISOString();localStorage.setItem('mockWorkspaceFileTimes',JSON.stringify(times));
    return {value:{outcome:'written',revision:sha256Hex(next),line:n,lineSha256:sha256Hex(sourceLines(next)[n-1].text),done}};
  }
  if(command==='read_work_file'&&localStorage.getItem('mockWorkFileError')===args?.path)throw new Error('File is temporarily unavailable');
  if(command==='read_workspace'&&localStorage.getItem('mockWorkspaceReadError')===args?.id)throw 'Workspace has over 100 source files; split the evidence into smaller workspaces';
  // Results and digests (next-level P02/P05): the same rules as
  // workspace_results.rs over the mock notes. Reviews in mockResultReviews.
  const mockHash=(text:string)=>{let out='';for(let k=0;k<8;k++){let h=(0x811c9dc5^k*0x9e3779b1)>>>0;for(let i=0;i<text.length;i++){h^=text.charCodeAt(i);h=Math.imul(h,16777619)>>>0;}out+=h.toString(16).padStart(8,'0');}return out;};
  const reviews=():Record<string,any>=>JSON.parse(localStorage.getItem('mockResultReviews')??'{}');
  // A saved chat canvas (Canvas C3 · S3): outputs/canvases/<id>.canvas.html is the Result, its .canvas.json sidecar never is.
  const canvasFile=(n:string)=>{const m=/^outputs\/canvases\/([a-z0-9-]{1,64})\.canvas\.(html|json)$/.exec(n);return m?{id:m[1],sidecar:m[2]==='json'}:null;};
  const kindOf=(n:string)=>{const c=canvasFile(n);if(c&&!c.sidecar)return 'canvas';const l=n.toLowerCase();if(l.endsWith('.annotations.json'))return 'review';const e=l.split('.').pop()??'';return ['md','markdown','txt','docx'].includes(e)?'document':['csv','tsv','xlsx','xls'].includes(e)?'table':['png','jpg','jpeg','gif','webp','svg'].includes(e)?'image':['mp4','m4v','mov'].includes(e)?'video':['html','htm'].includes(e)?'page':e==='pdf'?'pdf':['json','yaml','ts','rs','py','js','patch','diff'].includes(e)?'data':'file';};
  const titleOf=(n:string,text:string)=>{if(/\.md$/i.test(n)){const body=text.startsWith('---\n')?text.slice(text.indexOf('\n---',4)+4):text;const h=body.split('\n').map(l=>l.trim()).find(l=>l.startsWith('# '));if(h)return h.replace(/^#+\s*/,'');}if(kindOf(n)==='review'){try{const t=JSON.parse(text).title;if(t)return String(t);}catch{/* not json */}}if(kindOf(n)==='canvas'){const t=/<hc-canvas\b[^>]*\btitle="([^"]*)"/i.exec(text)?.[1];if(t)return t;}const stem=(n.split('/').pop()??n).replace(/\.board\.json$/,'').replace(/\.canvas\.html$/,'').replace(/\.[^.]+$/,'').replace(/[-_]/g,' ').trim();return stem.charAt(0).toUpperCase()+stem.slice(1);};
  const receiptsFor=(vaultPath:string)=>{const convs=JSON.parse(localStorage.getItem('mockWorkspaceConversations')??'{}') as Record<string,any>;const hits:any[]=[];for(const c of Object.values(convs))for(const t of c.turns??[])if(t.receipt?.files?.some((f:any)=>f.path===vaultPath))hits.push(t.receipt);return hits.sort((a,b)=>String(b.finishedAt).localeCompare(String(a.finishedAt)))[0]??null;};
  const sidecarOf=(w:string,id:string)=>{try{const v=JSON.parse(notes.get(`${w}/outputs/canvases/${id}.canvas.json`)??'');return v?.hollowCanvas===1&&v.id===id?{title:String(v.title),revision:Number(v.revision)}:null;}catch{return null;}};
  const resultsOf=(id:string)=>{const d=detail(id);const rv=reviews()[id]??{};return d.files.filter(f=>f.kind==='output'&&!/(^|\/)\./.test(f.name)&&!canvasFile(f.name)?.sidecar).map(f=>{const text=notes.get(f.path)??'';const name=f.name;const sha=mockHash(text);const rec=rv[name];const cf=canvasFile(name);const side=cf?sidecarOf(d.workspace.path,cf.id):null;const review=!rec?'draft':rec.state==='accepted'?(rec.acceptedSha256===sha?'accepted':'changed'):rec.state==='ready'?'ready':'draft';const r=receiptsFor(f.path);return {path:f.path,name,title:rec?.title??side?.title??titleOf(name,text),kind:kindOf(name),size:f.size,modified:f.modified??null,sha256:sha,review,acceptedSha256:rec?.acceptedSha256??null,acceptedAt:rec?.acceptedAt??null,readyAt:rec?.readyAt??null,hasSnapshot:!!(rec?.acceptedSha256&&localStorage.getItem(`mockResultSnap:${rec.acceptedSha256}`)!=null),acceptance:rec?.acceptance??null,history:[...(rec?.history??[])].reverse().slice(0,10),origin:r?{sessionKey:r.sessionKey??null,runId:r.runId,finishedAt:r.finishedAt,status:r.status,verification:r.verificationState}:null,checks:r?{total:r.checks.length,failed:r.checks.filter((c:any)=>c.outcome==='failed').length,unknown:r.checks.filter((c:any)=>!['passed','failed'].includes(c.outcome)).length}:{total:null,failed:null,unknown:null},...(cf?{canvas:{id:cf.id,revision:side?.revision??null}}:{})};}).sort((a,b)=>String(b.modified??'').localeCompare(String(a.modified??''))||a.name.localeCompare(b.name));};
  if(command==='workspace_results')return {value:resultsOf(String(args?.id))};
  if(command==='set_result_review'){
    const id=String(args?.id),path=String(args?.path),state=String(args?.review);const w=detail(id).workspace;const name=path.startsWith(w.path+'/')?path.slice(w.path.length+1):path;
    if(!name.startsWith('outputs/')||name.includes('..')||/(^|\/)\./.test(name))throw `${path} is not a result in workspace ${id}: results live in outputs/`;
    if(canvasFile(name)?.sidecar)throw `${path} is a saved canvas's record, not a result: review the canvas (its .canvas.html)`;
    const text=notes.get(`${w.path}/${name}`);if(text==null)throw `${name} no longer exists`;const sha=mockHash(text);
    if(args?.expectedSha256&&args.expectedSha256!==sha)throw `RESULT_CHANGED: ${name} changed since you opened it; review the new version`;
    if(state==='accepted'&&!args?.expectedSha256)throw 'accepting needs the version you reviewed (expectedSha256)';
    const all=reviews();const mine=all[id]??(all[id]={});const rec=mine[name]??(mine[name]={state:'draft',history:[]});const at=new Date().toISOString();
    rec.state=state;if(state==='accepted'){rec.acceptedSha256=sha;rec.acceptedAt=at;localStorage.setItem(`mockResultSnap:${sha}`,text);const r=receiptsFor(`${w.path}/${name}`);rec.acceptance={actor:'person',sha256:sha,bytes:text.length,runId:r?.runId??null,mode:null,basis:'reviewed this exact version',snapshot:true,at};}if(state==='ready')rec.readyAt=at;
    rec.history.push({state,sha256:sha,at,note:args?.note??null,actor:'person',runId:null});localStorage.setItem('mockResultReviews',JSON.stringify(all));
    return {value:resultsOf(id).find(r=>r.name===name)};
  }
  if(command==='list_all_results'||command==='list_all_results_peek'){const kind=args?.kind as string|null;return {value:read().filter(w=>w.kind!=='quick').flatMap(w=>resultsOf(w.id).filter(r=>!kind||r.kind===kind).map(r=>({...r,workspaceId:w.id,workspaceTitle:w.title}))).sort((a,b)=>String(b.modified??'').localeCompare(String(a.modified??''))).slice(0,Number(args?.limit??500))};}
  if(command==='result_snapshot')return {value:localStorage.getItem(`mockResultSnap:${String(args?.sha256)}`)};
  // The "needs you" query, exact-key lookup and conversation search (B·F15), paged like needs_you.rs (offsets stand in for its keyset cursors). The fixture's waiting items are its results to review.
  if(command==='list_needs_you'){const kinds=args?.kinds as string[]|null|undefined;const all=read().flatMap(w=>{try{return resultsOf(w.id).filter((r:any)=>r.review==='ready'||r.review==='changed').map((r:any)=>({id:`result:${r.path}`,kind:'result',sourceId:r.path,title:r.title,detail:w.title,state:r.review,at:r.readyAt??r.modified??null,workspaceId:w.id,sessionKey:null,path:r.path,category:r.kind,link:null}));}catch{return [];}}).filter(i=>!kinds||kinds.includes(i.kind)).sort((a,b)=>String(b.at??'').localeCompare(String(a.at??''))||a.id.localeCompare(b.id));const start=Math.min(Number(args?.cursor??0),all.length),limit=Math.min(200,Math.max(1,Number(args?.limit??50)));const items=all.slice(start,start+limit),end=start+items.length;return {value:{items,total:all.length,nextCursor:end<all.length?String(end):null,unavailable:[]}};}
  // As session_store.rs since 2026-11-04: each word in the title or in one of the person's messages (the fixture conversations' user turns, not Hollow's own "⟲ Hollow" ones; the fixture doesn't say which an agent sent), and a hit whose title lacks a word carries the first message holding the first such word, cut as search_snippet cuts it. JavaScript's toLowerCase folds every letter, where the backend's SQL folds ASCII letters only (see SearchWord).
  if(command==='search_sessions'){const words=[...new Set(String(args?.query??'').toLowerCase().split(/\s+/).filter(Boolean))].slice(0,32);const convs=JSON.parse(localStorage.getItem('mockWorkspaceConversations')??'{}') as Record<string,any>;const asked=(key:string):string[]=>((convs[key]?.turns??[]) as any[]).filter(t=>t?.role==='user'&&typeof t.text==='string'&&!t.text.trimStart().startsWith('⟲ Hollow')).map(t=>t.text);const snip=(text:string,word:string)=>{const cs=Array.from(text.replace(/\s+/g,' ').trim()),low=cs.map(c=>c.toLowerCase()),n=cs.length;let pos=0;const starts=low.map(l=>{const p=pos;pos+=l.length;return p;}),k=low.join('').indexOf(word),at=k<0?0:Math.max(0,starts.findIndex(p=>p>=k));let start=Math.max(0,Math.min(at-40,n-140)),end=Math.min(n,start+140);if(start>0&&cs[start-1]!==' '){const sp=cs.slice(start,Math.min(start+16,at)).indexOf(' ');if(sp>=0)start+=sp+1;}if(end<n&&cs[end]!==' '){const from=Math.max(end-16,Math.min(at+Array.from(word).length,end)),sp=cs.slice(from,end).lastIndexOf(' ');if(sp>=0)end=from+sp;}return `${start>0?'…':''}${cs.slice(start,end).join('')}${end<n?'…':''}`;};const all=(JSON.parse(localStorage.getItem('mockWorkspaceSessions')??'[]') as any[]).filter(s=>!s.archived&&words.length).flatMap(s=>{const title=String(s.title??'').toLowerCase(),msgs=asked(s.key),holds=(w:string)=>msgs.find(m=>m.toLowerCase().includes(w));if(!words.every(w=>title.includes(w)||holds(w)!=null))return [];const miss=words.find(w=>!title.includes(w));return [{workspaceId:read().find(w=>w.cwd===s.cwd)?.id??null,session:s,...(miss?{snippet:snip(holds(miss)!,miss)}:{})}];}).filter(t=>t.workspaceId).sort((a,b)=>String(b.session.lastActivityAt??'').localeCompare(String(a.session.lastActivityAt??''))||a.session.key.localeCompare(b.session.key));const start=Math.min(Number(args?.cursor??0),all.length),limit=Math.min(100,Math.max(1,Number(args?.limit??20)));const items=all.slice(start,start+limit),end=start+items.length;return {value:{items,nextCursor:end<all.length?String(end):null}};}
  if(command==='session_target'){const all=JSON.parse(localStorage.getItem('mockWorkspaceSessions')??'[]') as any[];const s=all.find(x=>x.key===args?.key);return {value:s?{workspaceId:read().find(w=>w.cwd===s.cwd)?.id??null,session:s}:null};}
  if(command==='keep_recording_transcript'){
    const id=String(args?.id),w=detail(String(args?.workspaceId)).workspace;const rel=`sources/recording-${id.slice(0,8)}.md`;const path=`${w.path}/${rel}`;
    if(notes.has(path))throw `${rel} already exists; nothing was overwritten`;
    notes.set(path,`---\nsource: plaud\nrecording_id: ${id}\nretention: kept by the user (opt-in)\n---\n\n# Recording ${id}\n`);saveNotes();
    const links=JSON.parse(localStorage.getItem('mockRecordingLinks')??'{}');links[id]={...(links[id]??{workspaceId:w.id,linkedAt:new Date().toISOString()}),keptCopy:path};localStorage.setItem('mockRecordingLinks',JSON.stringify(links));
    return {value:path};
  }
  if(command==='read_workspace')return {value:detail(String(args?.id))};
  const mockSessions=()=>(JSON.parse(localStorage.getItem('mockWorkspaceSessions')??'[]') as any[]).filter(s=>!s.archived);
  // `mockSessionTotals` fakes older conversations past the rail's list (P1-42).
  // The sidebar's Show older (overnight 11-04 S4b): a project's conversations past (afterAt, afterKey), newest first.
  // localStorage mockRailListed = N lists each workspace's N newest, as the
  // store lists its 25 newest, so Show older has rows to load (overnight 11-04 S4b);
  // the pinned keys (pins and the open chat) are listed past them, as there.
  if(command==='workspace_sessions'){const changes=JSON.parse(localStorage.getItem('mockSessionChanges')??'{}') as Record<string,[number,number]>;const totals=JSON.parse(localStorage.getItem('mockSessionTotals')??'{}') as Record<string,number>;const cap=Number(localStorage.getItem('mockRailListed')??0);const pinned=new Set((args?.pinned as string[]|null|undefined)??[]);return {value:read().map(w=>{const all=mockSessions().filter(s=>s.cwd===w.cwd);const sessions=cap>0?all.slice().sort((a,b)=>String(b.lastActivityAt).localeCompare(String(a.lastActivityAt))||String(a.key).localeCompare(String(b.key))).filter((s,i)=>i<cap||pinned.has(s.key)):all;return {projectAgentKey:isBuiltinWorkspace(w)?undefined:`s-project-${sha256Hex(w.id).slice(0,48)}`,workspaceId:w.id,total:Math.max(all.length,totals[w.id]??0),sessions,changes:sessions.filter(s=>changes[s.key]).map(s=>({key:s.key,additions:changes[s.key][0],deletions:changes[s.key][1]}))};})};}
  // Rail git glyphs: localStorage mockRailPr = {workspaceId: 'open'|'merged'|'closed'}.
  if(command==='build_rail_states'){const pr=JSON.parse(localStorage.getItem('mockRailPr')??'{}') as Record<string,string>;const ids=(args?.ids as string[])??[];return {value:read().filter(w=>ids.includes(w.id)&&(w as any).build?.exists).map(w=>({workspaceId:w.id,branch:(w as any).build.branch,ahead:1,prState:pr[w.id]??null,prNumber:pr[w.id]?12:null}))};}
  if(command==='update_workspace'){
    const id=String(args?.id);
    if(String(args?.status??'').toLowerCase()==='archived'&&(args?.pinned===true||args?.pinPosition!=null))throw "Archived workspaces can't be pinned: archive or pin, not both.";
    // As update_meta: archiving records the status it had (archivedFrom, P1-38); any other status drops it.
    const isArch=(s:unknown)=>String(s??'').trim().toLowerCase()==='archived';
    const was=(w:WorkWorkspace)=>args?.status==null?{}:isArch(args.status)?(isArch(w.status)?{}:{archivedFrom:w.status}):{archivedFrom:undefined};
    let list=read().map(w=>w.id===id?{...w,...(args?.title!=null?{title:String(args.title)}:{}),...was(w),...(args?.status!=null?{status:String(args.status)}:{})}:w);
    // Pins as workspace_pins.rs: archiving unpins; archived and built-in workspaces can't be pinned.
    if(String(args?.status??'').toLowerCase()==='archived')list=list.map(w=>w.id===id?{...w,pinned:undefined}:w);
    const pinned=args?.pinned as boolean|null|undefined,pos=args?.pinPosition as number|null|undefined;
    if(pinned!=null||pos!=null){
      const t=list.find(w=>w.id===id);if(!t)throw `No workspace ${id}`;
      if(pinned===false&&pos!=null)throw 'pinPosition places a pin; leave it out to unpin';
      if(pos!=null&&Number(pos)<1)throw 'pinPosition counts from 1 (the top)';
      if(t.kind)throw `${t.title} is built in and always has its own place; it can't be pinned.`;
      const order=list.filter(w=>w.pinned!=null&&w.status.toLowerCase()!=='archived').sort((a,b)=>a.pinned!-b.pinned!||(a.id<b.id?-1:1)).map(w=>w.id);
      if(pinned===false)list=list.map(w=>w.id===id?{...w,pinned:undefined}:w);
      else{
        if(t.status.toLowerCase()==='archived')throw `${t.title} is archived, and archived workspaces can't be pinned. Unarchive it first.`;
        const ids=order.filter(x=>x!==id);const slot=pos!=null?Math.min(Math.max(1,Number(pos))-1,ids.length):order.includes(id)?order.indexOf(id):ids.length;ids.splice(slot,0,id);
        list=list.map(w=>ids.includes(w.id)?{...w,pinned:ids.indexOf(w.id)+1}:w);
      }
    }
    localStorage.setItem('mockWorkspaces',JSON.stringify(list));return {value:list.find(w=>w.id===id)};
  }
  if(command==='set_workspace_canvas_theme'){
    const id=String(args?.id), name=args?.theme as string|null;
    if(name && !JSON.parse(localStorage.getItem('mockCustomThemes')??'[]').some((t:{name:string})=>t.name===name)) throw new Error("That custom theme isn't on this Mac");
    const list=read().map(w=>w.id===id?{...w,canvasTheme:name??undefined}:w);
    localStorage.setItem('mockWorkspaces',JSON.stringify(list));return {value:list.find(w=>w.id===id)};
  }
  // As workspaces.rs trash / trashed / restore (P1-41): the trash keeps each deleted workspace, newest first.
  const trashRead=():{workspace:WorkWorkspace;trashedAt:string}[]=>JSON.parse(localStorage.getItem('mockTrashedWorkspaces')??'[]');
  const trashSave=(rows:{workspace:WorkWorkspace;trashedAt:string}[])=>localStorage.setItem('mockTrashedWorkspaces',JSON.stringify(rows));
  if(command==='trash_workspace'){const id=String(args?.id),all=read(),hit=all.find(w=>w.id===id);localStorage.setItem('mockWorkspaces',JSON.stringify(all.filter(w=>w.id!==id)));if(hit)trashSave([{workspace:hit,trashedAt:new Date().toISOString().replace(/\.\d+Z$/,'Z')},...trashRead()]);return {value:undefined};}
  if(command==='trashed_workspaces'){const ids=new Set(read().map(w=>w.id));return {value:trashRead().map(t=>({id:t.workspace.id,title:t.workspace.title,trashedAt:t.trashedAt,...(ids.has(t.workspace.id)?{blocked:`Another workspace uses its folder (workspaces/${t.workspace.id}): delete or move that one first.`}:{})}))};}
  if(command==='restore_workspace'){
    const id=String(args?.id),rows=trashRead(),at=rows.findIndex(t=>t.workspace.id===id);
    if(at<0)throw 'There is nothing in the trash to restore for this workspace.';
    if(read().some(w=>w.id===id))throw `Another workspace uses this one's folder (workspaces/${id}), so the deleted one stays in the trash. Delete or move that workspace first.`;
    const back=rows[at].workspace;localStorage.setItem('mockWorkspaces',JSON.stringify([...read(),back]));trashSave(rows.filter((_,i)=>i!==at));return {value:back};
  }
  if(command==='rename_agent_session'||command==='archive_agent_session'){
    if(command==='archive_agent_session' && args?.archived && JSON.parse(localStorage.getItem('mockConversationOrganization')??'{}')[String(args?.key)]?.pinned) throw 'Unpin this conversation first';
    const all=JSON.parse(localStorage.getItem('mockWorkspaceSessions')??'[]') as any[];const hit=all.find(s=>s.key===args?.key);
    if(hit){if(command==='rename_agent_session')hit.title=String(args?.title);else hit.archived=Boolean(args?.archived);localStorage.setItem('mockWorkspaceSessions',JSON.stringify(all));if(command==='rename_agent_session')return {value:undefined};}
  }
  if(command==='attach_workspace_source'){
    const w=detail(String(args?.id)).workspace,source=String(args?.source),name=source.split('/').pop()!;
    const path=`${w.path}/sources/${name}`;if(notes.has(path))throw Error('Source already attached');
    // As workspaces.rs attach_source: the snapshot, a SOURCES.md entry and the provenance sidecar.
    const text=notes.get(source)??'# Existing project\n\nRetained source.',original=`/Users/fixture/vault/${source}`,captured=new Date().toISOString(),sha=sha256Hex(text);
    const entry=`\n## ${name.replace(/[[\]]/g,' ')}\n\n- Snapshot: [${name.replace(/[[\]]/g,' ')}](sources/${encodeURIComponent(name)})\n- Original: ${JSON.stringify(original)}\n- Captured: ${captured}\n- SHA-256: \`${sha}\`\n`;
    notes.set(path,text);notes.set(`${w.path}/SOURCES.md`,`${notes.get(`${w.path}/SOURCES.md`)??''}${entry}`);
    notes.set(`${w.path}/sources/.provenance/${name}.json`,JSON.stringify({original,captured,sha256:sha,size:new TextEncoder().encode(text).length,modified:null}));saveNotes();return {value:detail(w.id)};
  }
  // A reviewed snapshot refresh (plan L3 X5), as workspaces.rs preview_refresh / refresh_source.
  // The original's text: localStorage.mockSourceOriginals[original], else the vault note it names
  // (`…/vault/<path>`); neither means unavailable. mockSourceRefreshConflict='true' makes the next
  // apply refuse with SOURCE_CHANGED once, as if the original moved on after the preview.
  if(command==='preview_source_refresh'||command==='apply_source_refresh'){
    const w=detail(String(args?.id)).workspace,rel=String(args?.path),name=rel.replace(/^sources\//,'');
    if(!rel.startsWith('sources/')||!name||name.startsWith('.')||name.includes('/'))throw 'Only a snapshot in this workspace\'s sources/ folder can be refreshed';
    let origin:{original:string;captured:string;sha256:string;size:number;modified:string|null;previous?:unknown[]}|null=null;
    try{origin=JSON.parse(notes.get(`${w.path}/sources/.provenance/${name}.json`)??'null');}catch{origin=null;}
    if(!origin)throw 'Nothing records where this snapshot came from, so there is no original to compare it with';
    const before=notes.get(`${w.path}/${rel}`);if(before==null)throw 'The snapshot can\'t be read: not found';
    const originals=JSON.parse(localStorage.getItem('mockSourceOriginals')??'{}') as Record<string,string>;
    const inVault=/\/vault\/(.+)$/.exec(origin.original)?.[1];
    const after=originals[origin.original]??(inVault?notes.get(inVault):undefined);
    const size=(t:string)=>new TextEncoder().encode(t).length;
    const mine={sha256:sha256Hex(before),size:size(before),modified:null},theirs=after==null?null:{sha256:sha256Hex(after),size:size(after),modified:null};
    if(command==='preview_source_refresh')return {value:{path:rel,original:origin.original,available:!!theirs,recorded:{sha256:origin.sha256,size:origin.size,modified:origin.modified,captured:origin.captured},snapshot:mine,origin:theirs,contentChanged:!!theirs&&theirs.sha256!==mine.sha256,snapshotEdited:mine.sha256!==origin.sha256,text:theirs&&after!=null?{before,after}:null}};
    if(!theirs||after==null)throw 'The original can\'t be found or read, so the snapshot can\'t be refreshed. It is unchanged.';
    if(localStorage.getItem('mockSourceRefreshConflict')==='true'){localStorage.removeItem('mockSourceRefreshConflict');throw 'SOURCE_CHANGED: The original or the snapshot changed after you checked. Check again.';}
    if(theirs.sha256!==args?.expectedOriginSha||mine.sha256!==args?.expectedSnapshotSha)throw 'SOURCE_CHANGED: The original or the snapshot changed after you checked. Check again.';
    if(theirs.sha256===mine.sha256)throw 'The snapshot already has the same content as its original; nothing to refresh.';
    const captured=new Date().toISOString(),kept=`sources/.previous/${mine.sha256.slice(0,12)}-${name}`,shown=name.replace(/[[\]]/g,' ');
    const entry=`\n## ${shown}\n\n- Snapshot: [${shown}](sources/${encodeURIComponent(name)})\n- Original: ${JSON.stringify(origin.original)}\n- Captured: ${captured}\n- SHA-256: \`${theirs.sha256}\`\n- Refreshed: replaced the snapshot captured ${origin.captured} (SHA-256 \`${mine.sha256}\`); the earlier copy is kept as ${kept}\n`;
    notes.set(`${w.path}/${kept}`,before);notes.set(`${w.path}/${rel}`,after);
    notes.set(`${w.path}/sources/.provenance/${name}.json`,JSON.stringify({original:origin.original,captured,sha256:theirs.sha256,size:theirs.size,modified:null,previous:[...(origin.previous??[]),{sha256:mine.sha256,captured:origin.captured,kept}]}));
    notes.set(`${w.path}/SOURCES.md`,`${notes.get(`${w.path}/SOURCES.md`)??''}${entry}`);saveNotes();
    const fresh=JSON.parse(localStorage.getItem('mockSourceFreshness')??'{}');delete fresh[rel];localStorage.setItem('mockSourceFreshness',JSON.stringify(fresh));
    return {value:detail(w.id)};
  }
  if(['vault_create_note','vault_save_note_cas'].includes(command)&&String(args?.path).startsWith('workspaces/')){
    const path=String(args?.path),value=String(args?.content??''),current=notes.get(path)??'';
    if(command==='vault_create_note'&&notes.has(path))throw Error('Note already exists');
    const conflict=command==='vault_save_note_cas'&&(current!==args?.baseline||localStorage.getItem('mockWorkspaceConflict')==='true');
    if(!conflict){notes.set(path,value);saveNotes();}
    const note={path,content:conflict?current:value,mtime:new Date().toISOString(),size:value.length};
    return {value:command==='vault_save_note_cas'?{conflict,note}:note};
  }
  return null;
}

/** A tiny app icon so browser checks can see artwork without the network. */
const MOCK_APP_ICON = "data:image/svg+xml;base64," + btoa("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'><rect width='16' height='16' rx='4' fill='#8b7cf6'/><path d='M4 12V6a4 4 0 0 1 8 0v6' stroke='white' stroke-width='2' fill='none'/></svg>");

/** A path with a dot-named segment: Rust's walks skip those (`.provenance/`, `.revisions/`). */
const hiddenPath=(p:string)=>p.split('/').some(x=>x.startsWith('.'));
const READABLE=/\.(md|markdown|txt|csv|tsv|json|yaml|yml|html|htm|rs|ts|tsx|js|css|py|sql)$/i;
/** `WorkspaceDetail.sources` as workspaces.rs `source_context` lists it: the
 *  first 100 files under sources/ in path order, provenance from
 *  `sources/.provenance/<name>.json`. Text files are `complete` when they fit
 *  the 24 KB per turn together, else `excerpt` (each request gets the passages
 *  that best match it; agent effectiveness F3), with `includedBytes` the most
 *  one file can carry. The mock can't stat an original, so a tracked source
 *  reads `unchanged_metadata` unless `localStorage.mockSourceFreshness[path]`
 *  says otherwise. */
function mockSources(root:string,notes:Map<string,string>){
  const fresh=JSON.parse(localStorage.getItem('mockSourceFreshness')??'{}') as Record<string,string>;
  const listed=[...notes].filter(([p])=>p.startsWith(root+'/sources/')&&!hiddenPath(p.slice(root.length+1))).sort(([a],[b])=>a<b?-1:a>b?1:0).slice(0,100);
  const size=(text:string)=>new TextEncoder().encode(text).length;
  const textTotal=listed.filter(([p])=>READABLE.test(p)).reduce((n,[,text])=>n+size(text),0);
  return listed.map(([p,text])=>{
    const path=p.slice(root.length+1),bytes=size(text);
    let origin:{original?:string;captured?:string}|null=null;
    try{origin=JSON.parse(notes.get(`${root}/sources/.provenance/${p.split('/').pop()}.json`)??'null');}catch{origin=null;}
    const readable=READABLE.test(p);
    const coverage=!readable?'requires_reader':textTotal<=24_000?'complete':'excerpt';
    const included=!readable?0:Math.min(bytes,24_000);
    return {path,original:origin?.original??null,captured:origin?.captured??null,freshness:origin?(fresh[path]??'unchanged_metadata'):'unknown',coverage,includedBytes:included,totalBytes:bytes};
  });
}
