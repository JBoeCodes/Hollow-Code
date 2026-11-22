/**
 * The capability route ledger (consolidation D, 2026-10-13; review F08/F11;
 * plan completion L4: matrix N4–N8). One row per user-facing capability: its
 * primary home, the visible click path that reaches it, where it also
 * appears beside the need, when it is offered and what the person sees when
 * it isn't, and the proof.
 *
 * The proof is executable. Each row's `walk` is the click path as targets
 * (role + accessible name) and `lands` is what must then be on screen;
 * tests/smoke/route-ledger.journey.spec.ts clicks every walk from the demo
 * Home at desktop, the native minimum window and Largest text, so removing or
 * renaming a route's control fails the row. The same spec runs each row's
 * accelerators (palette row, `g` key, ⌘ key) and expects the same landmark,
 * so an alias leads to the same experience. `verified` names the deeper
 * journeys that also exercise the row.
 *
 * Accelerators are not listed here: each shared action (actionDefs.ts) names
 * its row (`route`), and `acceleratorsOf` reads them. Product rule: no
 * retained capability is shortcut- or palette-only; routeLedger.test.ts holds
 * the registries to it. docs/ROUTE_LEDGER.md is rendered from this file.
 *
 * Pure data and string helpers; imports only the action table.
 */
// `.ts`: scripts/route-ledger.mjs loads this file in Node, which needs it.
import { ACTION_DEFS, CONDITIONS, EFFECT_WORDS, actionLabel, type ActionDef } from './actionDefs.ts';

/** The primary homes (consolidation map, review ROADMAP). */
export type Home = 'Projects' | 'Needs you' | 'Tasks' | 'Library' | 'Automations' | 'Ask' | 'Settings' | 'Workspace' | 'Conversation' | 'Top bar';

export interface Verified {
  /** A spec under tests/smoke/, by file name. */
  spec: string;
  /** Its test title, verbatim (a prefix is enough). */
  test: string;
}

/** A control or landmark by its role and accessible name, as a person sees
 *  it (and a screen reader announces it). `text` is visible text; `label`
 *  an element named by its label alone (a menu's <summary>). */
export interface Target {
  role: string;
  name: string;
  /** How `name` matches: the whole name (default), its start, or a part. */
  match?: 'exact' | 'prefix' | 'contains';
  /** Inside this landmark. */
  in?: Target;
  /** Type this into it instead of clicking (an Ask's question). */
  fill?: string;
  /** When it isn't on screen (a small window), these clicks instead: the
   *  same control's other form, e.g. Settings' section list becomes a
   *  selector below 860px. On a landmark,
   *  the first is the small window's form of the landmark. */
  orElse?: readonly Target[];
}

/** A starting state the walk needs that the demo Home doesn't have, set up
 *  by the spec (`SEEDS` there): conflict copies in the vault, a saved canvas,
 *  a chat with subagents. */
export type Seed = 'conflict-copies' | 'saved-canvas' | 'explicit-appearance' | 'subagents' | 'tracker-conflict' | 'sync-clash' | 'paired-mac' | 'moving-chat';

export interface Route {
  /** Stable capability id: `home.capability`. */
  id: string;
  /** What the person can do, in the app's words. */
  capability: string;
  home: Home;
  /** The visible click path, in words. Never a key or the palette. */
  path: string;
  /** Where it also appears, beside the need. */
  context?: string;
  /** Its prerequisite: when it is offered. Absent: always. */
  when?: string;
  /** With `when`: what the person sees or is told while it doesn't hold. */
  unavailable?: string;
  seed?: Seed;
  /** The click path as targets, from the demo Home. */
  walk: readonly Target[];
  /** On screen once the walk is done. */
  lands: Target;
  /** Where the row's accelerators land, when that is the place the walk
   *  passes through rather than its last step (Resume: ⌘1 opens Projects,
   *  the walk goes on into a workspace). */
  accelLands?: Target | Readonly<Record<string, Target>>;
  verified: readonly Verified[];
}

const J = (n: number, file: string, test: string): Verified => ({ spec: `j${n}-${file}.journey.spec.ts`, test });
const R = (test: string): Verified => ({ spec: 'visible-routes.spec.ts', test: `route: ${test}` });
const MENUS = (test: string): Verified => ({ spec: 'visible-menus.spec.ts', test });

const J1 = J(1, 'resume', 'J1 click-only: resume the right workspace among same-named ones');
const J4 = J(4, 'decision', 'J4 click-only: resolve a decision that needs you');
const J6 = J(6, 'canvas-result', 'click-only: Library → Results → Canvases, compare versions');
const J8 = J(8, 'running', 'J8 click-only: inspect running work and its evidence');
const J9 = J(9, 'ask', 'J9 click-only: ask with this workspace or across all work');
const J10 = J(10, 'source', 'J10 click-only: attach a source and read its provenance');
const J11 = J(11, 'exact-result', 'click-only: inspect, accept, compare, draft a revision, leave and come back to the same state');
const J13 = J(13, 'find-answer', 'click-only: ask from Work, then save a reply as a note and open it');
const LIBRARY_ALL: Verified = { spec: 'library-all-list.spec.ts', test: 'Library opens on All: one list, filters, search, and a click previews in the side panel' };
const LIBRARY_TABS: Verified = { spec: 'library-tabs-state.spec.ts', test: 'Skills and History are Library tabs with the skills catalog and the vault’s history' };
const RECOVERY = (test: string): Verified => ({ spec: 'automation-recovery.journey.spec.ts', test: `click-only: ${test}` });

// ── Targets ──────────────────────────────────────────────────────────────
const t = (role: string, name: string, more: Partial<Target> = {}): Target => ({ role, name, ...more });
const btn = (name: string, inside?: Target): Target => t('button', name, inside ? { in: inside } : {});
const btnStart = (name: string, inside?: Target): Target => t('button', name, { match: 'prefix', ...(inside ? { in: inside } : {}) });
const MAIN = t('navigation', 'Main');
const SIDEBAR = t('complementary', 'Main navigation');
/** Collapsed Projects (Work until L1, 11-12) carries live counts in its name. */
const rail = (name: string) => name === 'Projects' ? btnStart(name, MAIN) : btn(name, MAIN);
const BELL = btn('Notifications', t('group', 'Everywhere'));
const BELL_DIALOG = t('dialog', 'Notifications');
const waiting = () => [BELL, btn('See all needs you', BELL_DIALOG)];
const SETTINGS = btn('Settings', t('navigation', 'Shared tools'));
const section = (name: string): Target => ({ ...btn(name, t('navigation', 'Settings sections')),
  orElse: [t('combobox', 'Settings section'), t('option', name, { match: 'prefix' })] });
const workView = (name: string) => btn(name, t('navigation', 'Projects views'));
const reviewView = (name: string) => btnStart(name, t('navigation', 'Waiting filters'));
const EVERYWHERE = t('group', 'Everywhere');
/** The Main agent orb: its name carries its state ("Main agent · resting"). */
const MAIN_ORB = btnStart('COO', EVERYWHERE);
/** Projects' Ask field: its question, its Where to ask menu, its button. */
const HOME_ASK = t('form', 'Ask');
const QUESTION = t('textbox', 'Ask anything', { in: HOME_ASK });
const WHERE = t('combobox', 'Where to ask', { in: HOME_ASK });
const scope = (name: string) => t('option', name, { match: 'prefix', in: t('listbox', 'Where to ask') });
const Q3 = 'Q3 regional operations review';
const OPEN_Q3 = btn(`Open ${Q3}`);
const PREVIEW = t('complementary', 'Preview');
const BUILD_WS = 'Results view for workspaces';
const LIBRARY_VIEW = t('navigation', 'Library view');
const libraryView = (name: string) => btnStart(name, LIBRARY_VIEW);
const RAIL_TREE = t('navigation', 'Project navigation');
/** The chat summary's Subagents section: collapsed by default since #1497,
 *  so its toggle opens it before the Subagents chip inside shows. */
const SUMMARY_SUBAGENTS = btnStart('Subagents', t('complementary', 'Chat summary'));
// Overnight 11-12 L4: Notes, Results and Recordings left the tab strip; each
// opens from All (Browse folders; Outputs → Grid; Meetings → Recordings).
const ALL = [rail('Library'), libraryView('All')];
const allFilter = (name: string) => btn(name, t('group', 'Library filter'));
const NOTES = [...ALL, btn('Browse folders')];
const RESULTS = [...ALL, allFilter('Outputs'), btn('Grid')];
const RECORDINGS = [...ALL, allFilter('Meetings'), btn('Recordings')];
const CANVASES = [...RESULTS, btn('Canvases', t('group', 'Result type'))];

