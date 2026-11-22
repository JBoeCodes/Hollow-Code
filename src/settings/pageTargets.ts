// Settings that live on another page (Settings plan PR 2, 2026-11-12). The
// search lists them next to the sections' own controls, labelled "in Library"
// or "in Automations", and choosing one opens that page on the tab that
// holds the setting. Pure data plus lazy openers: no store import at load, so
// the registry tests and the lazy Settings chunk stay light.
//
// Adding one is one line below: a label (unique among Settings' own control
// labels), the words a person might type, the page and tab it is on, and how
// to open it. Settings keeps its own link row only where people would look
// here; the search finds either.
// Same folding as settingsSections' search: case and accents ignored.
import { MOVED_LABELS } from '../settingsResolve';

const normalizeSearchText = (s: string) => s.toLowerCase().normalize('NFKD').replace(/\p{M}/gu, '');

export type SettingsPageName = 'Library' | 'Automations';

export interface SettingsPageEntry {
  label: string;
  /** Synonyms a person might type, never shown. */
  keywords: string;
  /** Where it lives, shown as "in <page>". */
  page: SettingsPageName;
  /** The tab on that page, when it has one. */
  tab?: string;
  open: () => void;
}

export const SETTINGS_PAGE_ENTRIES: readonly SettingsPageEntry[] = [
  { label: 'Learnings review', keywords: 'learnings learning memories memory suggestions approve reject saved replaced review what hollow learned', page: 'Library', tab: 'Learnings', open: () => void import('../store/libraryMode').then(m => m.openLibraryLearnings()) },
  { label: 'Automation schedules', keywords: 'schedule scheduled when runs nightly daily cron time recurring automations workflows', page: 'Automations', open: () => void import('../store/automationsViews').then(m => m.openAutomationsView()) },
  // What moved here from Settings in PR 4: built from the resolver's one list
  // (`MOVED_LABELS`), so a search hit and an old deep link can't disagree.
  ...Object.entries(MOVED_LABELS).flatMap(([label, m]) => m.search ? [{
    label, keywords: m.search.keywords, page: m.page === 'vault' ? 'Library' as const : 'Automations' as const, tab: m.search.tabLabel,
    open: () => void import('../store/settingsPageTarget').then(x => x.openSettingsPageTarget(m)),
  }] : []),
];

/** The page entries every typed term names; none for an empty query. One
 *  result per destination (page and tab): Learnings review and Ready to add
 *  both open Library, Learnings, so a query naming both lists one, the entry
 *  whose own label it names if either does, else the first. */
export function searchSettingsPages(query: string, entries: readonly SettingsPageEntry[] = SETTINGS_PAGE_ENTRIES): SettingsPageEntry[] {
  const terms = normalizeSearchText(query).split(/\s+/).filter(Boolean);
  if (!terms.length) return [];
  const names = (text: string) => terms.every(t => normalizeSearchText(text).includes(t));
  const hits = entries.filter(e => names(`${e.label} ${e.keywords} ${e.page} ${e.tab ?? ''}`));
  const byPlace = new Map<string, SettingsPageEntry>();
  for (const e of hits) {
    const place = `${e.page}\u0000${e.tab ?? ''}`;
    const kept = byPlace.get(place);
    if (!kept || (!names(kept.label) && names(e.label))) byPlace.set(place, e);
  }
  return hits.filter(e => byPlace.get(`${e.page}\u0000${e.tab ?? ''}`) === e);
}

/** "in Library": the page, as the search shows it. */
export const pageEntryWhere = (e: SettingsPageEntry) => `in ${e.page}`;

/** The question to ask before leaving for a page entry, or null when nothing is unsaved.
 *  `to` is the page's name, which the question says ("Opening Library drops them"). */
export function pendingForPage(entry: SettingsPageEntry, unsaved: string[]): { to: string; unsaved: string[]; page: SettingsPageEntry } | null {
  return unsaved.length ? { to: entry.page, unsaved, page: entry } : null;
}
