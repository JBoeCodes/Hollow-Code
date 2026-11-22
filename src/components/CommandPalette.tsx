import { EmptyArt } from './EmptyArt';
import { distinguishing, workspaceOrigin } from '../workspaceOrigin';
import { paletteRecents, recordPaletteRecent } from '../store/prefs';
import { isWorkspaceVaultItem } from '../workspaceContent';
import { workspaces, visitWorkspace, refreshWorkspaces, goToPage, railWorkspaces, newWorkspaceSession, startNewConversation, workspaceSessions, workspaceId, startQuickChat, mergeWorkspaceInto } from '../store/workspaces';
import { newWorkspaceFromChat } from '../conversationActions';
import { mergeTargets } from '../workspaceMerge';
import { lastCanvas } from '../store/lastCanvas';
import { askHere, askPlaceLabel, openHomeAsk } from '../store/ask';
import { askRowIndex, looksLikeSentence } from '../askScopes';
import { workspaceChoices } from '../sessionMove';
import { ErrorBoundary, Suspense, lazy, For, Show, createEffect, createMemo, createSignal, on, onCleanup, untrack } from 'solid-js';
import { experimentalActive } from '../store/studio';
import type { PalettePreviewTarget } from '../palettePreview';
const PalettePreview = lazy(() => import('./PalettePreview'));
import {
  commandPaletteOpen, closeCommandPalette, openCommandPalette, registerPaletteCommand, setShortcutsOverlayOpen, paletteScope, paletteSearchRequest, paletteMoveKey, paletteMergeFrom,
  activePage, setActivePage, requestOpenVaultNote,
  openSettingsFromOutside,
  vaultTree, tasksData, skillsList,
  flashFooter,
  setLibraryCollectionRequest, chatAgentSessionKey,
} from '../store';
import { previewShown } from '../store/preview';
import { openLibrarySkill } from '../store/libraryMode';
import { paletteQueryRequest, setPaletteQueryRequest, paletteReturnsToRoot, setPaletteReturnsToRoot } from '../store/overlays';
import { Icon, type IconName } from '../icons';
import { api } from '../api';
import { createExitPhase } from './exit';
import { focusOpener, giveFocusBack, giveFocusBackAfterRun } from '../focusReturn';
import { locationStamp } from '../store/locationStamp';
import { openModal } from '../modals/TaskModal';
import { pageShortcut, goHint, keyFor, shortcutFor, MOD, PALETTE_CONTEXTS } from '../shortcuts';
import { backTarget, digitOfKey, digitOfMenuCommand, rowDigit, rowForDigit, splitActionPrefix } from '../paletteNav';
import { ACTION_BY_PALETTE, ACTION_DEFS, actionLabel, type ActionDef } from '../actionDefs';
import { actionAvailable, runAction } from '../actionRun';
import { NAV_PAGES } from '../navigation';
import { SETTINGS_SECTION_GROUPS } from '../settingsSections';
import { matchesTaskQuery, rankTasks, toTaskQuery } from '../taskQuery';
import { isOpenTask } from '../taskStatus';
import { dueLabel } from '../utils';
import type { ConnectedFile, SearchResult, Task, VaultTree } from '../types';
import { Button, Segmented } from './ui';
import { latestRequest, runLatest } from '../latestRequest';
import { joinsRecent, paletteDefaultOrder, paletteEffectLine } from '../paletteDefault';
import { PALETTE_SCOPES, createConversationSearch, excerptLine, firstNewConversationRow, inScope, keptSelection, isWorkspaceResult, paletteScore, projectFileRow, searchedConversations, vaultSourceLabel, wantsConversationSearch, wantsProjectFiles, wantsVaultSearch, type PaletteSearchScope } from '../paletteSearch';

/// One row in the palette. `keywords` boosts matching beyond the visible
/// `label` (e.g. "Refresh calendar" also matches "outlook").
interface PaletteItem {
  id: string;
  label: string;
  hint?: string;
  /** Optional left-side icon name from `../icons`. */
  icon?: IconName;
  /** Group bucket for the section heading. */
  group: 'Ask' | 'Recent' | 'Conversation' | 'Conversations' | 'Navigation' | 'Actions' | 'Settings' | 'Notes' | 'Tasks' | 'Task matches' | 'Workspaces' | 'Skills' | 'Search results' | 'Results' | 'Project files';
  /** A second line under the label: a search excerpt or a file's folder (P07). */
  detail?: string;
  /** Where a search row came from ("Library", "Result · Q3 review", "Project · Launch"). */
  source?: string;
  /** Extra terms that should match a query but aren't shown in the label. */
  keywords?: string;
  /** Right-aligned keyboard shortcut, e.g. "⌘1". Decorative only — the
   *  actual binding lives in App.tsx. */
  shortcut?: string;
  /** Runs without closing the palette or joining Recent (a "More…" row). */
  keepOpen?: boolean;
  preview?: PalettePreviewTarget;
  run: () => unknown;
}

/// Score a query against a label + keywords: `paletteScore` (paletteSearch.ts),
/// word by word since P1-48 ("new task" finds New task).
const score = paletteScore;

/// Where the query hit the label, for match highlighting — [start, end)
/// of the first occurrence, or null when the match was keyword-only.
/// A plain indexOf of the whole query: a word-by-word match (P1-48, "task
/// new") whose words aren't one run in the label shows no highlight.
function matchRange(query: string, label: string): [number, number] | null {
  const q = query.toLowerCase().trim();
  if (!q) return null;
  const lower = label.toLowerCase();
  // toLowerCase isn't length-preserving for all of Unicode (İ → i + ̇ ),
  // which would misalign the indices against the original label — render
  // those labels plain, like keyword-only matches.
  if (lower.length !== label.length) return null;
  const idx = lower.indexOf(q);
  return idx >= 0 ? [idx, idx + q.length] : null;
}