export const ROUTES: readonly Route[] = [
  // Projects
  { id: 'work.resume', capability: 'Resume a workspace', home: 'Projects', path: 'Rail → Projects → Home → Recent projects, or All projects → Find → a row',
    context: 'Recent projects keep activity order; same-named projects carry an origin label. Needs you and Running retain run/result details',
    walk: [rail('Projects'), OPEN_Q3], lands: t('region', 'Workspace conversation'), accelLands: t('region', 'Projects'), verified: [J1] },
  { id: 'work.activity', capability: 'Running work and its evidence (was Mission Control, then Activity)', home: 'Projects', path: 'Rail → Projects → Running',
    context: 'A workspace header’s running state; a conversation’s receipt; an automation run’s Open goes to that automation',
    walk: [rail('Projects'), workView('Running')], lands: t('region', 'Running'),
    verified: [J8, { spec: 'activity-open.spec.ts', test: 'Activity: an automation run’s Open lands on that automation' }] },

  { id: 'work.pulls', capability: 'A project’s GitHub pull requests and issues: filter, open with checks and comments, triage (comment, label, assign, close), Ask about this', home: 'Projects', path: 'Rail → Projects → Pull requests',
    context: 'The chat card’s More git actions → Pull request shows one branch’s PR in the Preview pane', when: 'A project with a folder in a git repository on GitHub, and the GitHub CLI (gh) installed and signed in', unavailable: 'The list says in words what is missing: install gh, run gh auth login, or the folder has no GitHub remote',
    walk: [rail('Projects'), workView('Pull requests')], lands: t('region', 'Pull requests and issues'),
    verified: [{ spec: 'github-pages.spec.ts', test: 'GitHub pages: list, open, ask and triage behind a confirm' }] },
  // Review
  { id: 'review.app-messages', capability: 'Read and copy recent app messages', home: 'Top bar', path: 'Bell → Notifications → App messages',
    walk: [BELL, btn('Notifications', t('navigation', 'Notification views')), t('text', 'App messages', { in: BELL_DIALOG })], lands: t('region', 'App messages'),
    verified: [{ spec: 'notice-lifetime.spec.ts', test: 'evicted messages remain in the bell history with failure words and Copy' }] },
  { id: 'review.waiting', capability: 'Everything waiting on you', home: 'Needs you', path: 'Bell → Needs you → See all → All',
    context: 'Home and page waiting links also open the full list', walk: waiting(), lands: t('region', 'Waiting on you'), verified: [J4, R('Review views')] },
  { id: 'review.decision', capability: 'Resolve a decision that needs you', home: 'Needs you', path: 'Bell → See all → Decisions → the item’s own buttons',
    walk: [...waiting(), reviewView('Decisions')], lands: t('region', 'Review items and proposals'), verified: [J4] },
  { id: 'review.results', capability: 'Results waiting for you to accept or ask for a revision', home: 'Needs you', path: 'Bell → See all → Results',
    context: 'Each Result opens in the Files tab of the Preview pane with its review bar: Accept / Ask for a revision',
    walk: [...waiting(), reviewView('Results')], lands: t('region', 'Waiting on you'), verified: [R('Review views')] },
  { id: 'review.failures', capability: 'Runs and syncs that failed, with how to recover', home: 'Needs you', path: 'Bell → See all → Failures',
    walk: [...waiting(), reviewView('Failures')], lands: t('region', 'Failures', { in: t('region', 'Waiting on you') }), verified: [R('Review views')] },
  { id: 'review.conflict-copies', capability: 'Resolve iCloud conflict copies', home: 'Needs you', path: 'Bell → See all → Waiting → Decisions → the conflict-copies item',
    when: 'Only while the vault has conflict copies', unavailable: 'No conflict-copies item in Needs you; the palette’s Resolve conflict copies… says there are none',
    seed: 'conflict-copies', walk: [...waiting(), btnStart('2 conflict copies in the vault', t('region', 'Conflict copies'))], lands: t('dialog', 'Conflict copies'),
    verified: [{ spec: 'conflict-copies.spec.ts', test: 'the Waiting on you item opens the dialog' }] },
  // Context sync clash cards (CX6c): one card per file, every root; agents read settings.sync_clashes, the choices are the person's.
  { id: 'review.sync-clashes', capability: 'Resolve a context sync clash: Keep mine, Keep theirs, Keep both or Edit', home: 'Needs you', path: 'Bell → See all → Waiting → Decisions → a Two Macs changed item',
    context: 'Settings → Sources → Context → Personal or Vault sync → N clashes opens the same cards for that root',
    when: 'Only while two Macs changed the same lines of a synced file', unavailable: 'No clash item in Needs you and no Clashes line on the Context rows: nothing waits',
    seed: 'sync-clash', walk: [...waiting(), btnStart('Two Macs changed Identity.md', t('region', 'Context sync clashes'))], lands: t('dialog', 'Context sync clashes'),
    verified: [] },

  // Tasks
  { id: 'tasks.list', capability: 'Tasks', home: 'Tasks', path: 'Rail → Tasks', context: 'Projects → Tasks', walk: [rail('Tasks')], lands: t('textbox', 'Find a task'), verified: [R('rail destinations open their pages')] },
  { id: 'tasks.add', capability: 'Add anything: a task, note, link, file or recording', home: 'Library', path: 'Library → Library actions → Add a task, note, link, file or recording…',
    context: '⌘⌥⇧T and the tray’s Quick Capture Task… bring Hollow forward on it set to Task; Import a source opens it set to Library source',
    walk: [rail('Library'), btn('Library actions'), t('menuitem', 'Add a task, note, link, file or recording…')], lands: t('textbox', 'What to add', { in: t('dialog', 'Add') }), verified: [R('creation from the sidebar and Library')] },
  { id: 'tasks.new', capability: 'New task', home: 'Tasks', path: 'Projects → Tasks → New local task', context: 'Library → Library actions → Add… → Task; the menu bar’s File → New Task…',
    walk: [rail('Projects'), btn('Tasks'), btn('New local task')], lands: t('textbox', 'Task name'), accelLands: t('textbox', 'Task name', { match: 'prefix' }),
    verified: [{ spec: 'required.minimum.spec.ts', test: 'Tasks: open a task, edit it, return to the list' }] },
  { id: 'tasks.new-issue', capability: 'New issue in the task tracker', home: 'Tasks', path: 'Rail → Tasks → New issue',
    when: 'A task tracker is set up (Settings → Sources → Task tracker)', unavailable: 'Tasks shows Open Task tracker settings and New local task only',
    walk: [rail('Tasks'), btn('New issue')], lands: t('textbox', 'Issue title'),
    verified: [{ spec: 'tracker-linked-tasks.spec.ts', test: 'New issue creates one in the tracker and opens it' }] },
  { id: 'tasks.views', capability: 'Task views, filters, grouping and order', home: 'Tasks', path: 'Rail → Tasks → the view tabs; Filter; Group and Order',
    when: 'A task tracker is set up', unavailable: 'Tasks lists local tasks only, with search',
    walk: [rail('Tasks'), btn('By project')], lands: t('combobox', 'Group by'),
    verified: [{ spec: 'tracker-task-views.spec.ts', test: 'By project and By assignee group the open issues' }] },
  { id: 'tasks.projects', capability: 'The team’s projects: status, lead, target date, progress and issue counts', home: 'Tasks', path: 'Rail → Tasks → Projects',
    when: 'A task tracker is set up', unavailable: 'Tasks lists local tasks only',
    walk: [rail('Tasks'), btn('Projects', t('group', 'Views'))], lands: t('list', 'Projects'),
    verified: [{ spec: 'tracker-projects-view.spec.ts', test: 'the Projects tab lists the team’s projects with status, lead, target, progress and issues' }] },
  { id: 'tasks.conflicts', capability: 'Issues where the tracker changed a field you edited: re-apply yours (Use mine) or keep the tracker’s', home: 'Tasks', path: 'Rail → Tasks → Conflicts (n) → an issue',
    context: 'A row’s Conflict flag; the issue’s detail lists its conflicts', when: 'An issue has an open conflict', unavailable: 'Tasks shows no Conflicts control: nothing waits on a choice',
    seed: 'tracker-conflict', walk: [rail('Tasks'), btnStart('Conflicts', t('group', 'Views')), btnStart('ENG-32')], lands: btn('Use mine'),
    verified: [{ spec: 'tracker-conflicts.spec.ts', test: 'Conflicts (n) lists the issues with a conflict, persists, and empties once each is settled' }] },
  { id: 'tasks.comment', capability: 'Write a comment on an issue (posts to the tracker)', home: 'Tasks', path: 'Rail → Tasks → an issue → Comment',
    when: 'A task tracker is set up', unavailable: 'Tasks lists local tasks only',
    walk: [rail('Tasks'), btnStart('ENG-31')], lands: t('textbox', 'Comment'),
    verified: [{ spec: 'tracker-issue-comment.spec.ts', test: 'a comment posts and joins the thread' }] },
  { id: 'tasks.project', capability: 'A project’s details: the tracker’s fields, the portfolio fields (edit), Track in Linear', home: 'Tasks', path: 'Rail → Tasks → an issue → Open project',
    context: 'Library → Records → a project record', when: 'A task tracker is set up for Tasks; Library opens any project record', unavailable: 'Library → Records still opens each project record',
    walk: [rail('Tasks'), btnStart('ENG-31'), btn('Open project')], lands: t('complementary', 'Project properties'),
    verified: [{ spec: 'tracker-project-detail.spec.ts', test: 'an issue’s Project opens the project with the tracker’s fields and the portfolio' }] },
  { id: 'tasks.friday-status', capability: 'Write a project’s Friday status into Hollow now (a tracker copy is optional)', home: 'Tasks', path: 'Rail → Tasks → an issue → Open project → Write status now',
    context: 'Settings → Sources → Task tracker → Friday status (Fridays at 3 PM) and Status copy', when: 'A task tracker is set up and the project has a workspace', unavailable: 'A project with no workspace yet shows no Write status now; the next sync makes one',
    walk: [rail('Tasks'), btnStart('ENG-31'), btn('Open project')], lands: btn('Write status now'),
    verified: [{ spec: 'tracker-friday-status.spec.ts', test: 'Write status now writes the project’s status and says so' }] },
  { id: 'tasks.issue', capability: 'An issue’s details: every field, edits, activity, Open in Linear, Archive', home: 'Tasks', path: 'Rail → Tasks → an issue',
    when: 'A task tracker is set up', unavailable: 'Tasks lists local tasks only',
    walk: [rail('Tasks'), btnStart('ENG-31')], lands: t('complementary', 'Properties'),
    verified: [{ spec: 'tracker-issue-detail.spec.ts', test: 'the detail shows every field of the issue' }] },
  { id: 'tasks.issue-page', capability: 'An issue as the whole page: properties in a column on the right, Back to list', home: 'Tasks', path: 'Rail → Tasks → an issue → Open full page',
    when: 'A task tracker is set up', unavailable: 'Tasks lists local tasks only',
    walk: [rail('Tasks'), btnStart('ENG-31'), btn('Open full page')], lands: btn('Back to list'),
    verified: [{ spec: 'tracker-issue-detail.spec.ts', test: 'Open full page shows the issue as the page, and Back to list returns' }] },
  { id: 'tasks.origin', capability: 'See where a task came from, and go back there', home: 'Tasks', path: 'Projects → Tasks → a task → From: … (Open the plan item / Recordings)',
    when: 'Tasks made from a plan or a recording', unavailable: 'A task typed in by hand shows no From line',
    walk: [rail('Projects'), btn('Tasks'), t('text', 'Revise section 3 with the September close')], lands: btn('Open the plan item'),
    verified: [{ spec: 'task-origin.spec.ts', test: 'a task from a plan names the plan and opens it' }] },

  // Library
  { id: 'library.records', capability: 'Records: meetings, domains, War Room, captures and each project’s record (no people)', home: 'Library', path: 'Rail → Library → All → Browse folders → Library folder → Records',
    context: 'Records also lists every recorded decision with its state: Revisit or Settle again (they were on a workspace’s Overview)',
    walk: [...NOTES, t('combobox', 'Library folder'), t('option', 'Records', { match: 'prefix' })], lands: t('complementary', 'Library files'),
    verified: [{ spec: 'tracker-project-detail.spec.ts', test: 'Library, Records lists records but not people, and a project record opens as the project' }] },
  // Overnight 11-12 L3: one searchable list of everything Library loads, its first tab and default view.
  { id: 'library.all', capability: 'Everything in the Library in one searchable list: Recent, Meetings, People, Outputs; a click previews in the side panel', home: 'Library', path: 'Rail → Library → All',
    context: 'Library opens on it when no other tab is remembered; a note’s Open goes to Notes with the note open; beside its search, New note and Browse folders; Outputs offers Grid and Meetings offers Recordings; the palette’s Library: All',
    walk: [rail('Library'), libraryView('All')], lands: t('group', 'Library filter'), verified: [LIBRARY_ALL] },
  { id: 'library.notes', capability: 'Notes and sources', home: 'Library', path: 'Rail → Library → All → Browse folders',
    context: 'A note’s Open in All; Back to All returns. The palette’s Library: Notes', walk: NOTES, lands: t('complementary', 'Library files'),
    verified: [R('rail destinations open their pages'), { spec: 'required.minimum.spec.ts', test: 'Library: choose a note, read it, and come back' }] },
  { id: 'library.new-note', capability: 'New note', home: 'Library', path: 'Rail → Library → All → New note', context: 'Library actions → Add… → Note; the page header’s New note on every other Library view',
    walk: [...ALL, btn('New note')], lands: t('textbox', 'Note name'), verified: [R('creation from the sidebar and Library')] },
  { id: 'library.import', capability: 'Import a source into the Library', home: 'Library', path: 'Rail → Library → Library actions (⋯) → Import a source',
    context: 'Opens the Add sheet set to Library source, with Domain and Purpose. Library actions → Add… → Library source is the same; ⌘⌥⇧I and the tray’s Import a source… bring Hollow forward on it. Add… → Workspace file attaches a workspace snapshot instead',
    walk: [rail('Library'), btn('Library actions'), t('menuitem', 'Import a source')], lands: t('textbox', 'Domain (optional)', { in: t('dialog', 'Add') }),
    verified: [{ spec: 'next-level-capture.spec.ts', test: 'Import a source opens Add set to a Library source, and its Domain and Purpose reach the import' }] },
  { id: 'library.note-manage', capability: 'Rename, move or trash a note', home: 'Library', path: 'Rail → Library → All → Browse folders → a note’s ⋯ → Rename…, Move to folder… or Move to trash',
    context: 'The open note’s File actions in the reader. Not offered for snapshots or a workspace’s own files; Undo restores a trashed note',
    walk: [...NOTES, btn('More actions for Weekly ops sync 0')], lands: t('menuitem', 'Rename…'),
    verified: [{ spec: 'library-note-manage.spec.ts', test: 'rename updates the list and the reader' }, { spec: 'library-note-manage.spec.ts', test: 'trash closes the reader and Undo restores the note' }] },
  { id: 'library.results', capability: 'Every Result: files and canvases made in your workspaces', home: 'Library', path: 'Rail → Library → All → Outputs → Grid',
    context: 'Files open Preview, with a separate workspace Results action. Canvas details offer Back to all results, or Escape on a result row, outside the Canvases-only view. A canvas row in All opens here; Back to All returns. The palette’s Library: Results',
    walk: RESULTS, lands: t('group', 'Result type'), verified: [J6] },
  { id: 'library.recordings', capability: 'Recordings and their transcripts', home: 'Library', path: 'Rail → Library → All → Meetings → Recordings',
    context: 'A recording row in All; a task’s Open the recording; Back to All returns. The palette’s Library: Recordings',
    walk: RECORDINGS, lands: t('complementary', 'Recordings'), verified: [R('rail destinations open their pages')] },
  { id: 'library.connected', capability: 'Connected project folders and repositories', home: 'Library', path: 'Rail → Library → Connected projects',
    walk: [rail('Library'), libraryView('Connected projects')], lands: libraryView('Connected projects'), verified: [R('rail destinations open their pages')] },
  // S4 (2026-11-03): Conversation history was a hidden page reached from Settings; it is this tab now.
  { id: 'library.learnings', capability: 'Pending, saved, skipped and rejected learnings, with source quotes, reasons, bulk review and Undo', home: 'Library', path: 'Rail → Library → Learnings',
    context: 'A learning line beneath its source reply; the row of a pending learning in the bell or Needs you opens it; Settings → Personalization → Memory links here',
    walk: [rail('Library'), libraryView('Learnings')], lands: t('heading', 'Learnings'), verified: [{spec:'learnings.spec.ts',test:'Learnings owns review and rejected reasons'}] },
  { id: 'library.conversations', capability: 'Past conversations (Conversation history), archived ones included', home: 'Library', path: 'Rail → Library → Conversations',
    context: 'Settings → Advanced → This Mac → Open Library, History → Conversations; a project’s ⋯ on Projects or in the top bar → Open in Conversation history; a Library file’s conversations that touched it',
    walk: [rail('Library'), libraryView('Conversations')], lands: t('heading', 'Conversation history'),
    verified: [R('Settings help, usage, connectors, appearance, transfer, diagnostics, history')] },
  // Overnight 11-12 L2: Settings' skills catalog and History & recovery as Library tabs.
  { id: 'library.skills', capability: 'Skills: browse, make, import, use in a workspace', home: 'Library', path: 'Rail → Library → Skills',
    context: 'Settings → Connectors → Skills are in Library opens it',
    walk: [rail('Library'), libraryView('Skills')], lands: t('searchbox', 'Find a skill'), verified: [LIBRARY_TABS] },
  { id: 'library.history', capability: 'History & recovery for the whole vault: restore what a run changed', home: 'Library', path: 'Rail → Library → History',
    context: 'Settings → Advanced → This Mac → History & recovery is in Library opens it; Library actions → Show file history shows one note’s',
    walk: [rail('Library'), libraryView('History')], lands: t('listbox', 'Automated-write runs'), verified: [LIBRARY_TABS] },
  { id: 'library.profile-suggestions', capability: 'Profile questions and suggestions (Ready to add, the focus card, Review all) as You items in the one review queue', home: 'Library', path: 'Rail → Library → Learnings → Pending',
    context: 'Settings → Personalization → Waiting for you opens it; Look again and Keep learning stay in Personalization',
    walk: [rail('Library'), libraryView('Learnings')], lands: t('region', 'Learnings'), verified: [{ spec: 'app-smoke.spec.ts', test: 'about you fills itself from what agents know and asks for the rest' }] },
  { id: 'library.canvases', capability: 'Every canvas (was the canvas gallery)', home: 'Library', path: 'Rail → Library → All → Outputs → Grid → Canvases',
    context: 'A workspace’s Results', walk: CANVASES, lands: t('region', 'Results'), verified: [J6, R('Library → Results → Canvases')] },

  // Automations
  { id: 'automations.windows', capability: 'Background work windows in their configured time zone', home: 'Automations',
    path: 'Rail → Automations → Background work windows → a window', context: 'Open the disclosure for the clock and window buttons. Every window opens Automations. The tonight window is configured hours, not a queue screen.',
    walk: [rail('Automations')], lands: t('region', 'Background work windows'),
    verified: [{ spec: 'background-clock.journey.spec.ts', test: 'configured zone, midnight tail, keyboard owner routes and retained queue rules' }] },
  { id: 'automations.list', capability: 'Automations', home: 'Automations', path: 'Rail → Automations', walk: [rail('Automations')], lands: t('heading', 'Automations'),
    verified: [R('rail destinations open their pages')] },
  { id: 'automations.new', capability: 'New automation', home: 'Automations', path: 'Rail → Automations → New automation',
    context: 'Automations → Automation windows and the run queue → a window → Add automation prefills its saved window; a conversation’s ⋯ → Schedule in this chat… opens it bound to that chat (Runs in)',
    walk: [rail('Automations'), btn('New automation')], lands: t('textbox', 'Outcome', { match: 'prefix' }), verified: [{ spec: 'next-level-automations.spec.ts', test: 'the editor is outcome-first, shows the next three runs and re-reviews widened access' }] },
  { id: 'automations.background', capability: 'How background work runs: automations, heartbeat, COO check-ins and window jobs', home: 'Automations',
    path: 'Rail → Automations → How background work runs', context: 'Settings → Background work; the COO page’s More → How check-ins run',
    walk: [rail('Automations'), t('text', 'How background work runs')], lands: t('heading', 'COO check-ins'),
    verified: [RECOVERY('one explanation of what runs in the background, with a route to each')] },
  { id: 'automations.window-editor', capability: 'Automation windows (the editor and the overnight cap link) and the run queue, beside the schedule dial', home: 'Automations',
    path: 'Rail → Automations → Automation windows and the run queue', context: 'Settings → Background work → Open windows and run queue opens it. Add automation from a window opens the editor in place; leaving with unsaved window edits asks first',
    walk: [rail('Automations'), t('text', 'Automation windows and the run queue')], lands: t('list', 'Automation windows'),
    verified: [{ spec: 'automation-windows.spec.ts', test: 'edits a window, saves it, and the saved hours survive a reload' }] },
  { id: 'automations.run-review', capability: 'What a run did, and a retry that won’t repeat its outside actions', home: 'Automations',
    path: 'Rail → Automations → an automation → What happened / Run record → guarded retry or recorded evidence',
    context: 'The same Run record is under reply More actions → Show run record and workspace Activity → a run',
    when: 'A run that failed or stopped', unavailable: 'A finished run shows its results instead of a retry',
    walk: [rail('Automations'), btn('Open Morning Brief')], lands: t('region', 'What happened'),
    verified: [RECOVERY('a failed run says what it did'), RECOVERY('outside actions are listed and the guarded retry carries the failed run')] },
  { id: 'automations.options', capability: 'Preview or edit an automation', home: 'Automations',
    path: 'Rail → Automations → an automation → Automation options → Preview configuration or Edit',
    walk: [rail('Automations'), btn('Open Morning Brief'), btn('Automation options')],
    lands: t('menuitem', 'Preview configuration'),
    verified: [{ spec: 'automation-run-strip.spec.ts', test: 'the run strip keeps its explanation and keyboard routes to preview and edit' }] },
  { id: 'automations.runs-on', capability: 'Choose the Mac an automation runs on (no owner, this Mac or a paired Mac)', home: 'Automations',
    path: 'Rail → Automations → an automation → Automation options → Edit → Inputs & scope → Runs on',
    context: 'A list row says when an automation runs on another Mac, or on none and how to fix it',
    walk: [rail('Automations'), btn('Open Morning Brief'), btn('Automation options'), t('menuitem', 'Edit')],
    lands: t('combobox', 'Runs on'),
    verified: [{ spec: 'automation-runs-on.spec.ts', test: 'Runs on pins an automation to this Mac, approved at save' },
      { spec: 'automation-runs-on.spec.ts', test: 'the list notes automations that run on another Mac or on none, and the editor says why' }] },

  // Ask
  // One Ask (2026-11-03): the one Ask, and every question is a chat. Its
  // accelerators (g v, Ask anything…) land on the field, focused.
  { id: 'ask.scoped', capability: 'Ask, with an explicit scope: every question is a chat', home: 'Ask', path: 'Rail → Projects → Ask anything → Where to ask (across all my work: the COO, on its page; or New chat, a prompt that starts a quick chat, which Add to project sends to that project’s lead)',
    context: 'In a workspace, its own message box', walk: [WHERE], lands: scope('New chat'), accelLands: QUESTION,
    verified: [J9, R('Ask scopes and the COO page'), J13] },
  { id: 'ask.main-agent', capability: 'The COO', home: 'Top bar', path: 'Top bar → COO (the orb)',
    context: 'Projects’ Ask field → Across all my work, to ask it on its page (⌥-Enter only puts it in the box); ⌘⇧M; View → COO',
    walk: [MAIN_ORB], lands: t('heading', 'COO'),
    verified: [J9, R('Ask scopes and the COO page'), { spec: 'main-orb.spec.ts', test: 'the orb opens the COO in one click' }] },
  { id: 'ask.dictate', capability: 'Dictate into the message box', home: 'Conversation', path: 'A conversation’s composer → Dictate (microphone)',
    walk: [OPEN_Q3], lands: btn('Dictate', t('region', 'Workspace conversation')),
    verified: [{ spec: 'visible-conversation.spec.ts', test: 'the composer’s Dictate, model and Stop are buttons' }] },

  // Sidebar creation/search and top-bar navigation.
  // L8 (11-10): a project's one conversation is its lead (its sidebar row);
  // another one in a project is a menu item, and it is listed under the
  // lead's Earlier chats.
  { id: 'create.conversation', capability: 'New conversation', home: 'Workspace', path: 'Projects → All projects → a project’s ⋯ → New conversation', context: 'The same ⋯ beside the open project’s name in the top bar; a project row’s right-click',
    when: 'A project exists', unavailable: 'With no project, New chat starts a quick chat',
    // From the open workspace, the palette's row starts it there too. From
    // Home (where its accelerators are run) the palette asks which project,
    // and ⌘N, with no project open, starts a quick chat (2026-11-14; L8 had
    // opened the COO).
    walk: [btn('All projects'), btn(`Actions for ${Q3}`, t('region', 'Projects'))], lands: t('menuitem', 'New conversation'),
    accelLands: { 'act:new-conversation': t('combobox', 'Choose a workspace for the new conversation'), 'new-conversation': t('heading', 'Quick chats') }, verified: [MENUS('Home rows, the open workspace and its open conversation each have a visible menu button')] },
  { id: 'create.quick-chat', capability: 'New chat: a quick chat outside any project', home: 'Conversation', path: 'Sidebar → Quick chats → New quick chat (+)',
    context: 'Projects → Ask anything → Where to ask → New chat; ⌘N with no project open; chats with no project list under Sidebar → Quick chats (2026-11-14; L8 had sent them to the COO)',
    walk: [btn('New quick chat', RAIL_TREE)], lands: t('textbox', 'Chat message'), verified: [R('creation from the sidebar and Library')] },
  { id: 'create.workspace', capability: 'New project', home: 'Workspace', path: 'Sidebar → Projects +', context: 'Projects’ empty state; form → Create project or Create and start; Command-Enter creates, Escape cancels',
    walk: [btn('New project', RAIL_TREE)], lands: t('textbox', 'Project name'), verified: [R('creation from the sidebar and Library')] },
  { id: 'find.search', capability: 'Search anything; jump to a workspace or chat', home: 'Conversation', path: 'Top bar → Search chats → Search everything',
    context: 'Projects → All projects → Find a project', walk: [btn('Search chats'), t('option', 'Search everything', { in: SIDEBAR })], lands: t('combobox', 'Search', { in: t('dialog', 'Command palette') }),
    // ⌘⇧K opens the sidebar's own chat search (L1), not the palette.
    accelLands: { 'search-chats': t('combobox', 'Search chats', { in: t('search', 'Search chats') }) },
    verified: [R('sidebar search, sidebar, back and forward')] },
  { id: 'nav.back-forward', capability: 'Back and forward', home: 'Top bar', path: 'Top bar → ← / →',
    when: 'After moving between pages', unavailable: 'Back and Forward are greyed out',
    walk: [rail('Projects'), btn('Tasks'), rail('Library'), btn('Back', t('group', 'History'))], lands: t('textbox', 'Find a task'),
    verified: [R('sidebar search, sidebar, back and forward')] },
  { id: 'nav.sidebar', capability: 'Show or hide the sidebar', home: 'Top bar', path: 'Top bar → sidebar button',
    walk: [{ ...btn('Hide sidebar'), orElse: [btn('Show sidebar')] }], lands: { ...btn('Show sidebar'), orElse: [btn('Close sidebar')] }, verified: [R('sidebar search, sidebar, back and forward')] },

  // Workspace
  { id: 'preview.pane', capability: 'Preview a file, a web page or a canvas beside any page', home: 'Conversation',
    path: 'Click a file or a link (a reply’s changed-file chips and links, a tool’s file, a citation) → the Preview pane opens on the right; × hides it',
    context: 'One mixed document/web tab strip with Close and New tab (Tools: Files, Browser, Subagents, Changes); closing the pane restores the persistent chat summary. PDF Previous / Next / Fit / zoom / Refresh / Read page text. Back and forward walk what was opened; a canvas opens there with Expand; web tabs keep Annotate → Comment / Box / Draw; the Files picker pins project notes and changed files above conversation files; a closed tab comes back from a New tab’s Closed section (Reopen closed tab), the palette’s Reopen closed view or View → Reopen Closed View (no key)',
    walk: [btn(`Open ${BUILD_WS}`), btnStart('ResultsView.tsx', t('list', 'Changed files'))], lands: PREVIEW,
    accelLands: { 'act:toggle-side-pane': PREVIEW, 'toggle-side-pane': PREVIEW, 'act:reopen-closed-view': PREVIEW, 'reopen-closed-view': PREVIEW },
    verified: [R('the Preview pane opens on a click and hides itself'), { spec: 'annotation-keyboard.journey.spec.ts', test: 'keyboard places a snapshot-pixel comment' }] },
  { id: 'workspace.files', capability: 'A project’s notes (WORKSPACE.md, PLAN.md, SOURCES.md, AGENTS.md) as files; PLAN.md’s checkboxes tick there', home: 'Workspace',
    path: 'Open a workspace → top bar Project files → the Preview pane’s Files tab (its Files picker pins the notes)',
    context: 'a task’s From: plan link opens PLAN.md at its row. Earlier versions are in History & recovery',
    when: 'A project (not Chats)', unavailable: 'Chats has no project notes and no Project files button',
    walk: [OPEN_Q3, btn('Project files')], lands: PREVIEW, accelLands: { 'act:open-project-files': PREVIEW },
    verified: [{ spec: 'project-files.spec.ts', test: 'Project files opens WORKSPACE.md, the picker pins the notes, and a PLAN.md box ticks the file' }] },
  { id: 'workspace.changes', capability: 'A build workspace’s changed files, diff, commit, push and pull request', home: 'Workspace',
    path: 'Open a build workspace → its branch in the top bar → the Changes tab in the Preview pane',
    context: 'The Files picker’s Changed files group lists the files, and its last row opens the Changes tab',
    when: 'A build workspace with its worktree', unavailable: 'Other workspaces have no branch button and no Changed files group',
    walk: [btn(`Open ${BUILD_WS}`), btnStart('Changes on')], lands: t('region', 'Commit and publish'), accelLands: { 'act:open-changes': t('region', 'Commit and publish') },
    verified: [{ spec: 'build-changes.spec.ts', test: 'the branch opens Changes in the Preview pane' }] },
  { id: 'workspace.back-to-conversation', capability: 'Back to the project’s lead', home: 'Workspace', path: 'Sidebar → the project row (it opens the project’s lead, L8)',
    context: 'The open project’s row stays highlighted in the sidebar; the conversation stays mounted behind other pages (overnight 11-04 U4), so its scroll and draft are as left',
    walk: [OPEN_Q3, rail('Library'), btnStart(Q3, RAIL_TREE)], lands: t('textbox', 'Chat message'),
    verified: [{ spec: 'required.minimum.spec.ts', test: 'Workspace: Project files opens beside the conversation and closing it brings the draft back' }] },
  { id: 'workspace.menu', capability: 'Rename, status, merge, reveal or archive a workspace', home: 'Workspace', path: 'The workspace’s ⋯ (Home row or workspace header)',
    context: 'Home → All projects; or right-click on a project row', walk: [btn('All projects'), btn(`Actions for ${Q3}`, t('region', 'Projects'))], lands: t('menuitem', 'Merge into…'),
    verified: [MENUS('Home rows, the open workspace and its open conversation each have a visible menu button')] },
  // Beside the need (matrix N8): review and revise beside the Result. A
  // Result opens in the Files tab with its review bar (COO rebuild L4; it was
  // the workspace panel's Results section).
  { id: 'workspace.result-review', capability: 'Accept a Result, or ask for a revision, where it is shown', home: 'Workspace',
    path: 'Library → All → Outputs → Grid → a Result → the Preview pane’s review bar → Accept / Ask for a revision',
    context: 'Bell → Needs you → See all → Results, a Projects return card’s Result, and a reply’s changed-file chip for a file in outputs/ (its click, or Review result on its right-click menu and the reply’s More actions) all open it with the bar',
    walk: [...RESULTS, btn(Q3, t('region', 'Results'))], lands: btn('Ask for a revision', PREVIEW),
    verified: [J11, { spec: 'revision-draft.spec.ts', test: 'a draft survives asking for a revision, the Preview pane and leaving the workspace' }, { spec: 'turn-footer.spec.ts', test: 'a changed file in outputs/ opens in the Preview pane with its review bar' }] },

  // Conversation
  { id: 'conversation.menu', capability: 'Rename, pin, move, archive, copy the ID of, start a workspace from, or schedule an automation in a conversation (one menu everywhere)', home: 'Conversation', path: 'An open conversation → the ⋯ beside its name above it',
    context: 'Double-click the name to rename it in place; Conversation history → a row’s ⋯ (or its right-click), or the ⋯ beside the open conversation’s title there', walk: [OPEN_Q3, btn('Actions for this conversation', t('group', 'Conversation'))], lands: t('menuitem', 'Move to project…', { match: 'prefix' }),
    verified: [MENUS('Home rows, the open workspace and its open conversation each have a visible menu button')] },
  { id: 'conversation.subagents', capability: 'See what a chat started: its subagents with their time and tokens, each one’s session, Stop and a message to it', home: 'Conversation',
    path: 'A chat that started subagents → the chat summary’s Subagents section → its Subagents chip → the Subagents tab in the Preview pane → a row (its back arrow returns to the list)',
    context: 'A “Started a subagent” line in the transcript opens the same tab on that subagent. The sidebar never lists subagents',
    when: 'While the chat has started at least one subagent', unavailable: 'The chat summary’s Subagents section reads None yet',
    seed: 'subagents', walk: [OPEN_Q3, SUMMARY_SUBAGENTS, btnStart('Subagents:')], lands: t('tabpanel', 'Subagents'),
    verified: [R('the Subagents chip opens the chat’s Subagents tab'), { spec: 'subagents-tab.spec.ts', test: 'the chip and the tab count, tick and open a subagent' }] },
  { id: 'conversation.switch', capability: 'Move between conversations: a project’s lead, and its other chats', home: 'Conversation',
    path: 'Sidebar → a project row (its lead) → the chat summary’s Subagents section → its Subagents chip → Earlier chats → a row (read it, or message it from its composer)',
    context: 'Chats with no project: the COO → its Subagents chip → Earlier chats. Library → Conversations lists every chat (L8: the sidebar lists none)',
    walk: [OPEN_Q3, btnStart(Q3, RAIL_TREE), SUMMARY_SUBAGENTS, btnStart('Subagents:')], lands: t('heading', 'Earlier chats', { match: 'prefix' }),
    // Previous and Next step through the sidebar's project rows: each opens a project's lead.
    accelLands: t('region', 'Workspace conversation'),
    verified: [{ spec: 'rail-tree.spec.ts', test: 'a project row opens its lead, and its other chats are under Earlier chats' }] },
  { id: 'conversation.remote', capability: 'Open a chat on another Mac (the peer link): read it live, send it a message, stop it and answer its card at its home', home: 'Conversation',
    path: 'Sidebar → the machine strip → the Mac → one of its chats', context: 'Its quick chats also list under Quick chats with the Mac’s badge, every chat under Library → Conversations → On <Mac> (the popover’s Open all in Library), and a project’s chats in that project’s Conversations list; right-click a remote chat for Open, Stop and Copy conversation ID; a Mac that is connecting offers Cancel',
    when: 'Another Mac is paired with this one', unavailable: 'The sidebar shows no machine strip until a Mac is paired (Settings → Devices → Pair a Mac)', seed: 'paired-mac',
    walk: [btnStart('Mac mini', t('group', 'Macs', { in: SIDEBAR })), btnStart('Pick a release name', t('dialog', 'Chats on Mac mini'))], lands: t('region', 'Chat on Mac mini'),
    verified: [{ spec: 'peer-machines.spec.ts', test: 'a chat on another Mac: read live, send, stop and answer its card' }] },
  { id: 'conversation.move-to-runner', capability: 'Move a chat to the paired runner: its history goes once, a running turn stops at a safe point and keeps going there; the result says moved, waiting for the other Mac, refused (why) or busy', home: 'Conversation',
    path: 'An open conversation → the ⋯ beside its name above it → Move to runner → Move', context: 'The same conversation menu on a Quick chats row (right-click) and in Library → Conversations. A moved chat then lists once, at its new Mac, badged moved',
    when: 'A paired Mac takes chats (the runner, or the only Mac paired)', unavailable: 'The conversation menu has no Move to runner until such a Mac is paired (Settings → Devices → Pair a Mac)', seed: 'paired-mac',
    walk: [OPEN_Q3, btn('Actions for this conversation', t('group', 'Conversation'))], lands: t('menuitem', 'Move to runner'),
    verified: [{ spec: 'peer-machines.spec.ts', test: 'Move to runner moves a chat, which then lists once at its new Mac, badged moved' }] },
  { id: 'conversation.move-keep', capability: 'Keep a chat that is moving on this Mac, or release it here when the other Mac never answers (after a confirm that says that Mac may keep a copy)', home: 'Conversation',
    path: 'A moving chat → its notice above the composer → Keep it here, or Release here… → Release here', context: 'A chat that moved says where it lives now, with Open on that Mac; one that moved to a Mac you forgot says so',
    when: 'While the chat is moving to another Mac, or waiting for it after Keep it here', unavailable: 'A chat that never moved shows no notice', seed: 'moving-chat',
    walk: [OPEN_Q3, btn('Actions for this conversation', t('group', 'Conversation')), t('menuitem', 'Move to runner'), btn('Move')], lands: btn('Keep it here', t('status', 'Where this chat runs')),
    verified: [{ spec: 'peer-machines.spec.ts', test: 'a moving chat: Keep it here, Release here behind its confirm, and Forget says what it releases' }] },
  { id: 'conversation.continue-offline', capability: 'Continue a chat that moved to another Mac here while that Mac is out of reach: a copy of it on this Mac, badged offline copy, whose new turns sync there once it answers (into the chat, or as their own chat next to it); the copy says if it was kept here and why', home: 'Conversation',
    path: 'A chat that moved → its notice above the composer → Continue here offline… → Continue here', context: 'Once a copy is open the notice offers Open offline copy; the copy offers Open the original. Quick chats badge an offline copy',
    when: 'The chat moved to another Mac', unavailable: 'A chat that never moved shows no notice', seed: 'paired-mac',
    walk: [OPEN_Q3, btn('Actions for this conversation', t('group', 'Conversation')), t('menuitem', 'Move to runner'), btn('Move')], lands: btn('Continue here offline…', t('status', 'Where this chat runs')),
    verified: [{ spec: 'peer-machines.spec.ts', test: 'an offline copy: its badge and kept reason, and Continue here offline on the moved chat' }] },
  { id: 'conversation.stop', capability: 'Stop the running turn', home: 'Conversation', path: 'A running conversation’s composer → Stop',
    when: 'While a turn runs or waits in the queue', unavailable: 'The composer shows Send instead of Stop',
    walk: [OPEN_Q3, t('textbox', 'Chat message', { fill: 'Summarise the plan' }), btn('Send')], lands: btn('Stop'),
    verified: [{ spec: 'visible-conversation.spec.ts', test: 'the composer’s Dictate, model and Stop are buttons' },
      { spec: 'chat-workspace.spec.ts', test: 'slash commands, multiline input, explicit send and stop keep their behavior' }] },
  { id: 'conversation.steer', capability: 'Steer the running turn, queue a message for after it, or Stop and send', home: 'Conversation',
    path: 'A running conversation’s composer → its send button (Steer now, or Queue for after where the agent can’t take a message mid-turn) or Send options',
    when: 'While a turn runs', unavailable: 'The composer shows Send instead',
    walk: [OPEN_Q3, t('textbox', 'Chat message', { fill: 'Summarise the plan' }), btn('Send')], lands: btn('Send options'),
    verified: [{ spec: 'steering.journey.spec.ts', test: 'Send options: Queue for after and Stop and send wait as chips, with Cancel, by keyboard' },
      { spec: 'steering.journey.spec.ts', test: 'Steer now: Enter sends the message into the running turn, shown as yours' }] },
  { id: 'conversation.commands', capability: 'The composer’s commands, and Claude Code’s own MCP-server and plugin lists', home: 'Conversation',
    path: 'A conversation’s composer → + (Attach and commands) → a command (put in the message box), or Claude Code’s MCP servers / plugins (listed at once)',
    context: 'Typing / in the composer lists the same commands; the same menu’s Attach files… opens the file picker (any file), and in a workspace chat its Add to Sources… opens the Sources picker',
    when: 'The MCP-server and plugin lists: while Claude answers the conversation', unavailable: 'With another agent the menu lists only Hollow’s commands; that agent’s own setup lives in its own CLI',
    walk: [OPEN_Q3, btn('Attach and commands', t('region', 'Workspace conversation')), t('menuitem', 'Claude Code’s MCP servers (its own list)')], lands: t('text', '$ claude mcp list'),
    verified: [{ spec: 'visible-conversation.spec.ts', test: 'the composer’s + menu attaches files and lists every command' }, J10] },
  { id: 'conversation.save-note', capability: 'Save a reply as a note', home: 'Conversation', path: 'A conversation → a reply’s More actions → Save as note (into the workspace’s Results; from Chats or the COO, into the Library)',
    context: 'The toast that says where it went has Open, which shows it in the Preview pane',
    walk: [OPEN_Q3, btn('More actions', t('group', 'Reply details'))], lands: t('menuitem', 'Save as note'), verified: [J13] },
  { id: 'conversation.branch-here', capability: 'Branch a conversation from a reply (CH1)', home: 'Conversation', path: 'A conversation → a finished reply’s More actions → Branch from here… → Branch',
    context: 'The new chat opens in the same project with the original’s turns up to that reply and a notice with Back to the original; the “Continue from here in a new conversation” handoff stays beside it',
    when: 'The reply ended a run and no turn is running on it', unavailable: 'A reply with no run, or the one still being written, has no Branch from here',
    walk: [OPEN_Q3, btn('More actions', t('group', 'Reply details'))], lands: t('menuitem', 'Branch from here…'),
    verified: [{ spec: 'chat-branch.spec.ts', test: 'Branch from here confirms, branches and opens the branch with its notice' }] },
  { id: 'conversation.find', capability: 'Find in a conversation; its outline', home: 'Conversation', path: 'A conversation → Actions for this conversation → Find in this conversation / Outline (also the dashes in its left gutter)',
    walk: [OPEN_Q3, btn('Actions for this conversation'), t('menuitem', 'Find in this conversation')], lands: t('searchbox', 'Find in this conversation'),
    verified: [{ spec: 'next-level-conversation.spec.ts', test: 'a 1,000-turn thread renders a bounded window and find reaches early turns' }] },
  { id: 'conversation.side-chat', capability: 'Ask a quick question about a chat in a second chat beside it, without disturbing its run (the side chat reads what the chat has said, runs read only unless you allow the chat’s own access, never more, and puts a reply in the main chat’s message box only when you press Add to main chat)', home: 'Conversation',
    path: 'A conversation → the Side chat button beside its name (Close side chat, or Esc, closes it)',
    context: 'Actions for this conversation → Open side chat; the palette’s Toggle side chat. New side chat and an Earlier side chats list sit in the pane’s header; Same access as the main chat is under its message box. Not the COO’s page',
    when: 'In a project chat or Chats', unavailable: 'The COO’s page has no side chat button',
    walk: [OPEN_Q3, btn('Open side chat')], lands: t('complementary', 'Side chat'),
    verified: [{ spec: 'side-chat.spec.ts', test: 'the side chat opens beside the chat, asks its own question and closes' }] },
  { id: 'conversation.context', capability: 'Folder and connectors', home: 'Conversation', path: 'A conversation’s composer → Folder and connectors',
    context: 'Two lines: the working folder and connector status, with a link to Settings. Context composition is in the usage ring',
    walk: [OPEN_Q3, btnStart('Folder and connectors:')], lands: t('menuitem', 'Connectors ·', { match: 'prefix' }), verified: [{ spec: 'next-level-composer-connectors.spec.ts', test: 'folder and connectors has two lines and a visible settings route' }] },
  { id: 'conversation.permissions', capability: 'What the agent may do in this conversation (permissions)', home: 'Conversation', path: 'A conversation’s composer → the access chip beside the + (it names the mode in the chat’s agent’s own words: Claude’s Accept edits, Codex’s Ask for approval, Antigravity’s Skip permissions)',
    context: 'The model button → Permissions holds the same choices with each mode’s description',
    walk: [OPEN_Q3, btnStart('Access: ')], lands: t('menuitem', ' · current', { match: 'contains' }), verified: [{ spec: 'agent-settings.spec.ts', test: 'safer permissions start a new native session while preserving conversation context' }] },
  { id: 'conversation.usage', capability: 'This conversation’s usage and account', home: 'Conversation', path: 'A conversation’s composer → Show usage',
    context: 'Context → What’s using it and Cache; Continue in a new conversation past the hand-over point',
    walk: [OPEN_Q3, btn('Show usage')], lands: btn('Show usage'), verified: [{ spec: 'chat-workspace.spec.ts', test: 'the usage ring shows context, the last turn and the plan limits' }] },
  { id: 'conversation.model', capability: 'Switch the model', home: 'Conversation', path: 'A conversation’s composer → the model button',
    context: 'Every installed agent, the COO’s conversations included',
    when: 'With an agent installed', unavailable: 'The model button names no agent; Settings → Models says how to add one',
    walk: [OPEN_Q3, btnStart('Model: ')], lands: t('group', 'Models'),
    verified: [{ spec: 'visible-conversation.spec.ts', test: 'the composer’s Dictate, model and Stop are buttons' }, { spec: 'main-agent-provider.spec.ts', test: 'a fresh COO page keeps the chosen agent and offers every provider' }] },
  { id: 'canvas.attach-to-issue', capability: 'Attach a copy of a canvas to a tracker issue', home: 'Conversation', path: 'A saved canvas’s ⋯ → Attach to issue…',
    context: 'Library → All → Outputs → Grid → Canvases', when: 'A task tracker is set up', unavailable: 'Without a tracker the dialog finds no issues and says to sync the tracker',
    seed: 'saved-canvas', walk: [...CANVASES, btn('Release status'), t('button', 'Canvas actions'), t('menuitem', 'Attach to issue…')], lands: t('dialog', 'Attach “Release status” to an issue'),
    verified: [{ spec: 'tracker-friday-status.spec.ts', test: 'a saved canvas attaches a copy to an issue once' }] },
  { id: 'canvas.save-image', capability: 'Export a canvas as image or PDF', home: 'Conversation', path: 'A workspace canvas’s ⋯ → Export canvas…',
    context: 'Results and Library → All → Outputs → Grid → Canvases', when: 'A workspace or Chats canvas', unavailable: 'Pins have no export; conversation-only canvases need a workspace',
    seed: 'saved-canvas', walk: [...CANVASES, btn('Release status'), t('button', 'Canvas actions'), t('menuitem', 'Export canvas…')], lands: t('dialog', 'Export canvas'),
    verified: [{ spec: 'canvas-export.spec.ts', test: 'a gallery canvas exports its selected revision and options' }] },

  // Settings
  { id: 'settings.page', capability: 'Settings', home: 'Settings', path: 'Rail → Settings', context: 'Updates: Channel (Main or Nightly), Install updates overnight, Check now, Update token and Connect Hollow or Disconnect, pending-token decisions and the last result with OK', walk: [SETTINGS], lands: { ...t('navigation', 'Settings sections'), orElse: [t('combobox', 'Settings section')] }, verified: [R('rail destinations open their pages')] },
  { id: 'settings.scope', capability: 'What a Settings section changes and how it saves', home: 'Settings', path: 'Rail → Settings → General → Saves at once. Forms need Save.', context: 'Available in every Settings section, and again on each part',
    walk: [SETTINGS, section('General')], lands: t('text', 'Saves at once. Forms need Save.'),
    verified: [{ spec: 'settings-scope.spec.ts', test: 'every Settings section shows how it saves' }, { spec: 'settings-scope.journey.spec.ts', test: 'save note stays visible' }] },
  { id: 'settings.app-icon', capability: 'Upload a Dock icon or generate a logo, preview it and reset to default', home: 'Settings', path: 'Rail → Settings → General → App and COO icons → App icon → Upload an icon', context: 'Generate a logo beside it opens the same dialog',
    walk: [SETTINGS, section('General'), t('text', 'App and COO icons'), btn('Upload an icon', t('region', 'App icon'))], lands: t('dialog', 'App icon'),
    verified: [{ spec: 'custom-icons.spec.ts', test: 'upload previews, keeps the original, and a failed apply keeps the dialog' }] },
  { id: 'settings.main-agent-icon', capability: 'Upload a COO icon or generate a logo, with its status badges preserved', home: 'Settings', path: 'Rail → Settings → General → App and COO icons → COO icon → Upload an icon', context: 'COO page → top bar Actions for COO → Change icon… opens the same dialog',
    walk: [SETTINGS, section('General'), t('text', 'App and COO icons'), btn('Upload an icon', t('region', 'COO icon'))], lands: t('dialog', 'COO icon'),
    verified: [{ spec: 'custom-icons.spec.ts', test: 'cancel and rejected SVG leave the saved choice alone' }] },
  { id: 'settings.keys', capability: 'Every key, on one card', home: 'Settings', path: 'Rail → Settings → Keyboard shortcuts → Show all keys', context: 'Settings → Keyboard shortcuts → Show all keys',
    walk: [SETTINGS, section('Keyboard shortcuts'), btn('Show all keys')], lands: t('dialog', 'Keyboard shortcuts'), verified: [MENUS('Settings → Keyboard shortcuts → Show all keys opens every key')] },
  { id: 'settings.usage', capability: 'Usage: every agent’s limits, automatic account rotation, per-agent detail and budgets (Settings PR 8)', home: 'Settings', path: 'Rail → Settings → Usage (opens on Limits)', context: 'Tabs: Limits, All agents, one per agent with runs, Budgets. Accounts links here for automatic account rotation', walk: [SETTINGS, section('Usage')], lands: t('heading', 'Automatic account rotation'),
    verified: [R('Settings help, usage, connectors, appearance, transfer, diagnostics, history')] },
  { id: 'settings.usage.caps', capability: 'The caps Hollow keeps for you: COO daily hard stop and overnight cap (P1-07)', home: 'Settings', path: 'Rail → Settings → Usage → Limits → Spending caps',
    walk: [SETTINGS, section('Usage')], lands: t('list', 'Caps Hollow keeps for you'),
    verified: [{ spec: 'spending-caps.spec.ts', test: 'with no budgets of your own, Budgets lists the caps Hollow keeps with their spend' }] },
  { id: 'settings.usage.prices', capability: 'Model prices (Codex estimates)', home: 'Settings', path: 'Rail → Settings → Advanced → Prices',
    walk: [SETTINGS, section('Advanced'), t('text', 'Prices')], lands: t('group', 'Add or change a price (dollars per million tokens)'),
    verified: [{ spec: 'usage.spec.ts', test: 'Settings → Advanced → Prices lists the table and saves a price after a confirm' }] },
  { id: 'settings.connectors', capability: 'Connectors', home: 'Settings', path: 'Rail → Settings → Connectors',
    context: 'Settings → Connectors → Microsoft 365 → Sign in again, when the calendar sign-in has lapsed', walk: [SETTINGS, section('Connectors')], lands: t('heading', 'Your connectors'),
    verified: [R('Settings help, usage, connectors, appearance, transfer, diagnostics, history'), { spec: 'day-ring.spec.ts', test: 'a signed-out Microsoft 365 calendar says to sign in again and opens Connectors' }] },
  { id: 'settings.calendar', capability: 'Agenda calendar source and included calendars', home: 'Settings', path: 'Rail → Settings → Connectors → Agenda calendar',
    walk: [SETTINGS, section('Connectors')], lands: t('combobox', 'Agenda calendar'), verified: [{ spec: 'agenda-source.journey.spec.ts', test: 'Agenda has one source choice and Off survives a reload' }] },
  { id: 'settings.appearance', capability: 'Theme and appearance', home: 'Settings', path: 'Rail → Settings → General', walk: [SETTINGS, section('General')], lands: t('heading', 'Appearance'),
    verified: [R('Settings help, usage, connectors, appearance, transfer, diagnostics, history')] },
  { id: 'settings.system-appearance', capability: 'Follow live macOS appearance', home: 'Settings', path: 'Rail → Settings → General → Theme → Match macOS',
    seed: 'explicit-appearance', walk: [SETTINGS, section('General'), t('combobox', 'Theme'), t('option', 'Match macOS', { match: 'prefix' })], lands: t('combobox', 'Theme'),
    verified: [{ spec: 'appearance-preferences.journey.spec.ts', test: 'Match macOS follows live changes until an explicit palette is selected' }] },
  { id: 'settings.transfer', capability: 'Export or import settings', home: 'Settings', path: 'Rail → Settings → Advanced → Settings file → Export settings… / Import settings…',
    walk: [SETTINGS, section('Advanced'), t('text', 'Settings file')], lands: btn('Import settings…'), verified: [R('Settings help, usage, connectors, appearance, transfer, diagnostics, history')] },
  { id: 'settings.disk-cleanup', capability: 'Free disk space: delete old install build folders, remove finished worktrees, remove other vaults’ old search indexes (each shows what goes and asks first)', home: 'Settings',
    path: 'Rail → Settings → Advanced → This Mac → Disk space → Delete old install builds… / Remove finished worktrees… / Remove old indexes…', context: 'The low-disk warning in Needs you links to Settings → Advanced; Hollow’s databases names Remove old indexes… when other vaults’ indexes are there',
    walk: [SETTINGS, section('Advanced'), t('text', 'This Mac')], lands: btn('Delete old install builds…'),
    verified: [{ spec: 'disk-space-cleanup.spec.ts', test: 'Disk space: Delete old install builds… previews, Cancel deletes nothing, Delete deletes the list and measures again' },
      { spec: 'disk-space-cleanup.spec.ts', test: 'Disk space: Remove finished worktrees… by keyboard, Esc removes nothing, Remove removes the list and measures' },
      { spec: 'disk-space-cleanup.spec.ts', test: 'Disk space: Remove old indexes… lists other vaults’ indexes, Cancel removes nothing, Remove removes them and measures again' }] },
  // Repo features R7b: read-only; removing old indexes is Disk space's.
  { id: 'settings.databases', capability: 'See Hollow’s databases (whose each is, size, WAL, version) and check their integrity', home: 'Settings',
    path: 'Rail → Settings → Advanced → Diagnostics → Hollow’s databases → Measure / Check integrity',
    walk: [SETTINGS, section('Advanced'), t('text', 'Diagnostics')], lands: btn('Check integrity'),
    verified: [{ spec: 'disk-space-cleanup.spec.ts', test: 'Hollow’s databases: Measure lists the four kinds with owners, Check integrity counts rows and changes nothing' }] },
  { id: 'settings.diagnostics', capability: 'Save diagnostics; clear debug logs', home: 'Settings', path: 'Rail → Settings → Advanced → Diagnostics → Save diagnostics… / Clear debug logs',
    walk: [SETTINGS, section('Advanced'), t('text', 'Diagnostics')], lands: btn('Save diagnostics…'), verified: [R('Settings help, usage, connectors, appearance, transfer, diagnostics, history')] },
  { id: 'settings.build-identity', capability: 'See and copy this build’s identity', home: 'Settings', path: 'Rail → Settings → Advanced → Diagnostics → Copy build identity',
    context: 'Help or tray menu → Copy Build Identity; palette → Copy build identity (version, commit)', walk: [SETTINGS, section('Advanced'), t('text', 'Diagnostics')], lands: btn('Copy build identity'),
    accelLands: { 'act:copy-build': t('text', 'Build identity copied') },
    verified: [R('Settings help, usage, connectors, appearance, transfer, diagnostics, history'), { spec: 'build-identity.journey.spec.ts', test: 'version search copies the build identity through the shared action' }] },
  { id: 'settings.folder', capability: 'The Hollow folder: where projects and the vault live', home: 'Settings', path: 'Rail → Settings → Sources → Hollow folder',
    walk: [SETTINGS, section('Sources')], lands: t('heading', 'Hollow folder'), verified: [R('rail destinations open their pages')] },
  // Move Hollow home (2026-11-12): the dry run, the move and rollback; agents use system.move_hollow_home* with cards.
  { id: 'settings.move-home', capability: 'Move the vault and Projects into one real Hollow folder: check the move (changes nothing), quit and move, roll back', home: 'Settings',
    path: 'Rail → Settings → Sources → Hollow folder → Move Hollow home → Check the move / Quit and move… / Roll back…',
    walk: [SETTINGS, section('Sources'), t('text', 'Move Hollow home')], lands: btn('Check the move'),
    verified: [{ spec: 'move-home.spec.ts', test: 'Move Hollow home: Check the move shows the report, Quit and move waits for a running chat' },
      { spec: 'move-home.spec.ts', test: 'Move Hollow home: a finished move shows its checks and Roll back…' }] },
  // Settings, Sources, Where your work lives (H4b, T5): three role rows; a switch shows its dry run, then a confirm; Roll back.
  { id: 'settings.sources', capability: 'Where your work lives: tasks and projects (Vault only or Vault plus Linear, switched after a dry run, rolled back), read-only knowledge references, personal context and workspace task sources', home: 'Settings',
    path: 'Rail → Settings → Sources → Where your work lives → Tasks and projects / Knowledge / Personal context / Workspace task sources',
    walk: [SETTINGS, section('Sources'), t('text', 'Where your work lives')], lands: t('heading', 'Tasks and projects'),
    verified: [{ spec: 'sources-settings.spec.ts', test: 'shows the three role rows with their labels' },
      { spec: 'sources-settings.spec.ts', test: 'a switch shows its dry run first and writes nothing until confirmed' }] },
  // H6 (with CX21, F2): the first-run setup sheet; opens by itself after the first folder setup, again from here.
  { id: 'settings.first-run', capability: 'One welcome page: agents Ready or Details, ACP registry, optional context, imports and sources, first workspace or Chats, and Start without an agent', home: 'Settings',
    path: 'Rail → Settings → About → Open welcome page',
    context: 'Opens automatically for unfinished first-run setup; Sources keeps Run setup again',
    walk: [SETTINGS, section('About'), t('button', 'Open welcome page')], lands: t('heading', 'Your agents'),
    verified: [{ spec: 'first-run-setup.spec.ts', test: 'About reopens welcome, and Sources keeps its existing route' },
      { spec: 'first-run-setup.spec.ts', test: 'closing leaves setup open and a reload returns to the welcome page' }] },
  // Settings, Context (CX17): one row per context layer; agents read settings.context_layers_status, the rest is the person's.
  { id: 'settings.context-layers', capability: 'Context layers: connect this Mac’s context when another Mac appears (clone personal and org, name this Mac, write its home file; Not now, Not for this Mac); name this Mac and its home file; connect, sync, stop or keep local your personal context (Create as guided steps); the read-only org clone with Check access; project folders, a pre-push context check per repo and vault sync status', home: 'Settings',
    path: 'Rail → Settings → Sources → Context → Connect this Mac’s context (Connect, Not now, Not for this Mac) / Home / Personal (Sync now, Stop syncing, Connect, Create a private repo, Local only) / Organization (Check access) / Project folders / Context check (Install context check, Remove) / Vault sync',
    walk: [SETTINGS, section('Sources'), t('text', 'Context')], lands: t('text', 'Connect an existing repo'),
    verified: [R('rail destinations open their pages')] },
  // Repo features R14b: off by default; off deletes the index.
  { id: 'settings.content-index', capability: 'Search inside project files: the local content index agents rank connected files by, with its progress', home: 'Settings',
    path: 'Rail → Settings → Advanced → Search & indexes → Search inside project files (local index)', context: 'Library → Connected projects’ coverage line says “Contents indexed: N of M”',
    walk: [SETTINGS, section('Advanced'), t('text', 'Search & indexes')], lands: t('switch', 'Search inside project files (local index)'),
    verified: [{ spec: 'next-level-settings.spec.ts', test: 'Search inside project files: the switch shows progress, and off says so' }] },
  { id: 'settings.automation', capability: 'What runs without being asked: scheduled automations, the heartbeat and COO check-ins; a link row to the windows editor and run queue on Automations', home: 'Settings', path: 'Rail → Settings → Background work',
    context: 'Automations, windows and the run queue are on the Automations page (Settings PR 4). Run at login is in General, Chats at once in Chat',
    walk: [SETTINGS, section('Background work')], lands: t('heading', 'Background work'), verified: [R('rail destinations open their pages')] },
  { id: 'settings.shortcuts', capability: 'Change or turn off keyboard shortcuts', home: 'Settings', path: 'Rail → Settings → Keyboard shortcuts',
    walk: [SETTINGS, section('Keyboard shortcuts')], lands: t('heading', 'Keyboard shortcuts'), verified: [R('rail destinations open their pages')] },
  { id: 'settings.chat', capability: 'How conversations behave: send key, canvases, retitling, the permission mode for new chats, chats at once', home: 'Settings', path: 'Rail → Settings → Chat',
    walk: [SETTINGS, section('Chat')], lands: t('heading', 'Chat'), verified: [R('rail destinations open their pages')] },
  { id: 'settings.agents', capability: 'Agents and their models and effort', home: 'Settings', path: 'Rail → Settings → Models',
    context: 'Keeping agent CLIs up to date is Settings → Accounts → Agent updates (composer model menu → agent status; failed-turn row and unavailable-send notice). The remote runner is in Devices; each agent’s watchdog and deadline in Advanced → Run limits',
    walk: [SETTINGS, section('Models')], lands: t('heading', 'Models'), verified: [R('rail destinations open their pages')] },
  { id: 'settings.accounts', capability: 'Agent accounts, sign-in and switching, and agent updates', home: 'Settings', path: 'Rail → Settings → Accounts',
    walk: [SETTINGS, section('Accounts')], lands: t('heading', 'Accounts'), verified: [R('rail destinations open their pages')] },
  { id: 'settings.self-hosted', capability: 'Self-hosted models: add, edit, switch off, remove and test your own model servers', home: 'Settings', path: 'Rail → Settings → Models → Self-hosted models',
    context: 'Add a self-hosted model, or Use the Qwen on Cloud Run preset beside it, which opens the form filled (inside the add form it fills it again); each model’s Test, Edit and Remove. Background jobs (a model per job kind, Run them now, and a held job’s Run as now, Try again and Discard) are in Advanced → Hollow’s own model calls',
    walk: [SETTINGS, section('Models')], lands: t('heading', 'Self-hosted models'), verified: [R('rail destinations open their pages')] },
  { id: 'settings.skills', capability: 'Skills: a link row to Library, Skills (the catalog moved there in Settings PR 4)', home: 'Settings', path: 'Rail → Settings → Connectors → Skills are in Library',
    walk: [SETTINGS, section('Connectors')], lands: t('text', 'Skills are in Library'), verified: [R('rail destinations open their pages')] },
  { id: 'settings.companions', capability: 'Companion devices, what the backend copies, the remote runner (Runs on, Builds and tests, Overnight and worker chats, Runner GitHub access) and, once a paired Mac runs a runner COO, COO on this Mac (the dispatcher)', home: 'Settings', path: 'Rail → Settings → Devices',
    walk: [SETTINGS, section('Devices')], lands: t('heading', 'Devices'), verified: [R('rail destinations open their pages')] },
  { id: 'settings.pair-mac', capability: 'Pair a Mac, so this one opens its chats; add, remove and reorder its SSH aliases; forget one; let another Mac connect to this one and revoke it', home: 'Settings', path: 'Rail → Settings → Devices → Pair a Mac',
    context: 'The machine strip’s Pair again (a Mac that needs pairing again) opens it with the alias filled; Let another Mac connect and the Macs paired to this one are below it',
    walk: [SETTINGS, section('Devices'), btn('Pair a Mac')], lands: t('form', 'Pair a Mac'),
    verified: [{ spec: 'peer-machines.spec.ts', test: 'Settings, Macs: pair a Mac, Pair again from the strip, let another Mac connect' }] },
  { id: 'settings.move-on-sleep', capability: 'Move running chats to the runner when this Mac sleeps (off unless turned on; the runner’s own logins pay for those turns)', home: 'Settings', path: 'Rail → Settings → Devices → Macs → Move running chats to the runner when this Mac sleeps',
    when: 'The paired runner takes chats, or the setting is on', unavailable: 'The Macs panel shows no such row; nothing moves on sleep', seed: 'paired-mac',
    walk: [SETTINGS, section('Devices')], lands: t('switch', 'Move running chats to the runner when this Mac sleeps'),
    verified: [{ spec: 'peer-machines.spec.ts', test: 'Settings, Macs: the sleep move is off until turned on, and saves either way' }] },
  { id: 'settings.about-you', capability: 'Your profile', home: 'Settings', path: 'Rail → Settings → Personalization',
    walk: [SETTINGS, section('Personalization')], lands: t('heading', 'Personalization'), verified: [R('rail destinations open their pages')] },
  { id: 'settings.memory', capability: 'Memories Hollow keeps, and what it learns', home: 'Settings', path: 'Rail → Settings → Personalization → Memory',
    walk: [SETTINGS, section('Personalization')], lands: t('heading', 'Memory'), verified: [R('rail destinations open their pages')] },
  { id: 'settings.prioritizer', capability: 'The context prioritizer', home: 'Settings', path: 'Rail → Settings → Advanced → Search & indexes → Context prioritizer',
    walk: [SETTINGS, section('Advanced'), t('text', 'Search & indexes')], lands: t('heading', 'Context prioritizer'), verified: [R('rail destinations open their pages')] },
  { id: 'settings.semantic', capability: 'Search by meaning (an on-device index)', home: 'Settings', path: 'Rail → Settings → Advanced → Search & indexes → Search by meaning',
    walk: [SETTINGS, section('Advanced'), t('text', 'Search & indexes')], lands: t('heading', 'Search by meaning'), verified: [R('rail destinations open their pages')] },
  { id: 'settings.recovery', capability: 'History & recovery: a link row to Library, History (the restore browser moved there in Settings PR 4)', home: 'Settings', path: 'Rail → Settings → Advanced → This Mac → History & recovery is in Library',
    walk: [SETTINGS, section('Advanced'), t('text', 'This Mac')], lands: t('text', 'History & recovery is in Library'), verified: [R('rail destinations open their pages')] },
  { id: 'settings.system', capability: 'Advanced, in closed groups: This Mac (capability doctor, backup, runtime, disk, History & recovery), Search & indexes, Hollow’s own model calls (with Background jobs), Run limits, App builds and web backend, Prices, Settings file, Diagnostics', home: 'Settings', path: 'Rail → Settings → Advanced',
    walk: [SETTINGS, section('Advanced')], lands: t('heading', 'Advanced'), verified: [R('rail destinations open their pages')] },
];

