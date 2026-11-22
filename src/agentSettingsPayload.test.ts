import { describe, expect, it } from 'vitest';
import { AGENT_FORM_PIECES, type AgentFormPiece, agentDeadline, agentSettingsPayload, agentWatchdog, appBuildsOnForSave, buildsOnForSave, compactWindowForSave, deadlineMinutes, draftFromSettings, isRunner, RUNNER, runsOnForSave, sameAgentSettings, savePayload, unattendedRunsOnForSave, watchdogMinutes, whenOutForSave, withAgentDeadline, withAgentWatchdog, withPiece } from './agentSettingsPayload';
import type { AgentSettings } from './types';

describe('the Codex watchdog in the payload (Paperclip PR 15b)', () => {
  it('omits the default, sends off as null and keeps other fields', () => {
    const base = { defaultAgent: 'codex', models: { codex: 'm' }, efforts: { codex: 'high' } };
    expect(agentSettingsPayload({ ...base, codexNoOutputTimeoutMin: 30 })).toEqual({ defaultAgent: 'codex', models: { codex: 'm' }, efforts: { codex: 'high' } });
    expect('codexNoOutputTimeoutMin' in agentSettingsPayload(base)).toBe(false);
    expect(agentSettingsPayload({ ...base, codexNoOutputTimeoutMin: null }).codexNoOutputTimeoutMin).toBeNull();
    expect(agentSettingsPayload({ ...base, codexNoOutputTimeoutMin: 12 })).toEqual({ ...base, codexNoOutputTimeoutMin: 12 });
  });

  it('normalizes like the backend: absent and non-positive are the default, large clamps', () => {
    expect(watchdogMinutes(undefined)).toBe(30);
    expect(watchdogMinutes(null)).toBeNull();
    expect(watchdogMinutes(0)).toBe(30);
    expect(watchdogMinutes(-5)).toBe(30);
    expect(watchdogMinutes(Number.NaN)).toBe(30);
    expect(watchdogMinutes(5000)).toBe(1440);
    expect(watchdogMinutes(7.4)).toBe(7);
  });

  it('a saved default and an absent field are the same settings', () => {
    const saved = { defaultAgent: null, models: {} };
    expect(sameAgentSettings(saved, { ...saved, codexNoOutputTimeoutMin: 30 })).toBe(true);
    expect(sameAgentSettings(saved, { ...saved, codexNoOutputTimeoutMin: null })).toBe(false);
    expect(sameAgentSettings({ ...saved, codexNoOutputTimeoutMin: null }, { ...saved, codexNoOutputTimeoutMin: null })).toBe(true);
  });
});

describe('every agent\'s watchdog and deadline (merged plan slice 3)', () => {
  const base = { defaultAgent: null, models: {} };

  it('reads Codex from its own field and the others from the map, absent as the default', () => {
    for (const agent of ['claude', 'codex', 'antigravity', 'grok']) expect(agentWatchdog(base, agent)).toBe(30);
    const set = { ...base, codexNoOutputTimeoutMin: 12, noOutputTimeoutMin: { claude: null, grok: 45 } };
    expect(agentWatchdog(set, 'codex')).toBe(12);
    expect(agentWatchdog(set, 'claude')).toBeNull();
    expect(agentWatchdog(set, 'grok')).toBe(45);
    expect(agentWatchdog(set, 'antigravity')).toBe(30);
    expect(withAgentWatchdog(base, 'codex', null).codexNoOutputTimeoutMin).toBeNull();
    expect(withAgentWatchdog(base, 'claude', null).noOutputTimeoutMin).toEqual({ claude: null });
  });

  it('saves only what differs from the default, off as null, deadlines when set', () => {
    let draft = withAgentWatchdog(base, 'claude', null);
    draft = withAgentWatchdog(draft, 'grok', 30);
    draft = withAgentWatchdog(draft, 'antigravity', 90);
    draft = withAgentDeadline(draft, 'codex', 20);
    draft = withAgentDeadline(draft, 'claude', 45);
    expect(agentSettingsPayload(draft)).toEqual({
      defaultAgent: null,
      models: {},
      noOutputTimeoutMin: { antigravity: 90, claude: null },
      deadlineMin: { claude: 45, codex: 20 },
    });
    expect(agentDeadline(draft, 'codex')).toBe(20);
    expect(agentDeadline(withAgentDeadline(draft, 'codex', null), 'codex')).toBeNull();
    // Nothing set: neither map is sent, so older payloads are unchanged.
    expect(agentSettingsPayload({ ...base, noOutputTimeoutMin: { grok: 30 }, deadlineMin: {} })).toEqual(base);
    // A stray codex entry in the map never shadows Codex's own field.
    expect('noOutputTimeoutMin' in agentSettingsPayload({ ...base, noOutputTimeoutMin: { codex: 5 } })).toBe(false);
  });

  it('normalizes deadlines like the backend: empty, zero and junk are none, large clamps', () => {
    expect(deadlineMinutes(undefined)).toBeNull();
    expect(deadlineMinutes(null)).toBeNull();
    expect(deadlineMinutes(0)).toBeNull();
    expect(deadlineMinutes(-3)).toBeNull();
    expect(deadlineMinutes(Number.NaN)).toBeNull();
    expect(deadlineMinutes(9000)).toBe(1440);
    expect(deadlineMinutes(19.6)).toBe(20);
    expect(sameAgentSettings(base, { ...base, deadlineMin: { claude: 0 } })).toBe(true);
    expect(sameAgentSettings(base, { ...base, deadlineMin: { claude: 10 } })).toBe(false);
  });
});

