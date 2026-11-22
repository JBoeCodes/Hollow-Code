// "Open the recording" from a task (plan completion item 4). The request
// only carries what to show; Library's Recordings answers by selecting the
// recording. One module, so the task code pulls in no Library chunk. (Open
// the plan item opens PLAN.md at its row in the Files tab since COO rebuild
// L4: TaskOrigin.tsx.)
import { createSignal } from 'solid-js';

// A request the view didn't answer within half a minute (the person went
// elsewhere first) is dropped, so it can't mark something much later.
const FRESH_MS = 30_000;
const [recording, setRecording] = createSignal<{ id: string; at: number } | null>(null);
export const recordingFocusRequest = () => { const r = recording(); return r && Date.now() - r.at < FRESH_MS ? r.id : null; };
export function requestRecording(id: string) { setRecording({ id, at: Date.now() }); }
export function clearRecordingFocus() { setRecording(null); }