export const ROUTE_BY_ID: Readonly<Record<string, Route>> = Object.fromEntries(ROUTES.map(r => [r.id, r]));

/** A right-click menu and where each of its items is also a click (review
 *  decision C, matrix N3): no action is right-click only. routeLedger.test.ts
 *  checks every `onContextMenu` in src/ is listed; context-menus.spec.ts
 *  right-clicks each and finds every item on its visible route. */
export interface RightClick { file: string; on: string; visible: string; test: string }
const RC = (test: string) => `right-click parity: ${test}`;
export const RIGHT_CLICK: readonly RightClick[] = [
  { file: 'src/components/StudioHome.tsx', on: 'A project row on Projects', visible: 'Its ⋯ (Actions for …): the same items', test: RC('workspace rows and the rail') },
  { file: 'src/components/Rail.tsx', on: 'A sidebar project row', visible: 'The project’s ⋯ on Projects and in the top bar (its sidebar row has none since 11-07: a right-click opens the same menu; L8, 11-10: the sidebar lists no conversations)', test: RC('workspace rows and the rail') },
  { file: 'src/components/RailMachines.tsx', on: 'A chat in the machine strip’s popover (another Mac’s chat, M2)', visible: 'The row opens it; the open chat has Stop and Copy conversation ID; Rename, Move and Archive are its home Mac’s, shown disabled', test: RC('chats on another Mac') },
  { file: 'src/components/RemoteConversations.tsx', on: 'Another Mac’s chat in Library → Conversations (On <Mac>, or a project’s list)', visible: 'The row opens it; the open chat has Stop and Copy conversation ID; Rename, Move and Archive are its home Mac’s, shown disabled', test: RC('chats on another Mac') },
  { file: 'src/components/RailQuickChats.tsx', on: 'A sidebar Quick chats row (2026-11-14)', visible: 'The ⋯ beside the open chat’s title (the same conversation menu), and Library → Conversations rows', test: RC('workspace rows and the rail') },
  { file: 'src/components/RowChildren.tsx', on: 'A subagent chat nested under its lead in the sidebar (3.6)', visible: 'The ⋯ beside the nested row (the same conversation menu)', test: RC('workspace rows and the rail') },
  { file: 'src/components/ConversationHistory.tsx', on: 'A row in Conversation history (Library → Conversations)', visible: 'Its ⋯ (Actions for …) and its Archive or Restore button; the open one’s title ⋯', test: RC('conversation history rows') },
  { file: 'src/pages/Workflows.tsx', on: 'An automation row', visible: 'Its Run now; Edit in its Automation options; Delete in Edit', test: RC('automation rows') },
  { file: 'src/pages/Skills.tsx', on: 'A skill row (Library → Skills)', visible: 'The skill’s details: Use in <workspace>, and on your own skill Edit skill and Delete skill', test: RC('skill rows') },
  { file: 'src/components/ReplyFileChips.tsx', on: 'A reply’s changed-file chip', visible: 'The reply’s More actions → Show this reply’s changes in history, and Review <file> for a chip that is a Result (its click opens the Preview pane with its review bar)', test: RC('a reply’s changed files') },
];

