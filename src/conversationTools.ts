/**
 * Long-conversation navigation (next-level P04): find in the thread, an
 * outline of the user's prompts, bounded rendering and an honest
 * "continue in a new conversation" handoff. Pure, unit-tested; ChatBody owns
 * the UI.
 */
import { splitNextFence } from './chatSegments';
import { visibleUserText } from './canvas/askAnswer';
import { graphemeCut, noteTitleLine, trimBlock } from './turnFooter';

export interface TurnLike { role: 'user' | 'assistant' | 'system'; text: string; agent?: string; tools?: { name: string }[] }

/** Turn indexes whose text contains `query` (case-insensitive), in order. */
export function findInTurns(turns: readonly TurnLike[], query: string): number[] {
  const q = query.trim().toLowerCase();
  if (q.length < 2) return [];
  const out: number[] = [];
  turns.forEach((t, i) => { if (t.role !== 'system' && t.text.toLowerCase().includes(q)) out.push(i); });
  return out;
}

export interface OutlineEntry { index: number; text: string }

/** The user's prompts, one line each. A bookmarked reply is not listed:
 *  conversation bookmarks are gone, and the outline stays the prompts. */
export function conversationOutline(turns: readonly TurnLike[]): OutlineEntry[] {
  const out: OutlineEntry[] = [];
  turns.forEach((t, i) => {
    if (t.role !== 'user') return;
    const line = t.text.split('\n').map(l => l.trim()).find(Boolean) ?? 'Attachment';
    out.push({ index: i, text: line.length > 120 ? `${line.slice(0, 117)}…` : line });
  });
  return out;
}

/** The index of the newest user turn, or -1 (overnight C5). A turn's footer
 *  is the latest when no user turn follows it: `index >= lastUserIndex`.
 *  One pass per change, where each footer used to copy and scan the rest
 *  of the conversation (and, in the store, subscribe to every later turn). */
export function lastUserIndex(turns: readonly Pick<TurnLike, 'role'>[]): number {
  for (let i = turns.length - 1; i >= 0; i--) if (turns[i].role === 'user') return i;
  return -1;
}

/** The name shown above the open conversation's chat (overnight 11-07, D1):
 *  its saved title, or the sidebar's own word for an unnamed one. Null when
 *  the sidebar's list doesn't hold this conversation (yet), so no title is
 *  shown rather than a wrong one. */
export function chatTitleShown(session: { title: string | null } | null | undefined): string | null {
  return session ? session.title?.trim() || 'Conversation' : null;
}

/** What finishing an edit of that title does: the new name (trimmed, 120
 *  code points at most, cut on a whole grapheme, as the sidebar's rename), or null for no change (an
 *  empty name, or the same one). */
export function renamedTitle(draft: string, current: string | null | undefined): string | null {
  const name = graphemeCut(draft.trim(), 120).trim();
  return name && name !== (current ?? '').trim() ? name : null;
}

/** A reply's More menu action, bound to the conversation and reply it was
 *  opened on (ENG-78): a menu left open across the next-conversation
 *  shortcut does nothing rather than save, branch or retry another
 *  conversation's reply. `still` is `sameChatAsNow()` taken as the menu's
 *  items were built; `index` the reply's place then. */
export function forThisChat(still: () => boolean, index: number, run: (index: number) => void): () => void {
  return () => { if (still()) run(index); };
}

/** Continue from here (ENG-78): the hand-off goes into the new
 *  conversation's draft only once it opened; a refused open (a send in its
 *  preflight) leaves this conversation's draft alone. True when placed. */
export async function branchIntoNewChat(open: () => Promise<boolean>, place: () => void): Promise<boolean> {
  if (!(await open())) return false;
  place();
  return true;
}

/** How many of the latest turns render at first; earlier ones load on request. */
export const TURN_WINDOW = 80;
/** First rendered index for a window that must include `need` (if given). */
export function windowStart(total: number, current: number | null, need?: number): number {
  const base = current ?? Math.max(0, total - TURN_WINDOW);
  return need == null ? Math.min(base, Math.max(0, total - 1)) : Math.max(0, Math.min(base, need - 4));
}

/**
 * The first message of a new conversation that continues from turn `upto`
 * of another. It is a written handoff, and says so: the new agent gets this
 * text, not the old session, whichever provider either uses.
 */
