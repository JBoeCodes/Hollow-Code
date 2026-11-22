import { beforeEach, describe, expect, it, vi } from 'vitest';

// The terminal drawer's store (parity 7.2): shells per chat, output drawn a frame
// later, the mode gate, draft adoption and what closing does.
const api = vi.hoisted(() => ({
  terminalOpen: vi.fn(), terminalGate: vi.fn(), terminalWrite: vi.fn(), terminalResize: vi.fn(), terminalClose: vi.fn(), terminalAdoptChat: vi.fn(),
}));
vi.mock('../api', () => ({ api }));
vi.mock('../nativeEvents', () => ({ listen: () => Promise.resolve(() => {}) }));

import {
  DRAFT_CHAT, PENDING_MAX, activeTerminal, adoptDraftTerminals, checkTerminalGate, closeChatTerminals, closeTerminal, flushTerminalOutput, liveTerminalCount,
  openTerminal, resetTerminalsForTests, resizeTerminal, setActiveTerminal, terminalEvents, terminalRefusal, terminalsOf, typeInto,
} from './terminals';
import { closePreviewDocument, dropSubagentPreviews, openPreview, previewDocumentKey, previewDocuments, resetPreviewForTests } from './preview';

const ctx = { agent: 'claude', perm: 'auto', cwd: '/work/project' };
let n = 0;
beforeEach(() => {
  vi.clearAllMocks();
  resetTerminalsForTests();
  resetPreviewForTests();
  n = 0;
  api.terminalOpen.mockImplementation(async (_chat: string, _a: string | null, _p: string, cwd: string) => ({ id: `term-${++n}`, cwd, shell: '/bin/zsh', pid: 100 + n }));
  api.terminalGate.mockResolvedValue(null);
  api.terminalWrite.mockResolvedValue(undefined);
  api.terminalResize.mockResolvedValue(undefined);
  api.terminalClose.mockResolvedValue(undefined);
  api.terminalAdoptChat.mockResolvedValue(undefined);
});

describe('opening', () => {
  it('opens a shell for the chat in the folder given, with the chat\'s mode and agent', async () => {
    expect(await openTerminal('s-1', ctx, 30, 100)).toBeNull();
    expect(api.terminalOpen).toHaveBeenCalledWith('s-1', 'claude', 'auto', '/work/project', 30, 100);
    const t = activeTerminal('s-1')!;
    expect(t.label).toBe('Terminal 1');
    expect(t.cwd).toBe('/work/project');
    expect(t.screen.rows).toBe(30);
    expect(t.screen.cols).toBe(100);
    expect(terminalsOf('s-2')).toEqual([]);
  });

  it('numbers a chat\'s terminals and keeps the newest active', async () => {
    await openTerminal('s-1', ctx, 24, 80);
    await openTerminal('s-1', ctx, 24, 80);
    expect(terminalsOf('s-1').map(t => t.label)).toEqual(['Terminal 1', 'Terminal 2']);
    expect(activeTerminal('s-1')!.id).toBe('term-2');
    setActiveTerminal('s-1', 'term-1');
    expect(activeTerminal('s-1')!.id).toBe('term-1');
  });

  it('returns the backend\'s refusal in words, opens nothing and keeps it for the tab', async () => {
    api.terminalOpen.mockRejectedValueOnce('The terminal is off for this chat. Plan mode reads and changes nothing.');
    const err = await openTerminal('s-1', { ...ctx, perm: 'plan' }, 24, 80);
    expect(err).toMatch(/off for this chat/);
    expect(terminalsOf('s-1')).toEqual([]);
    expect(terminalRefusal('s-1')).toMatch(/Plan mode/);
    // A later open that works clears the refusal.
    expect(await openTerminal('s-1', ctx, 24, 80)).toBeNull();
    expect(terminalRefusal('s-1')).toBe('');
  });
});

describe('output and typing', () => {
  it('draws output on a frame, in order, and follows the shell\'s title', async () => {
    await openTerminal('s-1', ctx, 5, 20);
    const t = activeTerminal('s-1')!;
    terminalEvents.output('term-1', 'hello\r\n');
    terminalEvents.output('term-1', '\x1b]0;zsh: project\x07world');
    expect(t.screen.screenText()[0]).toBe(''); // not drawn yet
    flushTerminalOutput();
    expect(t.screen.screenText().slice(0, 2)).toEqual(['hello', 'world']);
    expect(t.label).toBe('zsh: project');
    expect(t.tick()).toBeGreaterThan(0);
  });

  it('keeps only the newest output when a flood outruns the screen', async () => {
    await openTerminal('s-1', ctx, 3, 10);
    terminalEvents.output('term-1', 'x'.repeat(PENDING_MAX));
    terminalEvents.output('term-1', '\r\nEND');
    flushTerminalOutput();
    expect(activeTerminal('s-1')!.screen.allText().endsWith('\nEND')).toBe(true);
  });

  it('ignores output for a terminal that is gone', () => {
    expect(() => terminalEvents.output('term-404', 'x')).not.toThrow();
  });

  it('sends typing to the shell and an answer the program asked for', async () => {
    await openTerminal('s-1', ctx, 5, 20);
    typeInto('term-1', 'ls\r');
    expect(api.terminalWrite).toHaveBeenCalledWith('term-1', 'ls\r');
    terminalEvents.output('term-1', '\x1b[6n');
    flushTerminalOutput();
    expect(api.terminalWrite).toHaveBeenCalledWith('term-1', '\x1b[1;1R');
  });

  it('marks an ended shell, says so on screen and stops sending its typing', async () => {
    await openTerminal('s-1', ctx, 5, 60);
    const t = activeTerminal('s-1')!;
    expect(liveTerminalCount('s-1')).toBe(1);
    terminalEvents.exit('term-1', 7);
    expect(t.exited()).toBe(true);
    expect(t.code()).toBe(7);
    expect(t.screen.allText()).toContain('The shell ended with code 7');
    expect(liveTerminalCount('s-1')).toBe(0);
    api.terminalWrite.mockClear();
    typeInto('term-1', 'x');
    expect(api.terminalWrite).not.toHaveBeenCalled();
  });

  it('follows the pane\'s size', async () => {
    await openTerminal('s-1', ctx, 24, 80);
    resizeTerminal('term-1', 40, 120);
    expect(activeTerminal('s-1')!.screen.cols).toBe(120);
    expect(api.terminalResize).toHaveBeenCalledWith('term-1', 40, 120);
    api.terminalResize.mockClear();
    resizeTerminal('term-1', 40, 120);
    expect(api.terminalResize).not.toHaveBeenCalled();
  });
});

