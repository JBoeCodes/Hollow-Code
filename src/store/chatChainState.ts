import { createSignal } from 'solid-js';

// One conversation from the person's side (2026-11-07): the chain of the
// conversation on screen (Rust `chat_chain::ChainView`), read by the chat's
// banner, its "Earlier in this conversation" and the cards that follow the
// chain. A small leaf in the App chunk; the lazy `components/ChatChainBar`
// keeps it current and draws it.

/** The part of a chain view the chat reads (Rust `chat_chain::ChainView`). */
export interface ChainFacts {
  key: string;
  current: string;
  currentTitle?: string | null;
  main: boolean;
  earlier: readonly { key: string; title?: string | null; createdAt: string }[];
  forwards: readonly { from: string; to: string; at: string; preview: string }[];
}

/** Where a conversation continued, or null while it is current. */
export function continuedIn(view: ChainFacts | null | undefined): { key: string; title: string } | null {
  if (!view || view.current === view.key) return null;
  return { key: view.current, title: view.currentTitle?.trim() || 'its current chat' };
}

/** The conversations whose cards show in `key`: itself and, while it is
 *  current, the chats before it in its chain. */
export function chainCardKeys(key: string | null, view: ChainFacts | null | undefined): Set<string> {
  const keys = new Set<string>(key ? [key] : []);
  if (key && view && view.key === key && view.current === key) for (const e of view.earlier) keys.add(e.key);
  return keys;
}

/** The shown conversation's chain, or null before it is read. */
export const [chatChain, setChatChain] = createSignal<ChainFacts | null>(null);

/** Where the shown conversation continued, or null while it is current. */
export const chatContinuedIn = () => { const v = chatChain(); return v && continuedIn(v); };

/** The conversations whose cards show in `key`. */
export const chainCardKeysFor = (key: string | null) => chainCardKeys(key, chatChain());
