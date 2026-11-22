// Move Hollow home (2026-11-12): the words around the dry run's plan and the
// run's journal (Rust `move_home.rs`). Settings, Sources, Hollow folder,
// Move Hollow home (`settings/MoveHomePanel.tsx`) renders what these return,
// so its decisions (what the report says, why Quit and move is disabled,
// when to poll, when Roll back shows) are unit-tested here. Pure.
import type { Blocker, MoveHomePlan, MoveHomeStatus, MoveKind, PlannedMove } from './types';

const KB = 1024;
const MB = 1024 ** 2;
const GB = 1024 ** 3;
const TB = 1024 ** 4;

/** `512 KB`, `4.2 MB`, `86 MB`, `1.2 GB`, `46 GB`, `1.1 TB`. */
export function moveSize(bytes: number): string {
  if (bytes >= TB) return `${(bytes / TB).toFixed(1)} TB`;
  if (bytes >= 10 * GB) return `${Math.round(bytes / GB)} GB`;
  if (bytes >= GB) return `${(bytes / GB).toFixed(1)} GB`;
  if (bytes >= 10 * MB) return `${Math.round(bytes / MB)} MB`;
  if (bytes >= MB) return `${(bytes / MB).toFixed(1)} MB`;
  return `${Math.max(bytes > 0 ? 1 : 0, Math.round(bytes / KB))} KB`;
}

/** `1 file`, `1,200 files`. */
export function count(n: number, one: string, many = `${one}s`): string {
  return `${n.toLocaleString('en-US')} ${n === 1 ? one : many}`;
}

/** The last path segment, for a folder named in a sentence. */
export function baseName(path: string): string {
  const parts = path.replace(/\/+$/, '').split('/');
  return parts[parts.length - 1] || path;
}

export const KIND_LABEL: Record<MoveKind, string> = {
  vault: 'Vault',
  projectsRoot: 'Projects folder',
  project: 'Folder beside the vault',
  personal: 'Personal projects',
  archive: 'To Archive',
};

/** `Vault: /a/hollow-vault → /h/Vault · 2.1 GB · 18,400 files, 300 in iCloud only (1.2 GB)`;
 *  a folder in iCloud adds ` · copied out of iCloud`. */
export function moveLine(m: PlannedMove): string {
  const icloud = m.dataless ? `, ${m.dataless.toLocaleString('en-US')} in iCloud only${m.datalessBytes ? ` (${moveSize(m.datalessBytes)})` : ''}` : '';
  const copy = m.method === 'copy' ? ' · copied out of iCloud' : '';
  return `${KIND_LABEL[m.kind] ?? m.kind}: ${m.from} → ${m.to} · ${moveSize(m.bytes)} · ${count(m.files, 'file')}${icloud}${copy}`;
}

/** `40 s`, `12 min`, `3 h 5 min`: a rough time. */
export function roughTime(secs: number): string {
  if (secs < 90) return `${Math.max(1, Math.round(secs))} s`;
  if (secs < 90 * 60) return `${Math.ceil(secs / 60)} min`;
  const m = Math.ceil(secs / 60);
  return `${Math.floor(m / 60)} h ${m % 60} min`;
}

/** The report's first line: how much moves where, and the free space. */
export function moveSummary(plan: MoveHomePlan): string {
  if (!plan.moves.length) return `Nothing to move: the vault and Projects are already in ${plan.hollowHome}.`;
  const files = plan.moves.reduce((n, m) => n + m.files, 0);
  const copies = plan.moves.filter(m => m.method === 'copy').length;
  const volume = !plan.sameVolume ? 'Another disk: the move is refused (Hollow only moves folders within one disk).'
    : copies ? `Same disk. ${count(copies, 'folder')} in iCloud ${copies === 1 ? 'is' : 'are'} copied out (about ${roughTime(plan.dataless.copySecs)}), the rest renamed.`
    : 'Same disk, so each move is a rename.';
  return `Moves ${count(plan.moves.length, 'folder')} (${moveSize(plan.totalBytes)}, ${count(files, 'file')}) into ${plan.hollowHome}. ${moveSize(plan.freeBytes)} free. ${volume}`;
}

