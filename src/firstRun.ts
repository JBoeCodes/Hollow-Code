// The first-run setup sheet's step logic and words (H6, with CX21 and F2;
// design `sources-and-import-2026-11-13` section 5). Pure, unit-tested in
// firstRun.test.ts. The sheet is `components/FirstRunSheet.tsx`; its
// progress lives in config.json `setup` (Rust `first_run.rs`).
import type { SetupStatus, SetupStep, SetupStepId, SetupStepState } from './firstRunApi';
import type { AgentDescriptor } from './types';

export const STEP_IDS: readonly SetupStepId[] = ['context', 'find', 'sources', 'agents', 'start'];

export const STEP_TITLES: Record<SetupStepId, string> = {
  context: 'Connect your context',
  find: 'Find what’s already here',
  sources: 'Where your work lives',
  agents: 'Your agents',
  start: 'Start',
};

/** One line under each step's title. */
export const STEP_LINES: Record<SetupStepId, string> = {
  context: 'Where Hollow reads your context from. The vault is set up; the rest is optional.',
  find: 'Bring in past conversations and the instruction files your agents already use.',
  sources: 'Tasks and knowledge stay in your vault unless you connect a task tracker.',
  agents: 'Which agents are installed on this Mac and signed in. Nothing to change here.',
  start: 'Name a first workspace, or start with a chat.',
};

/** The sheet's title (its dialog name). */
export const SHEET_TITLE = 'Welcome to Hollow';
export const ACP_REGISTRY_URL = 'https://agentclientprotocol.com/registry';

/** Complete only open fields, preserving old progress and making partial failures retryable. */
export async function finishWelcome(
  status: SetupStatus,
  save: (step: SetupStepId, outcome: 'done' | 'skipped') => Promise<SetupStatus>,
  progress: (status: SetupStatus) => void,
): Promise<void> {
  for (const id of STEP_IDS) {
    if (stateOf(status.steps, id) !== 'open') continue;
    status = await save(id, id === 'start' ? 'done' : 'skipped');
    progress(status);
  }
}

export interface WelcomeAgent extends AgentRow { capabilities: AgentDescriptor['capabilities'] }

/** The adapter registry owns membership; status reads may only enrich its rows. */
export function welcomeAgents(
  descriptors: readonly AgentDescriptor[],
  clis: Parameters<typeof agentRows>[0],
  accounts: Parameters<typeof agentRows>[1],
): WelcomeAgent[] {
  return descriptors.map(d => {
    const cli = clis.find(c => c.agent === d.id);
    const account = accounts.find(a => a.id === d.id);
    return { agent: d.id, label: d.label, installed: d.available && (cli?.installed ?? true),
      version: cli?.version ?? null, signedIn: account?.signedIn ?? null, capabilities: d.capabilities };
  });
}

export const welcomeReady = (row: WelcomeAgent): boolean => row.installed && row.signedIn === true;

export function welcomeCapabilityLines(row: WelcomeAgent): string[] {
  const c = row.capabilities;
  return [c.modelSelection && 'Choose a model', c.sessionResume && 'Resume conversations',
    c.ownedMcp && 'Use shared connectors', c.steer && 'Steer a running conversation',
    c.reasoning && 'Show reported thinking'].filter((v): v is string => !!v);
}

/** Whether the sheet should open for this progress. */
export const shouldOpen = (status: SetupStatus | null | undefined): boolean => !!status && !status.complete && !!status.current;

/** The step index the sheet opens at: the first open step, else the first. */
export function startIndex(status: SetupStatus | null | undefined): number {
  const at = status?.current ? STEP_IDS.indexOf(status.current) : -1;
  return at < 0 ? 0 : at;
}

/** A step's state in `steps` (missing: open). */
export function stateOf(steps: readonly SetupStep[], id: SetupStepId): SetupStepState {
  return steps.find(s => s.id === id)?.state ?? 'open';
}

/** Where the sheet goes after step `from` is marked: the next open step
 *  after it, else the first open one before it, else null (all finished). */
export function nextOpen(steps: readonly SetupStep[], from: number): number | null {
  for (let i = from + 1; i < STEP_IDS.length; i++) if (stateOf(steps, STEP_IDS[i]) === 'open') return i;
  for (let i = 0; i <= Math.min(from, STEP_IDS.length - 1); i++) if (stateOf(steps, STEP_IDS[i]) === 'open') return i;
  return null;
}

