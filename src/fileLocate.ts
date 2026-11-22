/**
 * Where a file a chat names may be (2026-11-04; Alex: "sometimes when I
 * click on a file it says cannot open file cause file has moved"). A reply's
 * file link was resolved against one folder only, the chat's own (or the
 * vault in a chat with none), so these all read as moved files:
 * - a vault-relative path (`workspaces/main/outputs/x.md`) in any project chat;
 * - a repository-relative path in a linked project's chat, whose folder is
 *   the vault workspace, not the repository;
 * - `outputs/x.md` in a build chat, whose folder is the worktree;
 * - an absolute path from a turn that ran on the Mac mini, which names the
 *   runner's copy (`…/hollow-remote/folders/<id>/…`), not this Mac's.
 * Now every place is tried, best first, and the backend opens the first that
 * holds the file (`chat_locate_file`). Pure, so the node suite can load it.
 */
import { joinPath, normalizePath } from './chatFileLinks';

/** A runner's working copy (remote sessions): a project folder's copy or a
 *  worktree session; what follows is the path inside the project. */
const RUNNER_COPY = /\/hollow-remote\/(?:folders|sessions)\/[^/]+\/(.+)$/;
/** A turn's scratch space: this Mac's (`$HOLLOW_SCRATCH_DIR`) or a runner's.
 *  Both are cleared when the turn ends. */
const SCRATCH = /^\/(?:private\/)?tmp\/hollow-run-|\/hollow-remote\/scratch\//;

/** One reading of the link: the folder it was read against and the path it
 *  gave there (`fileLinkFromHref` with that folder as its base). */
export interface LinkReading { base: string; path: string }

/** Every place the file may be, best first, deduplicated: each reading in
 *  the order the folders were given; a runner path mapped onto each folder;
 *  then the conversation's own files (`known`, absolute) whose path ends
 *  with the link's. Only a `relative` link (or a runner path's inside) is
 *  matched by its end: an absolute path names one file, so a deleted one
 *  never opens another of the same name (review, 2026-11-04). A bare name's
 *  match must be the only one. */
export function locateCandidates(readings: readonly LinkReading[], known: readonly string[] = [], opts: { relative?: boolean } = {}): string[] {
  const out: string[] = [];
  const add = (p: string) => {
    if (!p.startsWith('/')) return;
    const n = normalizePath(p);
    if (!out.includes(n)) out.push(n);
  };
  for (const r of readings) add(r.path);
  const bases = [...new Set(readings.map(r => r.base))];
  for (const r of readings) {
    const inside = RUNNER_COPY.exec(r.path)?.[1];
    if (inside) for (const base of bases) add(joinPath(base, inside));
  }
  const tails = new Set<string>();
  for (const r of readings) {
    const base = r.base.replace(/\/+$/, '');
    if (opts.relative && r.path.startsWith(`${base}/`)) tails.add(r.path.slice(base.length + 1));
    const inside = RUNNER_COPY.exec(r.path)?.[1];
    if (inside) tails.add(inside);
  }
  for (const tail of tails) {
    const hits = known.filter(k => k === `/${tail}` || k.endsWith(`/${tail}`));
    if (!tail.includes('/') && hits.length !== 1) continue;
    hits.forEach(add);
  }
  return out;
}

/** The folders a chat's file links are read against, best first and
 *  deduplicated: the chat's folder, the project's own folder and worktree,
 *  the repositories its turns changed, then the vault. */
export function linkBases(input: { cwd: string | null; workspaceFolder?: string | null; worktree?: string | null; repoRoots?: readonly (string | null | undefined)[]; vault: string | null }): string[] {
  const out: string[] = [];
  for (const b of [input.cwd, input.worktree, input.workspaceFolder, ...(input.repoRoots ?? []), input.vault]) {
    if (!b || !b.startsWith('/')) continue;
    const n = normalizePath(b).replace(/\/+$/, '') || '/';
    if (!out.includes(n)) out.push(n);
  }
  return out;
}

/** What the footer says when no place holds the file, by where it was. */
export function missingFileText(path: string): string {
  const name = path.replace(/[/\\]+$/, '').split(/[/\\]/).pop() || path;
  if (SCRATCH.test(path)) return `${name} was in the turn's scratch folder, which Hollow clears when a turn ends. Ask the agent to save it in the project instead.`;
  if (/\/hollow-remote\//.test(path)) return `${name} was on the Mac mini while the turn ran, and no copy of it is in this chat's folder on this Mac.`;
  return `Hollow couldn't find ${name} in this chat's folder, its project or the vault. It may have been moved or deleted since the reply named it.`;
}
