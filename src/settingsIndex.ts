/**
 * The Settings registry's eager half (build run 11-17, App size pass): each
 * section's id, label, icon, scope, parts and visible control labels, the
 * old ids and labels that still resolve, and the resolvers that turn a deep
 * link into a section. The launch path (`settingsResolve.ts`, through
 * `store/runtime.ts`) needs only these, so they ship in the App chunk.
 *
 * The search words for every section, part and control live in
 * `settingsSections.ts`, which merges them onto this table and owns search;
 * only the command palette and Settings itself load it, both lazily. A new
 * control gets its label here and its keywords there.
 *
 * Each `id` maps 1:1 to a <SettingsSection> in src/pages/Settings.tsx, and
 * each part to a <SettingsPart> inside it (Settings PR 3, 2026-11-12:
 * twelve sections; a former section or part kept whole keeps its old id).
 * `settings_keys.rs` reads this file's ids and aliases in a Rust test.
 */
import type { IconName } from './icons';

/** How a section's changes are kept (plan completion L4, matrix X11). */
export type SaveMode = 'as-you-change' | 'save-button' | 'mixed' | 'actions';

/** What a section's settings reach and when a change takes hold, said in
 *  its header (review V05 "Settings scope", matrix X11). */
export interface SettingsScope {
  /** Who or what it changes: this turn, future chats, scheduled runs, every
   *  agent, this workspace, this Mac. */
  affects: string;
  /** When a change takes hold. */
  takesEffect: string;
  saves: SaveMode;
}

export interface SettingsControlMeta {
  label: string;
  /** Synonyms a person might type, never shown. Set only by
   *  `settingsSections.ts` (its `SETTINGS_KEYWORDS`). */
  keywords?: string;
  /** The control a jump lands on when this one isn't shown, such as the
   *  switch that reveals it (P1-21). */
  near?: string;
  /** The part it sits in (a former section's id, S2). Set by the registry. */
  part?: string;
  /** The part's own heading: a jump lands on the part itself. */
  heading?: boolean;
}

/** A former section kept whole inside a section (S2), without its search
 *  words. Its id stays its DOM id (`settings-<id>`) and an alias of the
 *  section, so old links land on it; its label is its heading. */
export interface SettingsPartEntry {
  id: string;
  label: string;
  controls: readonly SettingsControlMeta[];
}

/** A section without its search words (`SettingsSectionMeta` adds them). */
export interface SettingsSectionEntry {
  id: string;
  label: string;
  icon: IconName;
  /** Visible headings and controls inside the section (audit H06). Search
   *  matches them and jumps to the one found; `label` must be the text shown
   *  on screen (a vitest guard checks the Settings sources contain it).
   *  Derived: the section's own controls, then each part's heading and
   *  controls, tagged with the part. */
  controls?: SettingsControlMeta[];
  /** Former sections kept whole inside this one (S2), in page order. */
  parts?: readonly SettingsPartEntry[];
  scope: SettingsScope;
}

type SectionSource = Omit<SettingsSectionEntry, 'controls'> & { controls: SettingsControlMeta[] };

