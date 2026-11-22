import { beforeEach, describe, expect, it, vi } from 'vitest';
const state = vi.hoisted(() => ({ key: 'origin', version: 7, page: 'today', route: vi.fn(async () => 'attached'), notice: vi.fn() }));
vi.mock('./store', () => ({ activePage: () => state.page, chatAgentSessionKey: () => state.key, chatConversationVersion: () => state.version,
  attachChatImage: vi.fn(), chatComposerOnScreen: () => true, flashFooter: state.notice }));
vi.mock('./store/workspaces', () => ({ openSessionByKey: vi.fn() }));
vi.mock('./store/sessionRequest', () => ({ appSessionRequest: () => null }));
vi.mock('./railFileDrop', () => ({ attachFilesToRailChat: state.route }));
vi.mock('./composerDrop', () => ({ FOLDER_REFUSED: 'Folders cannot attach', splitDrop: (dt: { items: { getAsFile(): File }[] }) => ({ folders: [], files: dt.items.map(item => ({ file: item.getAsFile(), known: true })) }) }));
import { railRowFileTarget } from './railRowFiles';

function drop() {
  const file = { name: 'note.txt' } as File;
  const data = { types: ['Files'], items: [{ kind: 'file', getAsFile: () => file }] };
  const event = { currentTarget: { classList: { remove: vi.fn(), add: vi.fn() } }, defaultPrevented: false, dataTransfer: data,
    preventDefault() { this.defaultPrevented = true; }, stopPropagation: vi.fn() };
  return { file, event, e: event as unknown as DragEvent };
}
beforeEach(() => { vi.clearAllMocks(); state.key = 'origin'; state.version = 7; state.page = 'today'; });
describe('row file event ownership', () => {
  it('captures row, navigation and file before lazy work and lets the window clear its veil', async () => {
    const h = drop(); let key = 'target';
    railRowFileTarget(() => key).onDrop(h.e);
    key = 'other'; state.key = 'changed'; state.version = 8;
    h.event.dataTransfer.items = [];
    await vi.dynamicImportSettled();
    expect(state.route).toHaveBeenCalledWith('target', [{ file: h.file, known: true }], { key: 'origin', version: 7, page: 'today' }, expect.any(Object));
    expect(h.event.defaultPrevented).toBe(true);
    expect(h.event.stopPropagation).not.toHaveBeenCalled();
  });
  it('consumes a project drop without a lead instead of attaching to the current chat', async () => {
    const h = drop(); railRowFileTarget(() => null).onDrop(h.e);
    await vi.dynamicImportSettled();
    expect(h.event.defaultPrevented).toBe(true); expect(state.route).not.toHaveBeenCalled();
    expect(state.notice).toHaveBeenCalledWith(expect.stringContaining('Open this project conversation'), 6000);
  });
  it('leaves a drag already claimed by a child alone', () => {
    const h = drop(); h.event.defaultPrevented = true;
    railRowFileTarget(() => 'target').onDrop(h.e);
    expect(state.route).not.toHaveBeenCalled();
  });
  it('refuses a remote-row drop without falling through to the local chat', () => {
    const h = drop();
    railRowFileTarget(() => null, 'Another Mac cannot receive this drop.').onDrop(h.e);
    expect(h.event.defaultPrevented).toBe(true); expect(state.route).not.toHaveBeenCalled();
    expect(state.notice).toHaveBeenCalledWith('Another Mac cannot receive this drop.', 6000);
  });
});
