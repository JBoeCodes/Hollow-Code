/** Agents read a Preview tab the person hands them (repo ideas 2026-10-27
 *  #5; read only in this run). The person shares one web tab with the
 *  conversation on screen (Preview toolbar → Share with chat); that chat's
 *  agent may read its text with `ui.read_tab` during one turn, the one
 *  running or else its next (Rust `tab_grants.rs`). The tab shows "In use by
 *  ‹chat›" with Stop. Pure. */

/** One shared tab (Rust `tab_grants::Grant`). */
export interface TabGrant { tabId: string; sessionKey: string; title: string; grantedAt: string }

/** The grant on `tabId`, if any. */
export const grantOn = (grants: readonly TabGrant[] | undefined, tabId: string | null | undefined): TabGrant | null =>
  (tabId && grants?.find(g => g.tabId === tabId)) || null;

/** The open chat's name for the strip: its first message, cut short. */
export function chatName(firstMessage: string | null | undefined): string {
  const words = (firstMessage ?? '').replace(/\s+/g, ' ').trim();
  if (!words) return 'the open chat';
  return words.length > 60 ? `${words.slice(0, 59)}…` : words;
}