// Settings PR 3 (overnight 11-12, the settings redesign plan 3.2): twelve
// sections in one list, named for what a person looks for. The seven before
// them (Interface, Agents, Connections, Knowledge, Automation, Usage, System)
// were split up with their panels whole; their ids, their parts' ids and the
// ids they had absorbed are aliases of the new ones (below and in
// `settings_keys.rs`), so every deep link, palette row and agent setting
// keeps landing. Advanced holds closed groups (3.3): a search hit inside one
// opens it. A panel that holds rows of two new homes stays whole in one of
// them until a later PR splits it (the swarm model in Background work, the
// web backend in Devices). S5b (11-12) split the agent form: the runner rows
// went to Devices, run limits and App builds to Advanced, Background jobs to
// Advanced, Hollow's own model calls; a label decides first, so their old
// (section, label) pairs land there (`settingsSectionForLabel`).
const SECTIONS: readonly SectionSource[] = [
  {
    id: 'general', label: 'General', icon: 'moon',
    scope: { affects: 'How Hollow looks, interrupts you and starts on this Mac', takesEffect: 'At once; a custom theme once you save it', saves: 'mixed' },
    controls: [{ label: 'Appearance' }, { label: 'App icon' }, { label: 'COO icon' }, { label: 'Texture' }, { label: 'Ambient motion' }, { label: 'Motion' }, { label: 'Panel animation' }, { label: 'Density' }, { label: 'Text size' }, { label: 'In-app pop-ups' }, { label: 'macOS system notifications' }, { label: 'Chat announcements' }, { label: 'Proactive nudges' }, { label: 'Run at login' }, { label: 'Keep running when I close the window' }, { label: 'Text font' }, { label: 'Code font' }, { label: 'Import VS Code theme', near: 'Appearance' }],
  },
  {
    // Canvas C3 wave 2 C4, P1-22: how agents answer, how you send, how new
    // chats start; and how many run at once (from Automation).
    id: 'chat', label: 'Chat', icon: 'message',
    scope: { affects: 'Every chat, with every agent: how it starts, how you send and how many run at once', takesEffect: 'The next message or new chat', saves: 'as-you-change' },
    controls: [{ label: 'Canvases' }, { label: 'Enter sends a message' }, { label: 'Retitle new chats' }, { label: 'Permission mode for new chats' }, { label: 'Chats running at once' }, { label: 'Chats at once in one workspace' }, { label: 'Subagents running at once from one chat' }],
  },
  {
    // No provider names in the scope (P1-22). Plan 3.2 row 3: the agent,
    // its model, effort and the self-hosted models; nothing about where it runs.
    id: 'models', label: 'Models', icon: 'settings',
    scope: { affects: 'New chats and runs: which agent and model, and how hard it thinks', takesEffect: 'New runs and the next message; agent defaults once you press Save agent defaults', saves: 'mixed' },
    controls: [{ label: 'Default agent' }, { label: 'When an agent runs out' }, { label: 'Models' }, { label: 'Effort' }, { label: 'Models the CFO may use' }],
    parts: [
      // Self-hosted models Q4 (2026-11-03). No gcloud here: that word finds
      // Advanced's tool folders alone.
      { id: 'selfhosted', label: 'Self-hosted models', controls: [{ label: 'Add a self-hosted model' }, { label: 'Use the Qwen on Cloud Run preset', near: 'Add a self-hosted model' }] },
    ],
  },
  {
    // No cost or token words: those find Usage alone.
    id: 'accounts', label: 'Accounts', icon: 'users',
    scope: { affects: 'Which account each agent runs as, and when Hollow updates each agent', takesEffect: 'The next run; a running conversation keeps its account', saves: 'mixed' },
    controls: [{ label: 'Claude' }, { label: 'Other providers' }, { label: 'Jev (TypeSafe)' }, { label: '1Password' }, { label: 'Agent updates' }, { label: 'Update all', near: 'Agent updates' }],
  },
  {
    // P1-22 (D4): About you and Memory. Saved and replaced memories live in
    // Library, Learnings (#1238); Memory's link opens that one home.
    id: 'personalization', label: 'Personalization', icon: 'book',
    scope: { affects: 'Every agent, in every workspace and automation: your profile while Share with agents is on, and the memories you approved', takesEffect: 'The next message or run', saves: 'mixed' },
    controls: [{ label: 'Share with agents' }, { label: 'Your profile' }, { label: 'Waiting for you' }, { label: 'Memory' }, { label: 'How many learnings to suggest' }],
  },
  {
    id: 'connectors', label: 'Connectors', icon: 'globe',
    scope: { affects: 'Apps and tools agents can use, in chats and the automations you allow', takesEffect: 'The next turn or run', saves: 'mixed' },
    controls: [{ label: 'Services' }, { label: 'Agenda calendar' }, { label: 'Built into Hollow' }, { label: 'Computer control' }, { label: 'Your connectors' }, { label: 'Add connectors' }, { label: 'Recordings' }, { label: 'Recordings in Hollow' }, { label: 'Skills are in Library' }],
  },
  {
    // H4 and A14's home: where Hollow's own work lives. Hollow folder keeps
    // its old part id (`vault`), so its links and styles still find it.
    id: 'sources', label: 'Sources', icon: 'folder',
    scope: { affects: 'Where Hollow keeps your work and where its tasks and projects come from', takesEffect: 'The task tracker at its next sync; the Hollow folder once you press Save', saves: 'mixed' },
    controls: [{ label: 'Where your work lives' }, { label: 'Tasks and projects' }, { label: 'Knowledge' }, { label: 'Personal context' }, { label: 'Workspace task sources' }, { label: 'Task tracker' }, { label: 'Run setup again' }],
    parts: [
      { id: 'vault', label: 'Hollow folder', controls: [{ label: 'Bring existing folders' }, { label: 'Move Hollow home' }] },
      // Settings, Context (CX17): one row per context layer. `context` is an
      // old alias of Advanced (Context prioritizer), so this part is `layers`.
      { id: 'layers', label: 'Context', controls: [{ label: 'Home' }, { label: 'Personal' }, { label: 'Organization' }, { label: 'Project folders' }, { label: 'Context check' }, { label: 'Vault sync' }] },
    ],
  },
  {
    id: 'background', label: 'Background work', icon: 'refresh',
    scope: { affects: 'Work that runs without you on this Mac: scheduled automations, the heartbeat and COO check-ins', takesEffect: 'Switches and limits at once; automation windows once you press Save windows', saves: 'mixed' },
    controls: [{ label: 'Scheduled automations' }, { label: 'Automations, windows and the run queue are on the Automations page' }, { label: 'Silent heartbeat' }, { label: 'COO check-ins' }, { label: 'Act on its own' }, { label: 'Start fresh after an hour away' }, { label: 'Largest swarm (chats)' }, { label: 'Swarm parts at once' }, { label: 'Swarm model' }],
  },
  {
    // Companion apps T0 (2026-10-12): the devices paired with this Mac.
    id: 'devices', label: 'Devices', icon: 'appWindow',
    scope: { affects: 'The runner and the devices paired with this Mac', takesEffect: 'A device and the backend copy once you pair or save them', saves: 'mixed' },
    controls: [{ label: 'Pair a device' }, { label: 'iPhone app' }, { label: 'Backend for the web app' }, { label: 'What the backend copies', near: 'Backend for the web app' },
      // M3a PR2: the COO as dispatcher, beside the Macs panel (M2 PR4); hidden until a paired Mac runs a runner COO, so a jump lands on Macs.
      { label: 'COO on this Mac', near: 'Macs' },
      // M5 UI: the sleep move, in the Macs panel; shown once the runner takes chats, so a jump lands on Macs.
      { label: 'Move running chats to the runner when this Mac sleeps', near: 'Macs' },
      // S5b (plan 3.2 row 9): the runner, from Models' agent form. Its rows show once one is set up.
      { label: 'Remote runner' }, { label: 'Runs on', near: 'Remote runner' }, { label: 'Builds and tests', near: 'Remote runner' }, { label: 'Overnight and worker chats', near: 'Remote runner' }, { label: 'Runner GitHub access', near: 'Remote runner' }],
  },
  {
    // Overnight 11-08 S5: Votes and Routing are gone. An old `feedback` link
    // opens Usage on Budgets. Prices are in Advanced.
    id: 'usage', label: 'Usage', icon: 'graph',
    scope: { affects: 'Nothing in your work: it reports what each run cost. Budgets set here pause the work they name', takesEffect: 'Budgets from their next check, once saved', saves: 'mixed' },
    controls: [{ label: 'Read live from Claude' }, { label: 'In rotation' }, { label: 'Automatic account rotation' }, { label: 'Move on at' }, { label: 'Check usage' }, { label: 'Spending caps' }, { label: 'By model' }, { label: 'Outcomes' }, { label: 'Usage by day' }, { label: 'Budgets' }, { label: 'Reserve for your own prompts' }, { label: 'Extra usage for unattended work' }, { label: 'Budget for each subagent' }, { label: 'Project budgets' }, { label: 'COO daily hard stop' }, { label: 'Overnight cap' }, { label: 'Value this month' }],
  },
  {
    id: 'shortcuts', label: 'Keyboard shortcuts', icon: 'keyboard',
    scope: { affects: 'Hollow’s keys on this Mac (three of them system-wide) and the menu bar', takesEffect: 'At once', saves: 'as-you-change' },
    controls: [{ label: 'Use keyboard shortcuts' }, { label: 'Change' }, { label: 'Reset all to defaults' }, { label: 'Show all keys' }, { label: 'Swipe to go back and forward' }],
  },
  {
    id: 'diffs', label: 'Diffs', icon: 'gitBranch',
    scope: { affects: 'How a build workspace’s changes read on this Mac', takesEffect: 'At once', saves: 'as-you-change' },
    controls: [{ label: 'Wrap long lines' }, { label: 'Show line numbers' }],
  },
  {
    id: 'browser', label: 'Browser', icon: 'globe',
    scope: { affects: 'Where web links open when you click them in Hollow on this Mac', takesEffect: 'At the next click', saves: 'as-you-change' },
    controls: [{ label: 'Open web links in' }],
  },
  {
    id: 'worktrees', label: 'Worktrees', icon: 'folder',
    scope: { affects: 'The git worktrees build workspaces leave on this Mac', takesEffect: 'When you confirm a clean-up', saves: 'actions' },
    controls: [{ label: 'Build workspace worktrees' }],
  },
  {
    id: 'archived', label: 'Archived', icon: 'archive',
    scope: { affects: 'Settled conversations: Restore brings one back to its project', takesEffect: 'At once', saves: 'actions' },
    controls: [{ label: 'Archived conversations' }],
  },
  {
    id: 'updates', label: 'Updates', icon: 'refresh',
    scope: { affects: 'Hollow updates on this Mac', takesEffect: 'At once', saves: 'as-you-change' },
    controls: [{ label: 'Channel' }, { label: 'Install updates overnight' }, { label: 'Check now' }, { label: 'Update token', near: 'Channel' }, { label: 'Connect Hollow', near: 'Channel' }, { label: 'Disconnect', near: 'Channel' }],
  },
  {
    id: 'about', label: 'About', icon: 'book',
    scope: { affects: 'Welcome and setup on this Mac', takesEffect: 'When you open the welcome page', saves: 'mixed' },
    controls: [{ label: 'Welcome to Hollow' }],
  },
  {
    // Overnight 11-08 S5 (D5) made one Advanced fold in System; the redesign
    // (3.3) makes it the twelfth entry, with closed groups. History &
    // recovery moved to Library, History in PR 4; This Mac links to it.
    id: 'advanced', label: 'Advanced', icon: 'terminal',
    scope: { affects: 'This Mac: its health, disk, indexes and diagnostics, Hollow’s own model calls, run limits, app builds and prices', takesEffect: 'Each button acts when you press it; forms once you press Save', saves: 'mixed' },
    controls: [{ label: 'This Mac' }, { label: 'Capability doctor' }, { label: 'Backup evidence' }, { label: 'Backup management', near: 'Backup evidence' }, { label: 'History & recovery is in Library' }, { label: 'Runtime' }, { label: 'Disk space' }, { label: 'Hollow’s own model calls' }, { label: 'Extraction model' }, { label: 'Background jobs' }, { label: 'Run limits' }, { label: 'No-output watchdog' }, { label: 'Run deadline' }, { label: 'App builds and web backend' }, { label: 'App builds' }, { label: 'Prices' }, { label: 'Settings file' }, { label: 'Diagnostics' }, { label: 'Diagnostics bundle' }, { label: 'Build identity' }, { label: 'Folders agents can run tools from' }, { label: 'Launch calls' }, { label: "Hollow's databases" }, { label: 'Debug logs' }],
    parts: [
      // A closed group of its own (D5). Aliases `context`, `vaultsearch` and
      // `indexes` open it.
      { id: 'indexes', label: 'Search & indexes', controls: [{ label: 'Search inside project files' }, { label: 'Search by meaning' }, { label: 'Index my vault for semantic search' }, { label: 'Indexing progress', near: 'Index my vault for semantic search' }, { label: 'Context prioritizer' }, { label: 'Jev relevance model' }, { label: 'Jev duplicate check' }] },
    ],
  },
];

