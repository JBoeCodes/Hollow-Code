// "Where things went" (Settings PR 9, 2026-11-12): the old-to-new list for the
// Settings redesign, and the date window for the page header's link to it.
// docs/WHATS_NEW.md carries the same list under "Where everything in Settings
// went"; `whereThingsWent.test.ts` keeps the two from drifting. Pure data and
// one date check, so the window is testable with an injected clock.

/** The link is shown through this day (two weeks after the redesign landed). */
export const WHERE_THINGS_WENT_UNTIL = '2026-11-26';

/** Whether the header link still shows: through the whole of the last day, in local time. */
export function whereThingsWentShown(now: Date = new Date()): boolean {
  const [y, m, d] = WHERE_THINGS_WENT_UNTIL.split('-').map(Number);
  return now.getTime() < new Date(y, m - 1, d + 1).getTime();
}

export interface WentGroup { from: string; rows: ReadonlyArray<{ what: string; now: string }> }

/** Each old section and its rows, with where they are now. */
export const WHERE_THINGS_WENT: readonly WentGroup[] = [
  { from: 'Interface', rows: [
    { what: 'Theme cards, custom themes, the themes folder', now: 'General, Appearance: one Theme menu, and Edit themes' },
    { what: 'Motion, Texture, Ambient motion, Density, Text size', now: 'General, Appearance' },
    { what: 'App and Main agent icons', now: 'General: App icon and COO icon (Upload an icon, Generate a logo)' },
    { what: 'Notifications, Proactive nudges, Scan now, Clear history', now: 'General, Notifications' },
    { what: 'Keyboard shortcuts', now: 'Keyboard shortcuts, in Commands and Go to keys tabs' },
    { what: 'Run at login, Keep running when I close the window', now: 'General, Startup' },
  ] },
  { from: 'Agents', rows: [
    { what: 'Default agent, When an agent runs out, each agent’s model and effort', now: 'Models' },
    { what: 'Remote runner, Runs on, Builds and tests, Overnight and worker chats, Runner GitHub access', now: 'Devices' },
    { what: 'App builds', now: 'Advanced, App builds and web backend' },
    { what: 'No-output watchdog, Run deadline', now: 'Advanced, Run limits' },
    { what: 'Extraction model', now: 'Advanced, Hollow’s own model calls' },
    { what: 'Agent updates', now: 'Accounts, Agent updates' },
    { what: 'Canvases, Enter sends, Retitle new chats, Permission mode for new chats', now: 'Chat' },
    { what: 'Claude accounts: add, edit, remove', now: 'Accounts, Claude' },
    { what: 'Spread usage, Move on at, Resume on another account, Check usage, In rotation', now: 'Usage, Limits' },
    { what: 'Codex, Antigravity and Grok sign-in; the Jev key', now: 'Accounts, Other providers and Jev' },
    { what: 'Self-hosted models', now: 'Models, Self-hosted models (Background jobs: Advanced, Hollow’s own model calls)' },
    { what: 'Skills: the catalog, New skill, Import skill', now: 'Library, Skills' },
  ] },
  { from: 'Connections', rows: [
    { what: 'Your connectors, Add connectors, GitHub, Microsoft 365', now: 'Connectors (one line each; Add a connector opens a sheet)' },
    { what: 'Service health table', now: 'Gone: each connector row shows its own state and Repair' },
    { what: 'Agenda calendar source and Calendar access', now: 'Connectors, Agenda calendar' },
    { what: 'Built into Hollow, Teammate roles', now: 'Connectors, Built into Hollow (Edit roles opens a sheet)' },
    { what: 'Recordings (Plaud)', now: 'Connectors, Recordings' },
    { what: 'Pair a device, paired devices, the iPhone app, Tailscale', now: 'Devices' },
    { what: 'Keep Hollow running with the lid closed', now: 'Devices, iPhone app' },
    { what: 'Web app backend, push notifications key', now: 'Devices, Backend for the web app' },
  ] },
  { from: 'Knowledge', rows: [
    { what: 'Share with agents, Keep learning, Your profile', now: 'Personalization' },
    { what: 'Profile questions and Ready to add', now: 'Library, Learnings (a count stays in Personalization, Waiting for you)' },
    { what: 'Learning suggestions', now: 'Personalization, How many learnings to suggest' },
  ] },
  { from: 'Automation', rows: [
    { what: 'Scheduled automations', now: 'Background work (a link to Automations)' },
    { what: 'Automation windows, the run queue', now: 'Automations (a fold beside the schedule dial)' },
    { what: 'Chats running at once, Chats at once in one workspace', now: 'Chat' },
    { what: 'Silent heartbeat, the COO check-ins, Act on its own, the swarm', now: 'Background work' },
    { what: 'Main agent daily hard stop, Overnight cap', now: 'Usage, Spending caps (edited there only)' },
  ] },
  { from: 'Usage', rows: [
    { what: 'Today, This week, This month, the chart, the tables', now: 'Usage: All agents and one tab per agent, with Last 30 days' },
    { what: 'Claude and Codex plan usage', now: 'Usage, Limits' },
    { what: 'Budgets', now: 'Usage, Budgets' },
    { what: 'Minutes saved per accepted run', now: 'Usage, Value this month' },
  ] },
  { from: 'System', rows: [
    { what: 'Capability doctor, Backup evidence, Runtime, Disk space', now: 'Advanced, This Mac' },
    { what: 'Developer readouts, Debug logs, Diagnostics, Build identity', now: 'Advanced, Diagnostics' },
    { what: 'Hollow folder, Bring existing folders', now: 'Sources, Hollow folder' },
    { what: 'History & recovery', now: 'Library, History' },
    { what: 'Search inside project files, Search by meaning, the Jev checks', now: 'Advanced, Search & indexes' },
    { what: 'Prices, Export and Import settings', now: 'Advanced, Prices and Settings file' },
    { what: 'Start locally buttons, Help: All keys, Startup warm-up, System health', now: 'Gone: the File menu, the Help menu, the command palette and the Health check cover them' },
  ] },
];

/** The list as plain text lines, `Old place, row: new home`, for the sheet and the What's new entry. */
export function wentLines(group: WentGroup): string[] {
  return group.rows.map(r => `${r.what}: ${r.now}`);
}
