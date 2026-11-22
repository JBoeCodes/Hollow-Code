import { describe, expect, it } from 'vitest';
import { buildCatalog, catalogCounts, entryLint, filterCatalog, lintBadge, lintFindingsInOrder, precedenceNote, skillAvailability, skillOwner, skillRuntime, useSkillPrompt } from './skillCatalog';
import type { Skill } from './types';

const skill = (over: Partial<Skill>): Skill => ({ name: 'x', source: 'shared', sourceKind: 'user', plugin: null, path: `/p/${over.name ?? 'x'}/${over.source ?? 'shared'}`, description: '', version: null, writable: false, ...over });

const list: Skill[] = [
  skill({ name: 'humanizer', source: 'provider:claude', description: 'Remove signs of AI writing.' }),
  skill({ name: 'humanizer', source: 'shared', writable: true, description: 'Remove signs of AI writing.' }),
  skill({ name: 'pdf', source: 'plugin:claude/anthropic/document-skills@1.2', sourceKind: 'plugin', plugin: 'document-skills', description: 'Work with PDFs.' }),
  skill({ name: 'review', source: 'provider:codex-system' }),
];

describe('skills catalog (audit H13)', () => {
  it('lists each name once with copies in backend precedence order', () => {
    const c = buildCatalog(list);
    expect(c.map(e => e.name)).toEqual(['humanizer', 'pdf', 'review']);
    expect(c[0].copies.map(s => s.source)).toEqual(['shared', 'provider:claude']);
  });
  it('explains ownership and who can use each copy', () => {
    expect(skillOwner(list[1]).badge).toBe('Shared');
    expect(skillOwner(list[1]).usedBy).toMatch(/No agent loads it by itself; a chat lists it to the agent when a request fits, and its Use in button asks the agent to read it\. Editable here/);
    expect(skillOwner(list[0]).usedBy).toMatch(/Installed for Claude\. Claude’s own runtime may load it by itself \(Hollow doesn’t verify that per run\).*Read-only/);
    expect(skillOwner(list[3]).usedBy).toMatch(/Bundled with Codex/);
    expect(skillOwner(list[2]).usedBy).toBe('From the document-skills plugin for Claude (anthropic, 1.2). Read-only here.');
  });
  it('states duplicate precedence without hiding copies', () => {
    const [h, p] = buildCatalog(list);
    expect(precedenceNote(h)).toMatch(/2 copies of “humanizer”.*Shared → Claude.*Nothing is hidden/);
    expect(precedenceNote(p)).toBeNull();
  });
  it('filters by ownership and search terms and counts entries per bucket', () => {
    const c = buildCatalog(list);
    expect(filterCatalog(c, '', 'plugin').map(e => e.name)).toEqual(['pdf']);
    expect(filterCatalog(c, 'claude', 'all').map(e => e.name)).toEqual(['humanizer']);
    expect(filterCatalog(c, 'ai writing', 'shared').map(e => e.name)).toEqual(['humanizer']);
    expect(catalogCounts(c)).toEqual({ all: 3, shared: 1, provider: 2, plugin: 1, marketplace: 0 });
  });
});

describe('skill availability for the selected agent (next-level P18)', () => {
  it('says which copy an agent loads by itself and which it only follows on request', () => {
    const [h, p, r] = buildCatalog(list);
    expect(skillRuntime(list[0])).toBe('claude');
    expect(skillRuntime(list[2])).toBe('claude');
    expect(skillRuntime(list[3])).toBe('codex');
    expect(skillRuntime(list[1])).toBeNull();
    const claude = skillAvailability(h, 'claude', 'Claude');
    expect(claude).toMatchObject({ auto: true, copy: { source: 'provider:claude' } });
    expect(claude.text).toMatch(/Claude’s own runtime may load the Claude copy by itself; Hollow doesn’t verify that per run.*other copy is read only if you choose it/);
    const codex = skillAvailability(h, 'codex', 'Codex');
    expect(codex).toMatchObject({ auto: false, copy: { source: 'shared' } });
    expect(codex.text).toMatch(/The Use in button asks it to read the Shared copy/);
    // A shared copy is in the skill index's roots (wave 4 · V2).
    expect(codex.text).toMatch(/A chat also lists it to the agent when a request fits\./);
    expect(skillAvailability(r, 'grok', 'Grok').text).not.toMatch(/A chat also lists it/);
    expect(skillAvailability(p, 'claude', 'Claude').auto).toBe(true);
    expect(skillAvailability(r, 'codex', 'Codex').auto).toBe(true);
    expect(skillAvailability(r, 'grok', 'Grok').auto).toBe(false);
  });
});