/** A section with its parts' headings and controls in its flat list. */
function withParts(s: SectionSource): SettingsSectionEntry {
  const parts = s.parts ?? [];
  return {
    ...s,
    controls: [...s.controls, ...parts.flatMap(p => [
      { label: p.label, part: p.id, heading: true },
      ...p.controls.map(c => ({ ...c, part: p.id })),
    ])],
  };
}

/** Every section, in index order, with its flat control list. */
export const SETTINGS_SECTIONS: readonly SettingsSectionEntry[] = SECTIONS.map(withParts);

/** Each section's entry by id. */
export const SETTINGS_INDEX_BY_ID: Readonly<Record<string, SettingsSectionEntry>> = Object.fromEntries(
  SETTINGS_SECTIONS.map(s => [s.id, s]));

/**
 * Old section id → the section that holds it now, one step to the final
 * section. Applied wherever a section id arrives from outside
 * (requestOpenSettingsSection callers, the Rust-emitted `settings:heartbeat`
 * deep link, capability-doctor fixSection values, `settings.get`) so
 * older links keep landing. `settings_keys.rs` keeps the same table for
 * `settings.get` (a vitest guard holds the two together). A label decides
 * first: `settingsSectionForLabel` sends a jump on to the section that
 * shows the control it names (plan section 6).
 */
