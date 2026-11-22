// What runs without being asked, explained together (plan completion L2,
// assignment item 3; review UI_UX "Automation / Runbook / Tonight" and
// "Settings: … app automation, scheduled automations, silent heartbeat and
// Main agent coexist"). One row per mechanism, the same five questions each:
// when it runs, what it can see, what it may do on its own, what it leaves
// behind, and where to change it. Defaults are the shipped ones and say so;
// each row points at the control that holds the live value.
//
// Sources (keep in step): `workflows.rs` / `app_broker::execute` (an
// automation never runs an External Hollow action and runs a confirm one only
// when its allowed tools name it), `workflow_retry.rs` + `run_effects.rs`
// (one automatic retry, never after outside effects), `heartbeat.rs`
// (HeartbeatConfig defaults, Read/Glob/Grep tools, Write only under
// drafts/heartbeat/, no Bash, no connectors), `main_agent.rs` SCHEDULE_NOTE
// and `main_agent::unattended` (docs/COO.md M3/M4), `night_queue.rs` +
// docs/SCHEDULER.md (the tonight window and overnight cap).

export type BackgroundSystemId = 'automations' | 'heartbeat' | 'mainAgent' | 'windowJobs';

/** The Silent heartbeat in one sentence, for this page and Settings,
 *  Automation alike (P1-04: the two used to disagree). */
export const HEARTBEAT_PURPOSE = 'A quiet agent pass over what changed in your notes, calendar and tasks: it prepares meeting context and suggests tasks for you to approve, and sends nothing.';

export interface BackgroundSystem {
  id: BackgroundSystemId;
  name: string;
  /** One line: what it is for. */
  purpose: string;
  when: string;
  scope: string;
  authority: string;
  outputs: string;
  /** Where its live settings are: the visible route. */
  change: { label: string; target: 'automations' | 'settingsAutomation'; control?: string };
}

export const BACKGROUND_SYSTEMS: readonly BackgroundSystem[] = [
  {
    id: 'automations',
    name: 'Scheduled automations',
    purpose: 'A procedure you set up once and checked, rerun on a schedule or a trigger.',
    when: 'On its schedule or trigger (a file change, an overdue task, a meeting soon), or when you press Run now. One run holds the vault at a time; others wait in the queue.',
    scope: 'Its workspace’s brief, plan, sources and Results, or the working folder it names; one with no folder works in the whole vault.',
    authority: 'Only the settings you approved in its editor: its permission mode and tool lists. It can’t ask you anything. Through Hollow it never takes an action that leaves this Mac, and a confirm-level one only when its allowed tools name it; connector tools and shell commands those settings allow can still act outside, and each run lists what they did.',
    outputs: 'Files in the vault (its result files), a record of what it changed and did, and a notification per its setting. A failed run retries once on its own only when nothing outside its files may have happened.',
    change: { label: 'Each automation’s Edit', target: 'automations' },
  },
  {
    id: 'heartbeat',
    name: 'Silent heartbeat',
    purpose: HEARTBEAT_PURPOSE,
    when: 'Hollow checks every 10 minutes for free; an agent pass runs only when notes changed, within work hours (by default 7:00 to 19:00), at least an hour apart, at most 10 a day.',
    scope: 'Your vault’s notes.',
    authority: 'Reading and searching notes. With draft writing on it can also write files; its instructions keep drafts under drafts/heartbeat/, and History records what it wrote. No shell, no connectors, nothing sent.',
    outputs: 'Notifications and task suggestions you approve or decline, plus any drafts.',
    change: { label: 'Settings → Background work → Silent heartbeat', target: 'settingsAutomation', control: 'Silent heartbeat' },
  },
  {
    id: 'mainAgent',
    name: 'COO check-ins',
    purpose: 'Your cross-workspace helper keeping an eye on work in progress.',
    when: 'When you message it; once when a subagent it started or a swarm it ran ends; and at check-ins (by default every 30 minutes, 8:00 to 18:00 on working days) only when something new came up, or for the day’s first.',
    scope: 'Its own conversation, with what waits on you, failed runs and the subagents it started across workspaces.',
    authority: 'On its own it works in its conversation’s own permission mode, and its Hollow actions are only the safe ones: start chats (which may edit files without asking but never run unrestricted) or stop them, change tasks, write notes in its own folder. Anything else becomes a suggestion for you. What it starts itself stops at its daily cap (by default $100).',
    outputs: 'A short check-in in today’s COO thread, suggestions you answer with Do it / Not now / Never, and the subagents it started.',
    change: { label: 'Settings → Background work → COO', target: 'settingsAutomation', control: 'COO check-ins' },
  },
  {
    id: 'windowJobs', name: 'Window jobs',
    purpose: 'Hollow’s built-in maintenance and draft jobs.',
    when: 'Once per configured window: the weekly update on Friday afternoon, and cleanup review, workspace snapshots and friction review nightly. Missed work catches up once on wake.',
    scope: 'Local workspace records, notes and recorded activity.',
    authority: 'These are built-in host jobs. Cleanup and friction prepare review findings; they never apply cleanup or start a fix on their own.',
    outputs: 'A weekly update draft, cleanup review card, workspace snapshots and friction counts.',
    change: { label: 'Automations → Automation windows', target: 'automations', control: 'Automation windows' },
  },
];

/** The rule every row shares, said once. */
export const BACKGROUND_COMMON =
  'These mechanisms leave records of their work. History can undo captured vault changes. Running on their own, they can’t stop to ask you: what needs you waits in Needs you.';
