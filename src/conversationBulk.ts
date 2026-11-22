// Bulk Settle, Snooze and Move on the sidebar's ticked chats (3.5). Each chat is written with its
// own revision and activity, as one-at-a-time does, so a chat that changed meanwhile is left alone
// and counted. One Undo (5 s) wakes every chat the action changed.
import { api } from './api';
import { pickSnoozeTime } from './snoozePicker';
import { acceptOrganization, organizationApi, refreshConversationOrganization } from './conversationOrganizationLoad';
import { conversationOrganization } from './store/conversationOrganization';
import { ORGANIZATION_UNDO_MS, rememberOrganizationUndo } from './store/organizationUndo';
import { chatAgentSessionKey, dropChatInteractionsFor, flashFooter, flashWithUndo, showConfirm } from './store';
import { lifecycleFailed, moveSessionTo, railWorkspaces, refreshWorkspaceSessions, selectWorkspace, workspaceId, workspaces } from './store/workspaces';
import { unselect } from './store/rowSelection';
import { QUICK_CHATS_ID } from './workspaceTypes';
import { workspaceChoices } from './sessionMove';

export const bulkWords = (n: number) => `${n} chat${n === 1 ? '' : 's'}`;

/** "Settled 3 chats; 1 changed or was busy and was left." */
export function bulkSummary(verb: string, done: number, left: number): string {
  return `${verb} ${bulkWords(done)}${left ? `; ${left} changed or ${left === 1 ? 'was' : 'were'} busy and ${left === 1 ? 'was' : 'were'} left` : ''}.`;
}

export async function bulkOrganize(keys: readonly string[], action: 'settle' | 'snooze') {
  try {
    let until: string | undefined;
    if (action === 'snooze') {
      const picked = await pickSnoozeTime(Date.now(), () => lifecycleFailed('Pick a future date and time.'));
      if (!picked) return;
      until = picked;
    }
    if (!await refreshConversationOrganization()) { lifecycleFailed('Could not read conversation organization. Try again.'); return; }
    const done: { key: string; revision: number; activity: string }[] = [];
    let left = 0;
    for (const key of keys) {
      try {
        const conversation = await api.readAgentSession(key);
        const saved = conversationOrganization()[key];
        const result = await organizationApi.change({ key, action, until, expectedRevision: saved?.revision ?? 0, expectedActivity: conversation.summary.lastActivityAt });
        acceptOrganization(result);
        if (result.settled) dropChatInteractionsFor(key);
        done.push({ key, revision: result.revision, activity: result.session.lastActivityAt });
      } catch { left++; }
    }
    await refreshWorkspaceSessions();
    const opened = chatAgentSessionKey();
    if (action === 'settle' && opened && done.some(d => d.key === opened) && workspaceId() === QUICK_CHATS_ID) await selectWorkspace(QUICK_CHATS_ID, undefined, true);
    unselect(keys);
    if (!done.length) { lifecycleFailed(bulkSummary(action === 'settle' ? 'Settled' : 'Snoozed', 0, left)); return; }
    rememberOrganizationUndo(flashWithUndo({ message: bulkSummary(action === 'settle' ? 'Settled' : 'Snoozed', done.length, left), kind: 'archive', ttlMs: ORGANIZATION_UNDO_MS, undo: async () => {
      let failed = 0;
      for (const d of done) {
        try { acceptOrganization(await organizationApi.change({ key: d.key, action: 'wake', expectedRevision: d.revision, expectedActivity: d.activity })); }
        catch { failed++; }
      }
      await refreshWorkspaceSessions();
      if (failed) lifecycleFailed(`${bulkWords(failed)} changed meanwhile and stayed as they are.`);
    } }));
  } catch (e) { lifecycleFailed(String(e)); }
}

/** Move the ticked chats to a project or back to Quick chats, picked in a dialog. */
export async function bulkMove(keys: readonly string[]) {
  const choices = workspaceChoices(workspaces(), railWorkspaces(), QUICK_CHATS_ID).slice(0, 14);
  if (!choices.length) { flashFooter('No project to move to yet.'); return; }
  const target = await new Promise<string | null>(resolve => {
    void showConfirm({
      title: `Move ${bulkWords(keys.length)}`, hideConfirm: true,
      aside: { heading: 'Move to', actionLabel: 'Move', rows: choices.map(c => ({ id: c.id, label: c.title, detail: c.hint })), onAction: id => resolve(id) },
    }).then(() => resolve(null));
  });
  if (!target) return;
  let moved = 0, left = 0;
  for (const key of keys) { if (await moveSessionTo(key, target, { noUndo: true, failed: () => { left++; } })) moved++; }
  unselect(keys);
  const name = choices.find(c => c.id === target)?.title ?? 'the project';
  flashFooter(`Moved ${bulkWords(moved)} to ${name}${left ? `; ${left} busy or refused and stayed` : ''}.`, 5000);
}
