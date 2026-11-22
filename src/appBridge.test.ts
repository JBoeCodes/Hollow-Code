import { describe, expect, it } from 'vitest';
import settingsKeysSource from '../src-tauri/src/settings_keys.rs?raw';
import appActionsSource from '../src-tauri/src/app_actions.rs?raw';
import { answerWith, applyPendingWith, navigateTarget, planForChanged, planForStarted, serializer, SETTINGS_CHANGING_ACTIONS, UI_SETTING_KEYS, type UiSetting } from './appBridgeCore';
import { SETTINGS_SECTION_ALIASES, SETTINGS_SECTION_IDS } from './settingsSections';


describe('appBridge refresh plans', () => {
  it('puts an agent-started session’s workspace in the rail and refreshes the lists', () => {
    expect(planForStarted({ sessionKey: 's-1', workspaceId: 'w-1', cwd: '/v/workspaces/w-1', startedBy: 's-0' }))
      .toEqual({ rail: 'w-1', workspaces: true, sessions: true });
    expect(planForStarted({ sessionKey: 's-1', workspaceId: null, cwd: '/v', startedBy: 's-0' }).rail).toBeNull();
  });

  it('refreshes what the vault watcher can’t see', () => {
    expect(planForChanged({ action: 'workspaces.create_workspace', area: 'workspaces' })).toEqual({ rail: null, workspaces: true, sessions: true, themes: false, settings: false });
    expect(planForChanged({ action: 'sessions.manage', area: 'sessions' })).toEqual({ rail: null, workspaces: false, sessions: true, themes: false, settings: false });
    expect(planForChanged({ action: 'goals.create_goal', area: 'goals' })).toEqual({ rail: null, workspaces: false, sessions: false, themes: false, settings: false });
    expect(planForChanged({ action: 'settings.appearance.write_custom_theme', area: 'settings.appearance' }).themes).toBe(true);
  });

  it('has open Settings panels read a setting an agent changed again (overnight H6)', () => {
    // Learning suggestions, directly or through change_setting.
    expect(planForChanged({ action: 'learning.set_learning_suggestions', area: 'learning' })).toEqual({ rail: null, workspaces: false, sessions: false, themes: false, settings: true });
    expect(planForChanged({ action: 'settings.change', area: 'settings' }).settings).toBe(true);
    // The extraction model and Retitle new chats.
    expect(planForChanged({ action: 'settings.extraction.set_extraction_settings', area: 'settings.extraction' }).settings).toBe(true);
    expect(planForChanged({ action: 'settings.extraction.set_chat_retitle', area: 'settings.extraction' }).settings).toBe(true);
    // Proposing a learning changes no setting.
    expect(planForChanged({ action: 'learning.submit_learning_proposal', area: 'learning' }).settings).toBe(false);
    expect(planForChanged({ action: 'settings.appearance.write_custom_theme', area: 'settings.appearance' }).settings).toBe(false);
  });

  it('names actions and an area the backend has (overnight H6)', () => {
    // Each id is `area.command` (act!) or a verb's own id; a rename in
    // app_actions.rs must fail here, not leave Settings stale.
    expect(appActionsSource).toContain('act!(Confirm, "learning", set_learning_suggestions,');
    expect(appActionsSource).toContain('verb!(Write, "settings.change", "settings",');
    expect(appActionsSource).toContain('act!(Confirm, "settings.extraction", set_extraction_settings,');
    expect(appActionsSource).toContain('act!(Confirm, "settings.extraction", set_chat_retitle,');
    expect(SETTINGS_CHANGING_ACTIONS).toEqual(['learning.set_learning_suggestions', 'settings.change']);
  });
});

/** The Rust settings table (`settings_keys.rs`): each row's key, section
 *  and owner, the parts it lists as having no key, and its retired ids. */
function rustKeys() {
  const source = settingsKeysSource;
  const rows = [...source.matchAll(/\b(ui|backend)!\("([^"]+)", "([^"]+)"/g)].map(m => ({ owner: m[1], key: m[2], section: m[3] }));
  const block = (name: string) => source.slice(source.indexOf(`pub(crate) const ${name}`), source.indexOf('];', source.indexOf(`pub(crate) const ${name}`)));
  const none = [...block('NONE').matchAll(/\("([^"]+)", "/g)].map(m => m[1]);
  const aliases = Object.fromEntries([...block('SECTION_ALIASES').matchAll(/\("([^"]+)", "([^"]+)"\)/g)].map(m => [m[1], m[2]]));
  return { rows, none, aliases };
}

