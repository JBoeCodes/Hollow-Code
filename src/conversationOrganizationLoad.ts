import { invoke } from './api';
import type { ConversationOrganization, OrganizationChange } from './conversationOrganization';
import { setConversationOrganization, setOrganizationError } from './store/conversationOrganization';
import { keepUnchangedRecord } from './keyedList';

export const organizationApi = {
  list: () => invoke<ConversationOrganization[]>('list_conversation_organization'),
  change: (change: OrganizationChange) => invoke<ConversationOrganization>('organize_conversation', { change }),
};
let pending: Promise<boolean> | null = null;
export function refreshConversationOrganization(): Promise<boolean> {
  if (pending) return pending;
  pending = (async () => {
    try {
      const rows = await organizationApi.list();
      // A list started before a newly accepted write can omit that row.
      // Organization rows are durable, so keep accepted keys absent there.
      setConversationOrganization(previous => keepUnchangedRecord(previous, { ...previous, ...Object.fromEntries(rows.map(row => {
        const old = previous[row.key];
        return [row.key, old && (old.revision > row.revision || Date.parse(old.session.lastActivityAt) > Date.parse(row.session.lastActivityAt)) ? old : row];
      })) }));
      setOrganizationError(false);
      return true;
    } catch { setOrganizationError(true); return false; }
    finally { pending = null; }
  })();
  return pending;
}
export function acceptOrganization(row: ConversationOrganization) {
  setConversationOrganization(previous => ({ ...previous, [row.key]: row }));
}
