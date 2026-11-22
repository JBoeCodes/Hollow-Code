// Settings, Advanced, Disk space (overnight D1): the words around
// `disk_guard`'s report. Pure, so the unit tests import it directly.
import type { DiskReport, InstallTargetsPlan, InstallTargetsPruned, StaleIndexesPlan, StaleIndexesRemoved, WorktreeHygieneReport, WorktreesCleaned } from './types';

const GB = 1024 ** 3;

/** `74 GB free · Hollow warns under 50 GB`, or the low warning naming
 *  which drive is low; with the vault's volume when it is another one
 *  (morning N6). */
export function diskSummary(r: DiskReport): string {
  const v = r.vault;
  const homeLow = r.freeBytes != null && r.freeBytes < r.thresholdGb * GB;
  const vaultLow = !!v?.low;
  const home = r.freeBytes == null ? "Free space couldn't be read" : `${Math.round(r.freeBytes / GB)} GB free`;
  const vaultPart = v ? `; the vault's volume ${v.freeBytes == null ? "couldn't be read" : `has ${Math.round(v.freeBytes / GB)} GB free`}` : '';
  const which = homeLow && vaultLow ? 'both drives are' : vaultLow ? "the vault's volume is" : homeLow ? (v ? 'your home drive is' : '') : '';
  if (homeLow && !v) return `${home}: under the ${r.thresholdGb} GB Hollow warns at`;
  if (which) return `${home}${vaultPart}: ${which} under the ${r.thresholdGb} GB Hollow warns at`;
  return `${home}${vaultPart} · Hollow warns under ${r.thresholdGb} GB`;
}

/** The report after a threshold change, without measuring again. */
export function withThreshold(r: DiskReport, thresholdGb: number): DiskReport {
  const under = (b: number | null | undefined) => b != null && b < thresholdGb * GB;
  const vault = r.vault ? { ...r.vault, low: under(r.vault.freeBytes) } : r.vault;
  return { ...r, thresholdGb, vault, low: under(r.freeBytes) || !!vault?.low };
}

/** A "Warn under" entry as a threshold, or why not (1–10,000 whole GB). */
export function parseThreshold(text: string): { gb: number } | { error: string } {
  const t = text.trim();
  if (!/^\d+$/.test(t)) return { error: 'Enter a whole number of GB.' };
  const gb = Number(t);
  return gb >= 1 && gb <= 10_000 ? { gb } : { error: 'Warn under 1 to 10,000 GB.' };
}

const KIND: Record<DiskReport['folders'][number]['kind'], string> = {
  'cargo-target': 'Build cache',
  worktree: 'Worktree',
  cache: 'Cache',
};

/** `142 GB`, `1.5 GB`, `340 MB`. */
export function sizeWords(bytes: number): string {
  return bytes >= GB ? `${(bytes / GB).toFixed(bytes >= 10 * GB ? 0 : 1)} GB` : `${Math.max(1, Math.round(bytes / 1024 ** 2))} MB`;
}

/** `Build cache · 142 GB`, or `not sized in time` rather than a guess. */
export function folderSize(f: DiskReport['folders'][number]): string {
  return `${KIND[f.kind]} · ${f.bytes == null ? 'not sized in time' : sizeWords(f.bytes)}`;
}

// ── The two cleanups (overnight H20) ─────────────────────────────────────
// The panel's buttons preview, show these words in the confirm dialog, then
// delete exactly the previewed list (its hash). They say what the agents'
// cards for build.prune_install_targets and build.clean_worktrees say
// (Rust `install_targets::describe`, `worktree_hygiene::describe`), in the
// panel's words, with each folder's size from the last measure when it has
// one. Those two point back here: when either side's wording changes (what
// goes, what stays and why), change the other.

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
const name = (path: string) => path.split('/').filter(Boolean).pop() ?? path;
/** The last measure's size of `path`, as ` · 12 GB`; empty when unknown. */
function measured(report: DiskReport | null, path: string): string {
  const bytes = report?.folders.find(f => f.path === path)?.bytes;
  return bytes == null ? '' : ` · ${sizeWords(bytes)}`;
}
/** `, about 20 GB as last measured` (`at least` when some weren't sized); empty when none were. */
function measuredTotal(report: DiskReport | null, paths: string[]): string {
  const sizes = paths.map(p => report?.folders.find(f => f.path === p)?.bytes).filter((b): b is number => b != null);
  if (!sizes.length) return '';
  return `, ${sizes.length === paths.length ? 'about' : 'at least'} ${sizeWords(sizes.reduce((a, b) => a + b, 0))} as last measured`;
}

/** What Delete old install builds… says when there is nothing to delete. */
export const NOTHING_TO_PRUNE = 'Nothing to delete: every install build folder is kept. Nothing changed.';
/** What Remove finished worktrees… says when there is nothing to remove. */
export const NOTHING_TO_CLEAN = 'Nothing to remove: no worktree is merged, unchanged and pushed, and there is no "~" folder. Nothing changed.';
/** `lsof` failed: the backend keeps every worktree and "~" folder
 *  (`inUseUnknown`; the agent's card says "Couldn't read which processes
 *  are running"). */
