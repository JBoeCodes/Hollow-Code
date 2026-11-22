import { api } from '../api';
import { batch } from 'solid-js';
import { conversationPermission } from '../permModes';
import { artifactContinuationPrompt, artifactSessionTurns, canResumeArtifactSession } from '../artifactSession';
import { unsafeCwdText } from '../chatCwd';
import {
  chatBusy, setChatBusy, chatInput, setChatInput, resetChatState,
  setChatAgentPersistent, setChatAgentSessionKeyPersistent, setChatSessionIdPersistent,
  setChatCwdPersistent, setChatTurns, pushChatTurn, setActivePage, setChatContinuedFrom,
  setChatPanelOpen, requestChatFocus, KNOWN_AGENTS, type ChatAgent,
  isBackgrounded, isBackgroundChat, discardBackgroundChat,
  setChatPermissionMode, applyNewChatPermissionMode, canLeaveConversation, holdConversationForSend,
} from './runtime';

/** Read and preflight before changing any conversation state. Never send. */
export async function continueArtifactSession(key: string, path: string, fromSummary = false): Promise<void> {
  if (chatBusy() || !canLeaveConversation()) throw new Error('Wait for the current conversation to finish opening or sending.');
  const release = holdConversationForSend();
  setChatBusy(true);
  try {
    const context = await api.readArtifactSessionContext(key);
    const session = context.detail.summary;
    const [available, missingCwd, live] = await Promise.all([
      api.agentAvailable(), api.chatPreflight(session.cwd), api.agentSessionLive(),
    ]).catch((err: unknown): never => {
      // An unsafe recorded folder (Paperclip PR 19): say why, keep the chat.
      const unsafe = unsafeCwdText(err);
      throw unsafe ? new Error(`${unsafe} The current conversation has been kept.`) : err;
    });
    if (live) throw new Error('Wait for the current agent turn to finish.');
    if (!KNOWN_AGENTS.includes(session.agent as ChatAgent) || !available.includes(session.agent)) {
      throw new Error(`${session.agent} is not installed. Install that agent before continuing this session.`);
    }
    if (missingCwd) throw new Error(`The recorded working folder no longer exists: ${missingCwd}. The current conversation has been kept.`);
    if (!fromSummary && !canResumeArtifactSession(context)) {
      throw new Error('Native resume is no longer available. Choose Continue from summary to start a new conversation.');
    }
    const native = !fromSummary;
    // Native resume adopts `session.key` on screen. A copy of it still held
    // in the background would take every event of the new turn (review
    // A·D4): refuse while its turn runs there, drop it once finished (its
    // turns are in the session store this continue reads from).
    if (native && isBackgroundChat(session.key)) {
      throw new Error('This conversation is still running. Open this conversation first, or wait for its turn to finish.');
    }
    if (native && isBackgrounded(session.key)) discardBackgroundChat(session.key);
    const draft = chatInput();
    const permission = conversationPermission(session.key, context.permissionMode, session.agent);
    batch(() => {
      // Release the artificial busy flag synchronously inside the navigation
      // hold, so guarded reset/provider setters actually apply.
      setChatBusy(false);
      resetChatState();
      setChatAgentPersistent(session.agent as ChatAgent);
      setChatCwdPersistent(session.cwd);
      if (native) {
        setChatAgentSessionKeyPersistent(session.key);
        setChatPermissionMode(permission.mode);
        setChatSessionIdPersistent(permission.resumeNative ? session.harnessSessionId : null);
      } else {
        applyNewChatPermissionMode();
        // A new conversation: the context compiler bundles the prior turns.
        setChatContinuedFrom([session.key]);
      }
      setChatTurns(artifactSessionTurns(context));
      pushChatTurn({ role: 'system', tools: [], text: native && !permission.resumeNative
        ? 'Showing recorded turn summaries. The next send starts a fresh native session with this conversation’s retained context and selected permissions.'
        : native
        ? 'Showing recorded turn summaries, not the full transcript. The next send resumes this session with its original agent and working folder. If the provider no longer has it, return to Sessions and choose Continue from summary.'
        : 'Showing prior turn summaries for context. The next send starts a new conversation from the summary in the composer.' });
      const prompt = artifactContinuationPrompt(path, context, native);
      setChatInput(draft.trim() ? `${draft}\n\n${prompt}` : prompt);
      setChatPanelOpen(false);
      setActivePage('chat');
      requestChatFocus();
    });
  } finally {
    setChatBusy(false);
    release();
  }
}