/// Recent commands — executed item ids persist across sessions (cap 24)
/// so the palette's empty-query state can float your last-used commands
/// to the top. Stale ids (a moved note, a finished task) silently drop
/// when they no longer resolve against the live item set.
/** The open workspace's PLAN.md, when it has one and a canvas can be made
 *  from it (F4). */
/** A shared action's key hint: its ⌘ chord, else its go-to keys, as set now. */
function actionHint(def: ActionDef): string {
  return (def.key ? keyFor(def.key) : '') || (def.go ? goHint(def.go) : '');
}
const recentIds = paletteRecents;
const recordRecent = recordPaletteRecent;

export function CommandPalette() {
  const previewMedia = matchMedia('(min-width: 1100px) and (min-height: 620px)');
  const [previewRoom, setPreviewRoom] = createSignal(previewMedia.matches);
  const resizePreview = () => setPreviewRoom(previewMedia.matches);
  previewMedia.addEventListener('change', resizePreview);
  onCleanup(() => previewMedia.removeEventListener('change', resizePreview));
  const previewShown = () => experimentalActive() && paletteScope() === 'all' && previewRoom();
  const [query, setQuery] = createSignal('');
  const [selectedIdx, setSelectedIdx] = createSignal(0);
  // Async full-text vault search results, merged into a "Search results"
  // group below the instant filename matches. Held in a signal because the
  // palette is otherwise synchronous; rapid typing must ignore stale
  // responses (see the request-token guard in the debounce effect).
  const [searchResults, setSearchResults] = createSignal<SearchResult[]>([]);
  // 'pending' until the file search answers, so an empty list is not shown
  // as "No matches" before it has (audit H09).
  const [searchPhase, setSearchPhase] = createSignal<'idle' | 'pending' | 'done' | 'error'>('idle');
  // Search scope chips (next-level P07) and the connected-project lookup
  // (`connected_files`), which finds a file by name across the whole
  // inventory, including files past a project's listing share.
  const [scope, setScope] = createSignal<PaletteSearchScope>('all');
  const [projectHits, setProjectHits] = createSignal<{ files: ConnectedFile[]; direct: ConnectedFile[] }>({ files: [], direct: [] });
  const [projectPhase, setProjectPhase] = createSignal<'idle' | 'pending' | 'done' | 'error'>('idle');
  // Conversations by title from every workspace (merged plan slice 4,
  // B·F15): the rail's list holds only the 25 newest per workspace. Paged:
  // a "More conversations…" row fetches the next page.
  const conversations = createConversationSearch((q, cursor, limit) => api.searchSessions(q, cursor, limit, workspaceId()));
  let inputRef: HTMLInputElement | undefined;
  let listRef: HTMLDivElement | undefined;

  // Exit choreography — hold the DOM 140ms after close so the dialog
  // settles back into the slab instead of blinking out. The content
  // gates below read `phase()` (not the open signal) so rows don't
  // blank mid-fade; the keydown handler stays gated on the open signal,
  // so keys release the instant the palette closes. Blur on closing so
  // a fast typist's next keystrokes never land in the dying input.
  const phase = createExitPhase(commandPaletteOpen, 140);
  createEffect(() => { if (phase() === 'closing') inputRef?.blur(); });
  // Memoized boolean so 'open' → 'closing' is invisible to the content:
  // items/search read visible(), and memo equality stops the notification
  // — the held DOM must not recompute (selection snap, row recreation)
  // at the exact moment the fade starts.
  const visible = createMemo(() => phase() !== 'closed');

  // Debounced full-text search (FE-06). Fires only for queries >= 3 chars;
  // shorter queries clear the async layer and lean on filename matching.
  // Every keystroke cancels the request in flight (`latestRequest`), so an
  // older response can never replace a newer one; the previous rows stay
  // visible while the new request runs (audit H09).
  const vaultOrder = latestRequest();
  const projectOrder = latestRequest();
  let conversationTimer: ReturnType<typeof setTimeout> | undefined;
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let projectTimer: ReturnType<typeof setTimeout> | undefined;
  createEffect(() => {
    const q = query().trim();
    // Only in scopes that list its rows (P1-48: Projects threw them away).
    const gone = !visible() || !wantsVaultSearch(scope());
    vaultOrder.cancel();
    clearTimeout(searchTimer);
    if (gone || q.length < 3) {
      setSearchResults([]);
      setSearchPhase('idle');
      return;
    }
    setSearchPhase('pending');
    searchTimer = setTimeout(() => runLatest(vaultOrder, () => api.vaultSearch(q), (o) => {
      if (o.ok) { setSearchResults(o.value.filter(isWorkspaceVaultItem)); setSearchPhase('done'); }
      else { setSearchResults([]); setSearchPhase('error'); }
    }), 180);
  });
  // Connected-project files by name or path (P07), from 2 characters.
  createEffect(() => {
    const q = query().trim();
    const want = visible() && paletteScope() === 'all' && wantsProjectFiles(scope());
    const limit = scope() === 'projects' ? 30 : 6;
    projectOrder.cancel();
    clearTimeout(projectTimer);
    if (!want || q.length < 2) {
      setProjectHits({ files: [], direct: [] });
      setProjectPhase('idle');
      return;
    }
    setProjectPhase('pending');
    projectTimer = setTimeout(() => runLatest(projectOrder, () => api.connectedFiles({ query: q, limit }), (o) => {
      if (o.ok) { setProjectHits({ files: o.value.files, direct: o.value.direct }); setProjectPhase('done'); }
      else { setProjectHits({ files: [], direct: [] }); setProjectPhase('error'); }
    }), 180);
  });
  // Conversations by title, every workspace (B·F15), from 2 characters.
  createEffect(() => {
    const q = query().trim();
    const want = visible() && paletteScope() === 'all' && wantsConversationSearch(scope(), q);
    conversations.cancel();
    clearTimeout(conversationTimer);
    if (!want) { conversations.clear(); return; }
    conversations.waiting();
    conversationTimer = setTimeout(() => void conversations.search(q), 180);
  });
  onCleanup(() => { clearTimeout(searchTimer); clearTimeout(projectTimer); clearTimeout(conversationTimer); vaultOrder.cancel(); projectOrder.cancel(); conversations.cancel(); });

  // Build the item set fresh from current signals on each open. Cached
  // until close so arrow-key navigation stays stable.
  // Palette rows that differ from their shared action (actionDefs.ts): a
  // `run` here is the def's `paletteOwn` (its reason is there); a `label`
  // names the object the row acts on. routeLedger.test.ts checks the two
  // lists agree.
  const PALETTE_OWN: Record<string, { label?: () => string; run?: () => unknown }> = {
    'act:new-conversation': { run: () => queueMicrotask(() => { if (workspaceId()) startNewConversation(); else { openCommandPalette('new-conversation'); setPaletteReturnsToRoot(true); } }) },
    'act:jump': { run: () => jumpChip() },
    'act:ask-vault': { run: () => { const q = query().trim(); openHomeAsk(looksLikeSentence(q) ? q : ''); } },
    'act:new-workspace-from-chat': { run: () => { const key = paletteMoveKey(); if (key) return newWorkspaceFromChat(key, null); } },
    // Names the canvas it will export.
    'act:save-canvas-image': { label: () => `Export canvas “${lastCanvas()!.title.replace(/\s+/g, ' ').slice(0, 60)}”…` },
    // Says which way it will go.
    'act:toggle-side-pane': { label: () => previewShown() ? 'Hide preview pane' : 'Show preview pane' },
  };

  const items = createMemo<PaletteItem[]>(() => {
    if (!visible()) return [];

    // New conversation… (frontier F4): only "which workspace", Quick chat first.
    if (paletteScope() === 'new-conversation') return newConversationItems();
    // A conversation's Move to… (2026-09-30): only where it can go.
    if (paletteScope() === 'move-session') return []; // Legacy requests redirect to the shared project picker.
    // A workspace's Merge into… (master plan #18): only where it can merge.
    if (paletteScope() === 'merge-workspace') return mergeWorkspaceItems();

    const all: PaletteItem[] = [];

    // ── Navigation ─────────────────────────────────────────────────
    for (const p of NAV_PAGES.filter(p => !p.hidden)) {
      all.push({
        id: `nav:${p.id}`,
        label: p.label,
        icon: p.icon,
        group: 'Navigation',
        // "go"/"open" stay searchable now the label has dropped the prefix.
        keywords: `${p.keywords} go open`,
        shortcut: pageShortcut(p.id),
        run: () => goToPage(p.id),
      });
    }

    // ── Actions (actionDefs.ts) ────────────────────────────────────
    // The shared action definitions, offered when their condition holds,
    // with their words, look and key hint; choosing one runs the same
    // dispatcher as its ⌘ key, `g` key and menu item (runAction). A row
    // whose palette behaviour differs says why in PALETTE_OWN. A run or
    // write row says what it does under its label (P0-02).
    for (const def of ACTION_DEFS) {
      if (!def.palette || def.paletteIn || !actionAvailable(def)) continue;
      const own = PALETTE_OWN[def.palette];
      all.push({
        id: def.palette, label: own?.label?.() ?? actionLabel(def), icon: def.icon, group: def.group ?? 'Actions', keywords: def.keywords,
        detail: paletteEffectLine(def.palette),
        shortcut: actionHint(def), run: own?.run ?? (() => runAction(def.id)),
      });
    }

    // ── Conversations, by title or workspace (frontier F6) ─────────
    const titles = new Map(workspaces().map(w => [w.id, w.title]));
    const conversations = Object.values(workspaceSessions())
      .flatMap(g => g.sessions.map(session => ({ session, workspaceId: g.workspaceId })))
      .sort((a, b) => (b.session.lastActivityAt || '').localeCompare(a.session.lastActivityAt || ''));
    for (const { session, workspaceId: wsId } of conversations) {
      const where = titles.get(wsId) ?? '';
      all.push({
        id: `conversation:${session.key}`,
        label: session.title?.trim() || 'Conversation',
        hint: where,
        icon: 'message',
        group: 'Conversations',
        keywords: `${where} conversation chat thread ${session.agent}`,
        run: () => visitWorkspace(wsId, session.key),
      });
    }

    // ── Settings sections ─────────────────────────────────────────
    // One jump row per section (from the pure settingsSections registry —
    // no Settings-chunk import). Only surfaces on query, so the empty
    // state stays curated.
    for (const grp of SETTINGS_SECTION_GROUPS) {
      for (const s of grp.items) {
        all.push({
          id: `settings:${s.id}`,
          label: `Settings: ${s.label}`,
          icon: s.icon,
          group: 'Settings',
          keywords: `settings ${grp.label} ${s.keywords}`,
          run: () => { openSettingsFromOutside(s.id); },
        });
        // Controls inside the section (audit H06), e.g. "Motion & atmosphere";
        // a control in a part (a former section, S2) names its part too.
        for (const c of s.controls ?? []) {
          const part = !c.heading && c.part ? s.parts?.find(p => p.id === c.part)?.label : undefined;
          all.push({
            id: `settings:${s.id}:${c.label}`,
            label: `Settings: ${s.label} › ${part ? `${part} › ` : ''}${c.label}`,
            icon: s.icon,
            group: 'Settings',
            keywords: `settings ${s.label} ${part ?? ''} ${c.label} ${c.keywords ?? ''}`,
            run: () => { openSettingsFromOutside(s.id, c.label); },
          });
        }
      }
    }

    // ── Recent notes (from vault tree, up to 20) ──────────────────
    const tree = vaultTree();
    if (tree) {
      const files: { path: string; name: string; mtime: string }[] = [];
      walkTree(tree, files);
      files.sort((a, b) => b.mtime.localeCompare(a.mtime));
      for (const f of files.slice(0, 20)) {
        all.push({
          id: `note:${f.path}`,
          label: f.name.replace(/\.md$/, ''),
          hint: f.path,
          icon: 'book',
          group: 'Notes',
          preview: { kind: 'Note', title: f.name.replace(/\.md$/, ''), path: f.path },
          keywords: f.path,
          run: () => { setActivePage('vault'); requestOpenVaultNote(f.path); },
        });
      }
    }

    // ── Tasks ──────────────────────────────────────────────────────
    // Open work only, ranked by the shared ranker — the raw list is
    // due-date-sorted and mostly finished tasks, so an unfiltered slice
    // offered rows reading "Done" over and over. Status is implicit now,
    // so the hint carries the due date instead.
    const tasks = rankTasks((tasksData()?.items ?? []).filter(t => isOpenTask(t.status)));
    for (const t of tasks.slice(0, 12)) {
      all.push({
        id: `task:${t.url}`,
        label: t.title,
        hint: [dueLabel(t, true), t.priority].filter(Boolean).join(' · '),
        icon: 'checkSquare',
        group: 'Tasks',
        preview: { kind: 'Task', title: t.title, body: t.description, facts: [t.status, dueLabel(t, true), t.priority, t.projectName].filter(Boolean).join(' · ') },
        keywords: `${t.projectName ?? ''} ${t.description ?? ''}`,
        run: () => openModal(t),
      });
    }

    for (const w of workspaces()) {
      // Twins say where each comes from (consolidation E).
      // Its origin is searchable too ("q3 imported"), L4 X14.
      all.push({id: `workspace:${w.id}`, label: w.title, hint: distinguishing(w, workspaces()) ?? w.status, icon: 'folder', group: 'Workspaces',
        preview: { kind: 'Workspace brief', title: w.title, path: `${w.path}/WORKSPACE.md`, facts: w.status },
        keywords: `workspace outcome resume ${workspaceOrigin(w)}`, run: () => visitWorkspace(w.id)});
    }

    // ── Skills ────────────────────────────────────────────────────
    // Skills live under the Workflows tab now — selecting one lands the
    // user on the Workflows page with the Skills sub-tab active AND that
    // skill's detail pane open (via the skillOpenRequest signal).
    const skills = skillsList() ?? [];
    for (const s of skills.slice(0, 30)) {
      all.push({
        id: `skill:${s.name}`,
        label: s.name,
        hint: s.description?.slice(0, 90),
        icon: 'zap',
        group: 'Skills',
        run: () => { void openLibrarySkill(s.name); },
      });
    }

    // Sessions intentionally omitted — they live only inside Chat now.

    return all;
  });

  function newConversationItems(): PaletteItem[] {
    const rows: PaletteItem[] = workspaceChoices(workspaces(), railWorkspaces()).map(c => c.quick
      ? { id: 'new-conversation:quick', label: 'Quick chat', hint: 'No project', icon: 'message', group: 'Workspaces',
          keywords: 'quick chat chats scratch ask anything no project no workspace', run: () => startQuickChat() }
      : { id: `new-conversation:${c.id}`, label: c.title, hint: c.hint, icon: 'folder', group: 'Workspaces',
          keywords: 'workspace new conversation', run: () => newWorkspaceSession(c.id) });
    const nw = ACTION_BY_PALETTE['act:new-workspace'];
    rows.push({ id: nw.palette!, label: actionLabel(nw), icon: nw.icon, group: 'Actions', keywords: nw.keywords, run: () => runAction(nw.id) });
    return rows;
  }

  function mergeWorkspaceItems(): PaletteItem[] {
    const from = paletteMergeFrom();
    if (!from) return [];
    return mergeTargets(workspaces(), railWorkspaces(), from).map(c => ({
      id: `merge-workspace:${c.id}`, label: c.title, hint: c.hint, icon: 'folder', group: 'Workspaces',
      keywords: 'workspace merge combine', run: () => mergeWorkspaceInto(from, c.id),
    }));
  }

  /// Items filtered by query, sorted by descending score. When the
  /// query is empty, show a small curated set (Navigation + Actions).
  const filtered = createMemo<PaletteItem[]>(() => {
    const q = query().trim();
    const all = items();
    const sc = scope();
    if (!q && paletteScope() !== 'all') return all;
    // A scope chip with no query lists what is in that scope (recent notes,
    // workspaces and conversations); Projects waits for a name.
    if (!q && sc !== 'all') return all.filter(i => inScope(sc, i.group) && !i.id.startsWith('workspace-section:'));
    if (!q) return paletteDefaultOrder(all, recentIds(), chatAgentSessionKey());
    // ── Natural-language task queries ────────────────────────────────
    // "overdue & p1", "today @urgent", "#hollow & active" — when the query
    // uses the task-filter grammar, evaluate it for real over the full task
    // set (the same matcher the Tasks FilterBar uses) instead of fuzzy
    // text-matching, and lead with those rows. Whitespace between clauses
    // counts as & so "overdue p1" works as spoken.
    const tq = sc === 'all' ? toTaskQuery(q) : null;
    if (tq) {
      const matches = rankTasks(
        (tasksData()?.items ?? []).filter(t => matchesTaskQuery(t, tq)),
      ).slice(0, 12);
      if (matches.length > 0) {
        const rows: PaletteItem[] = matches.map((t: Task) => ({
          id: `taskq:${t.url}`,
          label: t.title,
          hint: [dueLabel(t), t.priority, t.projectName].filter(Boolean).join(' · '),
          icon: 'checkSquare',
          group: 'Task matches',
          preview: { kind: 'Task', title: t.title, body: t.description, facts: [t.status, dueLabel(t), t.priority, t.projectName].filter(Boolean).join(' · ') },
          run: () => openModal(t),
        }));
        // The grammar matched — task rows ARE the answer; everything else
        // (fuzzy command/note hits) follows below for escape hatches.
        const rest = all
          .map(i => ({ i, s: score(q, i) }))
          .filter(x => x.s > 0 && !x.i.id.startsWith('task:'))
          .sort((a, b) => b.s - a.s)
          .slice(0, 10)
          .map(x => x.i);
        return withAsk(q, [...rows, ...rest], 1000);
      }
    }

    const scored = all
      .map(i => ({ i, s: score(q, i) }))
      .filter(x => x.s > 0 && inScope(sc, x.i.group))
      .sort((a, b) => b.s - a.s)
      .slice(0, 40)
      .map(x => x.i);
    const titles = new Map(workspaces().map(w => [w.id, w.title] as const));
    // Conversations found by title or message in every workspace (B·F15),
    // not only the rail's 25 newest, after the instant rows that didn't
    // already list them; each opens by its exact key. A hit by what was
    // asked shows that message's snippet as its second line.
    const listedConversations = new Set(scored.filter(i => i.id.startsWith('conversation:')).map(i => i.id.slice('conversation:'.length)));
    const conversationItems: PaletteItem[] = searchedConversations(conversations.hits(), listedConversations).map(c => ({
      id: `conversation:${c.key}`, label: c.title, hint: titles.get(c.workspaceId) ?? '', icon: 'message', group: 'Conversations',
      detail: c.detail, keywords: `conversation chat thread ${c.agent}`, run: () => visitWorkspace(c.workspaceId, c.key),
    }));
    // More matches than the first page: fetch the next one in place, then
    // select the first row it added (the list change reset it to row 0).
    if (conversations.next() != null) conversationItems.push({ id: 'conversations:more', label: 'More conversations…', icon: 'chevronDown',
      group: 'Conversations', keepOpen: true, run: () => moreConversations() });

    // Append async full-text matches as a separate "Search results" group
    // below the instant filename matches. Dedupe against note rows already
    // shown above (same vault path) so a file isn't listed twice.
    const seenPaths = new Set(
      scored.filter(i => i.id.startsWith('note:')).map(i => i.id.slice('note:'.length)),
    );
    const searchItems: PaletteItem[] = [];
    for (const r of searchResults()) {
      if (seenPaths.has(r.path)) continue;
      const result = isWorkspaceResult(r.path);
      const item: PaletteItem = {
        id: `search:${r.path}`,
        preview: /\.md$/i.test(r.path) ? { kind: 'Note', title: r.name.replace(/\.md$/, ''), path: r.path } : undefined,
        label: r.name.replace(/\.md$/, ''),
        detail: excerptLine(r.snippet) || r.path,
        source: vaultSourceLabel(r.path, id => titles.get(id)),
        icon: result ? 'fileText' : 'search',
        group: result ? 'Results' : 'Search results',
        keywords: r.path,
        run: () => { setActivePage('vault'); requestOpenVaultNote(r.path); },
      };
      if (inScope(sc, item.group)) searchItems.push(item);
    }
    // Notes before Results, so each group renders under one heading.
    searchItems.sort((a, b) => Number(a.group === 'Results') - Number(b.group === 'Results'));
    const projectItems: PaletteItem[] = [];
    if (inScope(sc, 'Project files')) {
      const hits = projectHits();
      for (const [f, direct] of [...hits.files.map(f => [f, false] as const), ...hits.direct.map(f => [f, true] as const)]) {
        // A folder with no workspace yet (overnight 10-11 Y1) is named by its top folder.
        const row = projectFileRow(f, titles.get(f.projectId) ?? (f.relative.includes('/') ? f.relative.split('/')[0] : undefined), direct);
        projectItems.push({
          id: `project-file:${f.path}`, label: row.label, detail: row.detail, source: row.source,
          icon: 'fileText', group: 'Project files', keywords: f.relative,
          run: () => { setActivePage('vault'); setLibraryCollectionRequest(`connected-file:${JSON.stringify({ path: f.path, find: q })}`); },
        });
      }
    }
    const rows = [...scored, ...conversationItems, ...searchItems.slice(0, 20), ...projectItems.slice(0, sc === 'projects' ? 30 : 6)];
    return sc === 'all' ? withAsk(q, rows, scored.length ? score(q, scored[0]) : undefined) : rows;
  });

  /// The command bar's Ask row (overnight 10-11 A2; One Ask, 2026-11-03): a
  /// query that reads as a sentence gets one row that asks it the way Your
  /// work's Ask field would, in the place chosen there (store/ask.ts
  /// `askHere`), first unless a strong label match leads (that keeps Enter).
  /// Its hint names that place as Home shows it (`askPlaceLabel`: "Quick
  /// chat", "Quick chat in <project>", "Across all my work").
  function withAsk(q: string, rows: PaletteItem[], best: number | undefined): PaletteItem[] {
    if (!looksLikeSentence(q)) return rows;
    const ask: PaletteItem = {
      id: 'ask:sentence', label: `Ask: “${q.length > 80 ? `${q.slice(0, 79)}…` : q}”`, icon: 'wand', group: 'Ask',
      hint: askPlaceLabel(), run: () => { closeCommandPalette(); void askHere(q); },
    };
    const at = Math.min(askRowIndex(best), rows.length);
    return [...rows.slice(0, at), ask, ...rows.slice(at)];
  }

  // When the rows change, keep the highlight on the row it was on; reset it
  // to the top only when the query or scope changed (P0-26: a live sessions
  // update used to snap it to row 0, so Enter could run "Stop the running
  // turn"). `selectedIdx` still indexes the previous rows here.
  let lastInput = '';
  createEffect(on(filtered, (list, prev) => {
    const input = `${paletteScope()}\u0000${scope()}\u0000${query()}`;
    const picked = prev?.[untrack(selectedIdx)]?.id;
    setSelectedIdx(keptSelection(list.map(i => i.id), picked, input === lastInput));
    lastInput = input;
  }));

  // "More conversations…": append the next page and keep the selection on
  // its first row. Skipped when the page was dropped or the query changed.
  async function moreConversations() {
    const q = query();
    const before = new Set(filtered().map(i => i.id));
    if (!(await conversations.more()) || query() !== q) return;
    const at = firstNewConversationRow(filtered().map(i => i.id), before);
    if (at >= 0) setSelectedIdx(at);
  }

  // What had focus when it opened: dismissing (Escape, the backdrop) gives
  // focus back there (H12: over the Main agent drawer it fell to the body),
  // and so does a row that doesn't navigate (execute(), A05).
  // Read only as it opens (`on`), before the input takes focus.
  let opener: HTMLElement | null = null;
  createEffect(on(commandPaletteOpen, open => { if (open) opener = focusOpener('.palette-backdrop'); }));
  const dismiss = () => { const back = opener; opener = null; closeCommandPalette(); giveFocusBack(back, '.palette-backdrop'); };
  /** Jump (⌘J, its row, the menu): the palette on its Workspaces chip. */
  function jumpChip() { openCommandPalette('jump'); setQuery(''); setScope('workspaces'); inputRef?.focus(); }
  /** ⌘1 to ⌘9 while the palette is open: choose the row that digit labels (a
   *  local key; the page digits keep their meaning once it is closed). */
  function chooseDigit(digit: number) {
    const at = rowForDigit(digit, filtered().length);
    const item = at == null ? undefined : filtered()[at];
    if (item) execute(item);
  }
  /** Backspace in an empty box, or the footer's Back (any text goes with it): one level up. */
  function goBack(button = false): boolean {
    const to = backTarget({ query: button ? '' : query(), scope: scope(), subpageFromRoot: paletteReturnsToRoot() });
    if (!to) return false;
    if (to === 'to-root') { setPaletteReturnsToRoot(false); openCommandPalette(); }
    setQuery('');
    if (inputRef) inputRef.value = '';
    setScope('all');
    setSelectedIdx(0);
    inputRef?.focus();
    return true;
  }
  const canGoBack = () => paletteReturnsToRoot() || scope() !== 'all';
  /** The box's text: a leading `>` is the Actions scope, not part of the query. */
  function typed(value: string) {
    const { actions, query: rest } = splitActionPrefix(value);
    if (actions && scope() === 'all' && paletteScope() === 'all') { setScope('actions'); setQuery(rest); if (inputRef) inputRef.value = rest; return; }
    setQuery(value);
  }
  /** The palette's own commands, by key or by the menu (P1-48); a held key
   *  is taken but runs once. */
  const ownCommand = (id: string, repeat = false): boolean => {
    // A page item's accelerator (⌘1 …) reaches the native menu before the
    // page: with the palette open it chooses that digit's row instead.
    const digit = digitOfMenuCommand(id);
    if (digit != null) { if (!repeat) chooseDigit(digit); return true; }
    if (id !== 'search' && id !== 'jump') return false;
    if (!repeat) { if (id === 'search') dismiss(); else jumpChip(); }
    return true;
  };
  onCleanup(registerPaletteCommand(id => ownCommand(id)));

  // Focus the input + reset state whenever the palette opens.
  createEffect(() => {
    if (commandPaletteOpen()) {
      void refreshWorkspaces();
      // The sidebar's Search everything opens on its words (overnight 11-04 S2).
      setQuery(untrack(paletteQueryRequest) ?? '');
      setPaletteQueryRequest(null);
      setScope(untrack(paletteSearchRequest) ?? 'all');
      setSelectedIdx(0);
      queueMicrotask(() => inputRef?.focus());
    }
  });

  // Keep the selected row scrolled into view.
  createEffect(() => {
    const idx = selectedIdx();
    if (!commandPaletteOpen() || !listRef) return;
    const row = listRef.querySelector(`[data-palette-idx="${idx}"]`) as HTMLElement | null;
    row?.scrollIntoView({ block: 'nearest' });
  });

  // The single execution choke point — Enter and click both land here.
  // recordRecent is deferred past the 140ms exit hold: the empty-query
  // `filtered` reads recentIds(), and a synchronous write would visibly
  // reshuffle the Recent group in the dying DOM mid-fade. A row that acts
  // without asking (Stop, Make a canvas, Clear debug logs) isn't recorded,
  // so ⌘K ↵ never repeats it (P0-02).
  // A row that doesn't move the person (a toggle, a copy) gives focus back to
  // the opener once it settles, as Escape does (A05); one that changes the
  // page, workspace or view (the location stamp) leaves focus where it went.
  function execute(item: PaletteItem) {
    if (item.keepOpen) {
      Promise.resolve(item.run()).catch(err => flashFooter(`× ${err?.message || err}`, 3500));
      return;
    }
    const back = opener; opener = null;
    const stamp = locationStamp();
    closeCommandPalette();
    if (joinsRecent(item.id)) setTimeout(() => recordRecent(item.id), 180);
    Promise.resolve(item.run()).catch(err => {
      flashFooter(`× ${err?.message || err}`, 3500);
    }).finally(() => giveFocusBackAfterRun(back, '.palette-backdrop', stamp, locationStamp));
  }

  // Global keyboard handler — only while open.
  createEffect(() => {
    if (!commandPaletteOpen()) return;
    const onKey = (e: KeyboardEvent) => {
      const list = filtered();
      // Its own keys (P1-47 rule, P1-48): the palette's chord closes it and
      // gives focus back; Jump's switches it to the Workspaces chip. App's
      // keymap stands down behind the palette, so it must answer them here.
      const own = shortcutFor(e, PALETTE_CONTEXTS)?.id;
      if (own && ownCommand(own, e.repeat)) { e.preventDefault(); return; }
      // Reading keys belong to the preview; palette shortcuts still answer.
      if (['Enter', 'ArrowDown', 'ArrowUp'].includes(e.key) && (e.target as Element)?.closest?.('.palette-preview')) return;
      const digit = digitOfKey(e);
      if (digit != null) {
        // Local to the open palette; App's keymap stands down behind it.
        e.preventDefault();
        if (!e.repeat) chooseDigit(digit);
        return;
      }
      if (e.key === 'Backspace' && !e.repeat && !e.metaKey && !e.ctrlKey && !e.altKey && !e.isComposing && !(e.target as Element)?.closest?.('.palette-preview') && goBack()) {
        e.preventDefault();
      } else if (e.key === 'Escape') {
        e.preventDefault();
        dismiss();
      } else if (e.key === 'ArrowDown') {
        e.preventDefault();
        setSelectedIdx(i => Math.min(i + 1, Math.max(list.length - 1, 0)));
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        setSelectedIdx(i => Math.max(i - 1, 0));
      } else if (e.key === 'Enter') {
        e.preventDefault();
        const item = list[selectedIdx()];
        if (item) execute(item);
      }
    };
    document.addEventListener('keydown', onKey, true);
    onCleanup(() => document.removeEventListener('keydown', onKey, true));
  });

  return (
    <Show when={phase() !== 'closed'}>
      <div class="palette-backdrop" classList={{ 'is-closing': phase() === 'closing' }} onClick={dismiss}>
        <div
          class="palette-dialog"
          classList={{ 'has-preview': previewShown() }}
          role="dialog"
          aria-modal="true"
          aria-label="Command palette"
          onClick={(e) => e.stopPropagation()}
        >
          <div class="palette-input-wrap">
            <span class="palette-search-icon"><Icon name="search" size={15} /></span>
            <input
              ref={inputRef}
              type="text"
              class="palette-input"
              value={query()}
              placeholder={scope() === 'actions' && paletteScope() === 'all' ? 'Run an action…' : paletteScope() === 'new-conversation' ? 'New conversation in…' : paletteScope() === 'move-session' ? 'Move conversation to…' : paletteScope() === 'merge-workspace' ? 'Merge this workspace into…' : 'Find a conversation, work, a source, or an action…'}
              aria-label={scope() === 'actions' && paletteScope() === 'all' ? 'Run an action' : paletteScope() === 'new-conversation' ? 'Choose a workspace for the new conversation' : paletteScope() === 'move-session' ? 'Choose where to move the conversation' : paletteScope() === 'merge-workspace' ? 'Choose the workspace to merge into' : 'Search'}
              // The arrow keys move a selection the input never leaves
              // (overnight B8): a combobox naming the selected option, so a
              // screen reader says which row Enter will run.
              role="combobox"
              aria-expanded={filtered().length > 0}
              aria-controls="palette-listbox"
              aria-autocomplete="list"
              aria-activedescendant={filtered().length > 0 ? `palette-opt-${selectedIdx()}` : undefined}
              onInput={(e) => typed(e.currentTarget.value)}
            />
          </div>
          <Show when={paletteScope() === 'all'}>
            <div class="palette-scopes">
              <Segmented label="Search in" class="palette-scope" value={scope()} items={PALETTE_SCOPES}
                onChange={(v) => { setScope(v); inputRef?.focus(); }} />
            </div>
          </Show>
          <div class="palette-body">
          <div class="palette-list" ref={listRef} id="palette-listbox" role={filtered().length > 0 ? 'listbox' : undefined} aria-label={filtered().length > 0 ? 'Results' : undefined} aria-busy={searchPhase() === 'pending' || projectPhase() === 'pending' || conversations.phase() === 'pending'}>
            <Show when={filtered().length > 0} fallback={
              <div class="palette-empty" role="status"><EmptyArt kind="search" compact evidence={{ready:searchPhase() !== 'pending' && projectPhase() !== 'pending' && conversations.phase() !== 'pending', failed:searchPhase() === 'error' || projectPhase() === 'error' || conversations.phase() === 'error', partial:scope() === 'projects' && query().trim().length < 2}}/>{searchPhase() === 'pending' || projectPhase() === 'pending' ? 'Searching your files…' : searchPhase() === 'error' || projectPhase() === 'error' ? 'File search didn’t finish. Actions and names are still listed as you type.' : conversations.phase() === 'pending' ? 'Searching your conversations…' : conversations.phase() === 'error' ? 'Conversation search didn’t finish. Actions and names are still listed as you type.' : scope() === 'projects' && query().trim().length < 2 ? 'Type a file name or path to look across your connected projects.' : 'No matches. Try a different term.'}</div>
            }>
              <PaletteRows items={filtered()} query={query().trim()} selectedIdx={selectedIdx()} onSelect={(i) => {
                const item = filtered()[i];
                if (item) execute(item);
              }} onHover={setSelectedIdx} />
            </Show>
          </div>
          <Show when={commandPaletteOpen() && previewShown()}>
            <ErrorBoundary fallback={<aside class="palette-preview">Preview unavailable. Results still open normally.</aside>}>
              <Suspense fallback={<aside class="palette-preview">Loading preview…</aside>}>
                <PalettePreview target={filtered()[selectedIdx()]?.preview} query={query()} returnFocus={() => { if (commandPaletteOpen()) inputRef?.focus(); }} />
              </Suspense>
            </ErrorBoundary>
          </Show>
          </div>
          <div class="palette-foot">
            <span><kbd>↑↓</kbd> navigate</span>
            <span><kbd>↵</kbd> open</span>
            <span><kbd>{MOD}1</kbd> to <kbd>9</kbd> pick a row</span>
            <Show when={paletteScope() === 'all'}><span><kbd>&gt;</kbd> actions</span></Show>
            <Show when={canGoBack()}><span><kbd>⌫</kbd> back</span></Show>
            <span><kbd>esc</kbd> close</span>
            {/* With rows on show, a failed conversation search still says so. */}
            <Show when={filtered().length > 0 && conversations.phase() === 'error'}><span role="status">Conversation search didn’t finish</span></Show>
            <span class="palette-foot-spacer" />
            <Show when={canGoBack()}><Button size="sm" variant="ghost" onClick={() => goBack(true)}>Back</Button></Show>
            {/* No `?` hint: the focused input swallows bare keys, so it can't
                fire from inside the palette. A button opens the card (P1-48). */}
            <Button size="sm" variant="ghost" onClick={() => { closeCommandPalette(); setShortcutsOverlayOpen(true); }}>Keyboard shortcuts</Button>
          </div>
        </div>
      </div>
    </Show>
  );
}