/** Blockers that stop the run, then the ones that are only told, each in
 *  its kind's order (chats first). */
export function groupBlockers(plan: MoveHomePlan): { blocking: Blocker[]; other: Blocker[] } {
  const order = (b: Blocker) => b.kind === 'chat' ? 0 : b.kind === 'process' ? 1 : 2;
  const sorted = [...plan.blockers].sort((a, b) => order(a) - order(b));
  return { blocking: sorted.filter(b => b.blocking), other: sorted.filter(b => !b.blocking) };
}

/** Why Quit and move is disabled; empty when it can run. */
export function startBlockedReason(plan: MoveHomePlan | null, status: MoveHomeStatus | null): string {
  if (!plan) return 'Check the move first.';
  if (status && (status.state === 'staged' || status.state === 'running')) return 'A move is already under way.';
  if (!plan.moves.length) return 'Nothing to move.';
  if (plan.ready) return '';
  const { blocking } = groupBlockers(plan);
  const chats = blocking.filter(b => b.kind === 'chat').length;
  if (chats) return `Wait for ${count(chats, 'chat')} to finish or stop ${chats === 1 ? 'it' : 'them'}, then check again.`;
  if (blocking.length) return `${blocking[0].detail}${blocking.length > 1 ? ` (and ${blocking.length - 1} more below)` : ''}`;
  if (!plan.sameVolume) return 'The Hollow folder is on another disk. Hollow only moves folders within one disk.';
  if (plan.partial) return 'The check ran out of time before it finished. Check again.';
  return 'Something in the report stops the move. See the lines marked above.';
}

/** One block of the report: a heading and its lines. `warn` marks a block
 *  that needs the person's attention. */
export interface ReportSection { id: string; title: string; lines: string[]; warn?: boolean }

const plural = (n: number, one: string, many: string) => n === 1 ? one : many;

/** The dry run as the panel shows it, in the order a person checks it:
 *  what moves, what links stay, what is re-keyed, clashes, iCloud-only
 *  files, what blocks, repos, agents' records, login items and scripts,
 *  backup, notes. Empty blocks are left out. */
