import { describe, expect, it } from 'vitest';
import { conversationAbout } from './conversationAbout';

// Plan L3 (2026-10-14; review V05, matrix X10): ownership and resume, said
// the same way for every kind of conversation.
const stint = (agent: string) => ({ agent, harnessSessionId: 'x', startedAt: '', endedAt: null, reason: null });
const text = (a: ReturnType<typeof conversationAbout>) => a.rows.map(r => `${r.label}: ${r.detail}`).join('\n');

describe('conversationAbout', () => {
  it('a workspace conversation belongs to its workspace and resumes its own session', () => {
    const a = conversationAbout({ kind: 'workspace', workspaceTitle: 'Q3 review', agent: 'claude', nativeSession: true, turns: 4, session: { adopted: false, resumable: 'native', stints: [stint('claude')], agent: 'claude' } });
    expect(a.kind).toBe('Workspace conversation');
    expect(text(a)).toContain('Belongs to: Q3 review');
    expect(text(a)).toContain('Kept in: Hollow keeps the whole conversation; Q3 review keeps a summary in its sessions folder.');
    expect(text(a)).toMatch(/Continues: Claude resumes its own session/);
  });

  it('quick chats and the COO say what they are and where they live', () => {
    const q = conversationAbout({ kind: 'quick', agent: 'codex', nativeSession: false, turns: 0 });
    expect(q.kind).toBe('Quick chat');
    expect(text(q)).toMatch(/no project, brief or sources. Move it to a project/);
    expect(text(q)).toContain('Continues: When you send, Codex starts a new session.');
    const m = conversationAbout({ kind: 'main', agent: 'claude', nativeSession: false, turns: 2 });
    expect(m.kind).toBe('COO conversation');
    expect(text(m)).toMatch(/one thread a day.*Conversation history/);
  });

  it('an imported conversation says it came from another app and how it continues', () => {
    const fresh = conversationAbout({ kind: 'workspace', workspaceTitle: 'W', agent: 'claude', nativeSession: false, turns: 6, session: { adopted: true, resumable: 'briefOnly', stints: [], agent: 'claude' } });
    expect(text(fresh)).toMatch(/Came from: Imported from another app. Your first message here writes its earlier turns in/);
    const continued = conversationAbout({ kind: 'workspace', workspaceTitle: 'W', agent: 'claude', nativeSession: true, turns: 8, session: { adopted: true, resumable: 'native', stints: [stint('claude')], agent: 'claude' } });
    expect(text(continued)).toContain('Came from: Imported from another app, then continued here.');
  });

  it('names agent changes and sessions it cannot resume', () => {
    const a = conversationAbout({ kind: 'workspace', workspaceTitle: 'W', agent: 'codex', nativeSession: false, turns: 3, session: { adopted: false, resumable: 'briefOnly', stints: [stint('claude'), stint('claude'), stint('codex')], agent: 'codex' } });
    expect(text(a)).toContain('Agents: Claude, then Codex');
    expect(text(a)).toMatch(/Codex can't resume its own session here/);
    const gone = conversationAbout({ kind: 'vault', agent: 'claude', nativeSession: false, turns: 3, session: { adopted: false, resumable: 'none', stints: [], agent: 'claude' } });
    expect(gone.kind).toBe('Vault conversation');
    expect(text(gone)).toMatch(/transcript is gone/);
  });
});