describe('agentSettingsPayload (Paperclip PR 14a, cross-m34 M1)', () => {
  it('keeps every field, trims values and drops empties', () => {
    expect(agentSettingsPayload({
      defaultAgent: 'codex',
      models: { codex: ' custom-model ', claude: '' },
      efforts: { claude: 'high', codex: ' ' },
    })).toEqual({ defaultAgent: 'codex', models: { codex: 'custom-model' }, efforts: { claude: 'high' } });
  });

  it('omits efforts when none is set, so older payloads are unchanged', () => {
    const payload = agentSettingsPayload({ defaultAgent: null, models: { claude: 'sonnet' }, efforts: {} });
    expect(payload).toEqual({ defaultAgent: null, models: { claude: 'sonnet' } });
    expect('efforts' in payload).toBe(false);
    expect('efforts' in agentSettingsPayload({ defaultAgent: null, models: {} })).toBe(false);
  });

  it('compares by content, not edit order or whitespace', () => {
    const saved = { defaultAgent: 'codex', models: { claude: 'a', codex: 'b' } };
    expect(sameAgentSettings(saved, { defaultAgent: 'codex', models: { codex: 'b ', claude: 'a' }, efforts: {} })).toBe(true);
    // Effort saves on change, outside the Save form (P0-19), so it isn't a form change.
    expect(sameAgentSettings(saved, { ...saved, efforts: { codex: 'low' } })).toBe(true);
    expect(sameAgentSettings(saved, { ...saved, models: { claude: 'a', codex: 'c' } })).toBe(false);
    expect(sameAgentSettings(saved, { ...saved, defaultAgent: null })).toBe(false);
  });
});

describe('Settings → Chat → Canvases in the payload (Canvas C3 wave 2 C4)', () => {
  it('sends off and most, never the default, and drops unknown values', () => {
    const base = { defaultAgent: null, models: {} };
    expect(agentSettingsPayload({ ...base, canvases: 'most' }).canvases).toBe('most');
    expect(agentSettingsPayload({ ...base, canvases: 'off' }).canvases).toBe('off');
    expect('canvases' in agentSettingsPayload({ ...base, canvases: 'helps' })).toBe(false);
    expect('canvases' in agentSettingsPayload(base)).toBe(false);
    expect('canvases' in agentSettingsPayload({ ...base, canvases: 'always' as never })).toBe(false);
  });

  it('a save that never showed Canvases keeps it, and the default compares equal to none', () => {
    const saved = { defaultAgent: 'claude', models: { claude: 'opus' }, canvases: 'off' as const };
    expect(agentSettingsPayload({ ...saved, efforts: { claude: 'high' } }).canvases).toBe('off');
    expect(sameAgentSettings({ ...saved, canvases: 'helps' }, { ...saved, canvases: undefined })).toBe(true);
    expect(sameAgentSettings(saved, { ...saved, canvases: 'most' })).toBe(false);
  });
});