describe('settings keys (PR A4)', () => {
  it('every Settings section has a key or says why not, and every key is in a live section (PR 3: twelve)', () => {
    const { rows, none } = rustKeys();
    expect(rows.length).toBeGreaterThan(30);
    for (const id of SETTINGS_SECTION_IDS) expect(rows.some(r => r.section === id) || none.includes(id), `section ${id}`).toBe(true);
    for (const r of rows) expect(SETTINGS_SECTION_IDS, r.key).toContain(r.section);
  });

  it('lists parts and keyless sections as keyless, and reads every retired id as the section the page opens (PR 3)', () => {
    const { rows, none, aliases } = rustKeys();
    expect(none.length).toBeGreaterThan(0);
    for (const id of none) {
      // A part's old id, read as its section; or a live section with no key (Sources).
      if (!SETTINGS_SECTION_IDS.includes(id)) expect(SETTINGS_SECTION_IDS, id).toContain(SETTINGS_SECTION_ALIASES[id]);
      expect(rows.some(r => r.section === id), `${id} is keyless`).toBe(false);
    }
    expect(aliases).toEqual(SETTINGS_SECTION_ALIASES);
  });

  it('puts each key in its new section (settings redesign plan, section 6)', () => {
    const at = Object.fromEntries(rustKeys().rows.map(r => [r.key, r.section]));
    expect(at).toMatchObject({
      theme: 'general', 'app.runOnStartup': 'general', shortcuts: 'shortcuts', 'chat.canvases': 'chat',
      'agents.defaultAgent': 'models', 'agents.whenOut': 'models', 'agents.effort.claude': 'models',
      'agents.updates.codex': 'accounts', 'jev.apiKey': 'accounts',
      'accounts.claudeCeiling': 'usage', claudeQuotaOptIn: 'usage',
      'agents.runsOn': 'devices', 'agents.buildsOn': 'devices', 'agents.unattendedRunsOn': 'devices', 'agents.cooDispatcher': 'devices',
      'agents.appBuildsOn': 'advanced', 'agents.codexNoOutputTimeoutMin': 'advanced', 'jev.enabled': 'advanced', semanticSearchEnabled: 'advanced', backupManagedElsewhere: 'advanced',
      'heartbeat.enabled': 'background', 'operator.contextEnabled': 'personalization', 'learning.suggestions': 'personalization',
    });
  });

  it('the webview owns exactly the Rust table’s UI keys, each through a store setter', () => {
    const ui = rustKeys().rows.filter(r => r.owner === 'ui').map(r => r.key).sort();
    expect([...UI_SETTING_KEYS].sort()).toEqual(ui);
  });

  it('answers get and set through the setter and refuses what it doesn’t own', async () => {
    let filter = 'all';
    const settings: Record<string, UiSetting> = {
      notifFilter: { get: () => filter, set: v => { if (!['all', 'agent'].includes(v as string)) throw new Error('bad filter'); filter = v as string; } },
      backupManagedElsewhere: { get: () => false, set: () => {} },
    };
    const navigate = async (page: string) => { if (page !== 'tasks') throw new Error(`navigate: unknown page ${page}.`); return page; };
    const ask = (op: string, args: Record<string, unknown>) => answerWith(settings, navigate, { id: 'ui-1', op, args });
    expect(await ask('set', { key: 'notifFilter', value: 'agent' })).toEqual({ ok: true, value: { old: 'all', new: 'agent' } });
    expect(await ask('get', { keys: ['notifFilter', 'backupManagedElsewhere'] })).toEqual({ ok: true, value: { notifFilter: 'agent', backupManagedElsewhere: false } });
    expect(await ask('set', { key: 'notifFilter', value: 'everything' })).toEqual({ ok: false, error: 'bad filter' });
    expect(filter, 'a refused value changes nothing').toBe('agent');
    expect(await ask('set', { key: 'agents.defaultAgent', value: 'codex' })).toEqual({ ok: false, error: 'agents.defaultAgent is not a UI setting.' });
    expect(await ask('navigate', { page: 'tasks' })).toEqual({ ok: true, value: 'tasks' });
    expect((await ask('navigate', { page: 'nowhere' })).ok).toBe(false);
    expect((await ask('drop', {})).ok).toBe(false);
  });
});

