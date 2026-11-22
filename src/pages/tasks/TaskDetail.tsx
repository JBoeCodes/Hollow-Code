// One task's detail pane (split out of pages/Tasks.tsx by ST11, 2026-10-14,
// no behaviour change): the task, its edit form, the blockers editor, and
// "Work on this" (which opens the task's workspace with the task added after
// any draft there). The linked-record form went with two-way sync (11-10).
import { For, Show, createSignal } from 'solid-js';
import { TaskOriginText, taskOrigin } from '../../components/TaskOrigin';
import { api } from '../../api';
import { MarkdownView } from '../../components/MarkdownView';
import { BlockedByPicker } from '../../components/BlockedByPicker';
import { blockerLabel, blockerWarningText } from '../../taskStructure';
import { tasksData, chatBusy, openVaultNote, flashFooter, chatInput, setChatInput, requestChatFocus } from '../../store';
import { addToDraft } from '../../chatQueue';
import { workspaces, selectWorkspaceOpened, workspaceId, workspaceError } from '../../store/workspaces';
import { taskErrorText } from '../../utils';
import { isOpenTask } from '../../taskStatus';
import { TaskEditor } from '../../components/TaskEditor';
import { Select } from '../../components/Select';
import { canonicalTaskStatus, priorityLabel, type TaskProjectChoice } from '../../taskEdit';
import { originWorkspace, workOnPrompt } from '../../taskBulk';
import { ErrorText } from '../../components/ui';
import { SourceLabelChip } from '../../components/SourceLabelChip';
import type { Task } from '../../types';

/** Why Work on this is off while a chat turn runs (P1-17). */
export const CHAT_BUSY_REASON = 'Available when the running chat turn finishes';

/** The one gate for both Work on this paths (the pane and the row menu):
 *  why it's off, or '' (P1-17). */
export const workOnBlocked = () => chatBusy() ? CHAT_BUSY_REASON : '';

export function TaskDetail(props:{url:string;task:()=>Task|undefined;workspace:()=>ReturnType<typeof workspaces>[number]|undefined;onClose:()=>void;refresh:()=>Promise<void>;choices:()=>TaskProjectChoice[]}) {
  const [localEditing,setLocalEditing]=createSignal(false);
  const [message,setMessage]=createSignal('');
  return <Show when={props.task()}>{task=><>
    <div class="task-detail-top"><span>Local task</span><button class="btn small ghost" aria-label="Close task details" onClick={props.onClose}>Close <span aria-hidden="true">×</span></button></div>
    <Show when={!localEditing()} fallback={<><h2 class="task-editor-heading" id="task-edit-heading">Edit task</h2><TaskEditor surface="panel" task={task()} headingId="task-edit-heading" projects={props.choices} onClose={()=>setLocalEditing(false)} onSaved={async()=>{setMessage('Saved.');await props.refresh();}}/></>}>
      <h2 class="task-detail-title">{task().title}</h2><div class="task-detail-state"><span classList={{done:!isOpenTask(task().status)}}>{canonicalTaskStatus(task().status)}</span><div class="work-actions"><button class="btn small" onClick={()=>setLocalEditing(true)}>Edit task</button></div></div>
      <dl class="task-properties"><Show when={taskOrigin(task().origin,task().title)}>{o=><div class="task-origin-row"><dt>From</dt><dd><TaskOriginText origin={o()}/></dd></div>}</Show><Show when={task().sourceLabel}>{l=><div class="task-source-row"><dt>Lives in</dt><dd><SourceLabelChip label={l()} withOpen/></dd></div>}</Show><div><dt>Project</dt><dd>{task().projectName??'No project'}</dd></div><div><dt>Due</dt><dd>{task().due?`${task().due!.slice(0,10)}${task().dueTime?` · ${task().dueTime}`:''}`:'No date'}</dd></div><div><dt>Priority</dt><dd>{priorityLabel(String(task().priority??''))}</dd></div><Show when={task().labels.length}><div><dt>Labels</dt><dd>{task().labels.join(', ')}</dd></div></Show><Show when={task().recurrence}><div><dt>Repeats</dt><dd>{task().recurrence}</dd></div></Show></dl>
      <WorkOnThis task={task()} linked={(()=>{const id=originWorkspace(task());return id?workspaces().find(w=>w.id===id):props.workspace();})()}/>
      <div class="task-detail-notes"><h3>Notes</h3><p>{task().description||'No additional notes yet.'}</p></div><Show when={task().subtasks.length}><div class="task-detail-steps"><h3>Steps</h3><For each={task().subtasks}>{s=><p><span aria-label={s.done?'Completed':'Open'}>{s.done?'✓':'○'}</span>{s.text}</p>}</For></div></Show><Show when={task().doneWhen}>{text=><details class="task-detail-reserved"><summary>Done when</summary><MarkdownView text={text()}/></details>}</Show><Show when={task().activity}>{text=><details class="task-detail-reserved"><summary>Log</summary><MarkdownView text={text()}/></details>}</Show>
      <TaskBlockers task={task} refresh={props.refresh}/>
      <div class="task-detail-links"><button class="btn small" disabled={!task().vaultPath} onClick={()=>openVaultNote(task().vaultPath)}>Open note</button></div>
    </Show>
    <Show when={message()}><p role="status" class="work-muted">{message()}</p></Show>
  </>}</Show>;
}

