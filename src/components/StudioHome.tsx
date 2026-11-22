import { setProjectDestinationTarget } from '../store/overlays';
import { SessionRollupBadge } from './SessionIndicator';
import { workspaceRollup } from '../store/sessionStates';
import { For, Show, Suspense, createEffect, createMemo, createSignal, lazy } from 'solid-js';
import { Select } from './Select';
import { Icon } from '../icons';
import { Button, EmptyState, ErrorDetails, ListRow } from './ui';
import { PinnedCanvasSlot } from './PinnedCanvasSlot';
import { ProjectIcon } from './ProjectIcon';
import { canLeaveConversation, openContextMenu } from '../store';
import { confirmUndoMerge, projectWorkspaces as workspaces, workspaceSessions, workspaceIcons, visitWorkspace, quickAskDraft, setQuickAskDraft, askProject, setAskProject, workspaceListError, workspaceListLoaded, workspaceListLoading, refreshWorkspaces } from '../store/workspaces';
import { isCurrentWorkspace, type WorkWorkspace } from '../workspaceTypes';
import { workspaceMenuItems } from '../workspaceMenu';
import { MoreButton } from './MoreButton';
import { WORKSPACE_STARTERS, latestWorkspaceSession, orderWorkspaces, type WorkspaceStarter } from '../studio';
import { ASK_SCOPES, type AskScope } from '../askScopes';
import { askHere, currentAskScope, homeAskFocusRequest, setHomeAskFocusRequest, setHomeAskScope } from '../store/ask';
import { distinguishing, workspaceTooltip, workspaceOrigin } from '../workspaceOrigin';
import '../styles/home.css';
import '../styles/chat-home.css';
const RecentlyDeleted = lazy(() => import('./RecentlyDeleted'));

/** Sam's chat-first Home. Existing project identities, Ask ownership and
 * recovery routes survive; daily details belong to their existing views. */
