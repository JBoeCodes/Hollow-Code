// Quick chats (2026-11-14, Alex's card 2): "when a quick chat is clearly
// project work, offer a one-click Move to <project> under the reply". Which
// project, if any, the latest exchange of a quick chat is clearly about.
// Pure, unit-tested; `components/QuickChatMoveOffer.tsx` shows the offer.
//
// Clearly means one project, and either its folder named (`workspaces/<id>`
// or its working folder's path) or its title said at least twice in the
// prompt and reply together. Two projects that both qualify: no offer.

export interface MoveProject { id: string; title: string; cwd?: string; path?: string }

/** Titles too short or too common to count when said alone. */
const MIN_TITLE = 4;
const COMMON = new Set(['work', 'notes', 'ideas', 'misc', 'personal', 'home', 'projects', 'project', 'chats', 'quick chats', 'test', 'tests', 'docs', 'main']);

const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** How many times `title` is said in `text` as whole words, any case. */
export function titleMentions(text: string, title: string): number {
  const t = title.trim();
  if (t.length < MIN_TITLE || COMMON.has(t.toLowerCase())) return 0;
  const re = new RegExp(`(?<![\\p{L}\\p{N}])${escape(t)}(?![\\p{L}\\p{N}])`, 'giu');
  return text.match(re)?.length ?? 0;
}

/** Whether `text` names project `p`'s folder. */
export function namesFolder(text: string, p: MoveProject): boolean {
  // A folder only three or fewer levels deep (`/Users/me/Hollow`) is too
  // broad to name one project.
  const deep = (x: string) => x.replace(/\/+$/, '').split('/').filter(Boolean).length >= (x.startsWith('/') ? 4 : 2);
  const paths = [p.path, p.cwd, `workspaces/${p.id}`].filter((x): x is string => !!x && deep(x));
  return paths.some(x => text.includes(x.replace(/\/+$/, '') + '/') || text.includes(x.replace(/\/+$/, '') + '`') || text.endsWith(x));
}

/** The one project the exchange (`prompt` and `reply`) is clearly about, or
 *  null. `projects` are the person's projects (no built-in workspaces). */
export function quickChatMoveTarget(prompt: string, reply: string, projects: readonly MoveProject[]): MoveProject | null {
  const text = `${prompt}\n${reply}`;
  if (!text.trim()) return null;
  const clear = projects.filter(p => namesFolder(text, p) || titleMentions(text, p.title) >= 2);
  return clear.length === 1 ? clear[0] : null;
}