describe('the compaction window in the payload (OB22)', () => {
  it('a save keeps a set window and never sends off', () => {
    const saved = { defaultAgent: 'claude', models: { claude: 'opus' }, compactWindow: 400000 };
    expect(agentSettingsPayload({ ...saved, efforts: { claude: 'high' } }).compactWindow).toBe(400000);
    expect('compactWindow' in agentSettingsPayload({ defaultAgent: null, models: {} })).toBe(false);
    expect(sameAgentSettings(saved, { ...saved, compactWindow: 600000 })).toBe(false);
  });

  it('sends only an offered size, so a hand-edited one read back never fails a save', () => {
    const base = { defaultAgent: null, models: {} };
    for (const size of [300000, 400000, 600000]) {
      expect(agentSettingsPayload({ ...base, compactWindow: size }).compactWindow).toBe(size);
    }
    for (const odd of [500000, 0, 123, 1_000_000, Number.NaN]) {
      expect('compactWindow' in agentSettingsPayload({ ...base, compactWindow: odd })).toBe(false);
    }
    expect(compactWindowForSave(null)).toBeUndefined();
    expect(compactWindowForSave(undefined)).toBeUndefined();
    expect(sameAgentSettings(base, { ...base, compactWindow: 500000 })).toBe(true);
  });
});

describe('Settings → Models keeps what it does not show (A4, RF-822-F1)', () => {
  it('a loaded compaction window gives a draft that is not dirty, and a save keeps it', () => {
    const loaded: AgentSettings = { defaultAgent: 'claude', models: { claude: 'opus' }, compactWindow: 400000 };
    const draft = draftFromSettings(loaded);
    expect(draft.compactWindow).toBe(400000);
    expect(sameAgentSettings(loaded, draft)).toBe(true);
    const edited = { ...draft, models: { claude: 'sonnet' } };
    expect(savePayload(edited, loaded)).toEqual({ defaultAgent: 'claude', models: { claude: 'sonnet' }, compactWindow: 400000 });
  });

  it('a save takes the window and Canvases from the live file, which other places set', () => {
    const draft = draftFromSettings({ defaultAgent: null, models: {}, compactWindow: 400000, canvases: 'off' });
    const live: AgentSettings = { defaultAgent: null, models: {}, compactWindow: 600000, canvases: 'most' };
    expect(savePayload(draft, live)).toEqual({ defaultAgent: null, models: {}, canvases: 'most', compactWindow: 600000 });
    // Turned off elsewhere since the pane opened: the save doesn't bring it back.
    expect('compactWindow' in savePayload(draft, { defaultAgent: null, models: {} })).toBe(false);
  });

  it('an effort set from the composer survives a model saved from a pane opened before it (P0-19)', () => {
    const opened: AgentSettings = { defaultAgent: 'claude', models: { claude: 'opus' }, efforts: { claude: 'medium' } };
    const draft = { ...draftFromSettings(opened), models: { claude: 'sonnet' } };
    // The composer saved high since the pane opened.
    const live: AgentSettings = { ...opened, efforts: { claude: 'high' } };
    expect(savePayload(draft, live)).toEqual({ defaultAgent: 'claude', models: { claude: 'sonnet' }, efforts: { claude: 'high' } });
    // Effort isn't part of the Save form, so it never makes the pane dirty.
    expect(sameAgentSettings(live, draftFromSettings(opened))).toBe(true);
  });

  it('copies every field, so an unedited draft never reads as a change', () => {
    const loaded: AgentSettings = {
      defaultAgent: 'codex', models: { codex: 'gpt-5' }, efforts: { codex: 'high' }, codexNoOutputTimeoutMin: null,
      noOutputTimeoutMin: { claude: 45 }, deadlineMin: { claude: 120 }, canvases: 'most', compactWindow: 300000,
    };
    expect(sameAgentSettings(loaded, draftFromSettings(loaded))).toBe(true);
    expect(savePayload(draftFromSettings(loaded), loaded)).toEqual(agentSettingsPayload(loaded));
  });
});