export const SETTINGS_SECTION_ALIASES: Readonly<Record<string, string>> = {
  // Settings PR 3 (11-12): the seven sections before the twelve.
  interface: 'general',
  defaults: 'models',
  operator: 'personalization',
  automation: 'background',
  system: 'advanced',
  // Retired before S2.
  capture: 'connectors',
  cadence: 'connectors',
  doctor: 'advanced',
  customization: 'models',
  agenttools: 'connectors',
  teamwork: 'connectors',
  computer: 'connectors',
  m365write: 'connectors',
  startup: 'general',
  routines: 'background',
  heartbeat: 'background',
  appearance: 'general',
  notifications: 'general',
  paths: 'sources',
  debug: 'advanced',
  setup: 'sources',
  // P1-22 (D4): fewer first-level sections.
  feedback: 'usage',
  context: 'advanced',
  vaultsearch: 'advanced',
  learning: 'personalization',
  companions: 'devices',
  // Run 15 P4: From other apps and its settings are removed.
  imports: 'connectors',
  // S2's parts (`shortcuts`, `chat` and `accounts` are sections again).
  skills: 'connectors',
  indexes: 'advanced',
  vault: 'sources',
  history: 'advanced',
  // Self-hosted models Q4: a part of Models.
  selfhosted: 'models',
  // Settings, Context (CX17): a part of Sources.
  layers: 'sources',
};