export function planSections(plan: MoveHomePlan): ReportSection[] {
  const sections: ReportSection[] = [];
  const add = (s: ReportSection) => { if (s.lines.length) sections.push(s); };

  add({
    id: 'moves', title: 'What moves',
    lines: [
      ...plan.moves.map(m => m.replacesSymlink ? `${moveLine(m)} (replaces the link at ${m.replacesSymlink})` : moveLine(m)),
      ...plan.mkdirs.map(d => `New folder: ${d}`),
    ],
  });
  add({ id: 'links', title: 'Links left at the old places', lines: plan.moves.filter(m => m.compatSymlink).map(m => `${m.from} links to ${m.to}`) });
  add({
    id: 'rekey', title: 'Records updated to the new paths',
    lines: [
      ...(plan.oldStem !== plan.newStem ? [`Vault stores renamed from ${plan.oldStem} to ${plan.newStem}`] : []),
      ...plan.stores.map(s => `${s.what}: ${s.from} → ${s.to}`),
      ...plan.columns.map(c => `Conversations ${c.table}.${c.column}: ${count(c.rows, 'row')}`),
      ...plan.files.map(f => `${f.path}: ${count(f.changes, 'change')} (${f.what})`),
      ...plan.ids.map(i => `Workspace ${i.old} becomes ${i.new} for ${baseName(i.projectTo)}${i.hasWorkspace ? '' : ' (no workspace folder yet)'}`),
      ...plan.orphans.map(o => `No project for workspace ${o}: left as it is`),
    ],
  });
  add({
    id: 'clashes', title: 'Name clashes',
    lines: plan.collisions.map(c => c.winner ? `${c.detail} Kept in Projects: ${c.winner}. The other goes to Archive.` : c.detail),
  });
  const d = plan.dataless;
  add({
    id: 'icloud', title: 'Files in iCloud only',
    lines: d.total || d.copyFolders.length ? [
      ...(d.total ? [`${count(d.total, 'file')} ${plural(d.total, 'is', 'are')} in iCloud only (${moveSize(d.bytes)} once downloaded).${d.partial ? ' The count stopped early, so there may be more.' : ''}`] : []),
      ...(d.copyFolders.length ? [
        `${count(d.copyFolders.length, 'folder')} in iCloud ${plural(d.copyFolders.length, 'moves', 'move')} by copy, not rename: a rename out of iCloud took minutes per 30,000 files and failed on the first real run. Each folder is cloned beside its new place (no extra disk space), compared entry by entry, and its original removed last; about ${roughTime(d.copySecs)} for all of them.`,
      ] : []),
      ...(d.toDownload ? [
        `Download first: ${count(d.toDownload, 'file')} (${moveSize(d.toDownloadBytes)}) in those folders, about ${roughTime(d.downloadSecs)}. Copying one downloads it alone and waits, so the move stays blocked until they are local; the command is under “What must stop first”.${d.dirs ? ` ${count(d.dirs, 'folder')} ${plural(d.dirs, 'is', 'are')} iCloud-only, so the size and time are a floor.` : ''}`,
      ] : []),
      ...(d.fileProviderDomain ? [`Kept by ${d.fileProviderDomain}.`] : []),
      ...d.roots.flatMap(r => [
        `${r.root}: ${count(r.count, 'file')}${r.bytes ? ` (${moveSize(r.bytes)})` : ''}${r.top.length ? `, most in ${r.top.map(([dir, n]) => `${dir} (${n.toLocaleString('en-US')})`).join(', ')}` : ''}`,
        ...(r.sample.length ? [`For example: ${r.sample.join(', ')}`] : []),
      ]),
    ] : [],
  });
  const { blocking, other } = groupBlockers(plan);
  add({
    id: 'blockers', title: 'What must stop first', warn: blocking.length > 0,
    lines: [
      ...blocking.map(b => `${b.detail} (stops the move)`),
      ...other.map(b => b.detail),
      ...plan.processes,
    ],
  });
  add({
    id: 'repos', title: 'Git repositories to repair',
    lines: plan.repos.map(r => r.skipped
      ? `${r.repoTo}: skipped, ${r.skipped}`
      : `${r.repoTo}: ${count(r.worktrees, 'worktree')} to repair${r.broken ? `, ${r.broken} already broken (reported, left alone)` : ''}`),
  });
  add({
    id: 'agents', title: 'Agents’ records',
    lines: plan.agents.map(a => [
      `${a.agent} (${a.path}): ${count(a.changes, 'change')}`,
      a.merges ? `${count(a.merges, 'folder')} merged` : '',
      a.dead ? `${a.dead} for folders that no longer exist, reported and kept` : '',
      a.ambiguous.length ? `not sure about ${a.ambiguous.join(', ')}, left alone` : '',
    ].filter(Boolean).join(', ')),
  });
  add({
    id: 'launch', title: 'Login items and scripts that name a moved path',
    lines: [...plan.launchAgents, ...plan.scripts].map(l => `${l.label} (${l.path}): ${l.onPlan ? 'updated by the move' : 'listed only, not changed'}`),
  });
  add({ id: 'backup', title: 'Backup', warn: plan.backup.covered === false, lines: [backupLine(plan)] });
  add({ id: 'notes', title: 'Notes', lines: plan.notes });
  return sections;
}

/** Whether a backup covers the Hollow folder, in words. */
export function backupLine(plan: MoveHomePlan): string {
  const head = plan.backup.covered === true ? 'A Time Machine backup covers the Hollow folder.'
    : plan.backup.covered === false ? 'No Time Machine backup covers the Hollow folder. Back it up before you move.'
      : 'Hollow couldn’t tell whether a backup covers the Hollow folder.';
  return plan.backup.detail ? `${head} ${plan.backup.detail}` : head;
}