export const CANT_READ_RUNNING = "Hollow couldn't read which programs are running, so no worktree is removed.";
/** In a confirm dialog opened before Check disk space measured anything. */
export const SIZES_UNKNOWN = 'Sizes unknown: press Check disk space first.';

/** What Remove finished worktrees… says when its preview lists nothing:
 *  why, when that is because running programs couldn't be read (then the
 *  list is always empty, so the dialog never opens to say it). */
export function nothingToCleanText(r: WorktreeHygieneReport): string {
  return r.inUseUnknown ? `${CANT_READ_RUNNING} Nothing changed.` : NOTHING_TO_CLEAN;
}

/** The backend's refusal when the list changed between the preview and the
 *  delete (the hash no longer matches). Written for an agent's card
 *  (`install_targets::prune_in`, `worktree_hygiene::clean_for`: "… changed
 *  since the card was shown. Nothing was …; ask again …"), so matched here
 *  rather than reworded there. */
const LIST_CHANGED = /changed since the card was shown/;

/** A cleanup's error in the panel's words: the changed-list refusal says
 *  what happened and what to press; anything else passes through. */
export function cleanupErrorText(message: string, verb: 'deleted' | 'removed'): string {
  return LIST_CHANGED.test(message)
    ? `The list changed since you saw it. Nothing was ${verb}; press the button again to see the new list.`
    : message;
}

/** The confirm's title for deleting old install build folders. */
export function pruneConfirmTitle(plan: InstallTargetsPlan): string {
  return `Delete ${plural(plan.remove.length, 'old install build folder')}?`;
}

/** The confirm's title for the worktree cleanup: what kinds go, and how many. */
export function cleanConfirmTitle(r: WorktreeHygieneReport): string {
  const worktrees = r.repos.reduce((n, x) => n + x.worktrees.filter(w => w.removable).length, 0);
  const tildes = r.repos.reduce((n, x) => n + x.tildeFolders.length, 0);
  const parts = [worktrees ? plural(worktrees, 'finished worktree') : '', tildes ? plural(tildes, '"~" folder') : ''].filter(Boolean);
  return `Remove ${parts.join(' and ') || plural(r.remove.length, 'path')}?`;
}

/** The confirm's words for deleting old install build folders: every folder
 *  that goes (with its size when measured), every one that stays and why. */
export function pruneConfirmBody(plan: InstallTargetsPlan, report: DiskReport | null): string {
  const lines = [
    `Deletes ${plural(plan.remove.length, 'install build folder')} in ~/Library/Caches${measuredTotal(report, plan.remove)}. ` +
      'They hold build output Hollow can make again: the next app build from that workspace takes longer. The apps in /Applications are not touched.',
    ...(report ? [] : [SIZES_UNKNOWN]),
    '', 'These go:',
  ];
  const day = (d: string | null) => d ? `, last used ${d.slice(0, 10)}` : '';
  for (const t of plan.targets.filter(t => t.keep == null)) lines.push(`• ${t.name}${day(t.lastUsed)}${measured(report, t.path)}`);
  for (const p of plan.leftovers) lines.push(`• ${name(p)} (left by an interrupted delete)${measured(report, p)}`);
  const kept = plan.targets.filter(t => t.keep != null);
  if (kept.length) lines.push('', 'These stay:', ...kept.map(t => `• ${t.name} (${t.keep}${day(t.lastUsed)})`));
  if (plan.notes.length) lines.push('', ...plan.notes);
  lines.push('', 'If this list changes before you confirm, nothing is deleted.');
  return lines.join('\n');
}

/** The confirm's words for removing finished worktrees and "~" folders. */
export function cleanConfirmBody(r: WorktreeHygieneReport, report: DiskReport | null): string {
  const worktrees = r.repos.flatMap(x => x.worktrees.filter(w => w.removable));
  const tildes = r.repos.flatMap(x => x.tildeFolders);
  const lines: string[] = [];
  if (r.inUseUnknown) lines.push(CANT_READ_RUNNING, '');
  if (worktrees.length) {
    lines.push(`Removes ${plural(worktrees.length, 'worktree')} that ${worktrees.length === 1 ? 'is' : 'are'} merged, unchanged, fully pushed and not in use. ` +
      'Branches are kept, a build workspace using one goes back to its vault folder, and build output git ignores in them goes with them:');
    for (const w of worktrees) {
      const more = w.ignoredCount - w.ignored.length;
      const ignored = w.ignoredCount ? ` — also deletes ${w.ignored.join(', ')}${more > 0 ? ` and ${more} more` : ''}` : '';
      lines.push(`• ${w.path}${w.branch ? ` [${w.branch}]` : ''}${w.workspace ? ` (workspace ${w.workspace})` : ''}${measured(report, w.path)}${ignored}`);
    }
    if (!report) lines.push(SIZES_UNKNOWN);
  }
  if (tildes.length) {
    if (lines.length) lines.push('');
    lines.push(`Deletes ${plural(tildes.length, '"~" folder')} of stray build output${r.tildeBytes ? ` (${sizeWords(r.tildeBytes)})` : ''}:`);
    for (const t of tildes) lines.push(`• ${t.path}${t.bytes == null ? '' : ` · ${sizeWords(t.bytes)}`}`);
  }
  const kept = r.repos.reduce((n, x) => n + x.worktrees.filter(w => !w.removable).length, 0);
  const branches = r.repos.reduce((n, x) => n + x.localOnlyBranches.length, 0);
  if (kept || branches || r.remove.length) lines.push('');
  if (kept) lines.push(`Kept: ${plural(kept, 'worktree')} (in use, changed, unpushed, unmerged, locked, just started, built the installed or rollback app, or needed by a build waiting to install).`);
  if (branches) lines.push(`Not touched: ${plural(branches, 'branch', 'branches')} with commits no remote has.`);
  if (r.remove.length) lines.push('"Not in use" means no program\'s working folder is inside it; an editor or tool working there from elsewhere isn\'t seen.');
  lines.push('', 'If this list changes before you confirm, nothing is removed.');
  return lines.join('\n');
}

