/**
 * Open a file a chat names in the Preview pane (2026-11-04): a reply's file
 * link, a tool row's file, a reply chip, a run record's row. The link is
 * read against each of the chat's folders (fileLocate.ts); the backend says
 * which place holds the file (`chat_locate_file`, nothing read); that one
 * opens beside the page. When none does, the footer says why in words
 * (moved or deleted, a turn's scratch folder, a Mac mini copy), instead of
 * the pane's "It isn't at this path".
 *
 * 2026-11-10: a relative path no place of the chat holds is looked for under
 * every project's folder and linked repository, then across the vault
 * (`chat_find_file`): one match opens, several offer a picker, and only none
 * says it's missing. An absolute path never opens another file of its name.
 */
import { api } from '../api';
import { fileLinkFromHref, joinPath, normalizePath, type CitedLines, type FileLink } from '../chatFileLinks';
import { linkBases, locateCandidates, missingFileText } from '../fileLocate';
import { chatArtifacts, chatConversationVersion, chatCwd, chatTurns, fileClick, openFileInSidePane, vaultRoot, type FileClick } from './chatRuntime';
import { flashFooter, flashWithUndo } from './snackbars';
import { showConfirm } from './overlays';
import { isLatestPreviewIntent } from './preview';
import { workspaceDetail, workspaces } from './workspaces';

/** The most files of the conversation's own a link is matched against. */
const KNOWN_CAP = 2000;

/** The folders this chat's links are read against, best first. */
async function chatLinkBases(vault: string | null): Promise<string[]> {
  const cwd = chatCwd();
  const open = workspaceDetail()?.workspace;
  const here = open && (open.cwd === cwd || open.build?.worktree === cwd) ? open : null;
  const folder = here ? (here.path.startsWith('/') ? here.path : vault ? joinPath(vault, here.path) : null) : null;
  const repoRoots = chatTurns.flatMap(t => (t.receipt?.repoChanges ?? []).map(r => r.root));
  return linkBases({ cwd, worktree: here?.build?.worktree ?? null, workspaceFolder: folder, repoRoots, vault });
}

/** Every project's folder and linked repository, absolute: where a path no
 *  place of the chat holds is looked for next (`chat_find_file`). */
function projectRoots(vault: string | null): { roots: string[]; titles: Map<string, string> } {
  const roots: string[] = [];
  const titles = new Map<string, string>();
  for (const w of workspaces()) {
    const folder = w.path.startsWith('/') ? normalizePath(w.path) : vault ? joinPath(vault, w.path) : null;
    for (const root of [folder, w.cwd?.startsWith('/') ? normalizePath(w.cwd) : null]) {
      if (!root || roots.includes(root)) continue;
      roots.push(root);
      titles.set(root, w.title);
    }
  }
  return { roots, titles };
}

/** Several files end with the path a link names: the person picks one. Each
 *  row is the file's place in the vault (or its full path) and its project. */
function pickFile(name: string, matches: string[], titles: Map<string, string>, vault: string | null, lines: Partial<CitedLines>) {
  const inVault = (p: string) => (vault && p.startsWith(`${vault.replace(/\/+$/, '')}/`) ? p.slice(vault.replace(/\/+$/, '').length + 1) : p);
  const projectOf = (p: string) => [...titles.keys()].filter(r => p.startsWith(`${r}/`)).sort((a, b) => b.length - a.length)[0];
  const rows = matches.map((p, i) => {
    const root = projectOf(p);
    return { id: String(i), label: inVault(p), detail: root ? titles.get(root) : undefined };
  });
  void showConfirm({
    title: `Which ${name}?`,
    summary: `${matches.length} files in your projects end with this path. Pick the one to open.`,
    hideConfirm: true,
    aside: { heading: 'Files', actionLabel: 'Open', rows, onAction: id => openFileInSidePane(matches[Number(id)], lines) },
  });
}

/** Files the conversation already knows, absolute: the ones its tools wrote
 *  and the ones its run records list. */
