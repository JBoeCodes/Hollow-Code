import {
  chatBusy, chatAgentSessionKey, chatSessionId, chatTurns, resetChatState, restoreBackgroundChat, KNOWN_AGENTS, type ChatAgent,
  setChatAgentPersistent, setChatAgentSessionKeyPersistent, setChatSessionIdPersistent, setChatCwdPersistent,
  setChatPermissionMode, setChatTurns, rebuildChatArtifacts, setChatStoredTranscriptPending, type ChatTurn,
} from './runtime';
import { api } from '../api';
import { refreshRunQueue } from './runQueue';
import { conversationPermission } from '../permModes';

/** A failed or superseded read cannot leave the previous chat's native
 * policy usable. The caller owns the navigation/version guard. */
export async function readHistoryConversation(key: string, current: () => boolean) {
  setChatPermissionMode(conversationPermission(key).mode);
  setChatSessionIdPersistent(null);
  const detail = await api.readAgentConversation(key);
  if (!current()) return null;
  const permission = conversationPermission(key, detail.permissionMode, detail.summary.agent);
  setChatPermissionMode(permission.mode);
  setChatSessionIdPersistent(permission.resumeNative && detail.summary.resumable === 'native' ? detail.summary.harnessSessionId : null);
  return detail;
}

/** A Claude history row left open before run 15 P4 removed that tab: a
 *  native Claude Code session id with no Hollow conversation key and
 *  nothing on screen. It can't be listed or shown any more, so its id is
 *  cleared (the draft stays) and the next send starts a new conversation,
 *  as the screen says, instead of quietly continuing the old transcript.
 *  Never while a turn runs. True when it cleared one. */
export function retireLegacyHistorySelection(): boolean {
  if (chatBusy() || chatAgentSessionKey() || !chatSessionId() || chatTurns.length > 0) return false;
  setChatSessionIdPersistent(null);
  return true;
}

/** A conversation picked in Conversation history: a Hollow conversation
 *  (`key`) or a legacy Claude Code transcript (`key` null, `sessionId` set). */
export interface HistoryOpenTarget {
  key: string | null;
  agent: string;
  sessionId: string | null;
  cwd: string | null;
}

/** Put the conversation picked in Conversation history on screen (merged
 *  plan slice 4, review A·D4). One still held in the background (its turn
 *  running, or finished there and not shown since) comes back from its
 *  snapshot and buffered events, still busy while it runs, so the composer
 *  queues behind that turn: the same path `selectWorkspace` takes. Anything
 *  else is pointed at and `'load'` tells the caller to read its stored
 *  transcript. Null while a turn is on screen: nothing changes. */
export function openHistoryConversation(target: HistoryOpenTarget): 'live' | 'load' | null {
  if (chatBusy()) return null;
  // Clears the previous conversation's draft and artifacts either way;
  // a restore then overwrites everything else it holds.
  resetChatState();
  if (target.key && restoreBackgroundChat(target.key)) return 'live';
  if (KNOWN_AGENTS.includes(target.agent as ChatAgent)) setChatAgentPersistent(target.agent as ChatAgent);
  setChatAgentSessionKeyPersistent(target.key);
  // The stored read must establish this conversation's policy before a
  // native session can be resumed, including after a failed load.
  setChatPermissionMode(target.key ? conversationPermission(target.key).mode : 'acceptEdits');
  setChatSessionIdPersistent(null);
  setChatCwdPersistent(target.cwd);
  return 'load';
}

/** Show the stored turns Conversation history read, then look for the
 *  conversation's live run, as opening it in its workspace does: one still
 *  running after a reload is adopted (Stop, the working mark) instead of
 *  looking idle (overnight 11-13 c-11). After the turns, never before: an
 *  adoption first makes the page's load give up (busy). */
export function showHistoryTurns(turns: ChatTurn[]) {
  setChatTurns(turns); rebuildChatArtifacts(turns);
  setChatStoredTranscriptPending(null);
  refreshRunQueue(0);
}
