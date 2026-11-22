import { afterEach, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ prep: vi.fn(), recap: vi.fn(), note: vi.fn(), confirm: vi.fn() }));
vi.mock('../api', () => ({ api: { prepPack: mocks.prep, meetingRecap: mocks.recap, vaultNote: mocks.note } }));
vi.mock('./overlays', () => ({ showConfirm: mocks.confirm }));
import { closePrepPack, openMeetingRecap, openPrepPack, prepBusy, prepError, prepPack, rewritePrepPack } from './prepPack';

const saved = { meetingId: 'meeting', subject: 'Planning', start: '', path: 'outputs/prep/planning.md', workspaceId: null, markdown: '# Planning', kept: true, contentHash: 'saved', edited: false };
afterEach(() => { closePrepPack(); vi.resetAllMocks(); });

async function open(edited = false, recap = false) {
  const pack = { ...saved, edited };
  mocks.prep.mockResolvedValue(pack);
  mocks.recap.mockResolvedValue({ ...pack, kind: 'recap', queued: 0, dropped: 0 });
  mocks.note.mockResolvedValue({ revision: 'saved' });
  if (recap) openMeetingRecap('meeting', 'Planning', '2026-10-22T12:00:00Z');
  else openPrepPack('meeting', 'Planning');
  await vi.waitFor(() => expect(prepBusy()).toBe(false));
}

it('rewrites unchanged notes directly with the revision read now', async () => {
  await open();
  rewritePrepPack();
  await vi.waitFor(() => expect(mocks.prep).toHaveBeenCalledTimes(2));
  expect(mocks.confirm).not.toHaveBeenCalled();
  expect(mocks.prep).toHaveBeenLastCalledWith('meeting', true, 'saved');
});

it('cancel preserves the loaded note without another model request', async () => {
  await open(true);
  mocks.confirm.mockResolvedValue(false);
  rewritePrepPack();
  await vi.waitFor(() => expect(prepBusy()).toBe(false));
  expect(mocks.confirm).toHaveBeenCalledOnce();
  expect(mocks.prep).toHaveBeenCalledOnce();
  expect(prepPack()?.markdown).toBe(saved.markdown);
});

it('closing while confirmation is open prevents an approved rewrite', async () => {
  await open(true);
  let approve!: (value: boolean) => void;
  mocks.confirm.mockImplementation(() => new Promise(resolve => { approve = resolve; }));
  rewritePrepPack();
  await vi.waitFor(() => expect(mocks.confirm).toHaveBeenCalledOnce());
  closePrepPack(); approve(true);
  await new Promise(resolve => setTimeout(resolve));
  expect(mocks.prep).toHaveBeenCalledOnce();
  expect(prepBusy()).toBe(false);
});

it('missing current revision refuses before confirmation or paid work', async () => {
  await open();
  mocks.note.mockResolvedValue({ revision: null });
  rewritePrepPack();
  await vi.waitFor(() => expect(prepError()).toContain('Could not verify'));
  expect(mocks.confirm).not.toHaveBeenCalled();
  expect(mocks.prep).toHaveBeenCalledOnce();
});

it('recap approval carries the freshly read revision instead of the opening revision', async () => {
  await open(false, true);
  mocks.note.mockResolvedValue({ revision: 'edited-after-opening' });
  mocks.confirm.mockResolvedValue(true);
  rewritePrepPack();
  await vi.waitFor(() => expect(mocks.recap).toHaveBeenCalledTimes(2));
  expect(mocks.confirm).toHaveBeenCalledOnce();
  expect(mocks.recap).toHaveBeenLastCalledWith('meeting', '2026-10-22T12:00:00Z', true, undefined, 'edited-after-opening');
});