/** The option's label and the folders it would add. */
export function siblingsLine(plan: MoveHomePlan): string {
  if (!plan.besideVault.length) return '';
  return `${count(plan.besideVault.length, 'folder')} beside the vault: ${plan.besideVault.map(baseName).join(', ')}. They join Projects.`;
}

const STEP_LABEL: Record<string, string> = {
  probe: 'Move probe',
  snapshot: 'Backup snapshot',
  mkdirs: 'New folders',
  moves: 'Moving folders',
  move: 'Moving folders',
  symlinks: 'Links at the old places',
  stores: 'Renaming vault stores',
  sessions: 'Updating conversations',
  ids: 'Workspace ids',
  config: 'Hollow’s settings',
  repos: 'Repairing git worktrees',
  agents: 'Agents’ records',
  launchd: 'Login items',
  scripts: 'Scripts',
  verify: 'Checking the result',
  retire: 'Removing the iCloud original',
  relaunch: 'Opening Hollow',
};

/** A journal step's name in words: known names, else the step id made
 *  readable (`move:vault` → `Move vault`). */
export function stepLabel(step: string): string {
  const [head, ...rest] = step.split(':');
  const known = STEP_LABEL[head];
  const tail = rest.join(':');
  if (known) return tail ? `${known}: ${tail}` : known;
  const words = step.replace(/[_:-]+/g, ' ').replace(/([a-z])([A-Z])/g, '$1 $2').trim().toLowerCase();
  return words ? words[0].toUpperCase() + words.slice(1) : step;
}

const STATE_WORDS: Record<MoveHomeStatus['state'], string> = {
  none: 'No move yet.',
  staged: 'Waiting for Hollow to quit, then the move starts.',
  running: 'Moving.',
  done: 'Moved.',
  failed: 'The move stopped.',
  rolledBack: 'Rolled back.',
};

/** The progress view's status line: state, steps done, the last result. */
export function statusLine(s: MoveHomeStatus): string {
  const done = s.steps.filter(x => x.state === 'done').length;
  const steps = s.steps.length && (s.state === 'running' || s.state === 'staged') ? ` ${done} of ${count(s.steps.length, 'step')} done.` : '';
  const failedChecks = s.checks.filter(c => !c.ok).length;
  const checks = s.checks.length ? ` ${failedChecks ? `${failedChecks} of ${count(s.checks.length, 'check')} failed.` : `All ${count(s.checks.length, 'check')} passed.`}` : '';
  const last = s.lastResult ? ` ${s.lastResult}` : '';
  return `${STATE_WORDS[s.state] ?? s.state}${steps}${checks}${last}`;
}

/** `done`, `under way`, `failed`, `undone` for a step's row. */
export function stepStateWord(state: MoveHomeStatus['steps'][number]['state']): string {
  return state === 'started' ? 'under way' : state;
}

/** Poll the journal while a move is staged or running. */
export function shouldPoll(s: MoveHomeStatus | null): boolean {
  return !!s && (s.state === 'staged' || s.state === 'running');
}

/** Roll back shows whenever the journal says a move can be undone; it is
 *  disabled (with the refusal shown) when the backend would refuse. */
export function rollbackState(s: MoveHomeStatus | null): { shown: boolean; disabledReason: string } {
  if (!s || !s.canRollBack) return { shown: false, disabledReason: '' };
  if (shouldPoll(s)) return { shown: true, disabledReason: 'Wait for the move to finish.' };
  return { shown: true, disabledReason: s.rollbackRefusal ?? '' };
}

/** The panel's second confirmation, in one sentence. */
export function confirmLine(plan: MoveHomePlan): string {
  return `Hollow quits, moves ${count(plan.moves.length, 'folder')} into ${plan.hollowHome} and opens again. A backup of every record it changes goes to ${plan.hollowHome}/Backups, so Roll back can undo it.`;
}
