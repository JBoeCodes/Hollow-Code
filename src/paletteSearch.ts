/**
 * Command-palette search scopes and result rows (next-level P07). The palette
 * mixes instant rows (actions, recent notes, workspaces) with two async
 * lookups: vault full-text search, the connected-project inventory
 * (`connected_files`) and conversations by title or message (`search_sessions`). A scope chip narrows what is listed; every search row
 * says where it came from and shows an excerpt or its folder.
 */
import { createSignal } from 'solid-js';
import type { ConnectedFile } from './types';
import type { SessionSearchPage, SessionTarget } from './workspaceTypes';
import { connectedFileTag } from './connectedFiles';
import { isWorkspaceSnapshot } from './sourceAuthority';
import { latestRequest } from './latestRequest';

export type PaletteSearchScope = 'all' | 'actions' | 'notes' | 'projects' | 'workspaces';

export const PALETTE_SCOPES: readonly { value: PaletteSearchScope; label: string }[] = [
  { value: 'all', label: 'All' },
  // Typing `>` first opens it too (paletteNav.ts); the chip is its click route.
  { value: 'actions', label: 'Actions' },
  { value: 'notes', label: 'Notes' },
  { value: 'projects', label: 'Projects' },
  { value: 'workspaces', label: 'Workspaces' },
];

/** Groups each scope lists. `all` lists everything. */
const SCOPE_GROUPS: Record<Exclude<PaletteSearchScope, 'all'>, readonly string[]> = {
  actions: ['Actions', 'Navigation', 'Conversation'],
  notes: ['Notes', 'Search results'],
  projects: ['Project files'],
  workspaces: ['Workspaces', 'Conversations', 'Results'],
};

export function inScope(scope: PaletteSearchScope, group: string): boolean {
  return scope === 'all' || SCOPE_GROUPS[scope].includes(group);
}

/** Whether a scope lists vault search's rows (Search results, Results), so
 *  the search is worth running at all (P1-48). */
export function wantsVaultSearch(scope: PaletteSearchScope): boolean {
  return inScope(scope, 'Search results') || inScope(scope, 'Results');
}

/** Words that join a phrase and match nothing ("go to tasks"), dropped from
 *  a phrase of three words or more when other words are left. */
const FILLER = new Set(['to', 'the', 'a', 'an', 'my', 'for', 'of']);
const words = (s: string) => s.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean);

/** How well `query` matches a row, higher is better (0 = no match).
 *  One word keeps the old tiers: exact label 1000, label starts with it 700,
 *  a label word starts with it 500, inside the label 300, inside the
 *  keywords 100. Several words (P1-48) match only when every word starts a
 *  word of the label or keywords, in any order: 600 plus 10 per word found in
 *  the label (so label hits rank first), or 150 when only keywords matched;
 *  the old tiers still apply to the whole phrase. */
export function paletteScore(query: string, item: { label: string; keywords?: string }): number {
  const q = query.toLowerCase().trim();
  if (!q) return 0;
  const label = item.label.toLowerCase();
  if (label === q) return 1000;
  if (label.startsWith(q)) return 700;
  const tokens = words(q);
  // Only in a phrase of three words or more: "to do" keeps both.
  const kept = tokens.length > 2 ? tokens.filter(t => !FILLER.has(t)) : tokens;
  const typed = kept.length ? kept : tokens;
  if (tokens.length > 1) {
    const inLabel = words(label), inKeywords = words(item.keywords ?? '');
    let labelHits = 0, all = true;
    for (const t of typed) {
      if (inLabel.some(w => w.startsWith(t))) labelHits++;
      else if (!inKeywords.some(w => w.startsWith(t))) { all = false; break; }
    }
    if (all) return labelHits ? Math.min(690, 600 + labelHits * 10) : 150;
  }
  if (label.split(/[^a-z0-9]+/i).some(w => w.startsWith(q))) return 500;
  if (label.includes(q)) return 300;
  if (item.keywords?.toLowerCase().includes(q)) return 100;
  return 0;
}

