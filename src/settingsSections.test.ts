import { describe, expect, it } from 'vitest';
import { SAVE_WORDS, SETTINGS_KEYWORDS, SETTINGS_ALIAS_CONTROLS, SETTINGS_SECTION_ALIASES, SETTINGS_SECTION_BY_ID, SETTINGS_SECTION_GROUPS, SETTINGS_SECTION_IDS, resolveSettingsSection, resolveSettingsControl, searchSettings, settingsPartHeading, settingsSectionForLabel, pickerControl } from './settingsSections';
import settingsPageSource from './pages/Settings.tsx?raw';
import selectSource from './components/Select.tsx?raw';
import { SETTINGS_INDEX_BY_ID, SETTINGS_SECTIONS } from './settingsIndex';

// Source text with its comments removed: block comments (JSX ones in braces
// too) and line comments, but not the double slash inside a URL.
function withoutComments(source: string): string {
  return source.replace(/\{\/\*[\s\S]*?\*\/\}/g, '').replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:"'`\\])\/\/[^\n]*/g, '$1');
}

describe('Obsidian workspace settings', () => {
  it('lists eighteen sections in one list, in the decided order (settings redesign 3.2, PR 3)', () => {
    expect(SETTINGS_SECTION_GROUPS).toHaveLength(1);
    expect(SETTINGS_SECTION_IDS).toEqual(['general', 'chat', 'models', 'accounts', 'personalization', 'connectors', 'sources', 'background', 'devices', 'usage', 'shortcuts', 'diffs', 'browser', 'worktrees', 'archived', 'updates', 'about', 'advanced']);
    expect(SETTINGS_SECTION_GROUPS[0].items.map(section => section.label)).toEqual(['General', 'Chat', 'Models', 'Accounts', 'Personalization', 'Connectors', 'Sources', 'Background work', 'Devices', 'Usage', 'Keyboard shortcuts', 'Diffs', 'Browser', 'Worktrees', 'Archived', 'Updates', 'About', 'Advanced']);
  });

  // Sources' Context (CX17) is a part too: `settings:layers` opens it.
  it('keeps the panels that waited for a later PR whole, as parts (PR 3)', () => {
    const parts = Object.fromEntries(SETTINGS_SECTION_GROUPS[0].items.map(section => [section.id, (section.parts ?? []).map(part => part.label)]));
    expect(parts).toEqual({
      general: [], chat: [], models: ['Self-hosted models'], accounts: [], personalization: [],
      connectors: [], sources: ['Hollow folder', 'Context'], background: [], devices: [], usage: [], shortcuts: [], diffs: [], browser: [], worktrees: [], archived: [], updates: [],
      about: [], advanced: ['Search & indexes'],
    });
    // An old deep link to it lands on Connectors.
    expect(resolveSettingsSection('imports')).toBe('connectors');
  });

  it('turns the seven sections before PR 3 into aliases of their main new home', () => {
    expect(Object.fromEntries(['interface', 'defaults', 'operator', 'automation', 'system'].map(id => [id, resolveSettingsSection(id)])))
      .toEqual({ interface: 'general', defaults: 'models', operator: 'personalization', automation: 'background', system: 'advanced' });
    for (const id of ['connectors', 'usage', 'chat', 'accounts', 'shortcuts']) expect(SETTINGS_SECTION_IDS, id).toContain(id);
  });

  it('lets a label decide first: a control that moved opens its new section (plan section 6)', () => {
    const lands = (id: string, label: string) => settingsSectionForLabel(resolveSettingsSection(id), label);
    expect(lands('defaults', 'Agent updates')).toBe('accounts');
    expect(lands('automation', 'Chats running at once')).toBe('chat');
    expect(lands('automation', 'Run at login')).toBe('general');
    expect(lands('usage', 'COO daily hard stop')).toBe('usage');
    expect(lands('defaults', 'Extraction model')).toBe('advanced');
    expect(lands('connectors', 'Task tracker')).toBe('sources');
    expect(lands('system', 'Hollow folder')).toBe('sources');
    // S5b (11-12): the agent form's pieces left Models.
    expect(lands('defaults', 'Remote runner')).toBe('devices');
    for (const label of ['Runs on', 'Builds and tests', 'Overnight and worker chats', 'Runner GitHub access']) expect(lands('models', label), label).toBe('devices');
    for (const label of ['No-output watchdog', 'Run deadline', 'App builds', 'Background jobs']) expect(lands('models', label), label).toBe('advanced');
    expect(lands('selfhosted', 'Background jobs')).toBe('advanced');
    expect(lands('defaults', 'Models the CFO may use')).toBe('models');
    expect(lands('interface', 'Use keyboard shortcuts')).toBe('shortcuts');
    expect(lands('connectors', 'Pair a device')).toBe('devices');
    expect(lands('defaults', 'No such row')).toBe('models');
  });

  it('puts what agents know about you under Personalization, and the indexes under Advanced, with no legacy labels', () => {
    expect(SETTINGS_SECTION_BY_ID.personalization.label).toBe('Personalization');
    expect(SETTINGS_SECTION_BY_ID.personalization.parts ?? []).toEqual([]);
    expect(SETTINGS_SECTION_BY_ID.advanced.parts?.map(part => part.id)).toContain('indexes');
    expect(SETTINGS_SECTION_GROUPS.flatMap(group => group.items).some(section => /legacy/i.test(section.label))).toBe(false);
  });

  it('keeps every control label unique, so a label names one home', () => {
    const labels = SETTINGS_SECTION_GROUPS.flatMap(group => group.items.flatMap(section => (section.controls ?? []).map(control => control.label)));
    expect(labels.filter((label, i) => labels.indexOf(label) !== i)).toEqual([]);
  });

  it('keeps every connector and agent tool in the one Connectors section', () => {
    for (const old of ['agenttools', 'teamwork', 'computer', 'capture']) expect(resolveSettingsSection(old)).toBe('connectors');
    expect(SETTINGS_SECTION_IDS).not.toContain('agenttools');
  });

  it('keeps previous vault-path and setup links on their current surface', () => {
    for (const old of ['paths', 'setup', 'vault']) {
      expect(resolveSettingsSection(old), old).toBe('sources');
      expect(SETTINGS_ALIAS_CONTROLS[old], old).toBe('Hollow folder');
    }
    expect(resolveSettingsSection('debug')).toBe('advanced');
  });

  it('resolves every compatibility link to one live, unique section', () => {
    expect(new Set(SETTINGS_SECTION_IDS).size).toBe(SETTINGS_SECTION_IDS.length);
    for (const id of Object.keys(SETTINGS_SECTION_ALIASES)) {
      expect(SETTINGS_SECTION_IDS).toContain(resolveSettingsSection(id));
    }
  });

  it('offers Usage as its own section found by cost and token keywords', () => {
    expect(SETTINGS_SECTION_IDS).toContain('usage');
    const matches = SETTINGS_SECTION_GROUPS.flatMap(group => group.items)
      .filter(section => /\btokens?\b|\bcost\b/i.test(`${section.label} ${section.keywords}`));
    expect(matches.map(section => section.id)).toEqual(['usage']);
  });

  it('sends an old feedback link to Usage, and indexes no Votes control (overnight 11-08 S5)', () => {
    expect(SETTINGS_SECTION_IDS).not.toContain('feedback');
    expect(resolveSettingsSection('feedback')).toBe('usage');
    expect(SETTINGS_ALIAS_CONTROLS.feedback).toBe('Budgets');
    const labels = SETTINGS_SECTION_GROUPS.flatMap(group => group.items.flatMap(section => (section.controls ?? []).map(control => control.label)));
    expect(labels).not.toContain('Votes');
    expect(labels).not.toContain('Routing');
  });

  it('keeps Notion (official) discoverable under connectors', () => {
    const matches = SETTINGS_SECTION_GROUPS.flatMap(group => group.items).filter(section => /notion/i.test(`${section.label} ${section.keywords}`));
    expect(matches.map(section => section.id)).toEqual(['connectors']);
  });
});