/** A retired section's deep link lands on the control it became (P1-22),
 *  or on its part's heading (S2). */
export const SETTINGS_ALIAS_CONTROLS: Readonly<Record<string, string>> = {
  doctor: 'Capability doctor',
  startup: 'Run at login',
  notifications: 'In-app pop-ups',
  routines: 'Scheduled automations',
  heartbeat: 'Silent heartbeat',
  debug: 'Debug logs',
  feedback: 'Budgets',
  context: 'Context prioritizer',
  vaultsearch: 'Search by meaning',
  learning: 'Memory',
  paths: 'Hollow folder',
  setup: 'Hollow folder',
  skills: 'Skills',
  indexes: 'Search & indexes',
  vault: 'Hollow folder',
  history: 'History & recovery',
  selfhosted: 'Self-hosted models',
  layers: 'Context',
};

/** Controls that left a section for another (Settings PR 8: Spread usage
 *  and the Claude account limits moved from Accounts to Usage, Limits). A
 *  link to the old place with one of these labels lands on the new one;
 *  `defaults` (the pre-PR 3 id) gets there too, since its label decides
 *  first (`settingsSectionForLabel`). Applied last in `resolveSettingsTarget`. */
export const SETTINGS_MOVED_CONTROLS: Readonly<Record<string, { from: string; to: string }>> = {
  'Automatic account rotation': { from: 'accounts', to: 'usage' },
  'Spread usage': { from: 'accounts', to: 'usage' },
  'Check usage': { from: 'accounts', to: 'usage' },
  'Move on at': { from: 'accounts', to: 'usage' },
  'In rotation': { from: 'accounts', to: 'usage' },
  'Resume on another account when one runs out': { from: 'accounts', to: 'usage' },
};

/** The section a (resolved) section and label land on, after moves. */
export function resolveMovedControl(section: string, label?: string): string {
  const moved = label ? SETTINGS_MOVED_CONTROLS[label] : undefined;
  return moved && moved.from === section ? moved.to : section;
}

export function resolveSettingsSection(id: string): string {
  return SETTINGS_SECTION_ALIASES[id] ?? id;
}

export function resolveSettingsControl(section: string, label?: string): string | undefined {
  // The old "Hollow’s own sources" link went to Notion setup, removed with Notion sync (11-10).
  if (section === 'connectors' && label === 'Hollow’s own sources') return 'Task tracker';
  return section === 'general' && label === 'Experimental visuals' ? 'Ambient motion' : label;
}

/** The live section that shows the control `label` names (plan section 6:
 *  a label decides first). A jump to ('automation', 'Chats running at
 *  once') lands in Chat, ('defaults', 'Agent updates') in Accounts. A label
 *  the section shows, an unknown one or none keeps `section`. */
export function settingsSectionForLabel(section: string, label?: string): string {
  if (!label || SETTINGS_INDEX_BY_ID[section]?.controls?.some(c => c.label === label)) return section;
  return SETTINGS_SECTIONS.find(s => s.controls?.some(c => c.label === label))?.id ?? section;
}
