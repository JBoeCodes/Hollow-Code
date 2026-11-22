import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { canLeaveConversation } from '../store';
import { openCommandPaletteWith } from '../store/overlays';
import { projectWorkspaces, visitWorkspaceConversation, workspaceId } from '../store/workspaces';
import { QUICK_CHATS_ID, QUICK_CHATS_LABEL, type SessionTarget } from '../workspaceTypes';
import { keyFor } from '../shortcuts';
import { Icon } from '../icons';
import { ProjectIcon } from './ProjectIcon';
import { railSearchRows, type RailSearchRow } from '../railSearch';
import { errMsg } from '../utils';
import { LoadingProgress } from './MotionKit';

/** The sidebar's Search chats (overnight 11-04 S2, as ChatGPT's): a field at
 *  the sidebar's top whose results replace its lists in place. Projects by
 *  title, then chats by title or by what was asked in them, newest first
 *  (`search_sessions`). ↑ ↓ move, Enter opens, Escape clears and then
 *  closes. The last row, Search everything, opens the command palette with
 *  the same words: global search's visible route since the top bar's
 *  search icon went. Loaded on first use, outside the App chunk. */
export default function RailSearch(props: { onClose: () => void }) {
  const [query, setQuery] = createSignal('');
  const [hits, setHits] = createSignal<SessionTarget[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal('');
  const [active, setActive] = createSignal(0);
  let input: HTMLInputElement | undefined;
  let list: HTMLDivElement | undefined;
  let asked = 0;
  onMount(() => input?.focus());
  // Ask the backend a beat after typing stops, from two characters (a search
  // reads every message); a late answer to older words never replaces a
  // newer one.
  createEffect(on(query, q => {
    const words = q.trim();
    setActive(0);
    if (words.length < 2) { asked++; setHits([]); setLoading(false); setError(''); return; }
    // Hits for the earlier words go at once, so Enter can't open a chat
    // that doesn't match what is typed now.
    setHits([]);
    const mine = ++asked;
    setLoading(true);
    const timer = setTimeout(() => {
      api.searchSessions(words, null, 20, workspaceId()).then(
        page => { if (mine === asked) { setHits(page.items); setError(''); } },
        e => { if (mine === asked) { setHits([]); setError(errMsg(e)); } },
      ).finally(() => { if (mine === asked) setLoading(false); });
    }, 200);
    onCleanup(() => clearTimeout(timer));
  }, { defer: true }));
  const rows = createMemo(() => railSearchRows(query(), projectWorkspaces(), hits()));
  // Rows can shrink after the arrows moved (hits arriving): stay in range.
  const current = () => Math.min(active(), rows().length - 1);
  const placeOf = (hit: SessionTarget) => hit.workspaceId === QUICK_CHATS_ID ? QUICK_CHATS_LABEL
    : projectWorkspaces().find(w => w.id === hit.workspaceId)?.title ?? '';
  const open = (row: RailSearchRow) => {
    if (row.kind === 'everything') { const words = query(); props.onClose(); openCommandPaletteWith(words); return; }
    // While a send is still being accepted the visit says why it waits, and
    // the search stays open with its words.
    const go = row.kind === 'project' ? () => visitWorkspaceConversation(row.workspace.id) : () => visitWorkspaceConversation(row.hit.workspaceId!, row.hit.session.key);
    void go();
    if (canLeaveConversation()) props.onClose();
  };
  // Keep the highlighted row in view as the arrows move it.
  createEffect(on(current, i => list?.querySelector(`[data-search-idx="${i}"]`)?.scrollIntoView({ block: 'nearest' }), { defer: true }));
  const onKey = (e: KeyboardEvent) => {
    if (e.isComposing) return;
    if (e.key === 'ArrowDown') { e.preventDefault(); setActive(Math.min(rows().length - 1, current() + 1)); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); setActive(Math.max(0, current() - 1)); }
    else if (e.key === 'Enter') { e.preventDefault(); const row = rows()[current()]; if (row) open(row); }
    else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); if (query()) setQuery(''); else props.onClose(); }
  };
  const searchKey = () => keyFor('search');
  return <div class="work-rail-search" role="search" aria-label="Search chats">
    <div class="work-rail-search-field">
      <Icon name="search" size={15}/>
      <input ref={input} type="text" role="combobox" aria-label="Search chats" placeholder="Search chats" autocomplete="off" spellcheck={false}
        aria-expanded="true" aria-controls="rail-search-results" aria-autocomplete="list" aria-activedescendant={`rail-search-${current()}`}
        value={query()} onInput={e => setQuery(e.currentTarget.value)} onKeyDown={onKey}/>
      <button type="button" class="icon-btn" aria-label="Close search" title="Close search (Esc)" onClick={() => props.onClose()}><Icon name="x" size={14}/></button>
    </div>
    <Show when={!query().trim()}><p class="work-rail-search-hint">Find a chat by its title or by what you asked in it.</p></Show>
    <div role="status" class="work-rail-search-status">
      <Show when={query().trim().length === 1}><p class="work-rail-search-hint">Keep typing to search chats.</p></Show>
      <Show when={query().trim().length > 1 && !loading() && !error() && rows().length === 1}><p class="work-rail-search-hint">No chats or projects match.</p></Show>
      <Show when={error()}>{e => <p class="work-rail-search-hint">Couldn't search chats: {e()}</p>}</Show>
    </div>
    <LoadingProgress loading={loading()} label="Searching chats" />
    <div ref={list} id="rail-search-results" class="work-rail-search-results" role="listbox" aria-label="Search results" aria-busy={loading()}>
      <For each={rows()}>{(row, i) => {
        const selected = () => current() === i();
        const choose = (e: MouseEvent) => { e.preventDefault(); open(row); };
        if (row.kind === 'project') return <div id={`rail-search-${i()}`} data-search-idx={i()} role="option" aria-selected={selected()} class="work-rail-search-row" classList={{ active: selected() }} onMouseDown={e => e.preventDefault()} onClick={choose} onMouseEnter={() => setActive(i())}>
          <ProjectIcon workspaceId={row.workspace.id}/><span class="work-rail-search-title">{row.workspace.title}</span><small>Project</small>
        </div>;
        if (row.kind === 'chat') return <div id={`rail-search-${i()}`} data-search-idx={i()} role="option" aria-selected={selected()} class="work-rail-search-row work-rail-search-chat" classList={{ active: selected() }} onMouseDown={e => e.preventDefault()} onClick={choose} onMouseEnter={() => setActive(i())}>
          <Icon name="message" size={14}/>
          <span class="work-rail-search-text">
            <span class="work-rail-search-title">{row.hit.session.title?.trim() || 'Conversation'}</span>
            <Show when={row.hit.snippet}>{s => <small class="work-rail-search-snippet">{s()}</small>}</Show>
          </span>
          <small>{placeOf(row.hit)}</small>
        </div>;
        return <div id={`rail-search-${i()}`} data-search-idx={i()} role="option" aria-selected={selected()} class="work-rail-search-row work-rail-search-everything" classList={{ active: selected() }} onMouseDown={e => e.preventDefault()} onClick={choose} onMouseEnter={() => setActive(i())}>
          <Icon name="search" size={14}/><span class="work-rail-search-title">{query().trim() ? `Search everything for “${query().trim()}”` : 'Search everything'}</span><Show when={searchKey()}>{k => <kbd aria-hidden="true">{k()}</kbd>}</Show>
        </div>;
      }}</For>
    </div>
  </div>;
}

