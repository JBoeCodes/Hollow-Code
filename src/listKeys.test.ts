import { describe, expect, it } from 'vitest';
import RAIL from './components/Rail.tsx?raw';
import RAIL_MINIS from './components/RailMinis.tsx?raw';
import HOME from './components/StudioHome.tsx?raw';
import WORKFLOWS from './pages/Workflows.tsx?raw';
import APP from './App.tsx?raw';
import STORE from './store/listKeys.ts?raw';
import CONTEXT_MENU from './components/ContextMenu.tsx?raw';
import { MENU_FOCUS, activeList, between, listKeyOf, stepIndex } from './listKeys';

const k = (key: string, mods: Partial<{ metaKey: boolean; ctrlKey: boolean; altKey: boolean }> = {}) => ({ key, metaKey: false, ctrlKey: false, altKey: false, ...mods });

describe('single-key list navigation (five asks A4)', () => {
  it('maps plain keys only', () => {
    expect(['j', 'k', 'e', 'r', '.'].map(c => listKeyOf(k(c))?.action)).toEqual(['next', 'prev', 'edit', 'rename', 'menu']);
    expect(listKeyOf(k('ArrowDown'))).toEqual({ action: 'next', arrow: true });
    expect(listKeyOf(k('x')), 'no selection model in these lists').toBeNull();
    expect(listKeyOf(k('j', { metaKey: true }))).toBeNull();
    expect(listKeyOf(k('j', { altKey: true })), '⌥J types ∆').toBeNull();
    expect(listKeyOf(k('J'))).toBeNull();
    expect(listKeyOf(k('Enter'))).toBeNull();
  });

  it('steps without wrapping; j from nowhere starts at the top, k at the bottom', () => {
    expect(stepIndex(-1, 5, 'next')).toBe(0);
    expect(stepIndex(-1, 5, 'prev')).toBe(4);
    expect(stepIndex(2, 5, 'next')).toBe(3);
    expect(stepIndex(4, 5, 'next')).toBe(4);
    expect(stepIndex(0, 5, 'prev')).toBe(0);
    expect(stepIndex(0, 0, 'next')).toBe(-1);
    // Focus on a toggle between rows 1 and 2 keeps the place.
    expect(stepIndex(1.5, 5, 'next')).toBe(2);
    expect(stepIndex(1.5, 5, 'prev')).toBe(1);
    expect(between([true, true, false, false])).toBe(1.5);
    expect(stepIndex(between([false, false]), 2, 'prev'), 'before the first row').toBe(0);
    expect(stepIndex(between([true, true]), 2, 'next'), 'after the last row').toBe(1);
  });

  it('acts on the list holding focus, else (j / k only) the page’s default list on screen, never a hidden one', () => {
    const shown = { getClientRects: () => [1] }, hidden = { getClientRects: () => [] };
    const pageList = { ...shown, id: 'page' }, hiddenList = { ...hidden, id: 'hidden' }, rail = { id: 'rail' };
    const doc = { querySelectorAll: () => [hiddenList, pageList] } as unknown as Document;
    const inRail = { closest: () => rail } as unknown as Element;
    const nowhere = { closest: () => null } as unknown as Element;
    expect(activeList(inRail, doc, 'menu')).toBe(rail);
    expect(activeList(nowhere, doc, 'next')).toBe(pageList);
    expect(activeList(nowhere, doc, 'menu'), '. needs a focused row').toBeNull();
    expect(activeList(nowhere, doc, 'next', true), 'the arrows never pull focus into a list').toBeNull();
    expect(activeList(inRail, doc, 'next', true)).toBe(rail);
    expect(activeList(null, { querySelectorAll: () => [hiddenList] } as unknown as Document, 'prev')).toBeNull();
  });

  it('the rail, Home’s workspaces and Automations are key lists; a row’s menu works by keyboard', () => {
    expect(RAIL).toContain('aria-label="Main navigation" data-key-list>');
    // Destinations, project rows and the collapsed Pinned icons; the Recent
    // heading is a row too (overnight 11-04 S1). L8: no conversation rows,
    // Chats heading or Chats mini. L1 (11-12): New chat and Search chats left.
    // The counted Working fold is also reachable through list navigation.
    expect(RAIL.match(/<button data-key-item/g)?.length).toBe(4);
    expect(RAIL_MINIS.match(/<button data-key-item/g)?.length).toBe(1);
    expect(RAIL).not.toContain('data-key-default');
    expect(HOME).toContain('data-key-list data-key-default');
    expect(HOME).toContain('<ListRow class="work-card" keyItem');
    expect(WORKFLOWS).toContain('class="split-list" data-key-list data-key-default');
    // The row's own open button is its key item (it was already a button).
    expect(WORKFLOWS).toContain('<button class="title workflow-open" data-key-item');
    // r and e open the row's menu on that item; the menu takes the keyboard.
    expect(MENU_FOCUS.rename.test('Rename…')).toBe(true);
    expect(MENU_FOCUS.edit.test('Open in a new window')).toBe(true);
    for (const bit of ['takeMenuFocus()', 'role="menuitem"', "e.key === 'ArrowDown'", 'back.focus()']) expect(CONTEXT_MENU, bit).toContain(bit);
    expect(APP).toContain("listen(document, 'keydown', e => { listKeydown(e); });");
    // A page's own handler that already took the key wins.
    expect(STORE).toContain('e.defaultPrevented');
  });
});