/** The shared actions that reach a row, by kind of accelerator. */
export function acceleratorsOf(route: string): { palette: string[]; go: string[]; keys: string[]; menu: string[] } {
  const defs = ACTION_DEFS.filter(d => d.route === route);
  const of = (k: 'palette' | 'go' | 'key' | 'menu') => defs.map(d => d[k]).filter((x): x is string => !!x);
  return { palette: of('palette'), go: of('go'), keys: of('key'), menu: of('menu') };
}

/** The rows an accelerator reaches. */
export function routesFor(kind: 'palette' | 'go' | 'keys', id: string): Route[] {
  const k = kind === 'keys' ? 'key' : kind;
  return ACTION_DEFS.filter(d => d[k] === id).map(d => ROUTE_BY_ID[d.route]).filter(Boolean);
}

/** Where accelerator `id` (a palette, go or key id) of `r` lands. */
export function accelLanding(r: Route, id: string): Target {
  const a = r.accelLands;
  if (!a) return r.lands;
  if ('role' in a) return a as Target;
  return (a as Record<string, Target>)[id] ?? r.lands;
}

/** A target in words: `button “Plan” in navigation “Workspace sections”`. */
export function describeTarget(x: Target): string {
  const own = x.role === 'text' ? `“${x.name}”` : x.role === 'label' ? `the “${x.name}” control` : `${x.role} “${x.match === 'contains' ? '…' : ''}${x.name}${x.match && x.match !== 'exact' ? '…' : ''}”`;
  const where = (x.fill ? `type “${x.fill}” in ${own}` : own) + (x.in ? ` in ${describeTarget(x.in)}` : '');
  return x.orElse ? `${where} (small window: ${x.orElse.map(describeTarget).join(', ')})` : where;
}