describe('settings search (audit H06)', () => {
  it('routes old source links to the task tracker, and Notion to the connector library', () => {
    expect(resolveSettingsControl('connectors', 'Hollow’s own sources')).toBe('Task tracker');
    expect(searchSettings('GitHub').flatMap(h => h.controls.map(c => c.label))).toContain('Services');
    expect(searchSettings('Notion official').flatMap(h => h.controls.map(c => c.label))).toEqual(['Your connectors']);
  });
  it('retains the old appearance control deep link after its rename', () => {
    expect(resolveSettingsControl(resolveSettingsSection('interface'),'Experimental visuals')).toBe('Ambient motion');
    expect(resolveSettingsControl('general','Texture')).toBe('Texture');
    expect(resolveSettingsControl('other','Experimental visuals')).toBe('Experimental visuals');
    expect(resolveSettingsControl('general',undefined)).toBeUndefined();
  });
  const ids = (q: string) => searchSettings(q).map(h => h.section.id);
  const controls = (q: string) => searchSettings(q).flatMap(h => h.controls.map(c => `${h.section.id}:${c.label}`));
  it('finds visible controls the old keyword list missed', () => {
    expect(ids('motion')).toEqual(['general']);
    expect(controls('motion')).toEqual(['general:Ambient motion','general:Motion']);
    expect(controls('experimental visuals')).toEqual(['general:Ambient motion']);
    expect(controls('texture')).toEqual(['general:Texture']);
    expect(ids('reduced')).toEqual(['general']);
    expect(ids('cinematic')).toEqual(['general']);
    expect(ids('reduce motion')).toEqual(['general']);
    // Whiteboards were removed (2026-10-07): nothing is named for them.
    expect(ids('whiteboard')).toEqual([]);
    expect(ids('backup')).toEqual(['advanced']);
    expect(controls('backup')).toEqual(['advanced:This Mac', 'advanced:Backup evidence', 'advanced:Backup management']);
    // A12: where to look when a tool is "command not found" for an agent.
    expect(controls('PATH')).toContain('advanced:Folders agents can run tools from');
    expect(controls('gcloud')).toEqual(['advanced:Folders agents can run tools from']);
    expect(ids('notifications')).toContain('general');
    expect(ids('notifications').length).toBeLessThanOrEqual(2);
  });
  it('does not list every control when the query only names the section', () => {
    expect(searchSettings('general').find(h => h.section.id === 'general')?.controls).toEqual([]);
  });
  it('finds every indexed control by its own label', () => {
    for (const g of SETTINGS_SECTION_GROUPS) for (const s of g.items) for (const c of s.controls ?? []) {
      const hit = searchSettings(c.label).find(h => h.section.id === s.id);
      expect(hit?.controls.map(x => x.label), `${s.id}: "${c.label}"`).toContain(c.label);
    }
  });
  // P18 acceptance: a common setting is at most two choices after opening
  // Settings (section, then the control on screen), or one search and its hit.
  it('reaches common settings with one search', () => {
    const common: [string, string, string][] = [
      ['dark', 'general', 'Appearance'],
      ['text size', 'general', 'Text size'],
      ['reduce motion', 'general', 'Motion'],
      ['default agent', 'models', 'Default agent'],
      ['permissions', 'chat', 'Permission mode for new chats'],
      ['run at login', 'general', 'Run at login'],
      ['turn off shortcuts', 'shortcuts', 'Use keyboard shortcuts'],
      ['memory', 'personalization', 'Memory'],
      // #1238: saved and replaced memories live in Library → Learnings, and
      // Memory's link opens it; no search word lands on a removed list.
      ['saved learning', 'personalization', 'Memory'],
      ['outdated', 'personalization', 'Memory'],
      // P1-22: Chat holds what shapes a conversation (PR 3: a section again).
      ['retitle', 'chat', 'Retitle new chats'],
      ['enter sends', 'chat', 'Enter sends a message'],
      ['chats at once', 'chat', 'Chats running at once'],
      ['budgets', 'usage', 'Budgets'],
      ['content index', 'advanced', 'Search inside project files'],
      ['your connectors', 'connectors', 'Your connectors'],
      // P1-21: "Projects folder" was a ghost row; the Hollow folder part shows where Projects is.
      ['projects folder', 'sources', 'Hollow folder'],
      // 'new task' no longer finds the Hollow folder (overnight 11-13 S1): its cut Make something now made one.
      ['start locally', 'sources', 'Hollow folder'],
      ['linear', 'sources', 'Task tracker'],
      ['agent updates', 'accounts', 'Agent updates'],
      ['pair a device', 'devices', 'Pair a device'],
      ['history & recovery', 'advanced', 'History & recovery is in Library'],
      // Self-hosted models Q4: a part of Models.
      ['self-hosted models', 'models', 'Self-hosted models'],
      ['qwen', 'models', 'Self-hosted models'],
      // Q10: its Background jobs block, in Advanced, Hollow's own model calls since S5b.
      ['background jobs', 'advanced', 'Background jobs'],
      ['payroll', 'advanced', 'Background jobs'],
      // S5b: the runner in Devices, the run limits in Advanced.
      ['builds and tests', 'devices', 'Builds and tests'],
      ['watchdog', 'advanced', 'No-output watchdog'],
    ];
    for (const [query, section, control] of common) {
      const [first] = searchSettings(query);
      expect(first?.section.id, query).toBe(section);
      if (control) expect(first.controls.map(c => c.label), query).toContain(control);
    }
  });
  // P1-21: visible controls the index missed, and no ghost rows.
  it('finds swarm, failover, import skill and the icon presets, and no Setup wizard', () => {
    expect(controls('swarm')).toContain('background:Largest swarm (chats)');
    expect(controls('extraction model')).toContain('advanced:Extraction model');
    expect(controls('failover')).toContain('usage:Automatic account rotation');
    expect(controls('import skill')).toContain('connectors:Skills are in Library');
    expect(controls('aurora')).toEqual(['general:App icon']);
    expect(controls('cut glass')).toEqual(['general:App icon']);
    expect(controls('setup wizard')).toEqual([]);
    expect(SETTINGS_SECTION_GROUPS.flatMap(g => g.items.flatMap(s => (s.controls ?? []).map(c => c.label)))).not.toContain('Setup wizard');
  });
  it('returns every section for an empty query', () => {
    expect(ids('').length).toBe(SETTINGS_SECTION_IDS.length);
  });
  it('the on-screen guard ignores comments', () => {
    expect(withoutComments('a // Setup wizard\nb /* Setup wizard */ c {/* Setup wizard */}')).not.toContain('Setup wizard');
    expect(withoutComments('href="https://x.test" // note')).toContain('https://x.test');
  });
  it('indexes only labels that are actually on screen', () => {
    // builtInCaps.ts: the labels of Budgets' built-in rows (P1-07), rendered by components/BuiltInCaps.tsx.
    // meetingsCopy.ts: the Plaud row's label, Recordings in Hollow (P1-20), rendered by settings/ConnectorRows.tsx.
    // Comments don't count: a label named only in a comment is a ghost (P1-21).
    const source = withoutComments(Object.values(import.meta.glob(['./**/*.tsx', './builtInCaps.ts', './meetingsCopy.ts'], { query: '?raw', import: 'default', eager: true }) as Record<string, string>).join('\n'));
    for (const g of SETTINGS_SECTION_GROUPS) for (const s of g.items) for (const c of s.controls ?? []) {
      expect(source.includes(c.label.replace(/&/g, '&')) || source.includes(c.label.replace(/&/g, '&amp;')), `${s.id}: "${c.label}" is not visible text`).toBe(true);
    }
  });
});

