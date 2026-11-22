import { describe, expect, it, vi } from 'vitest';
import { SETTINGS_PAGE_ENTRIES, pendingForPage, pageEntryWhere, searchSettingsPages, type SettingsPageEntry } from './pageTargets';
import { SETTINGS_SECTION_GROUPS, searchSettings } from '../settingsSections';
import { openFoldsAround, type FoldLike } from '../settingsJump';
import { resolveSettingsTarget, MOVED_LABELS } from '../settingsResolve';
import { buildCapabilityChecks, CAPABILITY_HOMES, type CapabilityDoctorInputs } from '../capabilityDoctor';

describe('page entries in the settings search', () => {
  it('finds a setting that lives on another page and says where', () => {
    const [hit] = searchSettingsPages('learnings');
    expect(hit.label).toBe('Learnings review');
    expect(pageEntryWhere(hit)).toBe('in Library');
    expect(hit.tab).toBe('Learnings');
    expect(searchSettingsPages('schedule').map(e => pageEntryWhere(e))).toContain('in Automations');
  });

  it('matches by page name and ignores case, and an empty query lists none', () => {
    expect(searchSettingsPages('LIBRARY').length).toBeGreaterThan(0);
    expect(searchSettingsPages('')).toEqual([]);
    expect(searchSettingsPages('zzzz nothing')).toEqual([]);
  });

  it('is one line to add: a new entry is found without other changes', () => {
    const open = vi.fn();
    const extra: SettingsPageEntry = { label: 'Zebra list', keywords: 'zebra', page: 'Library', tab: 'Skills', open };
    const [hit] = searchSettingsPages('zebra', [...SETTINGS_PAGE_ENTRIES, extra]);
    hit.open();
    expect(open).toHaveBeenCalledOnce();
  });

  it('keeps labels unique among every control Settings lists', () => {
    const own = new Set(SETTINGS_SECTION_GROUPS.flatMap(g => g.items.flatMap(s => (s.controls ?? []).map(c => c.label))));
    const labels = SETTINGS_PAGE_ENTRIES.map(e => e.label);
    expect(new Set(labels).size).toBe(labels.length);
    for (const l of labels) expect(own.has(l), l).toBe(false);
  });

  it('does not change what the sections themselves match', () => {
    expect(searchSettings('learnings').length).toBeGreaterThan(0);
  });
});

function fold(open: boolean, parent: FoldLike | null = null): FoldLike & { open: boolean } {
  const f: FoldLike & { open: boolean } = { open, parentElement: parent, closest: () => f };
  return f;
}
/** An element whose nearest enclosing disclosure is `inner`; `outer` encloses it. */
function rowIn(inner: FoldLike | null): FoldLike {
  return { parentElement: inner ? { parentElement: null, closest: (sel: string) => (sel === 'details' ? inner : null) } : null, closest: () => null };
}

describe('a hit inside a closed fold opens it', () => {
  it('opens the Advanced fold and the group inside it, innermost first', () => {
    const advanced = fold(false);
    const group = fold(false, { parentElement: null, closest: () => advanced });
    const row = rowIn(group);
    expect(openFoldsAround(row)).toBe(2);
    expect(group.open).toBe(true);
    expect(advanced.open).toBe(true);
  });

  it('leaves an already-open fold open and counts only what it opened', () => {
    const advanced = fold(true);
    expect(openFoldsAround(rowIn(advanced))).toBe(0);
    expect(advanced.open).toBe(true);
  });

  it('does nothing for a row outside any fold', () => {
    expect(openFoldsAround(rowIn(null))).toBe(0);
  });
});