export function StudioHome(props: { onNew: (starter?: WorkspaceStarter) => void }) {
  const [query, setQuery] = createSignal('');
  const [expanded, setExpanded] = createSignal(false);
  const [showArchived, setShowArchived] = createSignal(false);
  const shown = createMemo(() => {
    const words = query().trim().toLowerCase().split(/\s+/).filter(Boolean);
    return orderWorkspaces(workspaces().filter(w => {
      const name = (words.length > 1 ? `${w.title} ${workspaceOrigin(w, workspaceIcons()[w.id]?.owner)}` : w.title).toLowerCase();
      return (showArchived() || isCurrentWorkspace(w.status)) && words.every(word => name.includes(word));
    }), workspaceSessions(), [], 'recent');
  });
  const rows = () => expanded() ? shown() : shown().slice(0, 5);
  const named = (w: WorkWorkspace) => {
    const twin = distinguishing(w, workspaces(), workspaceIcons()[w.id]?.owner);
    return twin ? `${w.title} (${twin})` : w.title;
  };
  return <div class="work-overview studio-home chat-home">
    <PinnedCanvasSlot />
    <section class="home-conversation" aria-label="Start a conversation">
      <div class="home-chat-heading"><h2>What would you like to work on?</h2></div>
      <AskAnything/>
    </section>
    <section aria-label="Projects" class="home-projects" aria-busy={workspaceListLoading()}>
      <header class="home-projects-head"><h2>{expanded() ? 'All projects' : 'Recent projects'}</h2><div class="home-project-actions"><Button size="sm" icon="plus" disabled={!canLeaveConversation()} onClick={() => props.onNew()}>New project</Button></div></header>
      <Show when={workspaceListError()}><div class="home-project-error" role="alert"><Icon name="alert" size={16}/> Couldn’t load projects. <ErrorDetails detail={workspaceListError()}/><Button size="sm" disabled={workspaceListLoading()} onClick={() => void refreshWorkspaces({ join: true })}>Retry</Button></div></Show>
      <Show when={expanded()}><div class="home-project-filters"><label class="work-search"><Icon name="search" size={15}/><input type="search" aria-label="Find a project" placeholder="Find a project…" value={query()} onInput={e => setQuery(e.currentTarget.value)}/></label><label><input type="checkbox" checked={showArchived()} onChange={e => setShowArchived(e.currentTarget.checked)}/> Include archived</label></div></Show>
      <Show when={rows().length} fallback={<Show when={!workspaceListError()}><Show when={workspaceListLoaded()} fallback={<p class="work-muted" role="status">Loading projects…</p>}><EmptyState icon="folder" title={query() ? 'No matching projects' : 'A place for your ongoing work'}>{query() ? 'Try another name.' : 'Keep related conversations, sources and results together.'}</EmptyState></Show></Show>}>
      <div class="home-project-list" role="list" data-key-list data-key-default>
        <For each={rows()}>{w =>
          <div class="home-project-row" role="listitem"><ListRow class="work-card" keyItem disabled={!canLeaveConversation()} onClick={() => void visitWorkspace(w.id)} onContextMenu={e => openContextMenu(e, workspaceMenuItems(w))}
            leading={<ProjectIcon workspaceId={w.id}/>} label={`Open ${named(w)}`} description={[w.status.toLowerCase() !== 'active' ? w.status : '', w.mergedInto ? `Merged into ${workspaces().find(p => p.id === w.mergedInto)?.title ?? w.mergedInto}` : '', latestWorkspaceSession(workspaceSessions()[w.id])?.title?.trim(), workspaceRollup(w.id)?.label].filter(Boolean).join(' · ') || undefined} title={w.title} tooltip={workspaceTooltip(w, workspaceIcons()[w.id]?.owner)}
            meta={[w.mergedInto ? `Merged into ${workspaces().find(p => p.id === w.mergedInto)?.title ?? w.mergedInto}` : '', distinguishing(w, workspaces(), workspaceIcons()[w.id]?.owner), latestWorkspaceSession(workspaceSessions()[w.id])?.title?.trim()].filter(Boolean).join(' · ') || undefined}
            trailing={<><SessionRollupBadge rollup={workspaceRollup(w.id)}/><Show when={w.status.toLowerCase() !== 'active'}><span class="work-card-status" classList={{ 'is-blocked': w.status.toLowerCase() === 'blocked' }} data-status={w.status.toLowerCase()}>{w.status}</span></Show></>}/><Show when={w.mergedInto}><Button size="sm" variant="ghost" disabled={!canLeaveConversation()} aria-label={`Undo merge… (${named(w)})`} onClick={() => void confirmUndoMerge(w)}>Undo merge…</Button></Show><MoreButton label={`Actions for ${named(w)}`} items={() => workspaceMenuItems(w)}/></div>
        }</For>
      </div>
      </Show>
      <button type="button" class="btn ghost small" aria-expanded={expanded()} onClick={() => { setExpanded(v => !v); setQuery(''); setShowArchived(false);
        // All projects goes straight to Find (it did from the sidebar's All
        // projects row, which went in L1, 11-12).
        if (expanded()) queueMicrotask(() => document.querySelector<HTMLInputElement>('[aria-label="Find a project"]')?.focus()); }}>{expanded() ? 'Show recent projects' : 'All projects'}</button>
      <Show when={expanded() || (workspaceListLoaded() && !workspaces().length)}><Suspense fallback={null}><RecentlyDeleted top={!workspaces().length}/></Suspense></Show>
    </section>
  </div>;
}

/** Ask (2026-09-30; scope since consolidation D, 2026-10-13; its one home
 *  since the top bar's Ask ⌄ went, 2026-10-21; the one Ask, every question a
 *  chat, since One Ask, 2026-11-03). The scope is visible and
 *  chosen, so a question never lands in a quick chat while the person
 *  believed otherwise. A quick
 *  chat's draft lives in the store and is cleared only once the chat exists
 *  (B·F15): Home unmounts while it opens, and a failed open gives it back. */