function failures(failed: Array<[string, string]>, verb: string): string {
  return failed.length ? ` Couldn't ${verb} ${failed.map(([p, e]) => `${name(p)} (${e})`).join('; ')}.` : '';
}

/** The status line after Delete old install builds…. */
export function prunedText(out: InstallTargetsPruned): string {
  const done = out.removed.length ? `Deleted ${plural(out.removed.length, 'install build folder')}.` : 'Nothing was deleted.';
  return `${done}${failures(out.failed, 'delete')}`;
}

/** The status line after Remove finished worktrees…. */
export function cleanedText(out: WorktreesCleaned): string {
  const done = out.removed.length ? `Removed ${plural(out.removed.length, 'worktree or folder', 'worktrees and folders')}.` : 'Nothing was removed.';
  const left = out.remaining?.length ? ` ${plural(out.remaining.length, 'more is', 'more are')} still due: run it again.` : '';
  const measured = out.volumes?.length && out.freeBefore !== undefined && out.freeAfter !== undefined
    ? ` Free space read from the disk: ${gbText(out.freeBefore)} before, ${gbText(out.freeAfter)} after (${gbText(out.freed ?? 0)} freed${out.volumes.length > 1 ? `, summed over ${out.volumes.length} volumes` : ''}). That is the measured gain, not a size estimate; local snapshots can hold deleted files and keep it near 0 until they go.` : '';
  return `${done}${left}${measured}${failures(out.failed, 'remove')}`;
}

const gbText = (bytes: number) => `${(bytes / 1024 ** 3).toFixed(1)} GB`;

// ── Remove old indexes… (repo features R7b) ──────────────────────────────
// Other vaults' search and meaning-search indexes (`stale_indexes`), in the
// words of the agents' card (Rust `hollow_databases::describe`): when either
// side's wording of what goes and what stays changes, change the other.

/** What Remove old indexes… says when there is nothing to remove. */
export const NOTHING_STALE = "Nothing to remove: no other vault's index has gone unused for an hour. Nothing changed.";

const indexes = (n: number) => plural(n, 'old index', 'old indexes');
const KIND_WORDS: Record<StaleIndexesPlan['files'][number]['kind'], string> = {
  sessions: 'conversations', searchIndex: 'search index', embedIndex: 'meaning-search index', companion: 'companion ledger', projectIndex: 'project contents index',
};

/** The confirm's title for removing other vaults' old indexes. */
export function staleConfirmTitle(plan: StaleIndexesPlan): string {
  return `Remove ${indexes(plan.files.length)}?`;
}

/** The confirm's words: every index that goes with its size, the total,
 *  and what stays. */
export function staleConfirmBody(plan: StaleIndexesPlan): string {
  const n = plan.files.length;
  const lines = [
    `Removes ${plural(n, 'index', 'indexes')} of other vaults from ${plan.folder}, ${sizeWords(plan.bytes)} in all. ` +
      'Hollow rebuilds an index if it opens that vault again (meaning search embeds every note again).',
    '', 'These go, each with its -wal and -shm:',
    ...plan.files.map(f => `• ${name(f.path)} (${f.ownerName}, ${KIND_WORDS[f.kind]}${f.why ? `, ${f.why}` : f.modified ? `, last changed ${f.modified.slice(0, 10)}` : ''}) · ${sizeWords(f.bytes)}`),
  ];
  if (plan.kept.length) lines.push('', 'These stay:', ...plan.kept.map(k => `• ${name(k.path)} (${k.ownerName}, ${KIND_WORDS[k.kind]}): ${k.why}`));
  lines.push('', "This vault's files and every Sessions database are never removed.", 'If this list changes before you confirm, nothing is removed.');
  return lines.join('\n');
}

/** The status line after Remove old indexes…. */
export function removedIndexesText(out: StaleIndexesRemoved): string {
  const done = out.removed.length ? `Removed ${indexes(out.removed.length)}, ${sizeWords(out.bytes)}.` : 'Nothing was removed.';
  return `${done}${failures(out.failed, 'remove')}`;
}
