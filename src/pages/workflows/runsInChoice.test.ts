import { describe, expect, it } from 'vitest';
import { NEW_CHAT, inChatForSave, newAutomationChat, runsInHint, runsInOptions, setNewAutomationChat } from './runsInChoice';

// Run inside this chat (overhaul F1): the automation editor's Runs in choice.
describe('Runs in', () => {
  it('offers a new chat each run, and this chat when one is bound or requested', () => {
    expect(runsInOptions(null)).toEqual([{ value: NEW_CHAT, label: 'New chat each run' }]);
    expect(runsInOptions({ key: 's-20261112-0000aaaa', title: 'Overnight lead' })).toEqual([
      { value: NEW_CHAT, label: 'New chat each run' },
      { value: 's-20261112-0000aaaa', label: 'This chat: Overnight lead' },
    ]);
    expect(runsInOptions({ key: 's-20261112-0000aaaa', title: ' ' })[1].label).toBe('This chat: Untitled conversation');
  });

  it('says what each choice means, including the card and archive rules', () => {
    expect(runsInHint(NEW_CHAT)).toMatch(/new chat/);
    const bound = runsInHint('s-20261112-0000aaaa');
    expect(bound).toMatch(/new turn in this chat/);
    expect(bound).toMatch(/open question/);
    expect(bound).toMatch(/archived or deleted/);
    expect(bound).toMatch(/allowed and disallowed tools must be empty/);
    expect(bound).not.toMatch(/[\u2013\u2014]/);
  });

  it('a save sends the choice only when it changed; empty unbinds', () => {
    expect(inChatForSave('s-1', 's-1')).toBeUndefined();
    expect(inChatForSave('', 's-1')).toBe('');
    expect(inChatForSave('s-2', '')).toBe('s-2');
  });

  it('Schedule in this chat… hands the chat to the automations page once', () => {
    setNewAutomationChat({ key: 's-20261112-0000bbbb', title: 'Lead' });
    expect(newAutomationChat()).toEqual({ key: 's-20261112-0000bbbb', title: 'Lead' });
    setNewAutomationChat(null);
    expect(newAutomationChat()).toBeNull();
  });

  it('the new-automation form and the editor both offer it, and the chat menu opens it', async () => {
    const { default: page } = await import('../Workflows.tsx?raw');
    expect(page.match(/<RunsIn /g)?.length).toBe(2);
    expect(page).toMatch(/newAutomationChat\(\)/);
    const { default: actions } = await import('../../conversationActions.ts?raw');
    expect(actions).toMatch(/setNewAutomationChat\(\{ key, title \}\)/);
  });
});