/// Blocked by / Blocks in the Tasks pane (Paperclip PR 11b). Saves on its own
/// through `update_task` with the contentHash captured when editing began (a
/// change made elsewhere while the picker is open returns TASK_CONFLICT and
/// keeps the draft).
function TaskBlockers(props:{task:()=>Task;refresh:()=>Promise<void>}) {
  const all=()=>tasksData()?.items??[];
  const [editing,setEditing]=createSignal(false);const [draft,setDraft]=createSignal<string[]>([]);
  const [busy,setBusy]=createSignal(false);const [error,setError]=createSignal('');
  const [baseHash,setBaseHash]=createSignal<string|undefined>();
  const unresolved=(url:string)=>props.task().unresolvedBlockers.includes(url);
  const warnings=(url:string)=>props.task().blockerWarnings.filter(w=>w.url===url);
  function begin(){const t=props.task();setDraft([...t.blockedBy]);setBaseHash(t.contentHash||undefined);setError('');setEditing(true);}
  async function save(){
    const t=props.task();if(busy())return;setBusy(true);setError('');
    try{await api.updateTask({url:t.url,expected:baseHash(),blockedBy:draft()});setEditing(false);await props.refresh();}
    catch(e){setError(taskErrorText(e));}finally{setBusy(false);}
  }
  const has=()=>props.task().blockedBy.length>0||props.task().blocks.length>0;
  return <div class="task-detail-blockers">
    <Show when={editing()} fallback={<>
      <Show when={props.task().blockedBy.length}><h3>Blocked by</h3><ul class="task-blocker-list"><For each={props.task().blockedBy}>{url=><li classList={{resolved:!unresolved(url)}}><span class="task-blocker-mark" aria-label={unresolved(url)?'Not finished':'Finished'}>{unresolved(url)?'○':'✓'}</span><span>{blockerLabel(all(),url)}</span><For each={warnings(url)}>{w=><p class="task-blocker-warning" role="note">{blockerWarningText(w,blockerLabel(all(),url))}</p>}</For></li>}</For></ul></Show>
      <Show when={props.task().blocks.length}><h3>Blocks</h3><ul class="task-blocker-list"><For each={props.task().blocks}>{url=><li><span>{blockerLabel(all(),url)}</span></li>}</For></ul></Show>
      <button class="btn small task-blocker-edit" onClick={begin}>{has()?'Edit blockers':'Add a blocker'}</button>
    </>}>
      <h3>Blocked by</h3>
      <BlockedByPicker tasks={all} selfUrl={props.task().url} value={draft} onChange={next=>setDraft(next)} disabled={busy()}/>
      <div class="work-actions"><button class="btn small primary" disabled={busy()} onClick={()=>void save()}>{busy()?'Saving…':'Save blockers'}</button><button class="btn small ghost" disabled={busy()} onClick={()=>{setEditing(false);setError('');}}>Cancel</button></div>
    </Show>
    <Show when={error()}>{msg=><ErrorText class="task-detail-error" text={msg()}/>}</Show>
  </div>;
}

/// "Work on this": open workspace `wid` with task `t` added to its composer
/// after any draft there, never replacing it, and only once `wid` is the
/// workspace showing (morning M5; before, it overwrote the draft, and a
/// switch that didn't happen wrote into the workspace still showing).
export async function openWithTask(t:Task,wid:string){
  // The one gate both paths share (P1-17): their controls are off with this reason too.
  const blocked=workOnBlocked();if(blocked){flashFooter(blocked,4000);return;}
  if(!await selectWorkspaceOpened(wid)||workspaceId()!==wid){
    // A superseded open says nothing on its own; a failed one already did.
    if(!workspaceError())flashFooter('× The workspace didn’t open, so the task wasn’t added. Try Work on this again.',6000);
    return;
  }
  setChatInput(addToDraft(chatInput(),workOnPrompt(t)));requestChatFocus();
}

/// "Work on this" (next-level P09): opens the task's workspace with the task
/// as the first message. The workspace comes from an explicit link (the plan
/// the task was created from), never from matching
/// names; with no link the user chooses one, and nothing is saved by choosing.
function WorkOnThis(props:{task:Task;linked:ReturnType<typeof workspaces>[number]|undefined}) {
  const [choice,setChoice]=createSignal('');
  const go=(id:string)=>openWithTask(props.task,id);
  return <div class="task-workspace-link">
    <span>Work on this</span>
    <Show when={props.linked} fallback={<><p>This task isn’t linked to a workspace. Choose one to open it there with this task as the first message.</p><Select size="sm" label="Workspace for this task" placeholder="Choose a workspace…" disabled={!!workOnBlocked()} value={choice()} onChange={v=>{setChoice(v);void go(v);}} options={workspaces().filter(w=>!w.kind).map(w=>({value:w.id,label:w.title}))}/></>}>{w=>
      <><strong>{w().title}</strong><p>Opens the workspace with this task, its file and “done when” in the composer. Nothing is sent until you send it.</p><button class="btn small primary" disabled={!!workOnBlocked()} title={workOnBlocked()||undefined} aria-description={workOnBlocked()||undefined} onClick={()=>void go(w().id)}>Work on this <span aria-hidden="true">→</span></button></>}
    </Show>
    <Show when={workOnBlocked()}>{why=><small class="work-muted">{why()}</small>}</Show>
  </div>;
}
