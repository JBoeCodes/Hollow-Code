import { describe, expect, it } from 'vitest';
import { serializeAgentSettingsWrite } from './agentSettingsQueue';

type Settings = { limit: string; build: string };

/** A settings file with a slow read, saved the way AgentDefaults saves: read, merge one piece, write all. */
function fakeFile() {
  let file: Settings = { limit: 'a', build: 'a' };
  const read = async () => { const copy = { ...file }; await new Promise(r => setTimeout(r, 20)); return copy; };
  const write = async (next: Settings) => { file = next; };
  const save = (piece: keyof Settings, value: string) => async () => {
    const live = await read();
    await write({ ...live, [piece]: value });
  };
  return { save, now: () => file };
}

describe('agent settings saves from several pieces', () => {
  it('without the queue, two saves at once lose one change (the bug)', async () => {
    const f = fakeFile();
    await Promise.all([f.save('limit', 'b')(), f.save('build', 'b')()]);
    expect(f.now()).not.toEqual({ limit: 'b', build: 'b' });
  });

  it('through the queue, both keep their changes', async () => {
    const f = fakeFile();
    await Promise.all([
      serializeAgentSettingsWrite(f.save('limit', 'b')),
      serializeAgentSettingsWrite(f.save('build', 'b')),
    ]);
    expect(f.now()).toEqual({ limit: 'b', build: 'b' });
  });

  it('a failed save does not block the next one', async () => {
    const f = fakeFile();
    const bad = serializeAgentSettingsWrite(async () => { throw new Error('disk full'); });
    const good = serializeAgentSettingsWrite(f.save('limit', 'c'));
    await expect(bad).rejects.toThrow('disk full');
    await good;
    expect(f.now().limit).toBe('c');
  });
});