describe('truthful skill delivery (agent effectiveness F9, wave 4 · V1)', () => {
  const codexPlugin = skill({ name: 'deck', source: 'plugin:codex/openai/slides@2.0', sourceKind: 'plugin', plugin: 'slides' });
  it('Codex plugins are off in Hollow runs, so Codex never loads their skills there', () => {
    expect(skillRuntime(codexPlugin)).toBeNull();
    expect(skillOwner(codexPlugin).usedBy).toMatch(/Plugins are off in Hollow’s Codex runs, so Codex doesn’t load it there/);
    const [deck] = buildCatalog([codexPlugin]);
    expect(skillAvailability(deck, 'codex', 'Codex').auto).toBe(false);
  });
  it('never claims an agent loads a skill: only that a runtime may, and that Use in workspace asks', () => {
    const entries = buildCatalog([...list, codexPlugin]);
    const copy = [
      ...[...list, codexPlugin].map(s => skillOwner(s).usedBy),
      ...entries.map(e => precedenceNote(e) ?? ''),
      ...entries.flatMap(e => ['claude', 'codex', 'antigravity', 'grok'].map(a => skillAvailability(e, a, a).text)),
    ];
    for (const text of copy) {
      expect(text).not.toMatch(/every agent loads|work(s)? with any agent|(?<!may |No agent )loads (it|the|this copy) by itself|points it at/);
    }
  });
  it('Use in workspace asks the agent to read the SKILL.md, through Hollow’s app tools or its own file tools', () => {
    const prompt = useSkillPrompt({ name: 'humanizer', path: '/Users/me/.agents/skills/humanizer/SKILL.md' });
    expect(prompt).toContain('Use the skill "humanizer" for this task.');
    expect(prompt).toContain('run_action skills.read_skill with {"path": "/Users/me/.agents/skills/humanizer/SKILL.md"}');
    expect(prompt).toContain('open "/Users/me/.agents/skills/humanizer/SKILL.md" with your own file tools');
    expect(prompt).toContain('ask me to paste it');
    expect(prompt.endsWith('for this task:\n\n')).toBe(true);
  });
});

describe('skill lint badge (NV1)', () => {
  const lint = { score: 85, grade: 'fair' as const, findings: [
    { rule: 'no_trigger', area: 'description' as const, deduction: 15, message: 'No trigger wording.', suggestion: 'Add a Use when sentence.' },
    { rule: 'hidden_text', area: 'safety' as const, deduction: 20, message: 'Line 6 holds hidden characters.', suggestion: null },
    { rule: 'scripts_prerequisites', area: 'size' as const, deduction: 5, message: 'Scripts with no prerequisites.', suggestion: null },
  ] };
  it('finds the lint on the shared copy only', () => {
    const [h] = buildCatalog([
      skill({ name: 'humanizer', source: 'provider:claude' }),
      skill({ name: 'humanizer', source: 'shared', writable: true, lint }),
    ]);
    expect(entryLint(h)).toBe(lint);
    expect(entryLint(buildCatalog([skill({ name: 'pdf', source: 'provider:codex' })])[0])).toBeNull();
  });
  it('labels the score and says how many findings, a clean skill included', () => {
    expect(lintBadge(lint)).toEqual({ label: 'Lint 85', grade: 'fair', title: 'Skill lint: fair, 3 findings. Open the skill for the fixes.' });
    expect(lintBadge({ score: 100, grade: 'good', findings: [] }).title).toBe('Skill lint: good, no findings. Open the skill for the fixes.');
    expect(lintBadge({ score: 95, grade: 'good', findings: [lint.findings[2]] }).title).toContain('1 finding.');
  });
  it('lists findings worst first', () => {
    expect(lintFindingsInOrder(lint).map(f => f.rule)).toEqual(['hidden_text', 'no_trigger', 'scripts_prerequisites']);
  });
});
