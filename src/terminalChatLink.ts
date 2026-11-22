// What ties the terminal drawer's shells to the chat on screen (parity 7.2).
// Loaded with the Terminal tab and then alive for the whole session, whether
// or not the tab is showing, because a shell keeps running when its tab is
// left:
//  - a chat's mode or agent changes, or another chat comes on screen: ask the
//    backend whether that chat's mode still allows a shell, and end its
//    terminals when it does not (`checkTerminalGate`);
//  - a chat that had no key yet (an unsent draft) gets one at its first send:
//    its terminals follow it (`adoptDraftTerminals`).
import { createEffect, createRoot, on } from 'solid-js';
import { chatAgent, chatAgentSessionKey, chatPermissionMode } from './store/chatRuntime';
import { DRAFT_CHAT, adoptDraftTerminals, checkTerminalGate, terminalsOf } from './store/terminals';

createRoot(() => {
  createEffect(on([chatAgentSessionKey, chatAgent, chatPermissionMode], ([key, agent, perm], prev) => {
    if (prev && !prev[0] && key) adoptDraftTerminals(key);
    const chat = key ?? DRAFT_CHAT;
    if (terminalsOf(chat).length) void checkTerminalGate(chat, agent, perm);
  }));
});
