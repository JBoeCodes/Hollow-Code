import { describe, expect, it } from 'vitest';
import paletteSource from './CommandPalette.tsx?raw';

describe('CommandPalette newConversationItems', () => {
  it('labels the startQuickChat() row "Quick chat", not "The COO"', () => {
    const start = paletteSource.indexOf('function newConversationItems()');
    const end = paletteSource.indexOf('function mergeWorkspaceItems()');
    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(start);
    const fn = paletteSource.slice(start, end);
    expect(fn).toContain("id: 'new-conversation:quick', label: 'Quick chat'");
    expect(fn).toContain('run: () => startQuickChat()');
    expect(fn).not.toContain("label: 'The COO'");
  });
});