/** The conditional actions of a row: what they wait for, and what the
 *  person is told otherwise (actionDefs.ts CONDITIONS). */
export function conditionsOf(route: string): { action: ActionDef; when: string; unavailable: string }[] {
  return ACTION_DEFS.filter(d => d.route === route && d.when).map(d => ({ action: d, ...CONDITIONS[d.when!] }));
}

/** docs/ROUTE_LEDGER.md, generated. */
export function renderRouteLedger(): string {
  const cell = (s: string) => s.replace(/\|/g, '\\|');
  const acc = (r: Route) => {
    const a = acceleratorsOf(r.id);
    return [...a.palette.map(p => `palette \`${p}\``), ...a.go.map(g => `go \`${g}\``), ...a.keys.map(k => `key \`${k}\``), ...a.menu.map(m => `menu \`${m}\``)].join(', ') || '—';
  };
  const when = (r: Route) => r.when ? `${r.when}. Otherwise: ${r.unavailable}` : 'Always';
  const walk = (r: Route) => [...r.walk.map(describeTarget), `→ ${describeTarget(r.lands)}`].join('; ');
  const ver = (r: Route) => ['`route-ledger.journey.spec.ts` (its walk)', ...r.verified.map(v => `\`${v.spec}\` “${v.test}”`)].join('; ');
  const homes = [...new Set(ROUTES.map(r => r.home))];
  const lines = [
    '# Capability route ledger',
    '',
    'Generated from `src/routeLedger.ts` and `src/actionDefs.ts`: run `node scripts/route-ledger.mjs` after editing them;',
    '`src/routeLedger.test.ts` fails while this file is stale. The rule it enforces: every user-facing capability has a visible',
    'click path; palette actions, `g` go-to targets, ⌘ shortcuts and menu-bar items are accelerators to one of these rows, never',
    'the only way in (consolidation D, 2026-10-13; [programme ledger](CONSOLIDATION_LEDGER.md)).',
    '',
    'Each row’s walk is executable: `tests/smoke/route-ledger.journey.spec.ts` clicks it from the demo Home at desktop, the',
    'native minimum window and Largest text and expects its landmark, then runs each accelerator and expects the same landmark.',
    'Conditional rows say what the person sees while the condition doesn’t hold. Accelerators come from the shared action',
    'definitions (`src/actionDefs.ts`): their effect, condition and words are in [ACTIONS.md](ACTIONS.md).',
    '',
    `${ROUTES.length} capabilities in ${homes.length} homes.`,
  ];
  for (const home of homes) {
    lines.push('', `## ${home}`, '', '| Capability | Visible path | In context | When | Accelerators | Walk | Verified by |', '|---|---|---|---|---|---|---|');
    for (const r of ROUTES.filter(x => x.home === home)) {
      lines.push(`| ${cell(r.capability)} (\`${r.id}\`) | ${cell(r.path)} | ${cell(r.context ?? '—')} | ${cell(when(r))} | ${acc(r)} | ${cell(walk(r))} | ${cell(ver(r))} |`);
    }
  }
  return lines.join('\n') + '\n';
}

