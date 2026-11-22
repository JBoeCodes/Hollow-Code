import { describe, expect, it } from 'vitest';
import { catalogModel, customRows, effortLabel, levelsForModel, effortShown, menuGroups, modelLabel, retryModelChoices, sameRow, focusLeavesMenu, rowIndexByKey, rowKey } from './modelMenu';

const agents = [{ id: 'claude', label: 'Claude' }, { id: 'grok', label: 'Grok' }];
const catalogs = {
  claude: [{ id: 'opus', label: 'Opus 5.5' }, { id: 'sonnet', label: 'Sonnet 5' }],
  grok: [{ id: 'grok-4.7', label: 'grok-4.7 · default' }],
};

describe('model menu helpers', () => {
  it('groups every agent with its default first', () => {
    const groups = menuGroups(agents, catalogs, { agent: 'claude', model: 'opus' }, '');
    expect(groups.map(g => g.agent)).toEqual(['claude', 'grok']);
    expect(groups[0].rows.map(r => r.id)).toEqual(['', 'opus', 'sonnet']);
    expect(groups[0].rows[0].label).toBe('Claude default');
  });

  it('keeps a current model the catalog does not list', () => {
    const groups = menuGroups(agents, catalogs, { agent: 'claude', model: 'claude-opus-5-5' }, '');
    expect(groups[0].rows.at(-1)).toMatchObject({ id: 'claude-opus-5-5', custom: true });
  });

  it('searches model and agent names word by word', () => {
    expect(menuGroups(agents, catalogs, { agent: 'claude', model: '' }, 'son').flatMap(g => g.rows.map(r => r.id))).toEqual(['sonnet']);
    expect(menuGroups(agents, catalogs, { agent: 'claude', model: '' }, 'grok').map(g => g.agent)).toEqual(['grok']);
    expect(menuGroups(agents, catalogs, { agent: 'claude', model: '' }, 'nothing-like-this')).toEqual([]);
  });

  it('offers a typed ID to every agent when no catalog lists it', () => {
    expect(customRows(agents, catalogs, 'custom-model-v9').map(r => r.agent)).toEqual(['claude', 'grok']);
    expect(customRows(agents, catalogs, 'opus')).toEqual([]);
    expect(customRows(agents, catalogs, 'two words')).toEqual([]);
    expect(customRows(agents, catalogs, '  ')).toEqual([]);
  });

  it('labels the trigger and effort', () => {
    expect(modelLabel('Claude', '', catalogs.claude)).toBe('Claude default');
    expect(modelLabel('Claude', 'opus', catalogs.claude)).toBe('Opus 5.5');
    expect(modelLabel('Claude', 'x-1', catalogs.claude)).toBe('x-1');
    expect(effortLabel('high')).toBe('High');
    expect(effortLabel('xhigh')).toBe('Extra high');
    expect(effortLabel(undefined)).toBeUndefined();
    expect(sameRow({ agent: 'claude', id: 'opus', label: '' }, { agent: 'claude', model: 'opus' })).toBe(true);
  });
});

