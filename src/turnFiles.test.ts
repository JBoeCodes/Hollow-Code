import { describe, expect, it } from 'vitest';
import { attachedFilesBlock } from './composerFiles';
import { savedTurnFiles } from './turnFiles';
import { turnsFromConversation } from './chatHydrate';

const at = '/Users/j/vault/workspaces/a/.hollow-attachments';

describe('savedTurnFiles (B1a, ENG-104)', () => {
  it('rebuilds the chips from the block the composer wrote and drops it from the words', () => {
    const block = attachedFilesBlock([{ name: 'report.pdf', path: `${at}/report-1.pdf` }, { name: 'a/b<c>.txt', path: `${at}/a-b-c-1.txt` }]);
    expect(savedTurnFiles(`Summarize these\n\n${block}`)).toEqual({
      text: 'Summarize these',
      files: [{ name: 'report.pdf', path: `${at}/report-1.pdf` }, { name: 'a_b_c_.txt', path: `${at}/a-b-c-1.txt` }],
    });
  });

  it('reads a message that is only files', () => {
    const block = attachedFilesBlock([{ name: 'notes at home.md', path: `${at}/notes-1.md` }]);
    expect(savedTurnFiles(block)).toEqual({ text: '', files: [{ name: 'notes at home.md', path: `${at}/notes-1.md` }] });
  });

  it('leaves text without a block, or a quoted or mid-text one, as it is', () => {
    const block = attachedFilesBlock([{ name: 'a.pdf', path: `${at}/a-1.pdf` }]);
    for (const text of ['Plain words', `what does <attached-files count="1"> mean?`, `${block}\n\nand then more`]) {
      expect(savedTurnFiles(text)).toEqual({ text });
    }
  });

  it('leaves a block whose count does not match its lines', () => {
    const text = attachedFilesBlock([{ name: 'a.pdf', path: `${at}/a-1.pdf` }]).replace('count="1"', 'count="2"');
    expect(savedTurnFiles(text)).toEqual({ text });
  });

  it('makes no chip for a path outside an attachments folder or one that climbs out', () => {
    const block = attachedFilesBlock([
      { name: 'passwd', path: '/etc/passwd' },
      { name: 'up.txt', path: `${at}/../../secret.txt` },
      { name: 'ok.txt', path: `${at}/ok-1.txt` },
    ]);
    expect(savedTurnFiles(`Look\n\n${block}`)).toEqual({ text: 'Look', files: [{ name: 'ok.txt', path: `${at}/ok-1.txt` }] });
    const only = attachedFilesBlock([{ name: 'passwd', path: '/etc/passwd' }]);
    expect(savedTurnFiles(`Look\n\n${only}`)).toEqual({ text: 'Look' });
  });

  it('gives a reloaded user turn its chips, and leaves a reply that quotes a block alone', () => {
    const block = attachedFilesBlock([{ name: 'report.pdf', path: `${at}/report-1.pdf` }]);
    const base = { tools: [], agent: 'claude', runId: 'r1', startedAt: '2026-11-15T10:00:00Z', status: 'success', costUsd: null, error: null, receipt: null, transcriptIncomplete: false };
    const turns = turnsFromConversation({ turns: [
      { ...base, id: 'u', role: 'user', text: `Summarize\n\n${block}` },
      { ...base, id: 'a', role: 'assistant', text: `You sent\n\n${block}` },
    ] });
    expect(turns.map(t => [t.text, t.files])).toEqual([
      ['Summarize', [{ name: 'report.pdf', path: `${at}/report-1.pdf` }]],
      [`You sent\n\n${block}`, undefined],
    ]);
  });

  it("prefers the store's files for a reloaded user turn", () => {
    const base = { tools: [], agent: 'claude', runId: 'r1', startedAt: '2026-11-15T10:00:00Z', status: 'success', costUsd: null, error: null, receipt: null, transcriptIncomplete: false };
    const files = [{ name: 'report.pdf', path: `${at}/report-1-2-3.pdf`, size: 9 }];
    const [user, reply] = turnsFromConversation({ turns: [
      { ...base, id: 'u', role: 'user', text: 'Summarize', files },
      { ...base, id: 'a', role: 'assistant', text: 'Done', files },
    ] });
    expect([user.text, user.files]).toEqual(['Summarize', files]);
    expect(reply.files).toBeUndefined();
  });
});
