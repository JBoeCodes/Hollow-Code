// One conversation from the person's side (2026-11-07, "Why are there two
// Main agent chats?"): the words and list rule of a conversation chain
// (Rust `chat_chain`). Read only by lazy chunks (`components/ChatChainBar`,
// Library → Conversations), so none of it weighs on the App chunk. Pure.

/** A conversation list with the Main agent shown once: its newest
 *  conversation stands for it; the earlier ones open from the Main page. */
export function oneMainConversation<T extends { key: string; createdAt: string }>(sessions: readonly T[], isMain: (s: T) => boolean): T[] {
  const time = (iso: string) => { const t = Date.parse(iso); return Number.isNaN(t) ? 0 : t; };
  let newest: T | null = null;
  for (const s of sessions) if (isMain(s) && (!newest || time(s.createdAt) > time(newest.createdAt))) newest = s;
  return sessions.filter(s => !isMain(s) || s === newest);
}

/** A forward note in words, from the side of the conversation `key`. */
export function forwardWords(note: { from: string; at: string; preview: string }, key: string, time: (iso: string) => string): string {
  const what = note.preview ? `“${note.preview}”` : 'A message';
  return note.from === key
    ? `${what} was sent here after this conversation continued, so it went there (${time(note.at)}).`
    : `${what} was sent in an earlier chat after it continued here, so it came here (${time(note.at)}).`;
}

/** "Earlier in this conversation (3)", or with one: "(1 chat)". */
export function earlierLabel(count: number): string {
  return `Earlier in this conversation (${count === 1 ? '1 chat' : `${count} chats`})`;
}