describe('When an agent runs out (repo ideas 2026-10-27 #8)', () => {
  it('sends an agent, never Wait, and keeps it in the draft', () => {
    const base: AgentSettings = { defaultAgent: null, models: {} };
    expect('whenOut' in agentSettingsPayload(base)).toBe(false);
    expect('whenOut' in agentSettingsPayload({ ...base, whenOut: 'wait' })).toBe(false);
    expect(agentSettingsPayload({ ...base, whenOut: 'codex' }).whenOut).toBe('codex');
    expect(whenOutForSave(' Codex ')).toBe('codex');
    expect(whenOutForSave('gpt')).toBeUndefined();
    const loaded: AgentSettings = { ...base, whenOut: 'codex' };
    expect(draftFromSettings(loaded).whenOut).toBe('codex');
    expect(sameAgentSettings(loaded, draftFromSettings(loaded))).toBe(true);
    expect(sameAgentSettings(loaded, { ...draftFromSettings(loaded), whenOut: undefined })).toBe(false);
  });
});

describe('Builds and tests (smart routing 3)', () => {
  it('sends the runner, Auto or This Mac, never the default, and keeps it in the draft', () => {
    expect(buildsOnForSave('runner')).toBe('runner');
    expect(buildsOnForSave('mac-mini')).toBe('runner');
    expect(buildsOnForSave(' this-mac ')).toBe('this-mac');
    // M7 (PER-21): Auto is a choice that saves; absent is This Mac.
    expect(buildsOnForSave(' auto ')).toBe('auto');
    for (const none of ['', 'elsewhere', undefined, null]) expect(buildsOnForSave(none)).toBeUndefined();
    const base = { defaultAgent: 'claude', models: {} };
    expect(agentSettingsPayload({ ...base, buildsOn: 'this-mac' }).buildsOn).toBe('this-mac');
    expect(agentSettingsPayload({ ...base, buildsOn: 'auto' }).buildsOn).toBe('auto');
    expect('buildsOn' in agentSettingsPayload({ ...base, buildsOn: undefined })).toBe(false);
    const loaded = { ...base, buildsOn: 'mac-mini' };
    expect(draftFromSettings(loaded).buildsOn).toBe('mac-mini');
    expect(sameAgentSettings(draftFromSettings(loaded), loaded)).toBe(true);
  });
});

describe('App builds (smart routing 7)', () => {
  it('sends Remote runner, never This Mac, and keeps it in the draft', () => {
    expect(appBuildsOnForSave('runner')).toBe('runner');
    expect(appBuildsOnForSave(' mac-mini ')).toBe('runner');
    for (const here of ['this-mac', '', 'elsewhere', undefined, null]) expect(appBuildsOnForSave(here)).toBeUndefined();
    const base = { defaultAgent: 'claude', models: {} };
    expect(agentSettingsPayload({ ...base, appBuildsOn: 'mac-mini' }).appBuildsOn).toBe('runner');
    expect('appBuildsOn' in agentSettingsPayload({ ...base, appBuildsOn: 'this-mac' })).toBe(false);
    const loaded = { ...base, appBuildsOn: 'mac-mini' };
    expect(draftFromSettings(loaded).appBuildsOn).toBe('mac-mini');
    expect(sameAgentSettings(draftFromSettings(loaded), loaded)).toBe(true);
  });
});

describe("Grok's ACP switch (OC1)", () => {
  const base = { defaultAgent: 'claude', models: {} };
  it('sends only an on switch for Grok, and a save keeps the live one', () => {
    expect(agentSettingsPayload({ ...base, acp: { grok: true } }).acp).toEqual({ grok: true });
    expect('acp' in agentSettingsPayload({ ...base, acp: { grok: false } })).toBe(false);
    expect('acp' in agentSettingsPayload({ ...base, acp: { antigravity: true } })).toBe(false);
    const live = { ...base, acp: { grok: true } };
    expect(savePayload(draftFromSettings(base), live).acp).toEqual({ grok: true });
  });
});

