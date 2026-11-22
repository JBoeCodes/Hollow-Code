import type { AgendaEvent } from './agenda';
/** Rust `agenda::events` (next-level P10). `complete: false` (reason in
 * `partial`) when the read stopped before Graph's last page (merged plan
 * slice 5): the events are what was read, not the whole range. */
export interface AgendaEventsView { connected: boolean; fetchedAt: string; timeZone?: string | null; events: AgendaEvent[]; complete?: boolean; partial?: string | null; error: string | null; source: string; provider?: string; permission?: string; status?: string }
/** Rust `agenda::FocusBlock`: local planning, never a calendar event. */
export interface FocusBlock { id: string; title: string; start: string; end: string; taskUrl?: string | null; workspaceId?: string | null; note?: string | null; createdAt?: string; updatedAt?: string }

export interface CalendarSettings { source: 'mac' | 'microsoft' | 'off'; calendarIds: string[] | null }
export interface CalendarSourceStatus {
  settings: CalendarSettings;
  settingsValid?: boolean;
  permission: string;
  calendars: { id: string; title: string; account: string }[];
  error: string | null;
}