describe('Retry with another model (D1 gap 10)', () => {
  const claude = [
    { id: 'opus', label: 'Opus 5.5' }, { id: 'fable', label: 'Fable 5.1' }, { id: 'sonnet', label: 'Sonnet 5.5' },
    { id: 'haiku', label: 'Haiku 5.5' }, { id: 'claude-sonnet-5', label: '' }, { id: 'self-hosted:qwen', label: 'Qwen' },
  ];

  it('lists the catalog in its own order without the model in use, four at most', () => {
    expect(retryModelChoices(claude, 'opus').map(m => m.id)).toEqual(['fable', 'sonnet', 'haiku', 'claude-sonnet-5']);
    expect(retryModelChoices(claude, 'sonnet').map(m => m.id)).toEqual(['opus', 'fable', 'haiku', 'claude-sonnet-5']);
    expect(retryModelChoices(claude, 'opus', { limit: 2 }).map(m => m.id)).toEqual(['fable', 'sonnet']);
  });

  // Claude Code's real list (`agent_models.rs`): each alias, its 1M-context
  // twin, Opus plans, the pinned IDs, then self-hosted models (review of #1207).
  const live = [
    { id: 'opus', label: 'Opus 5.5' }, { id: 'opus[1m]', label: 'Opus 5.5 · 1M context' },
    { id: 'fable', label: 'Fable 5.1' }, { id: 'fable[1m]', label: 'Fable 5.1 · 1M context' },
    { id: 'sonnet', label: 'Sonnet 5.5' }, { id: 'sonnet[1m]', label: 'Sonnet 5.5 · 1M context' },
    { id: 'haiku', label: 'Haiku 5.5' }, { id: 'opusplan', label: 'Opus 5.5 plans, Sonnet 5.5 executes' },
    { id: 'claude-opus-5-5', label: 'Opus 5.5 · pinned' }, { id: 'claude-sonnet-5-5', label: 'Sonnet 5.5 · pinned' },
    { id: 'claude-sonnet-5', label: 'Sonnet 5 · pinned' }, { id: 'self-hosted:qwen', label: 'Qwen3.8 27B · self-hosted' },
  ];
  const newestByFamily = { opus: 'claude-opus-5-5', fable: 'claude-fable-5-1', sonnet: 'claude-sonnet-5-5', haiku: 'claude-haiku-5-5' };

  it('offers one row per family: no 1M twin, no pinned copy of a listed alias, Haiku inside the four', () => {
    expect(retryModelChoices(live, '', { newestByFamily }).map(m => m.id)).toEqual(['opus', 'fable', 'sonnet', 'haiku']);
    expect(retryModelChoices(live, 'opus', { newestByFamily }).map(m => m.id)).toEqual(['fable', 'sonnet', 'haiku', 'self-hosted:qwen']);
  });

  it('leaves out the whole family the chat uses, however it is named', () => {
    for (const current of ['opus[1m]', 'claude-opus-5-5', 'opus']) {
      expect(retryModelChoices(live, current, { newestByFamily }).map(m => m.id)).toEqual(['fable', 'sonnet', 'haiku', 'self-hosted:qwen']);
    }
    expect(retryModelChoices(live, 'sonnet[1m]', { newestByFamily }).map(m => m.id)).toEqual(['opus', 'fable', 'haiku', 'self-hosted:qwen']);
    // Past the families, self-hosted models, then the rest in catalog order.
    expect(retryModelChoices(live, 'haiku', { newestByFamily, limit: 8 }).map(m => m.id))
      .toEqual(['opus', 'fable', 'sonnet', 'self-hosted:qwen', 'opusplan', 'claude-sonnet-5']);
  });

  it('keeps the whole catalog when the chat uses the default or an unlisted model', () => {
    expect(retryModelChoices(claude, '').map(m => m.id)).toEqual(['opus', 'fable', 'sonnet', 'haiku']);
    expect(retryModelChoices(claude, 'claude-opus-5-5').map(m => m.id)).toEqual(['opus', 'fable', 'sonnet', 'haiku']);
  });

  it('names a model with no label by its id, skips repeats and blank ids, and tolerates no catalog', () => {
    expect(retryModelChoices([{ id: 'x', label: '' }, { id: 'x', label: 'X again' }, { id: '', label: 'Default' }], 'y')).toEqual([{ id: 'x', label: 'x' }]);
    expect(retryModelChoices(undefined, 'opus')).toEqual([]);
    expect(retryModelChoices([{ id: 'opus', label: 'Opus' }], 'opus')).toEqual([]);
  });
});

describe('Ultracode in the effort menu (Track U)', () => {
  it('shows Ultracode while the chat switch is on, else the saved level', () => {
    expect(effortShown('high', true)).toBe('Ultracode');
    expect(effortShown(undefined, true)).toBe('Ultracode');
    expect(effortShown('xhigh', false)).toBe('Extra high');
    expect(effortShown(undefined, false)).toBeUndefined();
  });
});

