/**
 * The one place a Settings target id is resolved (Settings PR 1, "freeze the
 * contract"). Every caller that names a section from outside the Settings
 * page (requestOpenSettingsSection and so the palette, appBridge, the action
 * runner and `settings:` links, plus the capability doctor's fixSection
 * values) goes through here, and so does the page's own jump. A target is
 * a section, or since PR 4 a page. PR 3 (twelve sections) maps the old ids here
 * through the live alias tables; `settingsLegacy.json` stays frozen at the
 * ids before it, the oracle `settingsResolve.test.ts` checks every old pair
 * against. PR 4 (11-12) adds page targets: Skills, History & recovery, the
 * automation windows editor, the run queue and the profile suggestions
 * moved to Library and Automations, so their old ids and labels open those.
 *
 * Pure data and functions: no component imports, so the main bundle can
 * carry it.
 */

import { SETTINGS_ALIAS_CONTROLS, SETTINGS_INDEX_BY_ID, resolveSettingsControl, resolveSettingsSection, settingsSectionForLabel, resolveMovedControl } from './settingsIndex';

export type SettingsTarget =
  | { kind: 'section'; section: string; part?: string; label?: string }
  | { kind: 'page'; page: string; tab?: string; label?: string };

/** Part headings and folds that are gone since Settings PR 3 (twelve
 *  sections): the section, and the control, each became. */
const RETIRED_LABELS: Readonly<Record<string, readonly [string, string?]>> = {
  'Keyboard shortcuts': ['shortcuts'],
  Chat: ['chat'],
  Accounts: ['accounts'],
  'Companion devices': ['devices'],
  Advanced: ['advanced'],
  'Developer readouts': ['advanced', 'Diagnostics'],
};

/** Controls Alex cut in Settings PR 6 (plan 5.1, C1 to C15), by the label
 *  they had, and the section and control where the capability still lives
 *  (5.1's last column). A label decides first, so any old section id with one
 *  of these lands there instead of on a row that is gone. */
export const CUT_LABELS: Readonly<Record<string, readonly [section: string, control?: string, from?: readonly string[]]>> = {
  'Bake-off monthly cap': ['usage', 'Budgets'],
  'Start locally': ['sources', 'Hollow folder'],
  'Make something now': ['sources', 'Hollow folder'],
  'Connect services': ['connectors', 'Your connectors'],
  'All keys': ['shortcuts', 'Show all keys'],
  // A generic word: only the sections that showed the Help row (Keyboard
  // shortcuts, and System before PR 3, now Advanced) send it to all keys.
  Help: ['shortcuts', 'Show all keys', ['shortcuts', 'advanced']],
  'Runtime capabilities': ['advanced', 'Runtime'],
  'Danger mode active in this chat': ['chat', 'Permission mode for new chats'],
  'Revert to Edits only': ['chat', 'Permission mode for new chats'],
  'Keys inside a surface': ['shortcuts', 'Show all keys'],
  'Startup warm-up': ['advanced', 'Diagnostics'],
  'System health': ['advanced', 'Capability doctor'],
  'Service health': ['connectors', 'Your connectors'],
  'Local prioritizing': ['advanced', 'Context prioritizer'],
  'Artwork preview': ['general', 'Texture'],
  'How Hollow adapts to you': ['personalization'],
};

/** Controls renamed since a link or search could have named them (Settings
 *  PR 7): the old label opens the new one. The old words also stay in the
 *  control's search keywords (`settingsSections.ts`). */
export const RENAMED_LABELS: Readonly<Record<string, string>> = {
  'Spread usage': 'Automatic account rotation',
  'Resume on another account when one runs out': 'Automatic account rotation',
  'Learning suggestions': 'How many learnings to suggest',
  // S5b (11-12): Models' Reasoning effort heading is gone; its one row is Effort.
  'Reasoning effort': 'Effort',
  // S5b: Devices' link row to the runner became the runner's own row.
  'Mac mini runner': 'Remote runner',
};

/** Controls that left Settings for another page in PR 4 (plan 3.4), by the
 *  label they had: a label decides first, so any old section id with one of
 *  these opens the page on it. Library tabs are `LibraryMode` values; the
 *  Automations tabs open its windows editor or the run queue. `where` is the
 *  words search and link rows say ("in Library"). */
