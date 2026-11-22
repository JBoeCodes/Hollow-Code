import { For, Show, batch, createEffect, createMemo, createSignal, on, onCleanup, lazy, Suspense, type JSX } from 'solid-js';
import { ActionMenu, type ActionMenuItem } from './ActionMenu';
import { api } from '../api';
import { latestRequest } from '../latestRequest';
import { motionReduced } from '../motion';
import { NoteRenderer } from './NoteRenderer';
import { HtmlDoc } from './HtmlDoc';
import { fileMode } from '../canvas/canvasGrants';
import { flashFooter, showPrompt } from '../store';
import { draftInWorkspace, draftInCurrentConversation } from '../store/workspaceDraft';
import { workspaceDetail } from '../store/workspaces';
import type { InlineAiHost } from '../editor/inlineAi';
import { localDateKey } from '../utils';
import { Skeleton, StatePanel, Button, ErrorText } from './ui';
import { ConflictResolver } from './ConflictResolver';
import { TablePreview } from './TablePreview';
import { SelectionActions } from './SelectionActions';
import { clearDraft, holdDraftOpen, onDraftDiscarded, readDraft, writeDraft } from '../fileDrafts';
import { afterReread, rereadGate, saveSnapshot } from '../fileSave';
import { markdownOutline } from '../docOutline';
import { openModal as openTaskSheet } from '../modals/TaskModal';
import { relativeWhen } from '../homeReturn';
import '../styles/documents.css';
import { Icon } from '../icons';
import { vaultChange } from '../store/vaultChanges';
import { vaultChangeTouches } from '../sectionSource';
import { readOnlyWord } from '../sourceAuthority';
const MarkdownEditor = lazy(() => import('../editor/MarkdownEditor').then(m => ({ default: m.MarkdownEditor })));
/** Inline AI for the editor (overnight 10-11 B5): one tool-free model call
 *  per request (`inline_ai`), the app's own one-line prompt. Dismiss stops
 *  a pending call (`inline_ai_cancel`, overnight S3). */
const INLINE_AI: InlineAiHost = {
  run: (request, requestId) => api.inlineAi({ ...request, requestId }).then(r => r.text),
  cancel: requestId => { api.inlineAiCancel(requestId).catch(() => { /* it already ended */ }); },
  ask: async (title, label, placeholder) => showPrompt({ title, confirmLabel: 'Go', input: { value: '', label, placeholder } }),
  today: () => localDateKey(),
};
type Draft = { text: string; baseline: string };
/** Per-instance part of the inline AI carry key: two editors on one file don't share a suggestion. */
let editorSeq = 0;
/** In-memory drafts by path (fast path); `fileDrafts` persists them across
 *  restarts (next-level P08). */
const drafts = new Map<string, Draft>();
// Review's Discard (P1-14) drops this copy too, so reopening the file
// doesn't bring the draft back.
onDraftDiscarded(path => drafts.delete(path));

/** Whether `path` has an unsaved draft here (this window's memory or the
 *  persisted copy). The Brief view opens its editor rather than the canvas
 *  when it does, so the draft is what the person sees. */
export function hasUnsavedDraft(path: string): boolean {
  const d = drafts.get(path) ?? readDraft(path);
  return !!d && d.text !== d.baseline;
}

/**
 * A workspace file: read, preview, edit and save (next-level P08).
 *
 * Saving is compare-and-swap against the version the draft started from. It
 * writes a snapshot; anything typed while it is in flight stays an unsaved
 * draft against the version just written and is saved next (`fileSave`).
 * Unsaved drafts persist across an app restart and come back with a
 * "restored draft" notice that can be dismissed. A conflict (the file changed
 * on disk) opens the three-way ConflictResolver instead of asking the user to
 * copy text by hand. Markdown previews offer an outline and selection actions
 * (make a task, ask about it); CSV/TSV render as a bounded table whose totals
 * cover every row.
 *
 * An edit made outside Hollow re-reads on `vault-changed` (Canvas C5a P2):
 * the view shows the new text without "Reload file" unless there is an
 * unsaved draft, which is never replaced; its save then meets the usual
 * compare-and-swap conflict. `startEditing` opens a Markdown file in the
 * editor rather than the preview (the Brief view's Edit).
 */
