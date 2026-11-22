import { createSignal } from 'solid-js';
import type { ConversationOrganization } from '../conversationOrganization';

export const [conversationOrganization, setConversationOrganization] = createSignal<Record<string, ConversationOrganization>>({});
export const [organizationError, setOrganizationError] = createSignal(false);
export const [organizationNow, setOrganizationNow] = createSignal(Date.now());
export const [hiddenOrganizationProjects, setHiddenOrganizationProjects] = createSignal<ReadonlySet<string>>(new Set());
export const [awakeOrganizationProjects, setAwakeOrganizationProjects] = createSignal<ReadonlySet<string>>(new Set());
