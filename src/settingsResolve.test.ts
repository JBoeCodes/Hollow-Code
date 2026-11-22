import { describe, expect, it } from 'vitest';
import legacy from './settingsLegacy.json';
import { CAPABILITY_HOMES } from './capabilityDoctor';
import settingsKeysSource from '../src-tauri/src/settings_keys.rs?raw';
import { MOVED_LABELS, resolveSettingsTarget } from './settingsResolve';
import { SETTINGS_ALIAS_CONTROLS, SETTINGS_SECTION_ALIASES, SETTINGS_SECTION_BY_ID, SETTINGS_SECTION_IDS, searchSettings } from './settingsSections';

const sources = import.meta.glob('./**/*.{ts,tsx}', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
const rustSources = import.meta.glob('../src-tauri/src/*.rs', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

/** A resolved target must be a live section and, with a label, one of its registered controls. */
function expectLands(id: string, label?: string) {
  const t = resolveSettingsTarget(id, label);
  // A control that moved to Library or Automations (PR 4) lands on that page.
  if (t.kind === 'page') { expect(['vault', 'workflows'], `${id}/${label}`).toContain(t.page); expect(t.tab, `${id}/${label}`).toBeTruthy(); return; }
  expect(t.kind, `${id}/${label}`).toBe('section');
  if (t.kind !== 'section') return;
  expect(SETTINGS_SECTION_IDS, `${id}/${label}`).toContain(t.section);
  if (t.label) expect(SETTINGS_SECTION_BY_ID[t.section].controls?.some(c => c.label === t.label), `${id}/${label} -> ${t.label}`).toBe(true);
}

describe('settingsLegacy.json (the frozen ids contract, before PR 3)', () => {
  it('holds seven sections, nine parts, thirty retired ids and fifteen alias controls', () => {
    expect(legacy.sections).toHaveLength(7);
    expect(Object.keys(legacy.parts)).toHaveLength(9);
    expect(Object.keys(legacy.aliases)).toHaveLength(30);
    expect(Object.keys(legacy.aliasControls)).toHaveLength(15);
  });

  it('is still the contract before PR 3: every old id is live or an alias, and Rust keeps the live table', () => {
    for (const id of [...legacy.sections, ...Object.keys(legacy.parts), ...Object.keys(legacy.aliases)]) {
      expect(SETTINGS_SECTION_IDS.includes(id) || id in SETTINGS_SECTION_ALIASES, id).toBe(true);
    }
    const rust = Object.fromEntries([...settingsKeysSource.slice(settingsKeysSource.indexOf('const SECTION_ALIASES')).split('];')[0].matchAll(/\("([^"]+)", "([^"]+)"\)/g)].map(m => [m[1], m[2]]));
    expect(rust).toEqual(SETTINGS_SECTION_ALIASES);
    expect(Object.keys(SETTINGS_ALIAS_CONTROLS).every(id => id in SETTINGS_SECTION_ALIASES)).toBe(true);
  });

  it('lands every old (section, label) pair on that control in the section that holds it now', () => {
    for (const c of legacy.controls) expectLands(c.section, c.label);
    // Part headings and folds that are gone open the section or control they became.
    expect(resolveSettingsTarget('interface', 'Keyboard shortcuts')).toEqual({ kind: 'section', section: 'shortcuts' });
    expect(resolveSettingsTarget('system', 'Developer readouts')).toEqual({ kind: 'section', section: 'advanced', label: 'Diagnostics' });
  });

  it('resolves a control newer than the freeze (Backup management, the link rows) to its own section and label', () => {
    const frozen = new Set(legacy.controls.map(c => c.label));
    let checked = 0;
    for (const section of SETTINGS_SECTION_IDS) for (const c of SETTINGS_SECTION_BY_ID[section].controls ?? []) {
      if (frozen.has(c.label)) continue;
      checked++;
      expect(resolveSettingsTarget(section, c.label), `${section}|${c.label}`).toMatchObject({ kind: 'section', section, label: c.label });
    }
    expect(checked).toBeGreaterThan(0);
  });
});

describe('resolveSettingsTarget', () => {
  it('resolves the new Updates section without changing frozen ids', () => {
    expect(resolveSettingsTarget('updates')).toEqual({ kind: 'section', section: 'updates' });
    expect(resolveSettingsTarget('updates', 'Channel')).toEqual({ kind: 'section', section: 'updates', label: 'Channel' });
  });
  it('lands every legacy section, part, alias and (section, label) pair on a registered control', () => {
    for (const id of legacy.sections) expectLands(id);
    for (const id of Object.keys(legacy.parts)) expectLands(id);
    for (const id of Object.keys(legacy.aliases)) expectLands(id);
    // A label lands in the section that holds it now, reached by any old id.
    const home = (label: string) => SETTINGS_SECTION_IDS.find(s => SETTINGS_SECTION_BY_ID[s].controls?.some(c => c.label === label));
    for (const old of [...legacy.sections, ...Object.keys(legacy.aliases)]) {
      for (const c of legacy.controls.filter(x => x.section === ((legacy.aliases as Record<string, string>)[old] ?? old))) {
        const now = home(c.label);
        if (now) expect(resolveSettingsTarget(old, c.label), `${old}/${c.label}`).toMatchObject({ section: now, label: c.label });
      }
    }
  });

  it('opens the part or control a retired id became (PR 3: the twelve sections)', () => {
    expect(resolveSettingsTarget('feedback')).toEqual({ kind: 'section', section: 'usage', label: 'Budgets' });
    expect(resolveSettingsTarget('shortcuts')).toEqual({ kind: 'section', section: 'shortcuts' });
    expect(resolveSettingsTarget('vault')).toEqual({ kind: 'section', section: 'sources', part: 'vault', label: 'Hollow folder' });
    expect(resolveSettingsTarget('heartbeat')).toEqual({ kind: 'section', section: 'background', label: 'Silent heartbeat' });
    expect(resolveSettingsTarget('defaults', 'Agent updates')).toEqual({ kind: 'section', section: 'accounts', label: 'Agent updates' });
    expect(resolveSettingsTarget('automation', 'Chats running at once')).toEqual({ kind: 'section', section: 'chat', label: 'Chats running at once' });
    expect(resolveSettingsTarget('connectors', 'Task tracker')).toEqual({ kind: 'section', section: 'sources', label: 'Task tracker' });
    expect(resolveSettingsTarget('companions')).toEqual({ kind: 'section', section: 'devices' });
    expect(resolveSettingsTarget('connectors', 'Hollow’s own sources')).toMatchObject({ section: 'sources', label: 'Task tracker' });
    expect(resolveSettingsTarget('interface', 'Experimental visuals')).toMatchObject({ section: 'general', label: 'Ambient motion' });
  });

  // openSettingsFromOutside callers count too (overnight 11-13 S3 moved four there).
  it('finds every requestOpenSettingsSection and openSettingsFromOutside call site in src and resolves each literal', () => {
    let sites = 0;
    let literals = 0;
    for (const [file, text] of Object.entries(sources)) {
      if (/\.test\.tsx?$/.test(file) || file.endsWith('settingsResolve.ts')) continue;
      for (const m of text.matchAll(/(?:requestOpenSettingsSection|openSettingsFromOutside)\(([^)]*)\)/g)) {
        if (/^\s*(id|control)\b/.test(m[1]) && file.endsWith('store/runtime.ts')) continue; // the definition
        sites++;
        const lit = m[1].match(/^\s*'([^']+)'(?:\s*,\s*'([^']+)')?/);
        if (!lit) continue;
        literals++;
        expectLands(lit[1], lit[2]);
      }
    }
    expect(sites).toBeGreaterThanOrEqual(28);
    expect(literals).toBeGreaterThanOrEqual(20);
  });

  it('resolves the action runner, the doctor fixSection values and Rust settings: links', () => {
    for (const m of sources['./actionRun.ts'].matchAll(/settings\('([^']+)'\)/g)) expectLands(m[1]);
    // Each doctor check's home (section and row), and the sign-in home a missing agent uses.
    const homes = [...Object.values(CAPABILITY_HOMES), { section: 'defaults', control: 'Accounts' }];
    expect(homes.length).toBeGreaterThan(4);
    for (const h of homes) expectLands(h.section, h.control);
    const links = new Set<string>();
    for (const text of Object.values(rustSources)) for (const m of text.matchAll(/"settings:([a-z0-9]+)"/g)) links.add(m[1]);
    expect(links.size).toBeGreaterThan(3);
    for (const id of links) expectLands(id);
  });
});

describe('renamed controls (Settings PR 7)', () => {
  it('opens How many learnings to suggest for the old Learning suggestions label, from any old id', () => {
    const now = { kind: 'section', section: 'personalization', label: 'How many learnings to suggest' };
    expect(resolveSettingsTarget('personalization', 'Learning suggestions')).toEqual(now);
    expect(resolveSettingsTarget('learning', 'Learning suggestions')).toEqual(now);
    expect(resolveSettingsTarget('defaults', 'Learning suggestions')).toEqual(now);
    expect(resolveSettingsTarget('personalization', 'How many learnings to suggest')).toEqual(now);
    expect(searchSettings('learning suggestions').some(h => h.controls.some(c => c.label === 'How many learnings to suggest'))).toBe(true);
  });
  it('S5b: Reasoning effort opens Effort, and Mac mini runner opens the runner in Devices', () => {
    expect(resolveSettingsTarget('defaults', 'Reasoning effort')).toEqual({ kind: 'section', section: 'models', label: 'Effort' });
    expect(resolveSettingsTarget('models', 'Reasoning effort')).toEqual({ kind: 'section', section: 'models', label: 'Effort' });
    expect(resolveSettingsTarget('devices', 'Mac mini runner')).toEqual({ kind: 'section', section: 'devices', label: 'Remote runner' });
    // The agent form's other pieces, by their old home.
    expect(resolveSettingsTarget('models', 'Runs on')).toEqual({ kind: 'section', section: 'devices', label: 'Runs on' });
    expect(resolveSettingsTarget('defaults', 'No-output watchdog')).toEqual({ kind: 'section', section: 'advanced', label: 'No-output watchdog' });
    expect(resolveSettingsTarget('selfhosted', 'Background jobs')).toEqual({ kind: 'section', section: 'advanced', label: 'Background jobs' });
  });
});

describe('page targets (Settings PR 4: moves to Library and Automations)', () => {
  it('opens Library, Skills for the skills part, its controls and the old section id', () => {
    const skills = { kind: 'page', page: 'vault', tab: 'skills' };
    expect(resolveSettingsTarget('skills')).toMatchObject(skills);
    for (const label of ['Skills', 'New skill', 'Import skill', 'Available to']) {
      expect(resolveSettingsTarget('connectors', label), label).toMatchObject({ ...skills, label });
      expect(resolveSettingsTarget('defaults', label), label).toMatchObject(skills);
    }
  });

  it('opens Library, History for the history part and its controls', () => {
    const history = { kind: 'page', page: 'vault', tab: 'history' };
    expect(resolveSettingsTarget('history')).toMatchObject(history);
    expect(resolveSettingsTarget('system', 'History & recovery')).toMatchObject(history);
    expect(resolveSettingsTarget('advanced', 'Automated-write runs')).toMatchObject(history);
  });

  it('opens Library, Conversations for the cut Conversation history button', () => {
    expect(resolveSettingsTarget('system', 'Conversation history')).toMatchObject({ kind: 'page', page: 'vault', tab: 'conversations' });
  });

  it('opens Automations on its windows editor or run queue', () => {
    expect(resolveSettingsTarget('automation', 'Automation windows')).toEqual({ kind: 'page', page: 'workflows', tab: 'windows', label: 'Automation windows' });
    expect(resolveSettingsTarget('background', 'Run queue')).toEqual({ kind: 'page', page: 'workflows', tab: 'queue', label: 'Run queue' });
  });

  it('opens Library, Learnings for the profile suggestions (Ready to add)', () => {
    expect(resolveSettingsTarget('operator', 'Ready to add')).toEqual({ kind: 'page', page: 'vault', tab: 'learnings', label: 'Ready to add' });
    expect(resolveSettingsTarget('personalization', 'Ready to add')).toMatchObject({ kind: 'page', page: 'vault', tab: 'learnings' });
  });

  it('names where each moved label lives, for search and link rows', () => {
    expect(Object.values(MOVED_LABELS).every(m => m.where === (m.page === 'vault' ? 'in Library' : 'in Automations'))).toBe(true);
  });

  it('keeps live labels on their sections', () => {
    expect(resolveSettingsTarget('automation', 'Silent heartbeat')).toEqual({ kind: 'section', section: 'background', label: 'Silent heartbeat' });
    expect(resolveSettingsTarget('operator', 'Memory')).toEqual({ kind: 'section', section: 'personalization', label: 'Memory' });
  });
});

describe('a generic cut label lands only from where it was (overnight 11-13 S1)', () => {
  it('sends Help to all keys from Keyboard shortcuts or System, and leaves it alone elsewhere', () => {
    expect(resolveSettingsTarget('shortcuts', 'Help')).toEqual({ kind: 'section', section: 'shortcuts', label: 'Show all keys' });
    expect(resolveSettingsTarget('system', 'Help')).toEqual({ kind: 'section', section: 'shortcuts', label: 'Show all keys' });
    expect(resolveSettingsTarget('connectors', 'Help')).toEqual({ kind: 'section', section: 'connectors', label: 'Help' });
    expect(resolveSettingsTarget('usage', 'Help')).toMatchObject({ kind: 'section', section: 'usage' });
  });

  it('keeps the specific cut labels from any section', () => {
    expect(resolveSettingsTarget('general', 'All keys')).toEqual({ kind: 'section', section: 'shortcuts', label: 'Show all keys' });
  });
});
