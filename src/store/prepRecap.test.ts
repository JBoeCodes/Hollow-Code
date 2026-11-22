import { afterEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ recap: vi.fn(), prep: vi.fn() }));
vi.mock('../api', () => ({ api: { meetingRecap: mocks.recap, prepPack: mocks.prep } }));
import { closePrepPack, openMeetingRecap, openPrepPack, prepBusy, prepMeeting, prepPack, prepRecap, useRecapRecording } from './prepPack';
const choices = { kind: 'needsRecording', subject: 'Meeting', day: '2026-10-18', candidates: [] };
const pack = { kind: 'recap', meetingId: 'b', markdown: '# Recap', path: 'outputs/recaps/b.md', queued: 1, dropped: 0 };
afterEach(() => { closePrepPack(); vi.clearAllMocks(); });
it('opens recap mode and chooses a recording through the same latest-request loader', async () => {
  mocks.recap.mockResolvedValueOnce(choices).mockResolvedValueOnce(pack);
  openMeetingRecap('b', 'Meeting', '2026-10-18T12:00:00Z');
  await vi.waitFor(() => expect(prepRecap()).toEqual(choices));
  expect(prepMeeting()?.kind).toBe('recap'); expect(prepPack()).toBeNull();
  useRecapRecording('recording');
  await vi.waitFor(() => expect(prepPack()).toEqual(pack));
  expect(mocks.recap).toHaveBeenLastCalledWith('b', '2026-10-18T12:00:00Z', false, 'recording', undefined);
});
it('drops old recap replies when another meeting opens, and on close', async () => {
  let finish!: (value: unknown) => void;
  mocks.recap.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  mocks.prep.mockResolvedValue({ meetingId: 'new', markdown: '# Prep' });
  openMeetingRecap('b', 'Meeting', '2026-10-18T12:00:00Z');
  openPrepPack('new', 'Next meeting');
  await vi.waitFor(() => expect(prepPack()?.meetingId).toBe('new'));
  finish(pack); await Promise.resolve();
  expect(prepRecap()).toBeNull(); expect(prepMeeting()?.kind).toBe('prep');
  openMeetingRecap('b', 'Meeting', '2026-10-18T12:00:00Z');
  closePrepPack(); finish(pack); await Promise.resolve();
  expect(prepMeeting()).toBeNull(); expect(prepBusy()).toBe(false); expect(prepRecap()).toBeNull();
});
