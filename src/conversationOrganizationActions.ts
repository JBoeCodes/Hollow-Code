import { api } from './api';
import type { OrganizationChange } from './conversationOrganization';
import { pickSnoozeTime } from './snoozePicker';
import { ORGANIZATION_UNDO_MS, rememberOrganizationUndo } from './store/organizationUndo';
import { conversationOrganization } from './store/conversationOrganization';
import { acceptOrganization, organizationApi, refreshConversationOrganization } from './conversationOrganizationLoad';
import { chatAgentSessionKey, dropChatInteractionsFor, flashFooter, flashWithUndo } from './store';
import { lifecycleFailed, openSessionByKey, refreshWorkspaceSessions, selectWorkspace, workspaceId } from './store/workspaces';

const failure = (e: unknown) => lifecycleFailed(String(e).includes('CONVERSATION_CHANGED') ? 'This conversation changed. Reopen its menu and try again.' : String(e));
export async function organizeConversation(key: string, action: OrganizationChange['action'] | 'automatic', after?: () => unknown, opts: { fresh?: string | null; beforeSettled?: () => void } = {}) {
  try {
    if (action === 'automatic') { await import('./conversationSettlement').then(m => m.chooseSettlement(key)); return; }
    if (!await refreshConversationOrganization()) { lifecycleFailed('Could not read conversation organization. Try again.'); return; }
    // Read the exact conversation, including an older or archived one.
    const conversation = await api.readAgentSession(key);
    const saved = conversationOrganization()[key];
    let until: string | undefined;
    if (action === 'snooze') {
      const picked = await pickSnoozeTime(Date.now(), () => lifecycleFailed('Pick a future date and time.'));
      if (!picked) return;
      until = picked;
    }
    const result = await organizationApi.change({ key, action, until, expectedRevision: saved?.revision ?? 0, expectedActivity: conversation.summary.lastActivityAt });
    acceptOrganization(result);
    if (result.settled) { opts.beforeSettled?.(); dropChatInteractionsFor(key); }
    await refreshWorkspaceSessions();
    await after?.();
    if (result.settled && opts.fresh && workspaceId() === opts.fresh && chatAgentSessionKey() === key) await selectWorkspace(opts.fresh, undefined, true);
    // One Undo for every organization action, 5 s, also reachable from
    // Edit → Undo Chat Action (organizationUndo.ts).
    const undoWith = (message: string, change: Omit<OrganizationChange, 'key' | 'expectedRevision' | 'expectedActivity'>) => {
      rememberOrganizationUndo(flashWithUndo({ message, kind: 'archive', ttlMs: ORGANIZATION_UNDO_MS, undo: async () => {
        try {
          const restored = await organizationApi.change({ key, ...change, expectedRevision: result.revision, expectedActivity: result.session.lastActivityAt });
          acceptOrganization(restored); await refreshWorkspaceSessions(); await after?.();
        } catch (e) { failure(e); }
      } }));
    };
    if (action === 'settle') undoWith('Conversation settled. Undo restores the chat; closed questions stay closed.', { action: 'wake' });
    else if (action === 'snooze') undoWith(`Snoozed until ${new Date(until!).toLocaleString()}`, { action: 'wake' });
    else if (action === 'pin') undoWith('Conversation pinned', { action: 'unpin' });
    else if (action === 'wake') {
      // Undo puts it back as it was: settled again, or snoozed to the same time.
      const again = saved?.settled ? { action: 'settle' as const } : saved?.snoozedUntil && Date.parse(saved.snoozedUntil) > Date.now() ? { action: 'snooze' as const, until: saved.snoozedUntil } : null;
      if (again) undoWith('Conversation awake', again);
      else flashFooter('Conversation awake');
      await openSessionByKey(key);
    } else flashFooter('Conversation unpinned');
  } catch (e) { failure(e); }
}