describe('the agent defaults form in pieces (Settings S5b)', () => {
  const loaded: AgentSettings = { defaultAgent: 'codex', models: { codex: 'm' }, efforts: { claude: 'high' }, codexNoOutputTimeoutMin: 45, noOutputTimeoutMin: { claude: null }, deadlineMin: { grok: 20 }, canvases: 'most', whenOut: 'claude', runsOn: 'mac-mini', unattendedRunsOn: 'same', buildsOn: 'this-mac', appBuildsOn: 'mac-mini' };

  it('puts every field the Save form holds in exactly one piece', () => {
    const fields = Object.values(AGENT_FORM_PIECES).flat();
    expect(new Set(fields).size).toBe(fields.length);
    // Saved elsewhere: efforts on change, Canvases in Chat, the compaction window by change_setting,
    // COO on this Mac at once from its own Devices row (M3a PR2).
    // OC1: Grok's ACP switch by change_setting too.
    const elsewhere = ['efforts', 'canvases', 'compactWindow', 'cooDispatcher', 'acp'];
    expect([...fields, ...elsewhere].sort()).toEqual(Object.keys(draftFromSettings(loaded)).sort());
  });

  it('takes only its own fields from the draft, so a save keeps every other field', () => {
    const draft = { ...draftFromSettings(loaded), defaultAgent: 'claude', runsOn: undefined, deadlineMin: {}, appBuildsOn: undefined };
    const live = draftFromSettings(loaded);
    for (const piece of Object.keys(AGENT_FORM_PIECES) as AgentFormPiece[]) {
      const sent = savePayload(withPiece(live, draft, piece), live);
      expect(sent.efforts).toEqual({ claude: 'high' });
      expect(sent.canvases).toBe('most');
      expect(sent.defaultAgent).toBe(piece === 'models' ? 'claude' : 'codex');
      expect(sent.runsOn).toBe(piece === 'runner' ? undefined : 'runner');
      expect(sent.deadlineMin).toEqual(piece === 'limits' ? undefined : { grok: 20 });
      expect(sent.appBuildsOn).toBe(piece === 'appBuilds' ? undefined : 'runner');
      // Untouched pieces stay as loaded.
      expect(sent.whenOut).toBe('claude');
      expect(sent.codexNoOutputTimeoutMin).toBe(45);
    }
  });

  it('a piece is dirty only for its own edits', () => {
    const draft = { ...draftFromSettings(loaded), buildsOn: undefined };
    expect(sameAgentSettings(loaded, withPiece(loaded, draft, 'models'))).toBe(true);
    expect(sameAgentSettings(loaded, withPiece(loaded, draft, 'runner'))).toBe(false);
  });
});

// M6 (PER-20, 2026-11-15): the runner's neutral name. Settings saved before
// it say `mac-mini` in each field: they still load as the runner, an
// unedited form is not dirty (so nothing saves on its own), and a save
// sends `runner`.
describe('the runner as `runner`, with old `mac-mini` settings still read (M6)', () => {
  const old: AgentSettings = { defaultAgent: 'claude', models: {}, runsOn: 'mac-mini', unattendedRunsOn: 'mac-mini', buildsOn: 'mac-mini', appBuildsOn: 'mac-mini' };

  it('reads either name as the runner', () => {
    for (const v of ['runner', 'mac-mini', ' runner ', ' mac-mini ']) expect(isRunner(v)).toBe(true);
    for (const v of ['this-mac', 'same', 'auto', 'mac', '', null, undefined]) expect(isRunner(v)).toBe(false);
    expect(RUNNER).toBe('runner');
  });

  it('an old file loads as the runner in each field and is not dirty', () => {
    const draft = draftFromSettings(old);
    expect(runsOnForSave(draft.runsOn)).toBe('runner');
    expect(buildsOnForSave(draft.buildsOn)).toBe('runner');
    expect(appBuildsOnForSave(draft.appBuildsOn)).toBe('runner');
    expect(unattendedRunsOnForSave(draft.unattendedRunsOn)).toBe('runner');
    expect(sameAgentSettings(old, draft)).toBe(true);
    // The same settings saved in the new name compare equal too.
    expect(sameAgentSettings(old, { ...old, runsOn: 'runner', buildsOn: 'runner', appBuildsOn: 'runner', unattendedRunsOn: 'runner' })).toBe(true);
  });

  it('a save writes `runner`', () => {
    const sent = savePayload(draftFromSettings(old), old);
    expect(sent.runsOn).toBe('runner');
    expect(sent.buildsOn).toBe('runner');
    expect(sent.appBuildsOn).toBe('runner');
    expect(sent.unattendedRunsOn).toBe('runner');
    expect(JSON.stringify(sent)).not.toContain('mac-mini');
  });
});