describe('doctor fixes go to the setting\'s real home', () => {
  const base: CapabilityDoctorInputs = {
    vaultExists: false, diagnosticsFailed: false, agents: [], agentProbeFailed: false, plaudStatus: 'needs-auth', plaudError: false, plaudLastSuccess: null,
    backup: { status: 'protected', summary: 'x', signals: [] }, backupFailed: false, prefs: { version: 1, declined: {}, lastVerified: {} }, now: 0,
  };
  const find = (id: string, over: Partial<CapabilityDoctorInputs> = {}) => buildCapabilityChecks({ ...base, ...over }).find(c => c.id === id)!;
  // The doctor keeps the old section ids (the frozen contract); each resolves to a registered row in its new section.
  const registered = (section: string, control: string) => {
    const t = resolveSettingsTarget(section, control);
    if (t.kind !== 'section') return false;
    const s = SETTINGS_SECTION_GROUPS.flatMap(g => g.items).find(i => i.id === t.section);
    // A retired part heading that is a whole section now (Accounts) resolves to the section alone.
    return !!s && (!t.label || !!s.controls?.some(c => c.label === t.label));
  };

  it('vault lands on Hollow folder', () => {
    expect(find('vault')).toMatchObject({ fixSection: 'system', fixControl: 'Hollow folder' });
  });
  it('a missing agent lands on Accounts; a found one on Runtime', () => {
    expect(find('agent-runtime')).toMatchObject({ fixSection: 'defaults', fixControl: 'Accounts' });
    expect(find('agent-runtime', { agents: ['claude'] })).toMatchObject({ fixSection: 'advanced', fixControl: 'Runtime' });
  });
  it('Plaud lands on Recordings in Connections', () => {
    expect(find('plaud')).toMatchObject({ fixSection: 'connectors', fixControl: 'Recordings' });
  });
  it('the backup lands on its management checkbox', () => {
    expect(find('backup')).toMatchObject({ fixSection: 'system', fixControl: 'Backup management' });
  });
  it('the task tracker lands on Task tracker', () => {
    const [t] = buildCapabilityChecks({ ...base, tracker: { state: 'needsSetup' } as never, now: 1 }).filter(c => c.id === 'tracker');
    expect(t).toMatchObject({ fixSection: 'connectors', fixControl: 'Task tracker' });
  });
  it('every home is a registered row', () => {
    for (const [id, home] of Object.entries(CAPABILITY_HOMES)) expect(registered(home.section, home.control), id).toBe(true);
    expect(registered('defaults', 'Accounts')).toBe(true);
  });
});

describe('moved controls are search entries built from MOVED_LABELS (Settings PR 4)', () => {
  it('lists Skills, History, Learnings profile suggestions, windows and the run queue once each, with where they are', () => {
    for (const [query, where, tab] of [['skills', 'in Library', 'Skills'], ['restore', 'in Library', 'History'], ['profile suggestions', 'in Library', 'Learnings'], ['schedule window', 'in Automations', 'Automation windows'], ['queue', 'in Automations', 'Run queue']] as const) {
      const hits = searchSettingsPages(query).filter(e => e.tab === tab);
      expect(hits, query).toHaveLength(1);
      expect(pageEntryWhere(hits[0])).toBe(where);
    }
  });
  it('has no duplicate labels and one entry per searchable moved label', () => {
    const labels = SETTINGS_PAGE_ENTRIES.map(e => e.label);
    expect(new Set(labels).size).toBe(labels.length);
    expect(Object.entries(MOVED_LABELS).filter(([, m]) => m.search).every(([l]) => labels.includes(l))).toBe(true);
  });
});

describe('leaving Settings for a page asks about unsaved forms', () => {
  const entry = SETTINGS_PAGE_ENTRIES[0];
  it('asks, naming the page, when a form is unsaved, and keeps the entry to open on Discard', () => {
    expect(pendingForPage(entry, ['Hollow folder'])).toEqual({ to: 'Library', unsaved: ['Hollow folder'], page: entry });
  });
  it('opens at once when nothing is unsaved', () => {
    expect(pendingForPage(entry, [])).toBeNull();
  });
});

describe('one search result per destination (overnight 11-13 S1)', () => {
  it('lists Library, Learnings once for "learnings review" and for "ready to add"', () => {
    expect(searchSettingsPages('learnings review').map(e => e.label)).toEqual(['Learnings review']);
    expect(searchSettingsPages('ready to add').map(e => e.label)).toEqual(['Ready to add']);
    // The query names neither label: the first entry for the place stays.
    expect(searchSettingsPages('learnings').filter(e => e.tab === 'Learnings').map(e => e.label)).toEqual(['Learnings review']);
  });

  it('keeps entries for different places', () => {
    const open = vi.fn();
    const a: SettingsPageEntry = { label: 'Alpha', keywords: 'shared', page: 'Library', tab: 'Skills', open };
    const b: SettingsPageEntry = { label: 'Beta', keywords: 'shared', page: 'Library', tab: 'History', open };
    const c: SettingsPageEntry = { label: 'Gamma', keywords: 'shared', page: 'Library', tab: 'History', open };
    expect(searchSettingsPages('shared', [a, b, c]).map(e => e.label)).toEqual(['Alpha', 'Beta']);
    expect(searchSettingsPages('shared gamma', [a, b, c]).map(e => e.label)).toEqual(['Gamma']);
  });

  it('no Settings control repeats Ready to add, which is its Library entry', () => {
    expect(searchSettings('ready to add').flatMap(h => h.controls.map(c => c.label))).toEqual([]);
  });
});
