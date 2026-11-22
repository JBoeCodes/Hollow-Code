import { api, invoke } from './api';
import type { ConversationOrganization } from './conversationOrganization';
import { acceptOrganization, organizationApi, refreshConversationOrganization } from './conversationOrganizationLoad';
import { conversationOrganization } from './store/conversationOrganization';
import { chatAgentSessionKey, flashWithUndo, flashFooter, showConfirm } from './store';
import { lifecycleFailed, refreshWorkspaceSessions } from './store/workspaces';
import { sessionState } from './store/sessionStates';

export async function chooseSettlement(key: string) {
  if (!await refreshConversationOrganization()) { lifecycleFailed('Could not read the current policy. Try again.'); return; }
  const policy = conversationOrganization()[key]?.autoSettle;
  const current = policy?.pr ? `PR #${policy.pr.number} merges (${policy.pr.url})` : policy ? 'Seven idle days' : 'Off';
  await showConfirm({ title: 'Automatic settlement', summary: `Current policy: ${current}. Applies once to this conversation’s current activity.`, body: 'New activity or a manual organization action turns this policy off. Running, queued, pinned, paused and attention-needed chats stay visible. Checks run while Hollow is open.', hideConfirm: true, aside: {
    heading: 'Choose a policy', actionLabel: 'Choose', rows: [
      { id: 'idle', label: 'After seven idle days', detail: 'Only after the last turn finished successfully.' },
      { id: 'merged', label: 'When this branch’s PR merges', detail: 'Verify and bind the open PR for this conversation’s build worktree.' },
      { id: 'off', label: 'Off', detail: 'Keep this conversation active until you settle it.' },
    ], onAction: kind => { void configure(key, kind); },
  } });
}

async function configure(key: string, kind: string) {
  try {
    if (!await refreshConversationOrganization()) throw new Error('Could not read conversation organization.');
    const session = await api.readAgentSession(key), saved = conversationOrganization()[key];
    const row = await invoke<ConversationOrganization>('configure_conversation_settlement', { change: { key, action: kind, expectedRevision: saved?.revision ?? 0, expectedActivity: session.summary.lastActivityAt } });
    acceptOrganization(row);
    flashFooter(kind === 'off' ? 'Automatic settlement is off' : row.autoSettle?.pr ? `Automatic settlement is bound to PR #${row.autoSettle.pr.number}` : 'Automatic settlement is on after seven idle days');
  } catch (e) { lifecycleFailed(String(e).includes('CONVERSATION_CHANGED') ? 'This conversation changed. Reopen its menu and try again.' : String(e)); }
}

let checking = false;
export async function checkSettlement() {
  if (checking) return;
  checking = true;
  try {
    // Protect the open key even on a different page. Native checks also refuse
    // live work, waiting cards, paused chats and unsuccessful last turns.
    const protectedKeys = Object.keys(conversationOrganization()).filter(key => sessionState(key) !== 'idle');
    const current = chatAgentSessionKey();
    if (current) protectedKeys.push(current);
    const rows = await invoke<ConversationOrganization[]>('sweep_conversation_settlement', { protected: protectedKeys });
    if (!rows.length) return;
    for (const row of rows) acceptOrganization(row);
    await refreshWorkspaceSessions();
    flashWithUndo({ kind: 'archive', message: `${rows.length} conversation${rows.length === 1 ? '' : 's'} settled automatically. Reasons are in Settled.`, undo: async () => {
      for (const row of rows) {
        try { acceptOrganization(await organizationApi.change({ key: row.key, action: 'wake', expectedRevision: row.revision, expectedActivity: row.session.lastActivityAt })); }
        catch { lifecycleFailed('A settled conversation changed. Open Settled to restore it.'); }
      }
      await refreshWorkspaceSessions();
    } });
  } catch { /* Offline PR checks and temporary read failures never settle work. */ }
  finally { checking = false; }
}