describe('thirteen plainly named sections (settings redesign PR 3)', () => {
  const controlsOf = (id: string) => (SETTINGS_SECTION_BY_ID[id]?.controls ?? []).map(c => c.label);
  it('shows eighteen sections in the index', () => {
    expect(SETTINGS_SECTION_IDS).toHaveLength(18);
  });
  it('sends every retired id to its new home, on the control or part it became', () => {
    const homes: Record<string, [string, string]> = {
      feedback: ['usage', 'Budgets'],
      context: ['advanced', 'Context prioritizer'],
      vaultsearch: ['advanced', 'Search by meaning'],
      learning: ['personalization', 'Memory'],
      doctor: ['advanced', 'Capability doctor'],
      startup: ['general', 'Run at login'],
      notifications: ['general', 'In-app pop-ups'],
      routines: ['background', 'Scheduled automations'],
      heartbeat: ['background', 'Silent heartbeat'],
      debug: ['advanced', 'Debug logs'],
      indexes: ['advanced', 'Search & indexes'],
      vault: ['sources', 'Hollow folder'],
      paths: ['sources', 'Hollow folder'],
      selfhosted: ['models', 'Self-hosted models'],
    };
    for (const [old, [home, control]] of Object.entries(homes)) {
      expect(SETTINGS_SECTION_IDS, old).not.toContain(old);
      expect(resolveSettingsSection(old), old).toBe(home);
      expect(SETTINGS_ALIAS_CONTROLS[old], old).toBe(control);
      expect(controlsOf(home), old).toContain(control);
    }
    // Plan section 6's list of the 30 retired ids, and the five sections before.
    const retired = ['capture', 'cadence', 'agenttools', 'teamwork', 'computer', 'm365write', 'imports', 'doctor', 'customization', 'startup', 'appearance', 'notifications', 'routines', 'heartbeat', 'paths', 'setup', 'debug', 'context', 'vaultsearch', 'feedback', 'learning', 'selfhosted', 'companions', 'vault', 'indexes', 'skills', 'history', 'interface', 'defaults', 'operator', 'automation', 'system'];
    for (const id of retired) expect(SETTINGS_SECTION_IDS, id).toContain(resolveSettingsSection(id));
    expect(resolveSettingsSection('companions')).toBe('devices');
    expect(resolveSettingsSection('customization')).toBe('models');
  });
  it('keeps the old section names as search words', () => {
    for (const [query, home] of [['response feedback', 'usage'], ['context prioritizer', 'advanced'], ['search by meaning', 'advanced'], ['companion devices', 'devices'], ['memory', 'personalization'], ['knowledge', 'personalization'], ['interface', 'general'], ['chat', 'chat'], ['skills', 'connectors'], ['search & indexes', 'advanced'], ['hollow folder', 'sources'], ['automation', 'background']]) {
      expect(searchSettings(query).map(h => h.section.id), query).toContain(home);
    }
  });
  it('opens Advanced on This Mac: the capability doctor and backup evidence first', () => {
    expect(controlsOf('advanced').slice(0, 3)).toEqual(['This Mac', 'Capability doctor', 'Backup evidence']);
  });
  it('keeps Budgets and Value this month, and no Votes, Routing or Friction page', () => {
    expect(controlsOf('usage')).toEqual(expect.arrayContaining(['Budgets', 'Value this month']));
    expect(controlsOf('usage')).not.toContain('Votes');
    expect(controlsOf('usage')).not.toContain('Routing');
    expect(controlsOf('usage')).not.toContain('Friction');
    expect(controlsOf('usage')).not.toContain('Prices');
    expect(controlsOf('advanced')).toContain('Prices');
  });
  const partOf = (id: string, label: string) => SETTINGS_SECTION_BY_ID[id]?.controls?.find(c => c.label === label)?.part;
  it('keeps the content index in Search & indexes, under Advanced', () => {
    expect(partOf('advanced', 'Search inside project files')).toBe('indexes');
    expect(controlsOf('personalization')).not.toContain('Search inside project files');
  });
  it('Chat holds Canvases, Enter sends, Retitle, the new-chat permission mode and chats at once', () => {
    for (const label of ['Canvases', 'Enter sends a message', 'Retitle new chats', 'Permission mode for new chats', 'Chats running at once', 'Chats at once in one workspace']) expect(controlsOf('chat'), label).toContain(label);
    expect(controlsOf('general')).not.toContain('Enter sends a message');
    expect(controlsOf('background')).not.toContain('Chats running at once');
  });
  it("Models' scope names no provider and stays short", () => {
    const affects = SETTINGS_SECTION_BY_ID.models.scope.affects;
    expect(affects.length).toBeLessThanOrEqual(120);
    expect(affects).not.toMatch(/claude|codex|antigravity|grok/i);
  });
});

