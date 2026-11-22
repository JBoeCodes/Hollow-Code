import { describe, expect, it } from 'vitest';
import studioHomeSource from './components/StudioHome.tsx?raw';
import paletteSource from './components/CommandPalette.tsx?raw';
import { ACTION_BY_GO, ACTION_BY_PALETTE } from './actionDefs';
import { ASK_MAX_CHARS, ASK_SCOPES, askRowIndex, defaultAskScope, looksLikeSentence } from './askScopes';
import { pageKeys } from './keymapSections';

describe('Ask scopes say what context they use (consolidation D)', () => {
  it('offers the COO and a quick chat: every question is a chat (One Ask)', () => {
    expect(ASK_SCOPES.map(s => s.id)).toEqual(['main', 'quick']);
  });
  it('every scope states its context and its action in words', () => {
    for (const s of ASK_SCOPES) { expect(s.context.length).toBeGreaterThan(20); expect(s.action.length).toBeGreaterThan(3); }
  });
  it('each scope has its own placeholder, saying what Enter does there (P1-11)', () => {
    const placeholders = ASK_SCOPES.map(s => s.placeholder);
    expect(new Set(placeholders).size).toBe(ASK_SCOPES.length);
    for (const p of placeholders) expect(p.length).toBeGreaterThan(10);
  });
  it('labels never start with "Do it", which only the COO’s suggestions use (P1-03, P1-11)', () => {
    for (const s of ASK_SCOPES) expect(s.label.startsWith('Do it')).toBe(false);
  });
  it('Across all my work says it asks, and that ⌥-Enter only places it (D2)', () => {
    const main = ASK_SCOPES.find(s => s.id === 'main')!;
    expect(main.action).toBe('Ask the COO');
    expect(main.placeAction).toBe('Put it in the box');
    expect(main.context).toContain('⌥-Enter');
  });
  it('Work’s Ask field shows the scope’s placeholder and its context as a muted line, two rows tall', () => {
    const src = studioHomeSource;
    expect(src).toContain('placeholder={chosen().placeholder}');
    expect(src).toContain('<p id="studio-ask-hint" class="studio-ask-hint">');
    expect(src).toContain("scope() === 'quick' && askProject()");
    expect(src).toContain(': chosen().context}</p>');
    expect(src).toContain('aria-describedby="studio-ask-hint"');
    expect(src).toMatch(/<textarea[^>]*rows=\{2\}/);
  });
  it('starts on a quick chat, and remembers a choice', () => {
    expect(defaultAskScope(null)).toBe('quick');
    expect(defaultAskScope('main')).toBe('main');
  });
});

describe('the command bar’s Ask row and Ask anything… (overnight 10-11 A2; One Ask)', () => {
  it('offers itself for sentences and questions, not for a word or two', () => {
    expect(looksLikeSentence('move every Atlas task due this week to Monday')).toBe(true);
    expect(looksLikeSentence("what's overdue?")).toBe(true);
    expect(looksLikeSentence('archive the empty amex workspaces')).toBe(true);
    expect(looksLikeSentence('settings')).toBe(false);
    expect(looksLikeSentence('open map')).toBe(false);
    expect(looksLikeSentence('   ')).toBe(false);
    expect(looksLikeSentence('- - ·')).toBe(false);
    expect(looksLikeSentence('a '.repeat(ASK_MAX_CHARS))).toBe(false);
  });

  it('leads the palette unless a strong label match keeps Enter', () => {
    expect(askRowIndex(undefined)).toBe(0);
    expect(askRowIndex(500)).toBe(0);
    expect(askRowIndex(700)).toBe(1);
    expect(askRowIndex(1000)).toBe(1);
  });

  it('is one row, asked the way Work’s Ask field asks', () => {
    expect(paletteSource).toContain("id: 'ask:sentence', label: `Ask: “");
    expect(paletteSource).toContain('void askHere(q)');
    // Its hint names the place, a quick chat's project included (askPlaceLabel).
    expect(paletteSource).toContain('hint: askPlaceLabel()');
    expect(paletteSource).not.toContain("'ask:vault'");
    expect(paletteSource).not.toMatch(/openAskHollow|openAskVault/);
  });

  it('Ask anything… and g v open Work’s Ask field, keeping Ask your vault’s ids', () => {
    const def = ACTION_BY_PALETTE['act:ask-vault'];
    expect(def?.id).toBe('ask-vault');
    expect(ACTION_BY_GO['ask-vault']).toBe(def);
    expect(def.route).toBe('ask.scoped');
    expect(ACTION_BY_PALETTE['act:ask-hollow']).toBeUndefined();
    // What was typed comes along when it reads as a sentence (paletteOwn).
    expect(def.paletteOwn).toMatch(/sentence/);
    expect(paletteSource).toContain("'act:ask-vault': { run: () => { const q = query().trim(); openHomeAsk(looksLikeSentence(q) ? q : ''); } }");
  });

  it('is on the keyboard card', () => {
    const rows = pageKeys('⌘').filter(k => k.section === 'Ask');
    expect(rows.map(r => r.keys)).toEqual([['⌘K']]);
  });
});
