/**
 * "This conversation": who owns it, where it is kept and how it continues
 * (plan L3, 2026-10-14; review V05 / U model "quick chats, Main agent,
 * history and imported chats look similar but have different ownership and
 * resume rules", matrix X10). Pure; the Conversation history header shows these rows.
 *
 * Facts it states, from the code:
 * - every conversation's full history is kept by Hollow (the session store,
 *   outside the vault); a workspace keeps a summary record in its
 *   `sessions/` folder (Rust `workspaces::record_session`);
 * - quick chats live in the built-in Quick chats space (`workspaces/
 *   quick-chats/`), the Main agent in its own space with one thread a day
 *   (`mainAgent.todaysMainSession`);
 * - an imported conversation (added from another app before run 15 P4
 *   removed From other apps; `sessions.adopted`) has no agent session of
 *   its own until its first Hollow turn, which gets the earlier turns
 *   written in;
 * - `resumable` says whether the agent can resume its own session (`native`),
 *   only gets a written summary (`briefOnly`) or has lost its transcript.
 */
import type { AgentSessionSummary } from './types';

export type ConversationKind = 'workspace' | 'quick' | 'main' | 'vault';

export interface AboutRow { label: string; detail: string }

export interface ConversationAbout {
  /** A short name for the kind: "Workspace conversation", "Quick chat", … */
  kind: string;
  rows: AboutRow[];
}

type SessionFacts = Pick<AgentSessionSummary, 'adopted' | 'resumable' | 'stints' | 'agent'>;

const AGENT_NAMES: Record<string, string> = { claude: 'Claude', codex: 'Codex', antigravity: 'Antigravity', grok: 'Grok' };
const agentName = (id: string) => AGENT_NAMES[id] ?? id;

export function conversationAbout(input: {
  kind: ConversationKind;
  workspaceTitle?: string;
  /** The listed session, when the conversation has one yet. */
  session?: SessionFacts | null;
  /** The agent this conversation now uses. */
  agent: string;
  /** A native agent session id is held (the next turn resumes it). */
  nativeSession: boolean;
  /** Turns shown in the conversation. */
  turns: number;
}): ConversationAbout {
  const rows: AboutRow[] = [];
  const title = input.workspaceTitle?.trim() || 'this workspace';
  const kind = input.kind === 'quick' ? 'Quick chat'
    : input.kind === 'main' ? 'COO conversation'
      : input.kind === 'vault' ? 'Vault conversation'
        : 'Workspace conversation';
  rows.push({
    label: 'Belongs to',
    detail: input.kind === 'quick' ? 'Quick chats: no project, brief or sources. Move it to a project to give it those.'
      : input.kind === 'main' ? 'The COO: one thread a day, across all your workspaces. Earlier days are in Conversation history.'
        : input.kind === 'vault' ? 'Your vault, not a workspace'
          : `${title}`,
  });
  rows.push({
    label: 'Kept in',
    detail: input.kind === 'vault'
      ? 'Hollow keeps the whole conversation (outside the vault).'
      : `Hollow keeps the whole conversation; ${input.kind === 'workspace' ? `${title}` : input.kind === 'quick' ? 'the Quick chats folder' : 'the COO space'} keeps a summary in its sessions folder.`,
  });
  const s = input.session;
  if (s?.adopted) {
    rows.push({
      label: 'Came from',
      detail: s.stints.length
        ? 'Imported from another app, then continued here.'
        : 'Imported from another app. Your first message here writes its earlier turns in and continues it as a normal conversation.',
    });
  }
  const agents = (s?.stints ?? []).map(st => st.agent).filter((a, i, all) => i === 0 || all[i - 1] !== a);
  if (agents.length > 1) rows.push({ label: 'Agents', detail: `${agents.map(agentName).join(', then ')}` });
  const name = agentName(input.agent);
  rows.push({
    label: 'Continues',
    detail: !input.turns && !s ? `When you send, ${name} starts a new session.`
      : s?.resumable === 'none' ? `The agent's own transcript is gone; ${name} starts a new session and Hollow writes in the latest part of this conversation.`
        : s?.resumable === 'briefOnly' ? `${name} can't resume its own session here, so Hollow writes in the latest part of this conversation each time it starts one.`
          : input.nativeSession ? `${name} resumes its own session, so it remembers this conversation. Another agent or model starts a new one, with the latest part written in.`
            : `${name} starts a new session with the latest part of this conversation written in.`,
  });
  return { kind, rows };
}