function chatKnownFiles(vault: string | null): string[] {
  const known = new Set<string>(chatArtifacts().map(a => a.path).filter(p => p.startsWith('/')));
  for (const t of chatTurns) {
    for (const repo of t.receipt?.repoChanges ?? []) for (const f of repo.files ?? []) known.add(joinPath(repo.root, f.path));
    for (const f of t.receipt?.files ?? []) {
      if (f.path.startsWith('/')) known.add(f.path);
      else if (vault) known.add(joinPath(vault, f.path));
    }
    if (known.size >= KNOWN_CAP) break;
  }
  return [...known].slice(0, KNOWN_CAP);
}

/** Open the file `href` names in the Preview pane. True when it opened.
 *  A reply's link (a path, `file://` URL or link, with any `:12` / `#L12-20`
 *  lines) is read as a link; `literal` takes a path as written (a tool row's,
 *  a chip's, a run record's), so `#` and `%` stay part of its name.
 *  `missing`: the way on the notice offers when no place holds the file (a
 *  reply chip's Show its history). `owner`: the click's `fileClick()`. */
export async function openNamedFile(href: string, home: string | null = null, opts: { literal?: boolean; missing?: { label: string; run: () => void }; owner?: FileClick } = {}): Promise<boolean> {
  // The click's conversation and its ask of the pane, both taken in its
  // handler (`fileClick`) before any chunk loaded: a newer open or close
  // wins (review, 2026-11-04).
  const owner = opts.owner ?? fileClick();
  const stale = () => !isLatestPreviewIntent(owner.intent) || owner.version !== chatConversationVersion() || owner.cwd !== chatCwd();
  if (stale()) return false;
  const vault = await vaultRoot();
  if (stale()) return false;
  const bases = await chatLinkBases(vault);
  if (stale()) return false;
  const read = (base: string | null): FileLink | null => opts.literal
    ? (href.startsWith('/') ? { path: normalizePath(href) } : base ? { path: joinPath(base, href) } : null)
    : fileLinkFromHref(href, base, home);
  const readings = (bases.length ? bases : [null]).flatMap(base => {
    const link = read(base);
    return link ? [{ base: base ?? '/', link }] : [];
  });
  if (readings.length === 0) { flashFooter('Hollow could not resolve this file path.'); return false; }
  const first = readings[0].link;
  const raw = href.trim();
  const relative = !raw.startsWith('/') && !raw.startsWith('~') && !/^file:/i.test(raw);
  const candidates = locateCandidates(readings.map(r => ({ base: r.base, path: r.link.path })), chatKnownFiles(vault), { relative });
  const found = candidates.length ? await api.chatLocateFile(candidates).catch(() => null) : null;
  // A click from a conversation since left, or before a newer open, opens nothing.
  if (stale()) return false;
  if (found) { openFileInSidePane(found, { line: first.line, end: first.end }); return true; }
  // A path relative to another project's folder (2026-11-10: one copied from
  // a child chat's report): every project's folder and repository, then the
  // vault. One match opens; several offer a picker.
  const base = readings[0].base.replace(/\/+$/, '');
  const tail = relative && first.path.startsWith(`${base}/`) ? first.path.slice(base.length + 1) : null;
  if (tail) {
    const { roots, titles } = projectRoots(vault);
    const matches = await api.chatFindFile(tail, roots).catch(() => [] as string[]);
    if (stale()) return false;
    if (matches.length === 1) { openFileInSidePane(matches[0], { line: first.line, end: first.end }); return true; }
    if (matches.length > 1) { pickFile(tail.split('/').pop() || tail, matches, titles, vault, { line: first.line, end: first.end }); return true; }
  }
  const text = `× ${missingFileText(first.path)}`;
  if (opts.missing) flashWithUndo({ message: text, kind: 'status', ttlMs: 10000, actionLabel: opts.missing.label, undo: opts.missing.run });
  else flashFooter(text, 8000);
  return false;
}
