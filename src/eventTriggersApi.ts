// Event triggers (COO rebuild F2): the IPC the automation editor's When:
// Event and the project menu's "Wake the lead for new issues" use. Shapes
// mirror `event_triggers.rs` (EventTriggerStatus, SetTrigger). Its own module
// (calls keep api.ts's mock routing), so the App chunk carries none of it.
import { invoke } from './api';

export type EventKind = 'tracker-issue' | 'pr-merged' | 'recording-ended';

export const EVENT_KINDS: { kind: EventKind; label: string; perWorkspace: boolean }[] = [
  { kind: 'tracker-issue', label: 'New issue in team AI', perWorkspace: true },
  { kind: 'pr-merged', label: 'Merged PR in a connected repo', perWorkspace: true },
  { kind: 'recording-ended', label: 'Recording or meeting ends', perWorkspace: false },
];

export interface EventTriggerWorkspace { id: string; title: string; since: string; leadWake: boolean }
export interface EventKindStatus {
  kind: EventKind; label: string; on: boolean; since: string | null; capPerHour: number;
  firedLastHour: number; perWorkspace: boolean; workspaces: EventTriggerWorkspace[]; automations: string[];
}
export interface RecentEvent { kind: EventKind; id: string; at: string; outcome: string; title: string; workspaceId?: string; started?: string[] }
export interface EventTriggerStatus {
  kinds: EventKindStatus[]; leadWakes: string[]; tracked: string[]; connected: string[];
  recent: RecentEvent[]; pendingWakes: number;
}
export interface SetEventTrigger { kind: EventKind; on?: boolean; workspaceId?: string; leadWake?: boolean; capPerHour?: number }

export const eventTriggersStatus = () => invoke<EventTriggerStatus>('event_triggers_status');
export const setEventTrigger = (input: SetEventTrigger) => invoke<EventTriggerStatus>('set_event_trigger', { input });

/** The event token in an automation's triggers, if any: `[kind, workspace]`. */
export function eventToken(triggers: readonly string[]): [EventKind, string | null] | null {
  for (const t of triggers) {
    const [head, ...rest] = t.trim().split(':');
    const kind = EVENT_KINDS.find(k => k.kind === head.toLowerCase())?.kind;
    if (kind) return [kind, rest.join(':') || null];
  }
  return null;
}

/** The token an automation saves for `kind` in `workspace`. */
export function tokenFor(kind: EventKind, workspace: string | null): string {
  return EVENT_KINDS.find(k => k.kind === kind)?.perWorkspace && workspace ? `${kind}:${workspace}` : kind;
}

/** After an automation with an event is saved: turn its trigger on (the
 *  kind, and its project for the per-project kinds). Saving is the person's
 *  click that turns it on; an older event still never fires. */
export async function turnOnFor(triggers: readonly string[]): Promise<void> {
  const token = eventToken(triggers);
  if (!token) return;
  const [kind, workspace] = token;
  await setEventTrigger(workspace ? { kind, workspaceId: workspace, on: true } : { kind, on: true });
}