describe('the highlighted row is kept by identity (ENG-81)', () => {
  it('a catalog that loads while the menu is open does not move Enter to another model', () => {
    const opus = { agent: 'claude', id: 'opus', label: 'Opus' };
    const before = [{ agent: 'claude', id: '', label: 'Default' }, opus];
    const key = rowKey(before[1]);
    const after = [before[0], { agent: 'claude', id: 'fable', label: 'Fable' }, { agent: 'claude', id: 'sonnet', label: 'Sonnet' }, opus];
    expect(after[rowIndexByKey(after, key)]).toBe(opus);
    expect(rowIndexByKey(after, 'listed:codex:gone')).toBe(0);
    expect(rowIndexByKey(after, null)).toBe(0);
    // A typed row and a listed one of the same id are different rows.
    expect(rowKey({ ...opus, typed: true })).not.toBe(rowKey(opus));
  });
});

describe('the model menu closes when focus leaves it (ENG-81)', () => {
  const node = (dialog = false) => ({ closest: (sel: string) => (dialog && sel.includes('dialog') ? {} : null) });
  const inside = node();
  const pop = { contains: (n: unknown) => n === inside };
  const trigger = node();
  it('Shift+Tab to its button or focus elsewhere closes it', () => {
    expect(focusLeavesMenu(trigger, pop, trigger)).toBe(true);
    expect(focusLeavesMenu(node(), pop, trigger)).toBe(true);
  });
  it('focus inside, in its confirm dialog, a Select popover or nowhere keeps it open', () => {
    expect(focusLeavesMenu(inside, pop, trigger)).toBe(false);
    expect(focusLeavesMenu(node(true), pop, trigger)).toBe(false);
    const popover = node();
    expect(focusLeavesMenu(popover, pop, trigger, n => n === popover)).toBe(false);
    expect(focusLeavesMenu(null, pop, trigger)).toBe(false);
  });
});

describe('Antigravity base models share one Effort control (2026-11-20)', () => {
  const agy = [
    { id: 'gemini-4-neon', label: 'Gemini 4 Neon', levels: ['low', 'medium', 'high', 'xhigh'] },
    { id: 'gemini-4-argon', label: 'Gemini 4 Argon', levels: ['low', 'medium', 'high'] },
    { id: 'gemini-pro-orchestrator', label: 'Orchestrator' },
  ];
  const all = ['low', 'medium', 'high', 'xhigh', 'max'];

  it('offers only the levels the model takes', () => {
    expect(levelsForModel(all, agy, 'gemini-4-neon')).toEqual(['low', 'medium', 'high', 'xhigh']);
    expect(levelsForModel(all, agy, 'gemini-4-argon')).toEqual(['low', 'medium', 'high']);
    expect(levelsForModel(all, agy, 'gemini-pro-orchestrator')).toEqual(all);
    expect(levelsForModel(all, agy, '')).toEqual(all);
    expect(levelsForModel(all, undefined, 'gemini-4-neon')).toEqual(all);
  });

  it('reads a saved variant id as its base model row', () => {
    expect(catalogModel(agy, 'gemini-4-neon-high')?.id).toBe('gemini-4-neon');
    expect(catalogModel(agy, 'gemini-4-argon-xhigh')).toBeUndefined();
    expect(levelsForModel(all, agy, 'gemini-4-argon-high')).toEqual(['low', 'medium', 'high']);
    const groups = menuGroups([{ id: 'antigravity', label: 'Antigravity' }], { antigravity: agy }, { agent: 'antigravity', model: 'gemini-4-neon-high' }, '');
    expect(groups[0].rows.some(r => r.custom)).toBe(false);
    expect(modelLabel('Antigravity', 'gemini-4-neon-high', agy)).toBe('Gemini 4 Neon');
    const row = groups[0].rows.find(r => r.id === 'gemini-4-neon')!;
    expect(sameRow(row, { agent: 'antigravity', model: 'gemini-4-neon-high' }, agy)).toBe(true);
    expect(sameRow(row, { agent: 'antigravity', model: 'gemini-4-neon-high' })).toBe(false);
  });
});
