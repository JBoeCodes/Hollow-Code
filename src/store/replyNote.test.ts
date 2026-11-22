import { beforeEach, describe, expect, it, vi } from 'vitest';

const flash = vi.hoisted(() => ({ footer: vi.fn(), undo: vi.fn() }));
vi.mock('../api', () => ({ api: new Proxy({}, { get: () => vi.fn(async () => null) }) }));
vi.mock('./chatRuntime', () => ({ vaultRoot: vi.fn(async () => null) }));
vi.mock('./preview', () => ({ openPreview: vi.fn() }));
vi.mock('./runtime', () => ({ openVaultNote: vi.fn() }));
vi.mock('./snackbars', () => ({ flashFooter: flash.footer, flashWithUndo: flash.undo }));

import { saveConversationAsNote, saveReplyAsNote } from './replyNote';

// Saving with nothing to save (overnight 11-07, D1; the Grok review of #1195):
// the Conversation tools show Save once a chat has more than two turns, and a
// chat of only notices has no words, so the button must say so, not do nothing.
describe('saving a note with nothing to save', () => {
  beforeEach(() => { flash.footer.mockClear(); flash.undo.mockClear(); });

  it('says so for a conversation with no words, and writes nothing', async () => {
    expect(await saveConversationAsNote('  \n ', 'A title', null)).toBeNull();
    expect(flash.footer).toHaveBeenCalledWith('Nothing to save in this conversation yet', 2000);
    expect(flash.undo).not.toHaveBeenCalled();
  });

  it('stays quiet for an empty reply: its button is only offered on a reply with words', async () => {
    expect(await saveReplyAsNote('   ', null, null)).toBeNull();
    expect(flash.footer).not.toHaveBeenCalled();
  });
});