describe('parts: former sections kept whole inside a section (S2)', () => {
  it('gives every part a heading control that lands on the part, and its old id opens it', () => {
    for (const section of SETTINGS_SECTION_GROUPS.flatMap(g => g.items)) for (const part of section.parts ?? []) {
      const heading = section.controls?.find(c => c.label === part.label);
      expect(heading, part.id).toMatchObject({ part: part.id, heading: true });
      expect(section.controls?.filter(c => c.label === part.label), `${part.id}: one control by its name`).toHaveLength(1);
      expect(settingsPartHeading(section.id, part.label), part.id).toBe(part.id);
      expect(resolveSettingsSection(part.id), part.id).toBe(section.id);
      expect(SETTINGS_ALIAS_CONTROLS[part.id], part.id).toBe(part.label);
      for (const c of part.controls) expect(section.controls?.find(x => x.label === c.label)?.part, `${part.id}: ${c.label}`).toBe(part.id);
    }
  });
  it('lands a jump on a part by its id, never on a row whose name starts the same', () => {
    expect(settingsPartHeading('models', 'Self-hosted models')).toBe('selfhosted');
    expect(settingsPartHeading('chat', 'Canvases')).toBeUndefined();
    expect(settingsPartHeading('chat', 'Chats running at once')).toBeUndefined();
    expect(settingsPartHeading('models', undefined)).toBeUndefined();
  });
  it('lists only the part heading when a query names a part', () => {
    expect(searchSettings('skills').find(h => h.section.id === 'connectors')?.controls.map(c => c.label)).toEqual(['Skills are in Library']);
  });
  it('keeps every retired id one step from a live section', () => {
    for (const [old, home] of Object.entries(SETTINGS_SECTION_ALIASES)) {
      expect(SETTINGS_SECTION_IDS, old).not.toContain(old);
      expect(SETTINGS_SECTION_IDS, `${old} → ${home}`).toContain(home);
    }
  });
});

