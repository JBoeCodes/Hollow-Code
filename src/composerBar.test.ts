import { describe, expect, it, vi } from 'vitest';
import { KEEP_DRAFT, accessChipName, accessMenuItems, composerAddItems, type ComposerCommand } from './composerBar';
import { permModeOptions } from './permModes';
import chatBody from './components/ChatBody.tsx?raw';
import actionMenuSource from './components/ActionMenu.tsx?raw';
import chatRuntime from './store/chatRuntime.ts?raw';

// The store's tables (store/chatRuntime.ts), restated: importing the store
// here would start its resources outside a component.
type Mode = 'bypassPermissions' | 'auto' | 'acceptEdits' | 'plan';
const PERM_MODE_ORDER: Mode[] = ['acceptEdits', 'auto', 'plan', 'bypassPermissions'];
const PERM_MODE_DANGER: Record<Mode, boolean> = { bypassPermissions: true, auto: false, acceptEdits: false, plan: false };
const PERM_MODE_LABELS: Record<Mode, string> = { bypassPermissions: 'Bypass all (danger)', auto: 'Auto', acceptEdits: 'Edits only', plan: 'Plan' };
const PERM_MODE_DESC: Record<Mode, string> = { bypassPermissions: 'b', auto: 'a', acceptEdits: 'e', plan: 'p' };
const PERM_MODE_SHORT_LABELS: Record<Mode, string> = { bypassPermissions: 'Full access', auto: 'Auto', acceptEdits: 'Edits only', plan: 'Plan' };

const flat = (text: string) => text.replace(/\s+/g, '');
const commands: ComposerCommand[] = [
  { name: '/mcp', desc: 'MCP', source: 'local' },
  { name: '/clear', desc: 'Start over', source: 'local' },
  { name: '/review', desc: 'A skill', source: 'claude' },
];
const base = () => ({
  agent: 'claude', commands, busy: false, draft: '',
  onAttach: vi.fn(), onInsert: vi.fn(), onList: vi.fn(), onShowAll: vi.fn(), onRefuse: vi.fn(),
});

describe('the composer + menu (overnight 11-12, C3)', () => {
  it('starts with Attach files…, then Add to Sources… in a workspace chat', () => {
    const p = { ...base(), onAddSource: vi.fn() };
    const items = composerAddItems(p);
    expect(items.map(i => i.label)).toEqual([
      'Attach files…',
      'Add to Sources…',
      'Claude Code’s MCP servers (its own list)',
      'Claude Code’s plugins (its own list)',
      '/clear · Start over',
      'All commands, with Claude Code’s skills…',
    ]);
    items[0].action(); items[1].action();
    expect(p.onAttach).toHaveBeenCalledOnce();
    expect(p.onAddSource).toHaveBeenCalledOnce();
  });
  it('has no Add to Sources… outside a workspace, and another agent gets the pointer row', () => {
    const labels = composerAddItems({ ...base(), agent: 'codex' }).map(i => i.label);
    expect(labels).not.toContain('Add to Sources…');
    expect(labels[1]).toBe('MCP-server and plugin lists: Claude chats only (switch in the model menu)');
    expect(labels.at(-1)).toBe('All commands…');
  });
  it('keeps a draft: a command goes only in an empty box', () => {
    const p = { ...base(), draft: 'half a thought' };
    composerAddItems(p).find(i => i.label.startsWith('/clear'))!.action();
    expect(p.onInsert).not.toHaveBeenCalled();
    expect(p.onRefuse).toHaveBeenCalledWith(KEEP_DRAFT);
    const q = { ...base(), draft: '/cl' };
    composerAddItems(q).find(i => i.label.startsWith('/clear'))!.action();
    expect(q.onInsert).toHaveBeenCalledWith(commands[1]);
  });
  it('turns Claude Code’s own lists off while a turn runs', () => {
    const items = composerAddItems({ ...base(), busy: true });
    expect(items.filter(i => i.disabled).map(i => i.label)).toEqual(['Claude Code’s MCP servers (its own list)', 'Claude Code’s plugins (its own list)']);
  });
});

