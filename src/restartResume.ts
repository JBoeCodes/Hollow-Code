/**
 * Resume after restart (overnight A1, P1-40): the "Resume N interrupted
 * chats" card's text. Pure and dependency-free (vitest loads it in node). The
 * rules live in `src-tauri/src/restart_resume.rs`; resuming is only ever the
 * person's click, one ticked chat at a time.
 */
import type { InterruptedChat, ResumeOutcome } from './generated/ipc';
import type { ChecklistRow } from './checklist';
import { CONTINUE_PROMPT } from './chatContinue';

/** Words the caller owns (the store's labels), so this file stays pure. */
export interface ResumeLabels {
  /** The workspace's title, or '' outside one. */
  where(workspaceId: string | null): string;
  /** "Claude", "Codex"… */
  agent(id: string): string;
  /** The permission mode's picker label, or '' when none was recorded. */
  mode(mode: string | null, agent?: string): string;
  /** The chat's own Ultracode switch (kept in the webview, per chat). */
  ultracode(sessionKey: string): boolean;
}

/** The chats an agent started that the quit also stopped: named, with an
 *  Open per row and nothing to tick (the depth-0 rule: only the person's own
 *  chats resume from here). */
export interface ResumeAside {
  heading: string;
  note: string;
  actionLabel: string;
  rows: { id: string; label: string; detail?: string }[];
}

const join = (parts: (string | null | undefined | false)[]) => parts.filter(Boolean).join(' · ');

/** The checklist the card's row opens: one ticked row per chat with what its
 *  resume runs with, Cancel focused; agent-started chats listed apart. */
export function resumeChecklist(chats: readonly InterruptedChat[], labels: ResumeLabels): {
  title: string; body: string; confirmLabel: string; cancelLabel: string; focusCancel: boolean;
  checklist: ChecklistRow[]; aside?: ResumeAside;
} {
  const mine = chats.filter(c => !c.agentStarted);
  const theirs = chats.filter(c => c.agentStarted);
  const n = mine.length;
  return {
    title: n === 1 ? 'Resume 1 interrupted chat?' : `Resume ${n} interrupted chats?`,
    body: [
      'Hollow quit while these were running. Each ticked chat gets one message, with the agent, model and permission mode shown:',
      `“${CONTINUE_PROMPT}”`,
    ].join('\n'),
    confirmLabel: 'Resume ticked',
    cancelLabel: 'Not now',
    // Each resume is a billed message: a stray Enter must not send them.
    focusCancel: true,
    checklist: mine.map(c => ({
      id: c.sessionKey,
      label: c.title,
      detail: join([labels.where(c.workspaceId), labels.agent(c.agent), c.model || 'default model', labels.mode(c.permissionMode, c.agent) || 'default permissions', labels.ultracode(c.sessionKey) && 'Ultracode on']),
      checked: true,
    })),
    aside: theirs.length ? {
      heading: `${theirs.length} agent-started chat${theirs.length === 1 ? '' : 's'} also stopped`,
      note: 'The agent that started each one heard how it ended. Open one and press Continue to carry it on.',
      actionLabel: 'Open',
      rows: theirs.map(c => ({ id: c.sessionKey, label: c.title, detail: join([labels.where(c.workspaceId), labels.agent(c.agent)]) })),
    } : undefined,
  };
}

/** The footer line after a resume: how many were queued, every chat that
 *  was not and why, and how many unticked chats stay offered. */
export function resumeSummary(outcomes: readonly ResumeOutcome[], titleOf: (sessionKey: string) => string, kept = 0): string {
  const queued = outcomes.filter(o => o.queued).length;
  const skipped = outcomes.filter(o => !o.queued);
  if (outcomes.length === 0) return 'Nothing to resume.';
  const later = kept > 0 ? ` ${kept} more ${kept === 1 ? 'stays' : 'stay'} in Needs you to resume later.` : '';
  if (skipped.length === 0) return `${queued === 1 ? 'Resumed 1 chat.' : `Resumed ${queued} chats.`}${later}`;
  const why = skipped.map(o => `${titleOf(o.sessionKey)} (${o.reason || 'no reason given'})`).join('; ');
  return `${queued > 0 ? `Resumed ${queued}; not resumed` : '× Not resumed'}: ${why}${later ? `.${later}` : ''}`;
}