describe('settings scope (plan completion L4, X11)', () => {
  it('every section says what it affects, when a change takes effect and how it saves', () => {
    for (const s of SETTINGS_SECTION_GROUPS.flatMap(g => g.items)) {
      expect(s.scope.affects, s.id).toMatch(/\S{3}/);
      expect(s.scope.takesEffect, s.id).toMatch(/\S{3}/);
      expect(Object.keys(SAVE_WORDS), s.id).toContain(s.scope.saves);
    }
  });
});

describe('Jev and the Hollow folder in search (overnight 11-13 S1)', () => {
  it('finds both Jev rows by TypeSafe and by their switch wording', () => {
    const jev = (q: string) => searchSettings(q).find(h => h.section.id === 'advanced')?.controls.map(c => c.label) ?? [];
    expect(jev('typesafe')).toEqual(expect.arrayContaining(['Jev relevance model', 'Jev duplicate check']));
    expect(jev('rank context')).toContain('Jev relevance model');
    expect(jev('check suggestions for duplicates')).toContain('Jev duplicate check');
  });

  it('no longer finds the Hollow folder for a new note or a new task', () => {
    expect(searchSettings('new note').map(h => h.section.id)).not.toContain('sources');
    expect(searchSettings('new task').map(h => h.section.id)).not.toContain('sources');
    expect(searchSettings('start locally').map(h => h.section.id)).toContain('sources');
  });
});