export function handoffText(turns: readonly TurnLike[], upto: number, meta: { title: string; agentLabel?: string }): string {
  const slice = turns.slice(0, upto + 1).filter(t => t.role !== 'system' && t.text.trim());
  const firstAsk = slice.find(t => t.role === 'user')?.text.trim() ?? '';
  const recent = slice.slice(-6);
  const clip = (s: string, n: number) => s.length > n ? `${s.slice(0, n - 1)}…` : s;
  const lines = [
    `Context handoff from “${meta.title}” (a written summary, not a resumed session${meta.agentLabel ? ` of ${meta.agentLabel}` : ''}).`,
    '',
    `It started with: ${clip(firstAsk.replace(/\s+/g, ' '), 400)}`,
    '',
    'Where it got to:',
    ...recent.map(t => `- ${t.role === 'user' ? 'Me' : 'Agent'}: ${clip(t.text.replace(/\s+/g, ' ').trim(), 600)}`),
    '',
    'Continue from here: ',
  ];
  return lines.join('\n');
}

/**
 * The whole conversation as Markdown, for Conversation tools' Copy
 * conversation (P1-27): a `## You` / `## <agent>` heading per turn of
 * talk (a reply's bubbles under one), without system notices, empty
 * bubbles, the hidden next-step block or an ask-canvas answer's JSON. A
 * message the person steered a running turn with is theirs: "You" (one the
 * agent never took is left out).
 */
export function conversationMarkdown(turns: readonly (TurnLike & { notice?: string; steer?: { state: string } })[], meta: { title?: string; agentLabel: (agent?: string) => string }): string {
  const blocks: string[] = meta.title?.trim() ? [`# ${meta.title.trim()}`] : [];
  let speaker = '';
  for (const t of turns) {
    const steer = t.role === 'system' && t.notice === 'steer' && t.steer?.state !== 'notDelivered';
    if (t.role === 'system' && !steer) continue;
    const text = trimBlock(t.role === 'assistant' ? splitNextFence(t.text).text : visibleUserText(t.text));
    if (!text) continue;
    const who = t.role === 'assistant' ? meta.agentLabel(t.agent) : 'You';
    if (who !== speaker) { blocks.push(`## ${who}`); speaker = who; }
    blocks.push(text);
  }
  return `${blocks.join('\n\n')}\n`;
}

/** The title of a conversation saved as a note (overnight 11-07, D1): its
 *  name, else the first line of what was first asked, else "Conversation";
 *  one line, 120 characters at most (as a reply note's). */
export function conversationNoteTitle(title: string | null | undefined, turns: readonly TurnLike[]): string {
  const ask = turns.find(t => t.role === 'user');
  const line = Array.from((title?.trim() || (ask ? noteTitleLine(visibleUserText(ask.text)) : '') || 'Conversation').replace(/\s+/g, ' '));
  return line.length > 120 ? `${graphemeCut(line.join(''), 119)}…` : line.join('');
}

/** Identity of a rendered conversation for scroll restoration: the same
 *  conversation with the same turns restores; anything else goes to the end. */
export const scrollSignature = (key: string | null | undefined, turns: readonly TurnLike[]) =>
  key ? `${key}:${turns.length}:${(turns[0]?.text ?? '').slice(0, 32)}` : '';

/** Scroll positions by [`scrollSignature`] (next-level P04): only the latest
 *  signature per conversation, and at most `cap` conversations, oldest
 *  dropped first. It kept one entry per conversation and turn count for the
 *  app's lifetime (overnight 10-11/12 P6); an older signature can't match
 *  again once the conversation grew. */
export class ScrollMemory<T> {
  private readonly bySig = new Map<string, T>();
  private readonly sigOf = new Map<string, string>();
  constructor(private readonly cap = 200) {}
  get(sig: string): T | undefined { return this.bySig.get(sig); }
  set(key: string, sig: string, value: T) {
    const old = this.sigOf.get(key);
    if (old !== undefined) { this.sigOf.delete(key); if (old !== sig) this.bySig.delete(old); }
    this.sigOf.set(key, sig);
    this.bySig.set(sig, value);
    while (this.sigOf.size > this.cap) {
      const [k, s] = this.sigOf.entries().next().value!;
      this.sigOf.delete(k);
      this.bySig.delete(s);
    }
  }
  get size() { return this.bySig.size; }
}
