import { describe, expect, it } from 'vitest';
import SHORTCUTS_RS from '../src-tauri/src/shortcuts.rs?raw';
import { GO_TARGETS, TYPING_SELECTOR, assignLeader, isTypingTarget, leaderKeyOf, leaderKeyOk, leaderMap, leaderStep, plainKey, type LeaderContext } from './keyboardLeader';
import GO_TO_TS from './goTo.ts?raw';
import { ACTION_BY_GO } from './actionDefs';

const key = (k: string, mods: Partial<{ code: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; repeat: boolean; isComposing: boolean }> = {}) =>
  ({ key: k, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, repeat: false, ...mods });
const cx = (over: Partial<LeaderContext> = {}): LeaderContext =>
  ({ pending: false, enabled: true, typing: false, modal: false, recording: false, keys: leaderMap(), ...over });

describe('the go-to leader', () => {
  it('starts on a plain g only outside fields, modals and the key recorder, with shortcuts on', () => {
    expect(leaderStep(key('g'), cx())).toEqual({ kind: 'start' });
    expect(leaderStep(key('G', { shiftKey: true }), cx()).kind, '⇧G is not g').toBe('ignore');
    expect(leaderStep(key('G'), cx()).kind, 'Caps Lock still starts it').toBe('start');
    expect(leaderStep(key('п', { code: 'KeyG' }), cx()).kind, 'a Cyrillic layout: the physical key').toBe('start');
    expect(leaderStep(key('g', { isComposing: true }), cx()).kind).toBe('ignore');
    expect(leaderStep(key('g', { metaKey: true }), cx()).kind).toBe('ignore');
    expect(leaderStep(key('g', { altKey: true }), cx()).kind, '⌥G types ©').toBe('ignore');
    expect(leaderStep(key('g', { repeat: true }), cx()).kind).toBe('ignore');
    for (const blocked of [{ typing: true }, { modal: true }, { recording: true }, { enabled: false }]) {
      expect(leaderStep(key('g'), cx(blocked)).kind, JSON.stringify(blocked)).toBe('ignore');
    }
    expect(leaderStep(key('t'), cx()).kind, 'a lone t is a page key, not the leader').toBe('ignore');
  });

  it('waiting: a key in effect runs its target, Escape or a chord cancels, a modifier alone waits, the rest is unknown', () => {
    const waiting = cx({ pending: true });
    expect(leaderStep(key('t'), waiting)).toEqual({ kind: 'run', target: 'tasks' });
    expect(leaderStep(key('M'), waiting)).toEqual({ kind: 'run', target: 'main-agent' });
    expect(leaderStep(key('Escape'), waiting)).toEqual({ kind: 'cancel' });
    // A chord, a dead key or composing passes through: ⌘K still opens the palette.
    expect(leaderStep(key('k', { metaKey: true }), waiting)).toEqual({ kind: 'pass' });
    expect(leaderStep(key('Dead'), waiting)).toEqual({ kind: 'pass' });
    expect(leaderStep(key('e', { isComposing: true }), waiting)).toEqual({ kind: 'pass' });
    expect(leaderStep(key('е', { code: 'KeyT' }), waiting), 'Cyrillic е on the T key').toEqual({ kind: 'run', target: 'tasks' });
    expect(leaderStep(key('Shift'), waiting)).toEqual({ kind: 'ignore' });
    expect(leaderStep(key('z'), waiting)).toEqual({ kind: 'unknown', key: 'z' });
    // A field, dialog or recorder that took over meanwhile gets the key.
    for (const over of [{ typing: true }, { modal: true }, { recording: true }]) {
      expect(leaderStep(key('h'), cx({ pending: true, ...over })), JSON.stringify(over)).toEqual({ kind: 'pass' });
    }
  });

  it('keys can be changed: a taken key moves, "" clears, defaults drop out of the saved changes', () => {
    expect(leaderKeyOf('tasks')).toBe('t');
    const { next, displaced } = assignLeader({}, 'tasks', 'h');
    expect(displaced).toBe('home');
    expect(next).toEqual({ tasks: 'h', home: '' });
    expect(leaderMap(next).get('h')).toBe('tasks');
    expect([...leaderMap(next).values()]).not.toContain('home');
    expect(assignLeader(next, 'tasks', 't').next).toEqual({ home: '' });
    expect(assignLeader({}, 'settings', '').next).toEqual({ settings: '' });
    expect(leaderKeyOk('a') && leaderKeyOk('7')).toBe(true);
    expect(leaderKeyOk('A') || leaderKeyOk('ab') || leaderKeyOk('') || leaderKeyOk('[')).toBe(false);
  });

  it('every target has an action, and shortcuts.rs lists the same targets', () => {
    // Each target is a shared action, run by the one dispatcher (actionRun.ts).
    for (const t of GO_TARGETS) expect(ACTION_BY_GO[t.id], t.id).toBeTruthy();
    expect(GO_TO_TS).toContain('m.runGo(id)');
    const start = SHORTCUTS_RS.indexOf('pub const LEADER_TARGETS');
    const pairs = [...SHORTCUTS_RS.slice(start, SHORTCUTS_RS.indexOf('];', start)).matchAll(/\("([a-z-]+)", "([a-z0-9])"\)/g)].map(m => [m[1], m[2]]);
    expect(pairs).toEqual(GO_TARGETS.map(t => [t.id, t.key]));
  });

  it('knows typing targets', () => {
    const el = (tagName: string, extra: Record<string, unknown> = {}) => ({ tagName, isContentEditable: false, closest: () => null, ...extra });
    expect(isTypingTarget(el('INPUT') as never)).toBe(true);
    expect(isTypingTarget(el('TEXTAREA') as never)).toBe(true);
    expect(isTypingTarget(el('DIV', { isContentEditable: true }) as never)).toBe(true);
    expect(isTypingTarget(el('SPAN', { closest: () => ({}) }) as never), 'inside an editor').toBe(true);
    expect(isTypingTarget(el('BUTTON') as never)).toBe(false);
    expect(isTypingTarget(null)).toBe(false);
    // Type-ahead lists and menus count as typing (the custom Select, ActionMenu).
    for (const role of ['combobox', 'listbox', 'menu', 'menuitem', 'textbox']) expect(TYPING_SELECTOR).toContain(`[role="${role}"]`);
    expect(plainKey(key('7'))).toBe('7');
    expect(plainKey(key('Enter'))).toBeNull();
  });
});
