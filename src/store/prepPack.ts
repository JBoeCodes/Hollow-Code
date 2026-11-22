// Meeting prep packs (overnight 10-11 A4): the dialog's state. Its own
// module so Today, the Agenda and the shell can open it without the view.
import { createSignal } from 'solid-js';
import { api } from '../api';
import { errMsg } from '../utils';
import { latestRequest } from '../latestRequest';
import { setPrepPackOpen } from './prepPackOpen';
import { showConfirm } from './overlays';
export { prepPackOpen } from './prepPackOpen';

/** Rust `prep_pack::PrepPack`. */
export interface PrepPack { meetingId: string; subject: string; start: string; path: string; workspaceId: string | null; markdown: string; kept: boolean; contentHash: string; edited: boolean }
export type RecapOutcome = ({ kind: 'recap'; queued: number; dropped: number } & PrepPack) | { kind: 'needsRecording'; subject: string; day: string; candidates: import('../recordingTypes').RecordingRow[] };

const [meeting, setMeeting] = createSignal<{ id: string; title: string; kind: 'prep' | 'recap'; start?: string } | null>(null);
const [pack, setPack] = createSignal<PrepPack | null>(null);
const [recap, setRecap] = createSignal<RecapOutcome | null>(null);
export const prepRecap = recap;
const [busy, setBusy] = createSignal(false);
const [error, setError] = createSignal('');
export const prepMeeting = meeting;
export const prepPack = pack;
export const prepBusy = busy;
export const prepError = error;

/** Only the newest load answers; closing drops the one in flight. */
const loads = latestRequest();

/** Open a meeting's pack: the kept one, or written now. A new open always
 *  loads (an earlier one still loading is dropped). */
export function openPrepPack(id: string, title: string, start?: string) {
  setMeeting({ id, title, kind: start ? 'recap' : 'prep', start }); setPack(null); setRecap(null); setError('');
  setPrepPackOpen(true);
  void load(false, true);
}

export const openMeetingRecap = (id: string, title: string, start: string) => openPrepPack(id, title, start);
export const useRecapRecording = (id: string) => { void load(false, false, id); };

export function closePrepPack() { loads.cancel(); setBusy(false); setMeeting(null); setPrepPackOpen(false); }

/** Retry a failed first preparation without leaving the meeting. */
export function retryPrepPack() { void load(false); }

/** Write it again. */
export function rewritePrepPack() { void load(true); }

async function load(regenerate: boolean, fresh = false, recordingId?: string) {
  const m = meeting();
  if (!m || (busy() && !fresh)) return;
  const mine = loads.start();
  setBusy(true); setError('');
  try {
    let expectedHash: string | undefined;
    if (regenerate) {
      const saved = pack();
      if (!saved) throw new Error('Open the saved note before writing it again.');
      const current = await api.vaultNote(saved.path);
      if (!loads.isLatest(mine)) return;
      if (!current.revision) throw new Error('Could not verify the saved note. Open it again before rewriting.');
      if (saved.edited || current.revision !== saved.contentHash) {
        const ok = await showConfirm({ title: 'Replace your edited note?', body: 'This replaces the saved note, including your edits. Open as a note to keep a copy first.', confirmLabel: 'Replace note', danger: true });
        if (!ok || !loads.isLatest(mine)) return;
      }
      expectedHash = current.revision;
    }
    if (m.kind === 'recap') {
      const p = await api.meetingRecap(m.id, m.start!, regenerate, recordingId, expectedHash);
      if (loads.isLatest(mine)) { setRecap(p); setPack(p.kind === 'recap' ? p : null); }
    } else {
      const p = await api.prepPack(m.id, regenerate, expectedHash);
      if (loads.isLatest(mine)) setPack(p);
    }
  } catch (e) {
    if (loads.isLatest(mine)) setError(errMsg(e));
  } finally {
    if (loads.isLatest(mine)) setBusy(false);
  }
}

/** The page without its frontmatter, for reading. */
export function packBody(markdown: string): string {
  return markdown.replace(/^---\n[\s\S]*?\n---\n+/, '');
}