export interface MovedLabel {
  page: 'vault' | 'workflows';
  tab: string;
  where: string;
  /** Set on the entries search lists on their own (`settings/pageTargets.ts`
   *  builds them from here, so there is one list): the tab's name and the
   *  words a person might type. The sub-controls leave these out. */
  search?: { tabLabel: string; keywords: string };
}

export const MOVED_LABELS: Readonly<Record<string, MovedLabel>> = {
  Skills: { page: 'vault', tab: 'skills', where: 'in Library', search: { tabLabel: 'Skills', keywords: 'skills capabilities templates instructions agent skills slash commands new skill import skill export available to catalog' } },
  'New skill': { page: 'vault', tab: 'skills', where: 'in Library' },
  'Import skill': { page: 'vault', tab: 'skills', where: 'in Library' },
  'Available to': { page: 'vault', tab: 'skills', where: 'in Library' },
  'History & recovery': { page: 'vault', tab: 'history', where: 'in Library', search: { tabLabel: 'History', keywords: 'restore undo revert snapshots runs diff automated writes recovery changed files' } },
  'Automated-write runs': { page: 'vault', tab: 'history', where: 'in Library' },
  'Conversation history': { page: 'vault', tab: 'conversations', where: 'in Library', search: { tabLabel: 'Conversations', keywords: 'past chats sessions transcripts conversations' } },
  'Ready to add': { page: 'vault', tab: 'learnings', where: 'in Library', search: { tabLabel: 'Learnings', keywords: 'profile suggestions questions suggested facts review pending add all about you' } },
  'Automation windows': { page: 'workflows', tab: 'windows', where: 'in Automations', search: { tabLabel: 'Automation windows', keywords: 'windows tonight nightly friday weekday evening hours days schedule window time zone add automation' } },
  'Run queue': { page: 'workflows', tab: 'queue', where: 'in Automations', search: { tabLabel: 'Run queue', keywords: 'queue waiting cancel running automations' } },
};

/** Part ids that are pages now (the old `skills` and `history` parts). */
const MOVED_PARTS: Readonly<Record<string, string>> = { skills: 'Skills', history: 'History & recovery' };

/** Where a section id (live, retired or a part's id) and an optional control
 *  label land. A retired id opens the section that holds it now (the alias
 *  tables in settingsIndex.ts), on the control or part heading it
 *  became; a label decides first, so a control that moved in PR 3 opens its
 *  new section ('defaults' with 'Agent updates' opens Accounts).
 *  `settingsLegacy.json` keeps the ids from before PR 3 as the test oracle. */
export function resolveSettingsTarget(id: string, rawLabel?: string): SettingsTarget {
  // One order: aliases, renamed labels, cut labels, page moves, then the
  // controls that moved between sections (SETTINGS_MOVED_CONTROLS, PR 8).
  const first = resolveSettingsSection(id);
  const label = rawLabel === undefined ? undefined : RENAMED_LABELS[rawLabel] ?? rawLabel;
  const cutRow = label ? CUT_LABELS[label] : undefined;
  // A cut label with `from` applies only to links from the sections that had it.
  const cut = cutRow && (!cutRow[2] || cutRow[2].includes(first)) ? cutRow : undefined;
  if (cut) return { kind: 'section', section: cut[0], ...(cut[1] ? { label: cut[1] } : {}) };
  const moved = MOVED_LABELS[label ?? MOVED_PARTS[id] ?? ''];
  if (moved) return { kind: 'page', page: moved.page, tab: moved.tab, ...(label ? { label } : {}) };
  let control = resolveSettingsControl(first, label ?? SETTINGS_ALIAS_CONTROLS[id]);
  let section = settingsSectionForLabel(first, control);
  const retired = control && !SETTINGS_INDEX_BY_ID[section]?.controls?.some(c => c.label === control) ? RETIRED_LABELS[control] : undefined;
  if (retired) [section, control] = retired;
  section = resolveMovedControl(section, control);
  const part = SETTINGS_INDEX_BY_ID[section]?.parts?.some(p => p.id === id) ? id : undefined;
  return { kind: 'section', section, ...(part ? { part } : {}), ...(control ? { label: control } : {}) };
}