describe('closing', () => {
  it('closes one terminal in the backend and moves to a neighbour', async () => {
    await openTerminal('s-1', ctx, 24, 80);
    await openTerminal('s-1', ctx, 24, 80);
    closeTerminal('s-1', 'term-2');
    expect(api.terminalClose).toHaveBeenCalledWith('term-2');
    expect(terminalsOf('s-1').map(t => t.id)).toEqual(['term-1']);
    expect(activeTerminal('s-1')!.id).toBe('term-1');
    // Output that arrives after the close is dropped, not drawn anywhere.
    terminalEvents.output('term-2', 'late');
    flushTerminalOutput();
  });

  it('closes every terminal of one chat and leaves the others', async () => {
    await openTerminal('s-1', ctx, 24, 80);
    await openTerminal('s-1', ctx, 24, 80);
    await openTerminal('s-2', ctx, 24, 80);
    closeChatTerminals('s-1');
    expect(api.terminalClose).toHaveBeenCalledTimes(2);
    expect(terminalsOf('s-1')).toEqual([]);
    expect(terminalsOf('s-2')).toHaveLength(1);
  });

  it('ends the chat\'s shells when its Terminal tab is closed by hand, not when the tab is only dropped', async () => {
    await openTerminal('s-1', ctx, 24, 80);
    openPreview({ kind: 'terminal', chat: 's-1' });
    // Another chat comes on screen: the tab goes, the shells keep running.
    dropSubagentPreviews(new Set(['s-2']));
    expect(previewDocuments().some(t => t.kind === 'terminal')).toBe(false);
    expect(api.terminalClose).not.toHaveBeenCalled();
    expect(terminalsOf('s-1')).toHaveLength(1);
    // The tab closed by hand: they end.
    openPreview({ kind: 'terminal', chat: 's-1' });
    closePreviewDocument(previewDocumentKey({ kind: 'terminal', chat: 's-1' }));
    expect(api.terminalClose).toHaveBeenCalledWith('term-1');
    expect(terminalsOf('s-1')).toEqual([]);
  });
});

describe('the mode gate', () => {
  it('leaves terminals alone while the mode allows a shell', async () => {
    await openTerminal('s-1', ctx, 24, 80);
    expect(await checkTerminalGate('s-1', 'claude', 'auto')).toBe('');
    expect(terminalsOf('s-1')).toHaveLength(1);
    expect(api.terminalGate).toHaveBeenCalledWith('s-1', 'claude', 'auto');
  });

  it('forgets the chat\'s terminals and keeps the reason when the mode forbids one', async () => {
    await openTerminal('s-1', ctx, 24, 80);
    await openTerminal('s-2', ctx, 24, 80);
    api.terminalGate.mockResolvedValueOnce('The terminal is off for this chat. Plan mode reads and changes nothing.');
    expect(await checkTerminalGate('s-1', 'claude', 'plan')).toMatch(/Plan mode/);
    expect(terminalsOf('s-1')).toEqual([]);
    expect(terminalRefusal('s-1')).toMatch(/off for this chat/);
    expect(terminalsOf('s-2')).toHaveLength(1);
    // The backend already ended them: the window does not ask it to close again.
    expect(api.terminalClose).not.toHaveBeenCalled();
    await checkTerminalGate('s-1', 'claude', 'auto');
    expect(terminalRefusal('s-1')).toBe('');
  });

  it('treats a failed gate call as a refusal, never as permission', async () => {
    await openTerminal('s-1', ctx, 24, 80);
    api.terminalGate.mockRejectedValueOnce('backend unavailable');
    expect(await checkTerminalGate('s-1', 'claude', 'auto')).toBe('backend unavailable');
    expect(terminalsOf('s-1')).toEqual([]);
    // It cannot be known whether the backend ended them, so each is closed.
    expect(api.terminalClose).toHaveBeenCalledWith('term-1');
  });
});

describe('a chat without a key yet', () => {
  it('hands its terminals to the key its first send creates', async () => {
    await openTerminal(DRAFT_CHAT, ctx, 24, 80);
    adoptDraftTerminals('s-9');
    expect(terminalsOf(DRAFT_CHAT)).toEqual([]);
    expect(terminalsOf('s-9').map(t => t.id)).toEqual(['term-1']);
    expect(activeTerminal('s-9')!.id).toBe('term-1');
    expect(api.terminalAdoptChat).toHaveBeenCalledWith(DRAFT_CHAT, 's-9');
    // Nothing to hand over: no call.
    api.terminalAdoptChat.mockClear();
    adoptDraftTerminals('s-10');
    expect(api.terminalAdoptChat).not.toHaveBeenCalled();
  });
});
