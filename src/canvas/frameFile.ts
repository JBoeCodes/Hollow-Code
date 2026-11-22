// The file an `hc-file` link in a canvas opens (bridge `navigate`). Pure and
// dependency-free, so the node vitest suite can load it.
import { localFileFromHref, normalizePath } from '../chatFileLinks';

/** `path` lies strictly inside `dir` (never `dir` itself). */
function inside(path: string, dir: string | null): boolean {
  if (!dir) return false;
  const d = normalizePath(dir);
  return path.startsWith(d === '/' ? '/' : `${d}/`) && path !== d;
}

/**
 * The absolute file a canvas's `navigate` target names, or null. A relative
 * path resolves against `base` (the conversation's folder, as for a reply's
 * file link), or the vault root when there is none. The result must stay
 * inside `base` or the vault: an agent-written canvas can't open a file
 * elsewhere on the Mac. Web, mail and other scheme links, `~` paths, network
 * paths and in-page anchors are not files.
 */
export function canvasFilePath(target: string, base: string | null, root: string | null): string | null {
  const t = target.trim();
  if (!t || /^[\\/]{2}/.test(t)) return null;
  const path = localFileFromHref(t, base || root);
  if (!path || [base, root].some(d => d && normalizePath(d) === path)) return null;
  return inside(path, base) || inside(path, root) ? path : null;
}
