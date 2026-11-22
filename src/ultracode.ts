/**
 * Merged plan Track U: the per-chat Ultracode switch's wording. Claude Code
 * runs standing dynamic-workflow orchestration (`--effort ultracode`); every
 * other agent runs at its model's top effort and never sees the word. The
 * receipt shows what each turn actually got (`effortLine`).
 */
export function ultracodeNote(agent: string, on: boolean): string {
  const what = agent === 'claude'
    ? 'Claude fans work out to subagents at any effort. It uses far more tokens; a CLI without it runs at xhigh and says so.'
    : 'This agent has no ultracode, so turns run at its model\u2019s top effort.';
  return on ? `On for this chat. ${what}` : `Off. ${what}`;
}

/** Where a conversation's switch is kept (localStorage), by its Hollow
 *  session key (P1-10): each conversation's own, like Canvases. */
export function chatUltracodeStorageKey(sessionKey: string): string {
  return `hollow.ultracode.${sessionKey}`;
}

/** A stored switch: `true` when on, null otherwise (off is the default and
 *  is never stored as a choice). */
export function parseStoredUltracode(raw: string | null): true | null {
  return raw === 'true' ? true : null;
}

/**
 * The switch after the conversation on screen changes from `prev` to `next`
 * (session keys). It stays with the same conversation; a new chat's first
 * turn (`ownTurn`: the key its own send minted) keeps a switch set before
 * that send and stores it under the new key; any other move (New chat,
 * opening or restoring another conversation) reads that conversation's own
 * value, off when it has none. Unlike Canvases, an unsent new chat's switch
 * never moves into a conversation opened from it (review M1: Ultracode is the
 * costly setting, and off is never stored, so nothing could resist it).
 */
export function ultracodeOnKeyChange(prev: string | null, next: string | null, current: boolean, stored: boolean, ownTurn: boolean): { on: boolean; persist: boolean } {
  if (prev === next) return { on: current, persist: false };
  if (!next) return { on: false, persist: false };
  // A handover or forward moves the screen to the chain's successor on this chat's own turn: it keeps the switch.
  if (ownTurn && current) return { on: true, persist: !stored };
  return { on: stored, persist: false };
}

/** Settings PR 7 (one home each): a conversation's own reasoning effort,
 *  chosen in the composer's Effort list and kept under its Hollow session key
 *  (localStorage, like Ultracode). It never touches the app default, which
 *  lives only in Settings, Models, Effort; a chat with none runs at that
 *  default. */
export function chatEffortStorageKey(sessionKey: string): string {
  return `hollow.effort.${sessionKey}`;
}

const CHAT_EFFORT_LEVELS = ['low', 'medium', 'high', 'xhigh', 'max'];

/** A stored level, or null (no choice: the app default). */
export function parseStoredEffort(raw: string | null): string | null {
  return raw && CHAT_EFFORT_LEVELS.includes(raw) ? raw : null;
}

/** The chat's effort after the conversation on screen changes, with the
 *  same rules as Ultracode ([`ultracodeOnKeyChange`]): it stays with the same
 *  conversation, a new chat's first turn stores a level picked before it, a
 *  forward to the chain's successor carries it, and
 *  any other move reads that conversation's own value (none when it has none). */
export function effortOnKeyChange(prev: string | null, next: string | null, current: string | null, stored: string | null, ownTurn: boolean): { level: string | null; persist: boolean } {
  if (prev === next) return { level: current, persist: false };
  if (!next) return { level: null, persist: false };
  if (!prev && ownTurn && current) return { level: current, persist: stored !== current };
  // A handover or forward to the chain's successor (own turn, prev set) carries the level when the successor has none.
  if (ownTurn && current && !stored) return { level: current, persist: true };
  return { level: stored, persist: false };
}
