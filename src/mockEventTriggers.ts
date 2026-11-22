// Browser-mock `event_triggers_status` / `set_event_trigger` (COO rebuild
// F2), kept in localStorage `mockEventTriggers` so a reload sees a change.
// Refuses what `event_triggers.rs` refuses, in its words. Tracked projects
// are the mock's ordinary projects (`mockEventTriggerTracked` overrides);
// connected ones are its build workspaces.

import type { EventKind, EventKindStatus, EventTriggerStatus, SetEventTrigger } from './eventTriggersApi';

// The backend's kinds and labels (a type-only import keeps api.ts out).
const EVENT_KINDS: { kind: EventKind; label: string; perWorkspace: boolean }[] = [
  { kind: 'tracker-issue', label: 'New issue in team AI', perWorkspace: true },
  { kind: 'pr-merged', label: 'Merged PR in a connected repo', perWorkspace: true },
  { kind: 'recording-ended', label: 'Recording or meeting ends', perWorkspace: false },
];

const KEY = 'mockEventTriggers';

interface Saved {
  kinds: Record<string, { on: boolean; since: string | null; cap: number }>;
  workspaces: Record<string, Record<string, string>>;
  leadWakes: Record<string, string>;
}

const empty = (): Saved => ({ kinds: {}, workspaces: {}, leadWakes: {} });
const read = (): Saved => ({ ...empty(), ...JSON.parse(localStorage.getItem(KEY) ?? 'null') ?? {} });
interface MockWorkspace { id: string; title: string; kind?: string; build?: unknown }
const projects = (): MockWorkspace[] => (JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as MockWorkspace[])
  .filter(w => w.id !== 'main' && w.id !== 'quick-chats' && w.kind !== 'quick');

function status(s: Saved): EventTriggerStatus {
  const all = projects();
  const title = (id: string) => all.find(w => w.id === id)?.title ?? id;
  const kinds: EventKindStatus[] = EVENT_KINDS.map(({ kind, label, perWorkspace }) => {
    const k = s.kinds[kind] ?? { on: false, since: null, cap: 6 };
    return {
      kind, label, on: k.on, since: k.since, capPerHour: k.cap, firedLastHour: 0, perWorkspace,
      workspaces: Object.entries(s.workspaces[kind] ?? {}).map(([id, since]) => ({ id, title: title(id), since, leadWake: kind === 'tracker-issue' && id in s.leadWakes })),
      automations: [],
    };
  });
  const override = localStorage.getItem('mockEventTriggerTracked');
  return {
    kinds, leadWakes: Object.keys(s.leadWakes),
    tracked: override ? JSON.parse(override) as string[] : all.filter(w => !w.build).map(w => w.id),
    connected: all.filter(w => w.build).map(w => w.id),
    recent: [], pendingWakes: 0,
  };
}

function set(input: SetEventTrigger): EventTriggerStatus {
  const s = read();
  const now = new Date().toISOString();
  const meta = EVENT_KINDS.find(k => k.kind === input.kind);
  if (!meta) throw new Error(`Unknown trigger kind "${input.kind}": use tracker-issue, pr-merged or recording-ended. Nothing changed.`);
  const kindOn = (kind: EventKind, on: boolean) => {
    const k = s.kinds[kind] ?? { on: false, since: null, cap: 6 };
    s.kinds[kind] = on ? { ...k, on: true, since: k.on ? k.since : now } : { ...k, on: false, since: null };
  };
  if (input.capPerHour != null) s.kinds[input.kind] = { ...(s.kinds[input.kind] ?? { on: false, since: null, cap: 6 }), cap: Math.min(60, Math.max(1, input.capPerHour)) };
  const ws = input.workspaceId?.trim();
  if (ws) {
    const found = projects().find(w => w.id === ws);
    if (!found) throw new Error(`No project ${ws}. Nothing changed.`);
    if (input.leadWake === true && !status(s).tracked.includes(ws)) throw new Error('This project isn\'t tracked in the task tracker yet (Track in, from the project\'s tracker menu). Nothing changed.');
    const map = s.workspaces[input.kind] ??= {};
    // The lead wake and the project's trigger are separate switches.
    if (input.leadWake === true) { s.leadWakes[ws] ??= now; kindOn(input.kind, true); }
    else if (input.leadWake === false) delete s.leadWakes[ws];
    if (input.on === true) { map[ws] ??= now; kindOn(input.kind, true); }
    else if (input.on === false) delete map[ws];
  } else if (input.on != null) kindOn(input.kind, input.on);
  localStorage.setItem(KEY, JSON.stringify(s));
  return status(s);
}

export const isMockEventTriggersCommand = (command: string) => command === 'event_triggers_status' || command === 'set_event_trigger';

export function mockEventTriggersCommand(command: string, args: Record<string, unknown>): EventTriggerStatus {
  return command === 'set_event_trigger' ? set(args.input as SetEventTrigger) : status(read());
}
