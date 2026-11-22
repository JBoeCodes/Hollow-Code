// The chat-announcements switch (0.4): a native banner and a dock badge when a chat
// finishes or needs you. Persisted on this Mac, on by default; the master "macOS system
// notifications" switch still silences the banner (the badge stays).
import { createSignal } from 'solid-js';

const KEY = 'hollow.chatAnnounce';
function stored(): boolean { try { return localStorage.getItem(KEY) !== 'off'; } catch { return true; } }
export const [chatAnnouncements, writeChatAnnouncements] = createSignal<boolean>(stored());
export function setChatAnnouncements(on: boolean) {
  writeChatAnnouncements(on);
  try { localStorage.setItem(KEY, on ? 'on' : 'off'); } catch { /* this window only */ }
}