/** Back from step `at`: the one before it, never below the first. */
export const prevIndex = (at: number): number => Math.max(0, at - 1);

/** Whether a step shows Back. */
export const hasBack = (at: number): boolean => at > 0;

/** A step dot's accessible name: "Step 2 of 5: Find what’s already here, skipped". */
export function dotLabel(at: number, state: SetupStepState): string {
  const id = STEP_IDS[at];
  const word = state === 'done' ? ', done' : state === 'skipped' ? ', skipped' : '';
  return `Step ${at + 1} of ${STEP_IDS.length}: ${STEP_TITLES[id]}${word}`;
}

/** The step counter under the dots. */
export const stepCounter = (at: number): string => `Step ${at + 1} of ${STEP_IDS.length}`;

/** The primary button's words: the last step finishes. */
export const continueLabel = (at: number): string => (at === STEP_IDS.length - 1 ? 'Finish' : 'Continue');

/** An agent row on Your agents, from Settings, Accounts' readouts. */
export interface AgentRow {
  agent: string;
  label: string;
  installed: boolean;
  version: string | null;
  /** null: not told (Claude Code's own login, or a check that failed). */
  signedIn: boolean | null;
}

/** "Installed (2.1.3), signed in", "Not installed", "Installed, could not check sign-in". */
export function agentLine(row: AgentRow): string {
  if (!row.installed) return 'Not installed';
  const version = row.version ? ` (${row.version})` : '';
  if (row.signedIn === null) return `Installed${version}`;
  return `Installed${version}, ${row.signedIn ? 'signed in' : 'signed out'}`;
}

/** The pill's tone, Settings, Accounts' words. */
export function agentState(row: AgentRow): 'ok' | 'missingToken' | 'unknown' {
  if (!row.installed || row.signedIn === false) return 'missingToken';
  return row.signedIn === null ? 'unknown' : 'ok';
}

/** Merge the CLI rows (installed, version) with the sign-in rows (Codex,
 *  Antigravity, Grok; Claude Code's login is its own and isn't read). */
export function agentRows(
  clis: readonly { agent: string; label: string; installed: boolean; version: string | null }[],
  accounts: readonly { id: string; label: string; installed: boolean; signedIn: boolean | null }[],
): AgentRow[] {
  const rows: AgentRow[] = clis.map(c => {
    const account = accounts.find(a => a.id === c.agent);
    return { agent: c.agent, label: c.label, installed: c.installed, version: c.version, signedIn: account ? account.signedIn : null };
  });
  for (const a of accounts) {
    if (!rows.some(r => r.agent === a.id)) rows.push({ agent: a.id, label: a.label, installed: a.installed, version: null, signedIn: a.signedIn });
  }
  return rows;
}

/** "2 of 4 agents ready" over the rows: installed and not signed out. */
export function agentsSummary(rows: readonly AgentRow[]): string {
  const ready = rows.filter(r => r.installed && r.signedIn !== false).length;
  if (!rows.length) return 'No agents found.';
  return `${ready} of ${rows.length} agents ready`;
}

/** Start's workspace name: why it can't be used, or null. Empty is fine (optional). */
export function workspaceNameProblem(name: string): string | null {
  const trimmed = name.trim();
  if (!trimmed) return null;
  if (trimmed.length > 80) return 'Keep the name under 80 characters.';
  if (/[/\\]/.test(trimmed)) return 'A name can’t contain a slash.';
  return null;
}

/** How many conversations the committed import batches hold
 *  (`session_import_status`'s `batches`), so Start can say what was
 *  imported after a quit and resume, when this session imported nothing. */
export function importedFromBatches(batches: readonly { conversations: number }[]): number {
  return batches.reduce((sum, b) => sum + Math.max(0, b.conversations), 0);
}

/** Start's summary of the import and the suggestions waiting in Learnings.
 *  `imported` is a conversation count, or null when it isn't known (then
 *  nothing is said about the import). */
export function startSummary(imported: number | null, suggestions: number | null): string[] {
  const out: string[] = [];
  if (imported !== null && imported > 0) {
    out.push(`Imported ${imported} conversation${imported === 1 ? '' : 's'}. They are in Library, Conversations.`);
  } else if (imported === 0) {
    out.push('No conversations imported. Settings, Sources, Where your work lives has Run setup again any time.');
  }
  if (suggestions && suggestions > 0) {
    out.push(`${suggestions} suggestion${suggestions === 1 ? '' : 's'} waiting in Library, Learnings.`);
  }
  return out;
}
