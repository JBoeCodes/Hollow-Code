import { createSignal } from 'solid-js';
import type { AgentSessionSummary } from '../types';

/** A Hollow session to open in Conversation history (a conversation outside any
 *  workspace). `session` is its summary when the sender already found it by
 *  exact key (`session_target`), so Chat can open it even when its own list
 *  (the 300 newest unarchived) doesn't carry it (B·F15). */
export interface SessionRequest { key: string; session?: AgentSessionSummary }

/** The pending request. Its own module so the workspace store and the app
 *  bridge can both set it without importing each other. */
export const [appSessionRequest, setAppSessionRequest] = createSignal<SessionRequest | null>(null);

/** Conversation history lists only this workspace's conversations (the
 *  sidebar's Open in Conversation history, P1-42) until Show all, or until
 *  the person leaves the page. */
export const [historyWorkspace, setHistoryWorkspace] = createSignal<string | null>(null);

/** What Chat does with request `want`, given its list of conversations
 *  (`listed`, undefined until it has loaded) and, while a turn is running,
 *  the key of the conversation on screen (`running`, else null):
 *  - `'running'`: another conversation's turn is on screen, so nothing can
 *    open now; Chat says so rather than reporting nothing;
 *  - `'shown'`: the running conversation is the one asked for;
 *  - `{ open }`: open this conversation — the listed one, else the summary
 *    the sender handed over, so an older or archived one still opens;
 *  - `{ lookUp }`: neither has it; find it by exact key, and say so on
 *    screen if no conversation has that key (never a silent nothing);
 *  - `'wait'`: no summary and the list isn't loaded yet. */
export function sessionRequestStep(
  want: SessionRequest,
  listed: readonly AgentSessionSummary[] | undefined,
  running: string | null = null,
): { open: AgentSessionSummary } | { lookUp: string } | 'wait' | 'running' | 'shown' {
  if (running != null) return running === want.key ? 'shown' : 'running';
  const session = listed?.find(s => s.key === want.key) ?? want.session;
  if (session) return { open: session };
  return listed ? { lookUp: want.key } : 'wait';
}

/** What Chat shows when a conversation was asked for while another one's
 *  turn is running (openSession does nothing then). */
export const SESSION_REQUEST_BUSY = '× The current conversation is still running: open that one again once it finishes.';
