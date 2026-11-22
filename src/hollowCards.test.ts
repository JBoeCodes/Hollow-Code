import { describe, expect, it } from 'vitest';
import type { AgentDescriptor } from './types';
import { hollowCardsNotice } from './hollowCards';
import chatBodySource from './components/ChatBody.tsx?raw';

// What `agent::capabilities` sends: only Claude and Codex take Hollow's
// per-run MCP config (`owned_mcp`).
const caps = (ownedMcp: boolean): AgentDescriptor['capabilities'] => ({
  modelSelection: true, sessionResume: true, toolRestrictions: false, ownedMcp, structuredOutput: true, effortLevels: [],
});
const AGENTS = [
  { id: 'claude', label: 'Claude', ownedMcp: true },
  { id: 'codex', label: 'Codex', ownedMcp: true },
  { id: 'antigravity', label: 'Antigravity', ownedMcp: false },
  { id: 'grok', label: 'Grok', ownedMcp: false },
];

describe('Hollow cards notice (merged plan slice 2 item 4)', () => {
  it('shows for Grok and Antigravity, and not for Claude or Codex', () => {
    for (const agent of AGENTS) {
      const notice = hollowCardsNotice(agent.id, agent.label, caps(agent.ownedMcp));
      if (agent.ownedMcp) {
        expect(notice, agent.id).toBeNull();
      } else {
        expect(notice, agent.id).toContain('No Hollow cards or app tools in this chat');
        expect(notice, agent.id).toContain(`${agent.label} can't take Hollow's per-run tool setup`);
        expect(notice, agent.id).toContain('a question can come only as a card at the end of a reply');
        // L4 (X13): it names the alternative, where to switch.
        expect(notice, agent.id).toContain('switch this chat to Claude or Codex in the model menu');
      }
    }
  });

  it('tells Grok chats that their own ~/.grok MCP servers still load', () => {
    expect(hollowCardsNotice('grok', 'Grok', caps(false))).toMatch(/MCP servers set up in ~\/\.grok still load\.$/);
    expect(hollowCardsNotice('antigravity', 'Antigravity', caps(false))).not.toContain('~/.grok');
  });

  it('follows the backend capability, with the known list only before descriptors load', () => {
    // A provider that gains a per-run MCP config loses the notice.
    expect(hollowCardsNotice('grok', 'Grok', caps(true))).toBeNull();
    expect(hollowCardsNotice('grok', 'Grok', undefined)).not.toBeNull();
    expect(hollowCardsNotice('antigravity', 'Antigravity', undefined)).not.toBeNull();
    expect(hollowCardsNotice('claude', 'Claude', undefined)).toBeNull();
    expect(hollowCardsNotice('codex', 'Codex', undefined)).toBeNull();
  });

  it('is rendered above the chat composer from the current agent', () => {
    expect(chatBodySource).toMatch(/hollowCardsNotice\(chatAgent\(\),[^\n]*currentAgentCapabilities\(\)\)/);
    expect(chatBodySource).toContain('data-notice="no-hollow-cards"');
  });
});
