import type { ConversationOrganization, OrganizationChange } from './conversationOrganization';

/** Sidebar destinations use the same single action and guards as row menus.
 * Never silently unpin before settling: a pin protects a conversation. */
export type RailDropSection = 'pinned' | 'active' | 'settled';
export interface RailDropPlan {
  action: OrganizationChange['action'] | null;
  label: string;
  reason?: string;
}

export function railDropPlan(section: RailDropSection, saved?: Pick<ConversationOrganization, 'pinned' | 'settled' | 'snoozedUntil'>, busy = false): RailDropPlan {
  if (section === 'pinned') return saved?.pinned
    ? { action: null, label: 'Already pinned' }
    : { action: 'pin', label: 'Pin conversation' };
  if (section === 'active') {
    if (saved?.pinned) return { action: 'unpin', label: 'Unpin conversation' };
    if (saved?.settled || saved?.snoozedUntil) return { action: 'wake', label: 'Wake conversation' };
    return { action: null, label: 'Already active' };
  }
  if (saved?.pinned) return { action: null, label: 'Unpin first', reason: 'Unpin this conversation before settling it.' };
  if (busy) return { action: null, label: 'Work in progress', reason: 'Wait for this conversation to finish before settling it.' };
  if (saved?.settled) return { action: null, label: 'Already settled' };
  return { action: 'settle', label: 'Settle conversation' };
}