/// Render rows grouped by their `group` field. Each row carries its
/// absolute index in the filtered list so keyboard nav matches.
function PaletteRows(props: {
  items: PaletteItem[];
  query?: string;
  selectedIdx: number;
  onSelect: (idx: number) => void;
  onHover: (idx: number) => void;
}) {
  const grouped = createMemo(() => {
    const out: { group: string; items: { item: PaletteItem; idx: number }[] }[] = [];
    let cur: typeof out[number] | null = null;
    props.items.forEach((item, idx) => {
      if (!cur || cur.group !== item.group) {
        cur = { group: item.group, items: [] };
        out.push(cur);
      }
      cur.items.push({ item, idx });
    });
    return out;
  });
  return (
    <For each={grouped()}>
      {(grp) => (
        <div class="palette-group" role="group" aria-label={grp.group}>
          <div class="rail-group-label palette-group-label" aria-hidden="true">
            <span>{grp.group}</span>
            <span class="rail-group-count">{grp.items.length}</span>
          </div>
          <For each={grp.items}>
            {({ item, idx }) => (
              <button
                type="button"
                class={`palette-row ${idx === props.selectedIdx ? 'selected' : ''}`}
                id={`palette-opt-${idx}`}
                role="option"
                aria-selected={idx === props.selectedIdx}
                tabIndex={-1}
                aria-keyshortcuts={rowDigit(idx) ? `${MOD === '⌘' ? 'Meta' : 'Control'}+${rowDigit(idx)}` : undefined}
                data-palette-idx={idx}
                onMouseEnter={() => props.onHover(idx)}
                onClick={() => props.onSelect(idx)}
              >
                <Show when={item.icon}>
                  <span class="palette-row-icon"><Icon name={item.icon!} size={14} /></span>
                </Show>
                {/* The matched substring catches light — keyword-only
                    matches (no label range) render plain. */}
                <Show when={item.detail} fallback={<RowLabel query={props.query} label={item.label} />}>
                  <span class="palette-row-text">
                    <RowLabel query={props.query} label={item.label} />
                    <span class="palette-row-detail">{item.detail}</span>
                  </span>
                </Show>
                <Show when={item.hint}>
                  <span class="palette-row-hint">{item.hint}</span>
                </Show>
                <Show when={item.source}>
                  <span class="palette-row-source">{item.source}</span>
                </Show>
                <Show when={item.shortcut}>
                  <kbd class="palette-row-shortcut">{item.shortcut}</kbd>
                </Show>
                <Show when={rowDigit(idx)}>
                  <kbd class="palette-row-digit" aria-hidden="true" title={`${MOD}${rowDigit(idx)} chooses this row`}>{rowDigit(idx)}</kbd>
                </Show>
              </button>
            )}
          </For>
        </div>
      )}
    </For>
  );
}

/// A row's label with the matched substring lit.
function RowLabel(props: { query?: string; label: string }) {
  return (
    <span class="palette-row-label">
      <Show when={matchRange(props.query ?? '', props.label)} keyed fallback={props.label}>
        {([s, e]) => (
          <>
            {props.label.slice(0, s)}
            <span class="palette-row-match">{props.label.slice(s, e)}</span>
            {props.label.slice(e)}
          </>
        )}
      </Show>
    </span>
  );
}

/// Walk a VaultTree (nested folder/file nodes) and collect File entries.
function walkTree(tree: VaultTree, out: { path: string; name: string; mtime: string }[]) {
  if (!tree) return;
  for (const k of Object.keys(tree)) {
    const node = tree[k];
    if (node?.kind === 'file') out.push({ path: node.path, name: node.name, mtime: node.mtime });
    else if (node?.kind === 'folder' && node.children) walkTree(node.children, out);
  }
}
