// Inline chat approvals: the Allow/Deny cards for confirm-gated MCP tools and
// their `approval-request` / `approval-resolved` listeners.
//
// Imports ./runtime (conversationHistoryShown, chatPanelOpen, setChatPanelOpen), which
// imports and re-exports this module: those are read only inside listener
// callbacks, never at top level, so the cycle is evaluation-order safe (see
// chatRuntime.ts).
// Moved verbatim out of store/runtime.ts (plan L3 X19, 2026-10-14); runtime.ts
// re-exports every symbol, so `store/runtime` imports keep working.
import { createSignal } from 'solid-js';
import { produce } from 'solid-js/store';
import { listen } from '../nativeEvents';
import { api } from '../api';
import { errMsg } from '../utils';
import type { ApprovalRequest, ApprovalResolved } from '../types';
import type { ChatToolUse } from '../chatTurnTypes';
import { approvalPlacement } from '../chatApprovalOrigin';
import { chainCardKeysFor } from './chatChainState';
import { badgeFromResolved, findToolById, rememberBadge, type ApprovalBadge } from '../chatApproval';
import { flashFooter } from './snackbars';
import { chatTurns, pendingApprovalBadges, setChatTurns } from './chatRuntime';
import { chatAgentSessionKey, chatPanelOpen, conversationHistoryShown, setChatPanelOpen } from './runtime';

// ── Inline chat approvals.
//
// When a confirm-gated MCP tool (M365 send/reply/delete, computer-control
// tier 2) wants permission, the Rust approval broker emits `approval-request`
// and the chat renders an Allow/Deny card (Claude-desktop style) instead of
// a native dialog. The user's click goes back via `approval_respond`;
// `approval-resolved` clears the card on every outcome — answered, timed
// out (120s on the MCP side), or the turn being cancelled.
export const [pendingApprovals, setPendingApprovals] = createSignal<ApprovalRequest[]>([]);
/** Ids whose Allow/Deny click is in flight — disables the card's buttons. */
export const [approvalsResponding, setApprovalsResponding] = createSignal<string[]>([]);

export async function respondToApproval(id: string, approved: boolean, reason?: string) {
  setApprovalsResponding(prev => (prev.includes(id) ? prev : [...prev, id]));
  try {
    await api.approvalRespond(id, approved, reason);
  } catch (e) {
    // Unknown id = the request already resolved (timeout / cancelled turn);
    // the resolved event has cleared or will clear the card.
    flashFooter(`× approval failed: ${errMsg(e)}`, 2500);
    setPendingApprovals(prev => prev.filter(a => a.id !== id));
    setApprovalsResponding(prev => prev.filter(x => x !== id));
  }
}

/// Badge the tool call an approval was bound to (and remember it for a row
/// that does not exist yet). An unbound approval badges nothing.
function setToolApproval(toolUseId: string | null | undefined, badge: ApprovalBadge) {
  if (!toolUseId) return;
  rememberBadge(pendingApprovalBadges, toolUseId, badge);
  const hit = findToolById(chatTurns, toolUseId);
  if (!hit) return;
  setChatTurns(produce(arr => {
    const tool = arr[hit.turnIdx]?.tools[hit.toolIdx];
    if (tool) tool.approval = badge;
  }));
}

/// The tool an approval card is about, when the broker bound it.
export function approvalTool(a: ApprovalRequest): ChatToolUse | null {
  const hit = findToolById(chatTurns, a.toolUseId);
  return hit ? chatTurns[hit.turnIdx].tools[hit.toolIdx] : null;
}

listen<ApprovalRequest>('approval-request', e => {
  setPendingApprovals(prev => [...prev, e.payload]);
  setToolApproval(e.payload.toolUseId, 'pending');
  // The MCP call is blocked on this answer — make sure a chat surface is on
  // screen. The side panel auto-hides in Conversation history, so opening it
  // is always safe.
  if (!conversationHistoryShown() && !chatPanelOpen()) setChatPanelOpen(true);
  // A card raised to the COO (2026-11-20) is answered in the COO chat: bring
  // it on screen when nothing the person is looking at shows the card. In the
  // asking chat itself (a note with a link) a footer line says where it is.
  const routed = e.payload;
  if (routed.routedTo) void import('./mainAgent').then(({ cooChatShown, openMainAgent }) => {
    if (cooChatShown() || !pendingApprovals().some(a => a.id === routed.id)) return;
    if (approvalPlacement(routed, chainCardKeysFor(chatAgentSessionKey()), false) === 'waiting') {
      flashFooter('Approval needed in the COO chat', 6000);
    } else void openMainAgent();
  });
}).catch(() => {/* not in tauri */});

listen<ApprovalResolved>('approval-resolved', e => {
  setPendingApprovals(prev => prev.filter(a => a.id !== e.payload.id));
  setApprovalsResponding(prev => prev.filter(x => x !== e.payload.id));
  setToolApproval(e.payload.toolUseId, badgeFromResolved(e.payload));
}).catch(() => {/* not in tauri */});