/** Whether a scope needs the connected-project lookup at all. */
export function wantsProjectFiles(scope: PaletteSearchScope): boolean {
  return scope === 'all' || scope === 'projects';
}

/** A workspace deliverable (`workspaces/<id>/outputs/…`): a Result, not a note. */
export function isWorkspaceResult(path: string): boolean {
  return /^workspaces\/[^/]+\/outputs\//.test(path);
}

const FOLDER_LABELS: Record<string, string> = {
  library: 'Library', tasks: 'Task', people: 'Person', learnings: 'Learning', workflows: 'Automation',
  briefings: 'Briefing', me: 'About you', drafts: 'Draft', goals: 'Goal', commitments: 'Commitment',
};

/** Where a vault hit came from, for the row's source label. */
export function vaultSourceLabel(path: string, workspaceTitle?: (id: string) => string | undefined): string {
  if (isWorkspaceResult(path)) {
    const id = path.split('/')[1];
    const title = workspaceTitle?.(id);
    return title ? `Result · ${title}` : 'Result';
  }
  // A workspace's sources/ copy says it is a snapshot (plan L3 X3, 2026-10-14).
  if (isWorkspaceSnapshot(path)) {
    const title = workspaceTitle?.(path.split('/')[1]);
    return title ? `Snapshot · ${title}` : 'Snapshot';
  }
  if (path.startsWith('workspaces/')) return 'Workspace';
  const top = path.includes('/') ? path.split('/')[0] : '';
  return FOLDER_LABELS[top] ?? 'Note';
}

/** One connected file as a palette row: name, source and folder or state. */
export function projectFileRow(f: ConnectedFile, projectTitle: string | undefined, direct = false): { label: string; source: string; detail: string } {
  const parts = f.relative.split('/');
  const tag = connectedFileTag(f);
  const folder = parts.slice(0, -1).join('/') || 'Projects folder';
  const notes = [tag?.label, direct ? 'found directly' : ''].filter(Boolean).join(' · ');
  return { label: parts[parts.length - 1] ?? f.relative, source: projectTitle ? `Project · ${projectTitle}` : 'Project file', detail: notes ? `${folder} · ${notes}` : folder };
}

/** A one-line excerpt: whitespace collapsed, clipped at a word near `max`. */
export function excerptLine(text: string | undefined, max = 140): string {
  const flat = (text ?? '').replace(/\s+/g, ' ').trim();
  if (flat.length <= max) return flat;
  const cut = flat.slice(0, max);
  const space = cut.lastIndexOf(' ');
  return `${(space > max * 0.6 ? cut.slice(0, space) : cut).trimEnd()}…`;
}

/** Whether to ask the backend for conversations by title or message (merged
 *  plan slice 4, B·F15): from 2 characters, in the scopes that list
 *  conversations. */
export function wantsConversationSearch(scope: PaletteSearchScope, query: string): boolean {
  return inScope(scope, 'Conversations') && query.trim().length >= 2;
}

/** Conversation search hits the palette adds as rows: each in the workspace
 *  it opens in, minus any already listed from the rail. The rail holds only
 *  the 25 newest per workspace, so older matches come only from here. A hit
 *  by what was asked (2026-11-04) carries its message's snippet as `detail`,
 *  the row's second line. */
export function searchedConversations(hits: readonly SessionTarget[], listed: ReadonlySet<string>): { key: string; workspaceId: string; title: string; agent: string; detail?: string }[] {
  const out: { key: string; workspaceId: string; title: string; agent: string; detail?: string }[] = [];
  for (const { workspaceId, session, snippet } of hits) {
    if (!workspaceId || listed.has(session.key)) continue;
    const row = { key: session.key, workspaceId, title: session.title?.trim() || 'Conversation', agent: session.agent };
    out.push(snippet ? { ...row, detail: excerptLine(snippet) } : row);
  }
  return out;
}

/** One page of conversation hits with its title matches first, each group
 *  in the backend's order (newest first). Within the page only: a later
 *  page appends after it, and the keyset cursor still names the backend's
 *  last row. A hit by text is one with a snippet. */
