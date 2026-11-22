/**
 * The command palette's list before anything is typed (P0-02, overnight
 * 2026-10-23). Pure, so its order can be tested without the palette.
 *
 * ⌘K opens with row 0 selected, so ⌘K ↵ runs whatever leads. A row whose
 * shared action starts or stops agent work, changes files or sends
 * something out without asking first never leads: it never joins Recent,
 * and it follows the rows that only show, open or go somewhere. Typed
 * queries keep their own ranking (CommandPalette.tsx).
 */
import { ACTION_BY_PALETTE, EFFECT_WORDS } from './actionDefs';

/** The fields of a palette row the default order reads. */
export interface DefaultRow { id: string; group: string }

const ACTS = new Set(['run', 'write', 'external']);

/** Whether choosing this palette row acts at once: its shared action runs,
 *  writes or sends with no preview or confirmation to answer. */
function actsWithoutAsking(id: string): boolean {
  const def = ACTION_BY_PALETTE[id];
  return !!def && !def.confirm && ACTS.has(def.effect);
}

/** Whether a row that ran joins Recent. One that acts without asking
 *  doesn't, so ⌘K ↵ can't run it again by reflex. */
export function joinsRecent(id: string): boolean {
  return !actsWithoutAsking(id);
}

/** The line a run, write or external row carries under its label: what it
 *  does to the person's work (actionDefs.ts EFFECT_WORDS). */
export function paletteEffectLine(id: string): string | undefined {
  const def = ACTION_BY_PALETTE[id];
  return def && ACTS.has(def.effect) ? EFFECT_WORDS[def.effect] : undefined;
}

/** Conversation rows the default list leaves to typing: stepping rows and
 *  ones a menu offers closer to hand. */
const TYPED_ONLY = ['act:prev-conversation', 'act:next-conversation', 'act:merge-workspace'];

/** Rows that only show or open something first, acting rows after them
 *  (stable within each). */
const safeFirst = <T extends DefaultRow>(rows: T[]) => [...rows.filter(r => !actsWithoutAsking(r.id)), ...rows.filter(r => actsWithoutAsking(r.id))];

/** The empty-query list: Recent, then this conversation's actions, the five
 *  latest other conversations, Actions and Navigation. `current` is the key
 *  of the conversation on screen, which isn't offered as a place to go. */
export function paletteDefaultOrder<T extends DefaultRow>(all: readonly T[], recents: readonly string[], current: string | null): T[] {
  // Recent commands float to the top of the curated default set:
  // re-running yesterday's view or page is ⌘K-↵. An acting row recorded
  // before P0-02 is skipped here too.
  const byId = new Map(all.map(i => [i.id, i] as const));
  const recent: T[] = [];
  for (const id of recents) {
    if (recent.length >= 8) break;
    const item = byId.get(id);
    if (item && joinsRecent(id)) recent.push({ ...item, group: 'Recent' } as T);
  }
  const seen = new Set(recent.map(r => r.id));
  // Actions before Navigation: the group renderer walks array order, and
  // the rail already covers navigation, so a verb is the better default.
  const pick = (g: string) => all.filter(i => i.group === g && !seen.has(i.id));
  // The current conversation's actions and the five latest conversations
  // lead (frontier F6); stepping rows stay reachable by typing. Stop and
  // Make a canvas follow the view rows; with no view row to lead the
  // group, the group goes last, after Navigation.
  const conv = safeFirst(pick('Conversation').filter(i => !TYPED_ONLY.includes(i.id)));
  const convLeads = conv.length > 0 && !actsWithoutAsking(conv[0].id);
  return [
    ...recent,
    ...(convLeads ? conv : []),
    ...pick('Conversations').filter(i => i.id !== `conversation:${current}`).slice(0, 5),
    ...safeFirst(pick('Actions')),
    ...pick('Navigation'),
    ...(convLeads ? [] : conv),
  ];
}
