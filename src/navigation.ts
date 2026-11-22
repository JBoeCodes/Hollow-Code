import type { PageId } from './store';
import type { IconName } from './icons';

export type NavSection = 'workspace' | 'knowledge' | 'system';

export interface PageNavMeta {
  id: PageId;
  label: string;
  icon: IconName;
  ext: string;
  section: NavSection;
  keywords: string;
  shortcutKey?: string;
  /** Compatibility IDs redirect to workspaces or Library. Hidden IDs never
   * appear in the palette or bind global shortcuts. */
  hidden?: boolean;
}

// Single source of truth for the app shell. Rail rows and their icons (P1-05),
// Cmd+digit order and command-palette navigation rows all derive from this
// list so new pages do not drift across chrome surfaces.
// Page digits are contiguous in rail order since frontier F6 (2026-09-29):
// Work ⌘1, Tasks ⌘2, Library ⌘3, Automations ⌘4; Review took ⌘5 (P1-14).
// Retired pages stay unbound.
export const NAV_PAGES: readonly PageNavMeta[] = [
  {
    id: 'today',
    // Work until L1 (11-12), Alex: "change work to be named projects".
    // "work" stays a search word, so typing it still finds the page.
    label: 'Projects',
    icon: 'folder',
    ext: 'md',
    section: 'workspace',
    keywords: 'work outcome workspace result resume new projects all projects',
    shortcutKey: '1',
  },
  {
    // The bell's Needs you > See all opens this full waiting/Incoming view.
    // Keep its stored id, palette entry and ⌘5; it no longer has a rail row.
    id: 'review',
    label: 'Needs you',
    icon: 'bell',
    ext: 'md',
    section: 'workspace',
    keywords: 'waiting on you inbox triage approvals questions decisions results to review failures incoming file classify conflicts drafts',
    shortcutKey: '5',
  },
  {
    id: 'tasks',
    label: 'Tasks',
    icon: 'checkSquare',
    ext: 'md',
    section: 'workspace',
    keywords: 'todo create workbench filters priority due vault suggestions commitments',
    shortcutKey: '2',
  },
  {
    id: 'vault',
    label: 'Library',
    icon: 'book',
    ext: 'md',
    section: 'workspace',
    keywords: 'obsidian notes files memory markdown search',
    shortcutKey: '3',
  },
  {
    // Conversation history: Library → Conversations since S4 (2026-11-03), no
    // longer a page of its own. The id stays the way in (currentPage sends it
    // to Library; store/runtime.ts shows the Conversations tab): Settings,
    // History & recovery, a project's ⋯ on Work or in the top bar (Open in Conversation history),
    // Library's sessions-that-touched-a-file, `g p`, and an agent's
    // navigate('session') for a conversation in no workspace (#318).
    // navigation.test.ts "hidden pages" pins these.
    id: 'chat',
    hidden: true,
    label: 'Conversation history',
    icon: 'message',
    ext: 'jsonl',
    section: 'knowledge',
    keywords: 'claude codex antigravity grok agent sessions artifacts',
  },
  {
    id: 'workflows',
    label: 'Automations',
    icon: 'zap',
    ext: 'md',
    section: 'knowledge',
    keywords: 'automation morning brief results outputs cron routines skills',
    shortcutKey: '4',
  },
  {
    // Off the rail: an old link opens Library (currentPage). Import a source
    // is the Add sheet set to a Library source (S5).
    id: 'ingest',
    label: 'Import',
    icon: 'upload',
    ext: 'md',
    section: 'knowledge',
    keywords: 'capture source pdf url youtube import knowledge quick',
    hidden: true,
  },
  {
    id: 'settings',
    label: 'Settings',
    icon: 'settings',
    ext: 'toml',
    section: 'system',
    keywords: 'config diagnostics privacy appearance connectors setup preferences',
    shortcutKey: ',',
  },
];

export const RAIL_WORKSPACE = NAV_PAGES.filter(page => page.section === 'workspace' && !page.hidden && page.id !== 'review');
export const RAIL_KNOWLEDGE = NAV_PAGES.filter(page => page.section === 'knowledge' && !page.hidden);
export const SETTINGS_PAGE = NAV_PAGES.find(page => page.id === 'settings')!;
/** The rail's rows (P1-05): its labelled destinations, then Settings at the
 *  bottom; the same ids and icons as the palette and ⌘ digits. Tasks is back
 *  in the sidebar since overnight 11-04 S1 (Alex: Work, Tasks, Library,
 *  Automations, as ChatGPT and Claude keep a short nav group). */
export const RAIL_PRIMARY = NAV_PAGES.filter(page => !page.hidden && page.id !== 'settings' && page.id !== 'review');
export const RAIL_BOTTOM = [SETTINGS_PAGE];

export const PAGE_NAV_BY_ID: Record<PageId, PageNavMeta> =
  NAV_PAGES.reduce((acc, page) => {
    acc[page.id] = page;
    return acc;
  }, {} as Record<PageId, PageNavMeta>);

// Full rail order, top to bottom. App.tsx reads this as the spatial truth for
// directional Threshold transitions; shortcut labels and help rows read the
// same order so animation, chrome, and keyboard affordances stay aligned.
export const PAGE_ORDER: PageId[] = NAV_PAGES.map(page => page.id);

export const PAGE_KEY: Partial<Record<PageId, string>> =
  NAV_PAGES.reduce((acc, page) => {
    if (page.shortcutKey) acc[page.id] = page.shortcutKey;
    return acc;
  }, {} as Partial<Record<PageId, string>>);

/** Conversation history (`chat`) and the retired Import page (`ingest`) open
 *  Library. Any other stored id that is no longer a page (Graph, Calendar,
 *  projects, imports, People, Today as its own brief, and the rest cut on
 *  2026-11-08) opens Work. That one fallback is not a named redirect. */
export function currentPage(id: string): PageId {
  if (id === 'chat' || id === 'ingest') return 'vault';
  if (id === 'today' || id === 'review' || id === 'tasks' || id === 'vault' || id === 'workflows' || id === 'settings') return id;
  return 'today';
}