export function WorkFileEditor(props: { path: string; readOnly?: boolean; onSaved?: () => void; title?: JSX.Element; onAsk?: () => void; toolbar?: JSX.Element; outline?: boolean; onLoaded?: (text: string) => void; startEditing?: boolean;
  /** A containing section can keep canvas actions away from unsaved edits. */
  onEditorState?: (editing: boolean, dirty: boolean) => void;
  /** More File actions from the host, above Show file in Finder (Library's Rename…, Move to folder…, Move to trash; P1-18). */
  actions?: ActionMenuItem[];
  /** Library owns its current conversation even when reading a workspace file. */
  askInCurrentConversation?: boolean;
  /** Say "Reviewing" rather than "Viewing" when not editing (a Result under review; plan L3 X2). */
  reviewing?: boolean;
  /** A `[[wikilink]]` clicked in preview or edit (P0-26); without it links do nothing. */
  onWikilinkClick?: (name: string) => void }) {
  const aiKey=`${++editorSeq}|`;
  const [text,setText]=createSignal(''); const [baseline,setBaseline]=createSignal('');
  const [disk,setDisk]=createSignal('');
  const [error,setError]=createSignal(''); const [loading,setLoading]=createSignal(false);
  const [loadError,setLoadError]=createSignal(''); const [retry,setRetry]=createSignal(0);
  const [loadedPath,setLoadedPath]=createSignal<string|null>(null);
  const [saving,setSaving]=createSignal(false); const [editing,setEditing]=createSignal(false);
  const [restoredAt,setRestoredAt]=createSignal<string|null>(null);
  const [conflict,setConflict]=createSignal<{ base: string; disk: string; draft: string }|null>(null);
  /** The conflict's identity, for keying the resolver: a new base, disk or
   *  draft remounts it so its merge is rebuilt (review finding A·D2). */
  /** A conflict whose file on disk can't be read (concurrency PR 2 delta
   *  review 3): the draft is kept against `base`, and nothing is offered to
   *  compare, adopt or merge until a reread succeeds. */
  const [unreadable,setUnreadable]=createSignal<{ reason: string; base: string; draft: string }|null>(null);
  /** One reread at a time; a response for an unreadable state that is no
   *  longer the current one is dropped (delta review 4). */
  const rereads=rereadGate<{ reason: string; base: string; draft: string }>();
  const [rereading,setRereading]=createSignal(false);
  const conflictKey=createMemo(()=>conflict(),null,{equals:(a,b)=>a===b||(!!a&&!!b&&a.base===b.base&&a.disk===b.disk&&a.draft===b.draft)});
  const [savedFlash,setSavedFlash]=createSignal(false);
  const [version,setVersion]=createSignal(0); const loads=latestRequest(); let flashTimer: ReturnType<typeof setTimeout>|undefined;
  const dirty=()=>text()!==baseline();
  createEffect(() => props.onEditorState?.(editing(), dirty()));
  onCleanup(() => props.onEditorState?.(false, false));
  const markdown=()=>/\.(md|markdown)$/i.test(props.path);
  const html=()=>/\.html?$/i.test(props.path);
  const table=()=>/\.(csv|tsv)$/i.test(props.path);
  const readable=()=>/\.(md|markdown|txt|csv|tsv|json|html?|yaml|yml|rs|ts|tsx|js|css|py|sql)$/i.test(props.path);
  const outline=createMemo(()=>props.outline&&markdown()&&!editing()?markdownOutline(text()):[]);
  let previewEl: HTMLDivElement | undefined;
  // Edit asked for before the file was read: honoured once, on the first load.
  let wantEditing = !!props.startEditing;
  // Review never lists the draft of a file open here (P1-14, D3).
  createEffect(() => onCleanup(holdDraftOpen(props.path)));
  createEffect(() => {
    retry();
    const path=props.path, token=loads.start();
    // One batch: the editor unmounts before its document is emptied, so an
    // inline AI suggestion is carried or cancelled, never lost (S3 review).
    batch(()=>{setError('');setLoadError('');setLoadedPath(null);setText('');setBaseline('');setDisk('');setLoading(true);setEditing(false);setRestoredAt(null);setConflict(null);setUnreadable(null);});
    if (!readable()) {setText('');setBaseline('');setLoading(false);return;}
    api.readWorkFile(path).then(note=>{
      if(!loads.isLatest(token))return;
      const stored=readDraft(path);
      const draft=drafts.get(path) ?? (stored ? { text: stored.text, baseline: stored.baseline } : undefined);
      setDisk(note.content);
      if (draft && draft.text !== draft.baseline && draft.baseline !== note.content) {
        // The file changed on disk while a draft was waiting (another app, an
        // agent, or a sync): don't silently rebase it. Ask, three ways.
        setText(draft.text); setBaseline(draft.baseline);
        setConflict({ base: draft.baseline, disk: note.content, draft: draft.text });
      } else {
        setText(draft?.text ?? note.content); setBaseline(draft?.baseline ?? note.content);
        if (draft && draft.text !== draft.baseline) { setEditing(true); setRestoredAt(stored?.savedAt ?? ''); }
        else if (wantEditing && markdown() && !props.readOnly) setEditing(true);
      }
      wantEditing = false;
      setVersion(v=>v+1);
      setLoadedPath(path);
      props.onLoaded?.(note.content);
    }).catch(e=>{if(loads.isLatest(token))setLoadError(String(e));}).finally(()=>{if(loads.isLatest(token))setLoading(false);});
  });
  // The file changed on disk (another app, an agent, a sync): re-read it in
  // place, without the loading state. A draft is never replaced; only the
  // disk copy behind it moves on, so "Discard my draft" shows the new file.
  createEffect(on(vaultChange, change => {
    const path = props.path;
    if (!change || !vaultChangeTouches(change, path) || !readable() || loading() || loadedPath() !== path) return;
    const token = loads.peek();
    api.readWorkFile(path).then(note => {
      if (!loads.isLatest(token) || loadedPath() !== path) return;
      setDisk(note.content);
      if (dirty() || saving() || conflict() || unreadable() || note.content === baseline()) return;
      // One batch: the editor remounts with its old state intact, so an
      // inline AI suggestion is carried, not replaced away (S3 review).
      batch(() => { setText(note.content); setBaseline(note.content); setVersion(v=>v+1); });
      props.onLoaded?.(note.content);
    }).catch(() => { /* moved or removed: keep what is shown; Reload file says why */ });
  }, { defer: true }));
  onCleanup(()=>{loads.cancel(); if(flashTimer) clearTimeout(flashTimer);});
  function change(value: string) {
    setText(value);const draft={text:value,baseline:baseline()};drafts.set(props.path,draft);
    if(!writeDraft(props.path,value,baseline())) setError('Your draft is only in memory: local draft storage is full. Save this file before closing Hollow.');
  }
  function forgetDraft(path: string) { drafts.delete(path); clearDraft(path); }
  function discardDraft() {
    forgetDraft(props.path); setText(disk()); setBaseline(disk()); setRestoredAt(null); setEditing(false); setVersion(v=>v+1);
    flashFooter('Draft discarded. The file is unchanged.');
  }
  /** Save `value` against `base`. `keepTyped` (a plain save) keeps text typed
   *  while the save is in flight; a conflict choice saves exactly `value`. */
  async function write(path: string, value: string, base: string, token: number, keepTyped: boolean): Promise<boolean> {
    const outcome=await saveSnapshot(path,value,base,{
      cas: api.vaultSaveNoteCas,
      latest: ()=>loads.isLatest(token)?text():drafts.get(path)?.text,
      keep: (t,b)=>{ drafts.set(path,{text:t,baseline:b}); if(!writeDraft(path,t,b)&&loads.isLatest(token)) setError('Your draft is only in memory: local draft storage is full. Save this file before closing Hollow.'); },
      forget: ()=>forgetDraft(path),
    },{keepTyped});
    if(outcome.kind==='conflict'){
      if(loads.isLatest(token)){ setConflict({ base, disk: outcome.disk, draft: outcome.draft }); setDisk(outcome.disk); }
      return false;
    }
    if(outcome.kind==='unreadable'){
      // No disk text to show: keep the draft (saveSnapshot stored it) and
      // wait for a reread instead of offering an empty "file on disk".
      if(loads.isLatest(token)){ batch(()=>{ setConflict(null); setText(outcome.draft); setBaseline(base); setUnreadable({ reason: outcome.reason, base, draft: outcome.draft }); }); }
      return false;
    }
    if(loads.isLatest(token)){
      setBaseline(outcome.baseline); setText(outcome.text); setDisk(outcome.baseline); setRestoredAt(null); setConflict(null);
      setSavedFlash(true); if(flashTimer) clearTimeout(flashTimer); flashTimer=setTimeout(()=>setSavedFlash(false),1800);
      props.onSaved?.();
    }
    return true;
  }
  async function save() {
    if(saving()||props.readOnly||!dirty()||loading()||loadedPath()!==props.path||conflict()||unreadable())return;
    const path=props.path,value=text(),token=loads.peek();setSaving(true);setError('');
    try { await write(path,value,baseline(),token,true); }
    catch(e){if(loads.isLatest(token))setError(String(e));else flashFooter(`× Save failed: ${String(e)}`,5000);}
    finally{setSaving(false);}
  }
  /** A conflict choice: save `value` against the disk version on screen.
   *  `against` is the disk version the choice was made from (a merge passes
   *  the one it was built against); if the file has moved on since, refuse. */
  async function resolve(value: string, against?: string) {
    const c=conflict(); if(!c||saving())return;
    if(against!==undefined&&against!==c.disk){ setError('The file changed again on disk after this merge was built. Review the new comparison before saving.'); return; }
    const path=props.path,token=loads.peek();setSaving(true);setError('');
    try {
      if(await write(path,value,c.disk,token,false)) { setEditing(false); setVersion(v=>v+1); flashFooter('Saved. The earlier version is in History & recovery.'); }
      else if(loads.isLatest(token)) flashFooter('The file changed again on disk. Nothing was saved; review the new comparison.',5000);
    }
    catch(e){if(loads.isLatest(token))setError(String(e));}
    finally{setSaving(false);}
  }
  /** After an unreadable conflict: read the file again. Only a successful
   *  read leads on, to the usual comparison (or straight back to the draft
   *  when the file is what it was based on). */
  async function reread() {
    const u=unreadable(); if(!u)return;
    const ticket=rereads.start(u); if(!ticket)return;
    setRereading(true);
    const path=props.path,token=loads.peek();
    const next=await api.readWorkFile(path)
      .then(note=>afterReread({ ok: true, content: note.content }, u.base, u.draft))
      .catch(e=>afterReread({ ok: false, error: String(e) }, u.base, u.draft));
    const current=rereads.finish(ticket,unreadable());
    setRereading(false);
    if(!current||!loads.isLatest(token)||loadedPath()!==path)return;
    if(next.kind==='unreadable'){ setUnreadable({ ...u, reason: next.reason }); flashFooter('× Still can’t read the file on disk. Your draft is kept.',5000); return; }
    if(next.kind==='conflict'){ batch(()=>{ setUnreadable(null); setDisk(next.disk); setConflict({ base: next.base, disk: next.disk, draft: next.draft }); }); return; }
    batch(()=>{ setUnreadable(null); setDisk(next.baseline); setBaseline(next.baseline); setText(next.text); setEditing(markdown()); setVersion(v=>v+1); });
  }
  function useDisk() {
    const c=conflict(); if(!c)return;
    forgetDraft(props.path); setText(c.disk); setBaseline(c.disk); setDisk(c.disk); setConflict(null); setEditing(false); setVersion(v=>v+1);
    flashFooter('Your draft was discarded; showing the file on disk.');
  }
  const quote=(t: string)=>t.split('\n').map(l=>`> ${l}`).join('\n');
  const where=()=>props.path.split('/').slice(2).join('/')||props.path;
  function selectionTask(t: string) {
    const title=t.split('\n')[0].slice(0,120);
    openTaskSheet(undefined,{ title, description: `From ${where()}:\n\n${quote(t)}` });
  }
  /** Make a canvas from this file or a passage (F4): offered only for a file
   *  of the open workspace while canvases are on in its conversation. */
  function selectionAsk(t: string) {
    const text = `About this passage in ${where()}:\n\n${quote(t)}\n\n`;
    const root = workspaceDetail()?.workspace.path;
    if (!props.askInCurrentConversation && root && props.path.startsWith(`${root}/`) && !props.path.split('/').includes('..')) void draftInWorkspace(text);
    else draftInCurrentConversation(text);
  }
  function jump(index: number) {
    const heads=previewEl?.querySelectorAll('h1,h2,h3,h4');
    const el=heads?.[index] as HTMLElement|undefined;
    el?.scrollIntoView({ block: 'start', behavior: motionReduced() ? 'auto' : 'smooth' });
  }
  return <div class="work-file" classList={{'has-outline':outline().length>2}} aria-busy={loading()||saving()} onKeyDown={e=>{if((e.metaKey||e.ctrlKey)&&e.key.toLowerCase()==='s'){e.preventDefault();void save();}}}>
    <div class="work-file-toolbar"><span class="work-file-name">{props.title ?? props.path.split('/').pop()}</span>
      {/* The toolbar always names the state (plan L3 X2, 2026-10-14; review U workspace): a snapshot says so (X3). */}
      <span class="work-muted work-file-state" classList={{saved:savedFlash()}} data-state={conflict()||unreadable()?'conflict':dirty()?'draft':savedFlash()?'saved':props.readOnly?'read-only':editing()&&markdown()?'editing':props.reviewing?'reviewing':'viewing'}>{conflict()||unreadable()?'Conflict: not saved':dirty()?'Unsaved draft':savedFlash()?<><Icon name="check" size={12}/>Saved</>:props.readOnly?readOnlyWord(props.path):editing()&&markdown()?'Editing':props.reviewing?'Reviewing':'Viewing'}</span>
      {props.toolbar}
      <Show when={markdown()&&!props.readOnly&&loadedPath()===props.path&&!conflict()&&!unreadable()}><button class="btn small ghost" onClick={()=>setEditing(v=>!v)}>{editing()?'Preview':'Edit'}</button><Show when={dirty()||saving()}><button class="btn small primary" disabled={saving()} onClick={save}>{saving()?'Saving…':'Save'}</button></Show></Show>
      <ActionMenu label="File actions" items={[...(props.onAsk?[{label:'Ask about this file',action:props.onAsk}]:[]),{label:'Reload file (keep draft)',action:()=>setRetry(v=>v+1)},...(dirty()&&!conflict()&&!unreadable()?[{label:'Discard my draft',action:discardDraft}]:[]),...(props.actions??[]),{label:'Show file in Finder',action:()=>api.vaultRevealInFinder(props.path).catch(e=>setError(String(e)))}]}/>
    </div>
    <Show when={error()}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
    <Show when={restoredAt()!==null&&!conflict()&&!unreadable()}>
      <div class="work-file-restored" role="status"><Icon name="history" size={14}/><span>Restored your unsaved draft{restoredAt()?` from ${relativeWhen(restoredAt())}`:''}. It hasn’t been saved yet.</span><Button size="sm" variant="ghost" onClick={discardDraft}>Discard draft</Button><Button size="sm" onClick={()=>setRestoredAt(null)}>Keep editing</Button></div>
    </Show>
    <Show when={!loading()} fallback={<Skeleton class="work-skeleton" label="Opening file" rows={6}/>}>
      <Show when={!loadError()} fallback={<StatePanel kind="failed" title="This file couldn’t be opened" actions={<Button onClick={()=>setRetry(v=>v+1)}>Retry opening file</Button>}>{loadError()} Any saved draft is kept. Retry, or use File actions to locate the original.</StatePanel>}>
      <Show when={readable()} fallback={<StatePanel kind="partial" title="Hollow can’t show this file here">Its contents aren’t read into Hollow. Use File actions → Show file in Finder to open it in its own app; an agent with a reader for this format can still use it.</StatePanel>}>
        <Show when={!unreadable()} fallback={<StatePanel kind="conflicted" title="Hollow can’t read the file on disk right now" error={unreadable()?.reason} actions={<Button variant="primary" disabled={saving()||rereading()} onClick={()=>void reread()}>{rereading()?'Reading…':'Read the file again'}</Button>}>Nothing was overwritten and your draft is kept. The file changed while you were editing, and Hollow can’t read the new version, so there is nothing to compare or merge yet. Fix the file’s permissions or wait for the other app to finish, then read it again.</StatePanel>}>
        <Show when={!conflict()} fallback={<Show when={conflictKey()} keyed>{c=><ConflictResolver name={where()} base={c.base} disk={c.disk} draft={c.draft} busy={saving()} onKeepDraft={()=>void resolve(c.draft,c.disk)} onUseDisk={useDisk} onSaveMerged={(t,against)=>void resolve(t,against)}/>}</Show>}>
        <div class="work-file-body">
          <Show when={outline().length>2}>
            <nav class="doc-outline" aria-label="Outline"><For each={outline()}>{h=><button type="button" class={`doc-outline-item l${h.level}`} onClick={()=>jump(h.index)}>{h.text}</button>}</For></nav>
          </Show>
          <div class="work-file-main" ref={previewEl}>
            <Show when={editing()&&markdown()} fallback={
              <Show when={!table()} fallback={<TablePreview text={text()} name={props.path}/>}>
                <Show when={html()} fallback={<SelectionActions onTask={selectionTask} onAsk={selectionAsk}><Show when={markdown()} fallback={<pre class="work-text">{text()}</pre>}><NoteRenderer content={text()} onWikilinkClick={props.onWikilinkClick} /></Show></SelectionActions>}><HtmlDoc code={text()} defaultMode={fileMode(props.path,false)} grantKey={`file:${props.path}`} /></Show>
              </Show>}>
              <Show when={version()} keyed>{(_version)=> <Suspense fallback={<Skeleton label="Loading editor" rows={6} />}><MarkdownEditor initialDoc={text()} onChange={change} onWikilinkClick={props.onWikilinkClick} ai={{ ...INLINE_AI, key: aiKey + props.path }}/></Suspense>}</Show>
            </Show>
          </div>
        </div>
        </Show>
        </Show>
      </Show>
      </Show>
    </Show>
  </div>;
}
