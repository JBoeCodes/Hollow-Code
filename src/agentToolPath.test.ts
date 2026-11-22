import { describe, expect, it } from 'vitest';
import { joinedToolPath, shellPathNote, toolPathRows } from './agentToolPath';
import type { AgentToolPath } from './generated/ipc';

const path: AgentToolPath = {
  entries: [
    { dir: '/h/.cargo/bin', from: 'hollow', exists: true },
    { dir: '/h/google-cloud-sdk/bin', from: 'shell', exists: true },
    { dir: '/h/gone/bin', from: 'shell', exists: false },
    { dir: '/Volumes/share/bin', from: 'shell', exists: null },
    { dir: '/usr/bin', from: 'inherited', exists: true },
    { dir: '/sbin', from: 'system', exists: true },
  ],
  shell: { state: 'read', detail: null },
};

describe('agent tool path (A12)', () => {
  it('lists each folder in order with its source, marking missing and unanswered ones', () => {
    expect(toolPathRows(path)).toEqual([
      { dir: '/h/.cargo/bin', from: 'hollow', label: 'hollow', missing: false, unknown: false },
      { dir: '/h/google-cloud-sdk/bin', from: 'shell', label: 'shell', missing: false, unknown: false },
      { dir: '/h/gone/bin', from: 'shell', label: 'shell · missing', missing: true, unknown: false },
      // Spare 1: a check that timed out is neither there nor missing.
      { dir: '/Volumes/share/bin', from: 'shell', label: "shell · didn't answer", missing: false, unknown: true },
      { dir: '/usr/bin', from: 'inherited', label: 'inherited', missing: false, unknown: false },
      { dir: '/sbin', from: 'system', label: 'system', missing: false, unknown: false },
    ]);
  });

  it('copies the joined PATH, missing folders included', () => {
    expect(joinedToolPath(path)).toBe('/h/.cargo/bin:/h/google-cloud-sdk/bin:/h/gone/bin:/Volumes/share/bin:/usr/bin:/sbin');
    expect(joinedToolPath({ ...path, entries: [] })).toBe('');
  });

  it('says nothing once the shell PATH was read', () => {
    expect(shellPathNote({ state: 'read', detail: null })).toBeNull();
  });

  it('says the shell PATH is still being read', () => {
    expect(shellPathNote({ state: 'pending', detail: null })).toBe('Still reading your shell\'s PATH.');
  });

  it('names why the shell PATH could not be read', () => {
    expect(shellPathNote({ state: 'failed', detail: 'timed out after 5000 ms' }))
      .toBe('Hollow couldn\'t read your shell\'s PATH (timed out after 5000 ms). Agents get Hollow\'s default folders.');
    expect(shellPathNote({ state: 'failed', detail: null }))
      .toBe('Hollow couldn\'t read your shell\'s PATH. Agents get Hollow\'s default folders.');
    expect(shellPathNote({ state: 'failed', detail: '  ' }))
      .toBe('Hollow couldn\'t read your shell\'s PATH. Agents get Hollow\'s default folders.');
  });
});