function AskAnything() {
  const scopes = ASK_SCOPES;
  // The chosen place outlives Home, like the text (plan completion item 1).
  const scope = currentAskScope;
  const setScope = (v: AskScope) => setHomeAskScope(v);
  // `g v`, the palette's Ask anything… (store/ask.ts openHomeAsk).
  let field: HTMLTextAreaElement | undefined;
  createEffect(() => {
    if (!homeAskFocusRequest()) return;
    setHomeAskFocusRequest(false);
    queueMicrotask(() => field?.focus());
  });
  const chosen = () => scopes.find(s => s.id === scope()) ?? scopes[scopes.length - 1];
  // ⌥ held in the field: a scope that can only place the text says so on its
  // button (D2); ⌥-Enter, or a click with ⌥ held, places it.
  const [option, setOption] = createSignal(false);
  const trackOption = (e: KeyboardEvent) => setOption(e.altKey);
  let altClick = false;
  // Every submit is remembered, a quick chat included (P1-11; askHere).
  const submit = (e?: Event, place = altClick) => {
    e?.preventDefault();
    altClick = false;
    if (!quickAskDraft().trim() || !canLeaveConversation()) return;
    const text = quickAskDraft();
    void askHere(text, { place }).then(ok => { if (ok && quickAskDraft() === text) setQuickAskDraft(''); });
  };
  // Two rows, like New workspace's "Desired result": Enter submits,
  // Shift-Enter starts a line.
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== 'Enter' || e.shiftKey || e.isComposing) return;
    e.preventDefault();
    submit(undefined, e.altKey);
  };
  return <form class="studio-ask" aria-label="Ask" onSubmit={e => submit(e)} onKeyDown={trackOption} onKeyUp={trackOption} onFocusOut={() => setOption(false)}>
    <Icon name="message" size={16}/>
    <textarea ref={field} rows={2} aria-label="Ask anything" aria-describedby="studio-ask-hint" placeholder={chosen().placeholder} value={quickAskDraft()} onInput={e => setQuickAskDraft(e.currentTarget.value)} onKeyDown={onKey}/>
    <div class="studio-ask-controls"><Select size="sm" class="studio-ask-scope" label="Where to ask" value={scope()} onChange={v => setScope(v as AskScope)} searchable={false}
      options={scopes.map(s => ({ value: s.id, label: s.label, detail: s.context }))}/>
    <Show when={scope() === 'quick'}><Button size="sm" type="button" disabled={!canLeaveConversation()} onClick={() => setProjectDestinationTarget({key:null,value:askProject(),pick:setAskProject})}>{askProject() ? `Project: ${workspaces().find(w=>w.id===askProject())?.title ?? 'Unavailable project'}` : 'Add to project'}</Button></Show>
    {/* Says where it goes (review U Home; L4, X15): each scope's own words. */}
    <Button size="sm" variant="primary" type="submit" disabled={!quickAskDraft().trim() || !canLeaveConversation()} title={chosen().context} onClick={e => { altClick = e.altKey; }}>{(option() && chosen().placeAction) || chosen().action}</Button></div>
    <p id="studio-ask-hint" class="studio-ask-hint">{scope() === 'quick' && askProject() ? `New conversation in ${workspaces().find(w=>w.id===askProject())?.title ?? 'the selected project'}.` : chosen().context}</p>
  </form>;
}

export function StarterShelf(props: { onNew: (starter: WorkspaceStarter) => void }) {
  return <section class="studio-starters" aria-label="Workspace starting points">
      <div class="work-section-heading"><h2>Start from</h2></div>
      <div class="studio-starter-grid"><For each={WORKSPACE_STARTERS}>{starter => <button class="studio-starter" disabled={!canLeaveConversation()} onClick={() => props.onNew(starter)}>
        <span class="studio-starter-icon"><Icon name={starter.icon} size={19}/></span><strong>{starter.title}</strong><span>{starter.description}</span><Icon name="chevronRight" size={14}/>
      </button>}</For></div>
    </section>;
}
