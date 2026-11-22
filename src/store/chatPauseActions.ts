// Pause all, Resume all and resume at a chosen time (P1b, P1c): the actions
// Running (Mission Control) and the paused composer's "Resume at..." control
// take. Kept apart from `store/chatPause.ts` (which the chat runtime loads in
// the App chunk) so only the surfaces that offer them load this.
import { api } from '../api';
import { latestRequest } from '../latestRequest';
import { errMsg } from '../utils';
import { clearChatPaused, markChatPaused, pausedChats, setChatPausing, setPausedResumeAt } from './chatPause';

const listRead = latestRequest();

/** Follow the backend's paused records (note, reason, resume time): every
 *  chat it lists is paused, and a chat it no longer lists isn't. A failed
 *  read changes nothing. */
export async function refreshPausedChats(): Promise<void> {
  const generation = listRead.start();
  try {
    const records = await api.chatPausedList();
    if (!listRead.isLatest(generation)) return;
    const listed = new Set(records.map(r => r.sessionKey));
    for (const key of Object.keys(pausedChats())) if (!listed.has(key)) clearChatPaused(key);
    for (const r of records) markChatPaused(r.sessionKey, r.note, r.pausedAt, { reason: r.reason, resumeAt: r.resumeAt });
  } catch { /* the state stays as it was */ }
}

/** P1b: pause every running chat at its next safe point. Each asked chat
 *  shows Pausing until its `paused` event (or its turn's end). Returns how
 *  many were asked; throws the backend's refusal. */
export async function pauseAllChats(): Promise<number> {
  const asked = await api.chatPauseAll();
  for (const key of asked) setChatPausing(key, true);
  return asked.length;
}

/** P1b: resume every paused chat whose time isn't still ahead (review fix:
 *  one waiting for its own time, such as a limit's reset, is skipped).
 *  Returns the counts; the ones refused or skipped keep their record. */
export async function resumeAllChats(): Promise<{ resumed: number; refused: number; skipped: number; firstError: string | null }> {
  const outcome = await api.chatResumeAll();
  for (const key of outcome.resumed) clearChatPaused(key, true);
  await refreshPausedChats();
  return { resumed: outcome.resumed.length, refused: outcome.refused.length, skipped: outcome.skipped?.length ?? 0, firstError: outcome.refused[0]?.[1] ?? null };
}

/** P1c: set (`at`, unix seconds) or clear (null) when `key` resumes on its
 *  own. A time that has already passed is the caller's to resume now (the
 *  composer adopts that turn); the backend resumes one that passed in
 *  flight and answers null. Returns an error message, or null. */
export async function setChatResumeAt(key: string, at: number | null): Promise<string | null> {
  try {
    const record = await api.chatSetResumeAt(key, at);
    if (record) setPausedResumeAt(key, record.resumeAt);
    else clearChatPaused(key, true);
    return null;
  } catch (err) {
    void refreshPausedChats();
    return errMsg(err);
  }
}

/** How many chats are paused: Resume all shows while any is. */
export const pausedCount = () => Object.keys(pausedChats()).length;
