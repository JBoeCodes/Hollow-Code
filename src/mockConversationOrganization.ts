import type { AgentSessionSummary } from './types';
import type { ConversationOrganization, OrganizationChange } from './conversationOrganization';

export function mockConversationOrganization(command: string, args: Record<string, unknown> | undefined, fallback: AgentSessionSummary): { value: unknown } | null {
  if (command === 'sweep_conversation_settlement') return { value: [] }; // Native policy checks own git and durable turn status.
  if (!['list_conversation_organization', 'organize_conversation', 'configure_conversation_settlement'].includes(command)) return null;
  const rows = JSON.parse(localStorage.getItem('mockConversationOrganization') ?? '{}') as Record<string, ConversationOrganization>;
  if (command === 'list_conversation_organization') return { value: Object.values(rows) };
  const change = args?.change as OrganizationChange;
  const sessions = JSON.parse(localStorage.getItem('mockWorkspaceSessions') ?? '[]') as (AgentSessionSummary & { archived?: boolean })[];
  const session = sessions.find(s => s.key === change.key) ?? (fallback.key === change.key ? fallback : undefined);
  if (!session) throw 'Unknown conversation';
  const old = rows[change.key];
  if ((old?.revision ?? 0) !== change.expectedRevision || session.lastActivityAt !== change.expectedActivity) throw 'CONVERSATION_CHANGED: refresh and try again';
  if (command === 'configure_conversation_settlement') {
    const kind = (args?.change as { action: string }).action;
    if (kind === 'merged') throw 'This conversation has no bound build worktree';
    if (!['idle', 'off'].includes(kind)) throw 'Choose a settlement policy';
    if (kind !== 'off' && (old?.pinned || old?.settled || old?.snoozedUntil)) throw 'Wake and unpin this conversation first';
    const revision = change.expectedRevision + 1;
    const row: ConversationOrganization = { ...(old ?? { key: change.key, pinned: false, snoozedUntil: null, settled: false, reason: null }), session, revision, autoSettle: kind === 'off' ? null : { kind: 'idle', activity: session.lastActivityAt, revision, pr: null } };
    rows[change.key] = row;
    localStorage.setItem('mockConversationOrganization', JSON.stringify(rows));
    return { value: row };
  }
  const hiding = change.action === 'snooze' || change.action === 'settle';
  if (hiding && old?.pinned) throw 'Unpin this conversation first';
  const queue = JSON.parse(localStorage.getItem('mockRunQueue') ?? '[]') as { source: string; sourceId: string; status: string }[];
  if (hiding && queue.some(r => r.source === 'chat' && r.sourceId === change.key && ['queued', 'claimed', 'running'].includes(r.status))) throw 'This conversation is working';
  if (change.action === 'snooze' && !(Date.parse(change.until ?? '') > Date.now())) throw 'Choose a future snooze time';
  const row: ConversationOrganization = { key: change.key, session, pinned: old?.pinned ?? false, snoozedUntil: old?.snoozedUntil ?? null, settled: old?.settled ?? false, reason: old?.reason ?? null, revision: change.expectedRevision + 1 };
  if (change.action === 'pin') { row.pinned = true; row.snoozedUntil = null; row.settled = false; row.reason = null; }
  if (change.action === 'unpin') row.pinned = false;
  if (change.action === 'wake') { row.snoozedUntil = null; row.settled = false; row.reason = null; }
  if (change.action === 'snooze') { row.snoozedUntil = change.until!; row.settled = false; row.reason = null; }
  if (change.action === 'settle') { row.snoozedUntil = null; row.settled = true; row.reason = 'Settled by you'; }
  rows[change.key] = row;
  localStorage.setItem('mockConversationOrganization', JSON.stringify(rows));
  const stored = sessions.find(s => s.key === change.key);
  if (stored) { stored.archived = row.settled; localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions)); }
  if (session.key === fallback.key) localStorage.setItem('mockArchivedSession', String(row.settled));
  return { value: row };
}
