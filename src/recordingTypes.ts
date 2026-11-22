/** Rust `recordings` (next-level P11). Recordings stay in Plaud; these are
 *  on-demand reads, explicit links and an opt-in kept copy. */
export interface RecordingRow { id: string; name: string; date: string | null; duration: string | null }
export interface RecordingLink { workspaceId: string | null; eventId: string | null; eventTitle: string | null; eventStart?: string | null; name: string | null; linkedAt: string; keptCopy: string | null }
export interface RecordingList { state: 'ok' | 'notConnected' | 'unavailable'; fetchedAt: string; source: string; recordings: RecordingRow[]; unparsed: number; error: string | null; links: Record<string, RecordingLink> }
export interface RecordingSegment { start: string; end: string; speaker: string | null; text: string }
export interface RecordingDetail {
  id: string; name: string | null; fetchedAt: string;
  transcript: RecordingSegment[] | null; transcriptState: 'available' | 'unavailable' | 'error';
  summary: string | null; summaryState: 'available' | 'unavailable' | 'error';
  audio: 'available' | 'unavailable' | 'unknown'; link: RecordingLink | null; errors: string[];
}