describe('settings the COO queued while the window was closed', () => {
  it('applies each queued entry through its setter at boot and skips what it can’t', async () => {
    let filter = 'all';
    let backup = false;
    const settings: Record<string, UiSetting> = {
      notifFilter: { get: () => filter, set: v => { if (!['all', 'agent'].includes(v as string)) throw new Error('bad filter'); filter = v as string; } },
      backupManagedElsewhere: { get: () => backup, set: v => { backup = v as boolean; } },
    };
    const applied = await applyPendingWith(settings, async () => ({
      settings: { notifFilter: 'agent', backupManagedElsewhere: true, 'agents.defaultAgent': 'codex', toString: 1 },
      versions: { notifFilter: 1, backupManagedElsewhere: 2, 'agents.defaultAgent': 3, toString: 4 },
    }));
    expect(applied).toEqual(['notifFilter', 'backupManagedElsewhere']);
    expect(filter).toBe('agent');
    expect(backup).toBe(true);
    // A refused value is dropped and the rest still apply; an empty queue does nothing.
    filter = 'all';
    expect(await applyPendingWith(settings, async () => ({ settings: { notifFilter: 'everything', backupManagedElsewhere: false } }))).toEqual(['backupManagedElsewhere']);
    expect(filter).toBe('all');
    expect(await applyPendingWith(settings, async () => null)).toEqual([]);
  });

  it('acknowledges each settled entry with its version only after applying it (L13 review)', async () => {
    const log: string[] = [];
    const settings: Record<string, UiSetting> = {
      theme: { get: () => 'dark', set: v => { log.push(`set theme ${String(v)}`); } },
      notifFilter: { get: () => 'all', set: () => { throw new Error('bad filter'); } },
    };
    const acked: Record<string, number>[] = [];
    const applied = await applyPendingWith(
      settings,
      async () => ({ settings: { theme: 'light', notifFilter: 'everything', 'not.ours': 1 }, versions: { theme: 7, notifFilter: 8 } }),
      async versions => { log.push('ack'); acked.push(versions); },
    );
    expect(applied).toEqual(['theme']);
    // Applied, refused and not-ours entries are all settled; a missing version reads 0.
    expect(acked).toEqual([{ theme: 7, notifFilter: 8, 'not.ours': 0 }]);
    expect(log).toEqual(['set theme light', 'ack']);
    // Nothing queued: no acknowledgement at all.
    acked.length = 0;
    await applyPendingWith(settings, async () => ({ settings: {}, versions: {} }), async v => { acked.push(v); });
    expect(acked).toEqual([]);
    // A drain that fails before acknowledging leaves the queue for the next one.
    await expect(applyPendingWith(settings, async () => { throw new Error('reloaded'); }, async v => { acked.push(v); })).rejects.toThrow('reloaded');
    expect(acked).toEqual([]);
  });

  it('runs a drain and a live set one at a time, in the order asked, past a failure', async () => {
    const inTurn = serializer();
    const order: string[] = [];
    let release!: () => void;
    const drain = inTurn(async () => {
      order.push('drain start');
      await new Promise<void>(resolve => { release = resolve; });
      order.push('drain end');
      throw new Error('drain failed');
    });
    const live = inTurn(async () => { order.push('live set'); return 'set'; });
    await Promise.resolve();
    expect(order).toEqual(['drain start']);
    release();
    await expect(drain).rejects.toThrow('drain failed');
    expect(await live).toBe('set');
    expect(order).toEqual(['drain start', 'drain end', 'live set']);
  });
});

describe('the pages ui.navigate opens (P0-06)', () => {
  it('names the page it shows: library opens Library', () => {
    expect(navigateTarget('library')).toBe('library');
    expect(navigateTarget('tasks')).toBe('tasks');
  });

  it('refuses retired and renamed pages with the one to use', () => {
    // From other apps was removed in run 15 P4. Agenda was cut overnight 11-08 S3.
    expect(() => navigateTarget('imports')).toThrow('navigate: imports is retired; use library.');
    expect(() => navigateTarget('calendar')).toThrow('navigate: calendar is retired; use tasks.');
    expect(() => navigateTarget('agenda')).toThrow('navigate: agenda is retired; use tasks.');
    expect(() => navigateTarget('today')).toThrow('navigate: today is retired; use home.');
    expect(() => navigateTarget('projects')).toThrow('navigate: projects is retired; use home.');
    for (const p of ['people', 'graph', 'ingest']) expect(() => navigateTarget(p)).toThrow(`navigate: ${p} is retired; use library.`);
    expect(() => navigateTarget('vault')).toThrow('navigate: vault is now library.');
    expect(() => navigateTarget('workflows')).toThrow('navigate: workflows is now automations.');
    expect(() => navigateTarget('nowhere')).toThrow('navigate: unknown page nowhere.');
    // Internal go targets aren't pages.
    expect(() => navigateTarget('jump')).toThrow('navigate: unknown page jump.');
  });
});