describe('the narrow section picker lands on the control it found (overnight 11-13 S1)', () => {
  it('goes to the first control the picker words match', () => {
    expect(pickerControl('general', 'density')).toBe('Density');
    expect(pickerControl('advanced', 'jev duplicate')).toBe('Jev duplicate check');
  });

  it('opens the section top for no words or words naming the section', () => {
    expect(pickerControl('usage', '')).toBeUndefined();
    expect(pickerControl('usage', undefined)).toBeUndefined();
    expect(pickerControl('usage', 'usage')).toBeUndefined();
    expect(pickerControl('nope', 'motion')).toBeUndefined();
  });

  it('is wired: Select hands its search words to onChange, and Settings jumps with them', () => {
    expect(selectSource).toContain('props.choose(o.value, searchable() ? query().trim() || undefined : undefined);');
    expect(settingsPageSource).toContain('onChange={(id, query) => jumpTo(id, pickerControl(id, query))}');
  });
});

// Build run 11-17 (App size pass): the search words live apart from the
// eager table in settingsIndex.ts, joined by id and label. A label renamed
// in one file and not the other would silently lose its words.
describe('search words and the eager index', () => {
  it('every words entry names a live section, part or control', () => {
    for (const [id, w] of Object.entries(SETTINGS_KEYWORDS)) {
      const s = SETTINGS_INDEX_BY_ID[id];
      expect(s, id).toBeDefined();
      const own = new Set((s.controls ?? []).filter(c => !c.part).map(c => c.label));
      for (const label of Object.keys(w.controls ?? {})) expect(own.has(label), `${id}: ${label}`).toBe(true);
      for (const [pid, pw] of Object.entries(w.parts ?? {})) {
        const p = s.parts?.find(x => x.id === pid);
        expect(p, `${id}.${pid}`).toBeDefined();
        for (const label of Object.keys(pw.controls ?? {})) expect(p!.controls.some(c => c.label === label), `${id}.${pid}: ${label}`).toBe(true);
      }
    }
  });
  it('every section and part has words, and the merged registry keeps the index order', () => {
    for (const s of SETTINGS_SECTIONS) {
      expect(SETTINGS_KEYWORDS[s.id]?.keywords, s.id).toBeTruthy();
      for (const p of s.parts ?? []) expect(SETTINGS_KEYWORDS[s.id]?.parts?.[p.id]?.keywords, `${s.id}.${p.id}`).toBeTruthy();
    }
    expect(SETTINGS_SECTION_IDS).toEqual(SETTINGS_SECTIONS.map(s => s.id));
  });
});
