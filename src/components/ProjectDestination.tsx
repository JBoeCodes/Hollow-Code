import { Show, createSignal, onMount, onCleanup } from 'solid-js';
import { api } from '../api';
import { projectDestinationTarget, setProjectDestinationTarget } from '../store/overlays';
import { projectWorkspaces, workspaceSessions, refreshWorkspaces, refreshWorkspaceSessions, workspaceListError, requestNewWorkspace, moveSessionTo, moveBlockedReason } from '../store/workspaces';
import { projectChoices, mostRecentProject } from '../projectDestination';
import { QUICK_CHATS_ID } from '../workspaceTypes';
import { workspaceOfSession } from '../store/workspaces';
import { distinguishing } from '../workspaceOrigin';
import { holdFocus } from '../focusHold';
import { errMsg } from '../utils';
import { Select } from './Select';
import { Button, PlainErrorText } from './ui';

/** Sam's shared explicit picker, in the host dialog so row menus and Home
 * use the same flow. Selection never moves a chat; Add confirms the target. */
export default function ProjectDestination() {
  const target = projectDestinationTarget()!;
  const [selected, setSelected] = createSignal('');
  const [loading, setLoading] = createSignal(true);
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal('');
  let card!: HTMLDivElement, alive = true, revision = 0;
  const current = () => alive && projectDestinationTarget() === target;
  const excluded = () => target.key ? workspaceOfSession(target.key) : null;
  const choices = () => projectChoices(projectWorkspaces(), workspaceSessions(), excluded());
  const reason = () => target.key ? moveBlockedReason(target.key) : null;
  const close = () => { if (!pending() && current()) setProjectDestinationTarget(null); };
  onCleanup(() => { alive = false; ++revision; });
  onMount(() => {
    const release = holdFocus(card); card.focus();
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !document.querySelector('.hs-pop')) {
        e.preventDefault(); e.stopImmediatePropagation(); close();
      }
    };
    document.addEventListener('keydown', key, true);
    onCleanup(() => { release(); document.removeEventListener('keydown', key, true); });
    void load();
  });
  async function load() {
    const ticket = ++revision;
    setLoading(true); setError(''); setSelected('');
    try {
      const [, ok] = await Promise.all([refreshWorkspaces({join:true}), refreshWorkspaceSessions()]);
      if (!current() || ticket !== revision) return;
      if (!ok || workspaceListError()) throw new Error('Projects could not be refreshed. Retry before choosing.');
      setSelected(choices().some(c => c.project.id === target.value) ? target.value! : mostRecentProject(projectWorkspaces(), workspaceSessions(), excluded()));
    } catch (e) { if (current() && ticket === revision) setError(errMsg(e)); }
    finally { if (current() && ticket === revision) setLoading(false); }
  }
  async function confirm(id: string | null) {
    if (!current() || pending() || loading() || error() || reason() || (!id && target.key)) return;
    setPending(true); setError('');
    try {
      if (id && id !== QUICK_CHATS_ID) {
        const detail = await api.readWorkspace(id);
        if (!projectChoices([detail.workspace], {}, excluded()).length) throw new Error('This project is no longer available. Refresh projects.');
      }
      if (!current()) return;
      if (reason()) throw new Error(reason()!);
      if (target.key && id) {
        const ok = await moveSessionTo(target.key, id, {failed: why => { if (current()) setError(why); }});
        if (!ok) return;
      } else target.pick?.(id);
      if (current()) setProjectDestinationTarget(null);
    } catch (e) { if (current()) setError(errMsg(e)); }
    finally { if (current()) setPending(false); }
  }
  return <div class="rbk-backdrop" onClick={e => { if (e.target === e.currentTarget) close(); }}>
    <div ref={card} class="rbk-card project-destination-card" role="dialog" aria-modal="true" aria-labelledby="project-destination-title" tabIndex={-1}>
      <h2 id="project-destination-title">{target.key ? 'Move to project' : 'Add to project'}</h2>
      <p>{target.key ? 'Move this conversation into a project. Its history and Hollow identity stay intact. Future turns use the project context and a fresh provider session.' : 'Choose the project for your next conversation. Nothing is sent until you ask.'}</p>
      <Show when={loading()}><p role="status">Loading projects…</p></Show>
      <Show when={!loading() && !error() && choices().length}>
        <Select label="Project" value={selected()} onChange={setSelected} disabled={pending()} placeholder="Choose a project…" options={choices().map(c => ({value:c.project.id,label:c.project.title,detail:distinguishing(c.project,projectWorkspaces()) ?? (c.activity ? 'Recently worked on' : undefined)}))}/>
        <p class="work-muted">Defaults to your most recently worked-on project, from saved conversation activity.</p>
      </Show>
      <Show when={!loading() && !error() && !choices().length}><p>No eligible projects.</p></Show>
      <Show when={error()}><div role="alert"><PlainErrorText text={error()}/><Button disabled={pending()} onClick={() => void load()}>Refresh projects</Button></div></Show>
      <Show when={reason()}><p role="status">{reason()}</p></Show>
      <div class="work-actions">
        <Button variant="primary" disabled={!selected() || loading() || pending() || !!error() || !!reason()} onClick={() => void confirm(selected())}>{pending() ? (target.key ? 'Moving…' : 'Adding…') : target.key ? 'Move conversation' : 'Use project'}</Button>
        <Show when={!target.key && target.value}><Button disabled={pending() || loading() || !!error()} onClick={() => void confirm(null)}>Keep with the COO</Button></Show>
        <Show when={target.key && workspaceOfSession(target.key) !== QUICK_CHATS_ID}><Button disabled={pending() || loading() || !!error() || !!reason()} onClick={() => void confirm(QUICK_CHATS_ID)}>Move to the COO</Button></Show>
        <Button disabled={pending()} onClick={() => { close(); void requestNewWorkspace(); }}>New project</Button>
        <Button variant="ghost" disabled={pending()} onClick={close}>Cancel</Button>
      </div>
    </div>
  </div>;
}