describe('the access chip (overnight 11-12, C3)', () => {
  it('says Full access in bypass, and the other modes by their short names', () => {
    const text = flat(chatRuntime);
    expect(text).toContain("exportconstPERM_MODE_SHORT_LABELS:Record<ChatPermissionMode,string>={bypassPermissions:'Fullaccess',auto:'Auto',acceptEdits:'Editsonly',default:'Askfirst',plan:'Plan',};");
    expect(text).toContain("exportconstPERM_MODE_ORDER:ChatPermissionMode[]=['acceptEdits','auto','plan','bypassPermissions'];");
    expect(accessChipName(PERM_MODE_SHORT_LABELS.bypassPermissions, PERM_MODE_DANGER.bypassPermissions)).toBe('Access: Full access (unrestricted)');
    expect(accessChipName(PERM_MODE_SHORT_LABELS.acceptEdits, false)).toBe('Access: Edits only');
  });
  it('lists every mode in the pickers’ order, the current one marked, then its note', () => {
    const onPick = vi.fn();
    const options = permModeOptions(PERM_MODE_ORDER, PERM_MODE_SHORT_LABELS, PERM_MODE_DESC, { unavailableModes: { plan: 'No plan mode here.' } } as never);
    const items = accessMenuItems({ options, current: 'auto', danger: PERM_MODE_DANGER, locked: false, note: 'Auto · note', onPick });
    expect(items.map(i => i.label)).toEqual(['Edits only', 'Auto · current', 'Plan (unavailable): No plan mode here.', 'Full access', 'Auto · note']);
    // Each mode keeps its description (or why it can't run) as hover text.
    expect(items.slice(0, 4).map(i => i.title)).toEqual(['e', 'a', 'No plan mode here.', 'b']);
    expect(items.map(i => !!i.disabled)).toEqual([false, true, true, false, false]);
    expect(items[3].danger).toBe(true);
    expect(items[4].informational).toBe(true);
    items[3].action();
    expect(onPick).toHaveBeenCalledWith('bypassPermissions');
  });
  it('ends with the shared line about Hollow’s own cards when given one (per-agent menus, 2026-11-12)', () => {
    const options = permModeOptions(PERM_MODE_ORDER, PERM_MODE_SHORT_LABELS, PERM_MODE_DESC, {} as never);
    const items = accessMenuItems({ options, current: 'auto', danger: PERM_MODE_DANGER, locked: false, note: 'Auto · note', footer: 'Hollow still asks.', onPick: () => {} });
    expect(items.map(i => i.label).slice(-2)).toEqual(['Auto · note', 'Hollow still asks.']);
    expect(items.at(-1)!.informational).toBe(true);
    expect(accessMenuItems({ options, current: 'auto', danger: PERM_MODE_DANGER, locked: false, note: 'n', onPick: () => {} })).toHaveLength(5);
  });
  it('changes nothing while a turn runs, and says so', () => {
    const options = permModeOptions(PERM_MODE_ORDER, PERM_MODE_LABELS, PERM_MODE_DESC, {} as never);
    const items = accessMenuItems({ options, current: 'plan', danger: PERM_MODE_DANGER, locked: true, note: 'Plan · read-only', onPick: () => {} });
    expect(items.slice(0, 4).every(i => i.disabled)).toBe(true);
    expect(items[4].label).toBe('Access can change once this turn finishes. Plan · read-only');
  });
  it('sits after the + at left; the model picker sits at right before the mic', () => {
    const text = flat(chatBody);
    expect(text.indexOf('<ComposerAccessChip/>')).toBeGreaterThan(text.indexOf('<ComposerCommandsagent='));
    expect(text.indexOf('<ChatModelMenu/>')).toBeGreaterThan(text.indexOf('<divclass="chat-composer-right">'));
    expect(text.indexOf('<DictateButtontarget=')).toBeGreaterThan(text.indexOf('<ChatModelMenu/>'));
    // Usage keeps its place by the mic, so its popover's right anchor holds.
    expect(text.indexOf('<ChatUsageMenuonContinue=')).toBeGreaterThan(text.indexOf('<ChatModelMenu/>'));
    expect(text.indexOf('<DictateButtontarget=')).toBeGreaterThan(text.indexOf('<ChatUsageMenuonContinue='));
  });
});

describe('the access chip with a current mode this agent can’t run (lane S re-review of #1304)', () => {
  it('still marks that row current, with why it can’t run', () => {
    const options = permModeOptions(PERM_MODE_ORDER, PERM_MODE_SHORT_LABELS, PERM_MODE_DESC, { unavailableModes: { plan: 'No plan mode here.' } } as never);
    const items = accessMenuItems({ options, current: 'plan', danger: PERM_MODE_DANGER, locked: false, note: 'Plan · note', onPick: () => {} });
    expect(items.map(i => i.label)).toEqual(['Edits only', 'Auto', 'Plan (unavailable) · current: No plan mode here.', 'Full access', 'Plan · note']);
    expect(items.filter(i => i.label.includes(' · current'))).toHaveLength(1);
    expect(items[2].disabled).toBe(true);
  });
});

describe('the access menu keeps a focused row when a turn ends (#1304)', () => {
  it('ActionMenu keys its rows by position, so rebuilt items update rows in place', () => {
    // accessMenuItems builds new objects whenever `locked` flips; a For keyed
    // by object would recreate every button and drop focus.
    expect(actionMenuSource).toContain('<Index each={props.items}>');
    expect(actionMenuSource).not.toContain('<For each={props.items}>');
    const options = permModeOptions(PERM_MODE_ORDER, PERM_MODE_SHORT_LABELS, PERM_MODE_DESC, {} as never);
    const mk = (locked: boolean) => accessMenuItems({ options, current: 'auto', danger: PERM_MODE_DANGER, locked, note: 'n', onPick: () => {} });
    expect(mk(true).length).toBe(mk(false).length);
    expect(mk(true).slice(0, 4).map(i => i.label)).toEqual(mk(false).slice(0, 4).map(i => i.label));
  });
});