export function titleMatchesFirst(items: readonly SessionTarget[]): SessionTarget[] {
  return [...items.filter(t => !t.snippet), ...items.filter(t => t.snippet)];
}

/** Conversation hits the palette asks for at a time. */
export const CONVERSATION_PAGE = 20;

/** The palette's conversation search, a page at a time (wave 2 review nit:
 *  only the first 20 hits were ever reachable). `search(q)` starts over and
 *  `more()` appends the page after the last hit, while `next()` says a page
 *  follows (the "More conversations…" row). `cancel()` drops the request in
 *  flight but keeps the rows shown (audit H09); `clear()` also empties them.
 *  A response overtaken by a newer call is dropped, and a failed `more()`
 *  keeps the rows it had. `more()` resolves true only when it appended a
 *  page, so the palette knows when to move the selection to the new rows. */
export function createConversationSearch(fetchPage: (query: string, cursor: string | null, limit: number) => Promise<SessionSearchPage>) {
  const [hits, setHits] = createSignal<SessionTarget[]>([]);
  const [next, setNext] = createSignal<string | null>(null);
  // Whether the newest search is out, answered or failed (P1-48), so the
  // palette is busy meanwhile and a failure never reads as "No matches".
  const [phase, setPhase] = createSignal<'idle' | 'pending' | 'done' | 'error'>('idle');
  const reads = latestRequest();
  let current = '';
  const cancel = () => { reads.cancel(); setPhase(p => (p === 'pending' ? 'idle' : p)); };
  /** A search is coming (the palette's debounce): busy from the keystroke,
   *  so neither "No matches" nor an old failure shows meanwhile. */
  const waiting = () => setPhase('pending');
  const clear = () => { cancel(); current = ''; setHits([]); setNext(null); setPhase('idle'); };
  const search = async (query: string) => {
    const mine = reads.start();
    current = query;
    setPhase('pending');
    try {
      const page = await fetchPage(query, null, CONVERSATION_PAGE);
      if (!reads.isLatest(mine)) return;
      setHits(titleMatchesFirst(page.items)); setNext(page.nextCursor); setPhase('done');
    } catch {
      if (reads.isLatest(mine)) { setHits([]); setNext(null); setPhase('error'); }
    }
  };
  const more = async (): Promise<boolean> => {
    const cursor = next();
    // Not while a new search is out: it would overtake it and leave it pending.
    if (cursor == null || phase() === 'pending') return false;
    const mine = reads.start();
    try {
      const page = await fetchPage(current, cursor, CONVERSATION_PAGE);
      if (!reads.isLatest(mine)) return false;
      setHits(prev => {
        const seen = new Set(prev.map(h => h.session.key));
        return [...prev, ...titleMatchesFirst(page.items.filter(h => !seen.has(h.session.key)))];
      });
      setNext(page.nextCursor);
      return true;
    } catch { return false; /* keep the rows shown */ }
  };
  return { hits, next, phase, search, more, cancel, clear, waiting };
}

/** Where the selection goes after "More conversations…" loads: the first
 *  conversation row in `ids` (the palette's rows, in order) that was not
 *  listed `before`, or -1 when the page added none. Keeps the keyboard on
 *  the rows just fetched instead of jumping back to the top. */
export function firstNewConversationRow(ids: readonly string[], before: ReadonlySet<string>): number {
  return ids.findIndex(id => id.startsWith('conversation:') && !before.has(id));
}

/** Where the selection goes when the palette's rows change (P0-26): onto the
 *  same row when only live data moved (a sessions update, a search page),
 *  the top when the query or scope changed or that row is gone. So Enter
 *  runs the row the person picked, never one that slid under the highlight. */
export function keptSelection(ids: readonly string[], selectedId: string | undefined, sameInput: boolean): number {
  if (!sameInput || selectedId === undefined) return 0;
  return Math.max(ids.indexOf(selectedId), 0);
}