/** docs/ACTIONS.md, generated: every shared action with its effect,
 *  condition, ways in and the ledger row whose walk proves it. */
export function renderActions(): string {
  const cell = (s: string) => s.replace(/\|/g, '\\|');
  const ways = (d: ActionDef) => [d.palette && `palette \`${d.palette}\``, d.go && `go \`${d.go}\``, d.key && `key \`${d.key}\``, d.menu && `menu \`${d.menu}\``].filter(Boolean).join(', ');
  const words = (d: ActionDef) => Object.entries(d.labels ?? {}).map(([k, v]) => `${k}: “${v}”`).join('; ') || '—';
  const lines = [
    '# Shared actions',
    '',
    'Generated from `src/actionDefs.ts` by `node scripts/route-ledger.mjs`; `src/routeLedger.test.ts` fails while this file is',
    'stale. One definition per action a person can run from more than one place. The palette, the `g` go-to keys, the ⌘',
    'shortcuts, sidebar creation and the macOS menu bar all run it through one dispatcher (`src/actionRun.ts`), so its',
    'behaviour, its condition and its words are set once. Its visible click path and proof are its row in the',
    '[route ledger](ROUTE_LEDGER.md).',
    '',
    '## Effects',
    '',
    '| Effect | Means |', '|---|---|',
    ...Object.entries(EFFECT_WORDS).map(([k, v]) => `| \`${k}\` | ${v} |`),
    '',
    '“Confirms” marks an action whose effect waits for a preview or a confirmation you answer.',
    '',
    '## Conditions',
    '',
    '| Condition | Offered | Otherwise the person is told |', '|---|---|---|',
    ...Object.entries(CONDITIONS).map(([k, c]) => `| \`${k}\` | ${c.when} | ${cell(c.unavailable)} |`),
    '',
    `## Actions (${ACTION_DEFS.length})`,
    '',
    '| Action | Effect | Condition | Ways in | Own words per surface | Ledger row (visible path) |', '|---|---|---|---|---|---|',
    ...ACTION_DEFS.map(d => `| ${cell(actionLabel(d))} (\`${d.id}\`) | ${d.effect}${d.confirm ? ', confirms' : ''} | ${d.when ? `\`${d.when}\`` : 'Always'} | ${ways(d) || '—'} | ${cell(words(d))} | \`${d.route}\`: ${cell(ROUTE_BY_ID[d.route]?.path ?? '(missing)')} |`),
  ];
  return lines.join('\n') + '\n';
}
