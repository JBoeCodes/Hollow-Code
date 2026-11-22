import { describe, expect, it } from 'vitest';
import { MIN_ROOM, TITLE_BAR, roomAbove } from './popoverRoom';
import MODEL_MENU from './components/ChatModelMenu.tsx?raw';

describe('an upward popover measures its room (overnight 11-04, C2)', () => {
  it('stops at the lowest clipping ancestor', () => {
    // Library's vault chat at 720x480: the pane clips at 182, the trigger sits at 430.
    expect(roomAbove(430, [0, 182, 137])).toBe(430 - 182 - 12);
  });
  it('never opens over the title bar', () => {
    expect(roomAbove(600, [])).toBe(600 - TITLE_BAR - 12);
    expect(roomAbove(600, [10, 20])).toBe(600 - TITLE_BAR - 12);
  });
  it('is never negative, and the menu keeps a usable floor', () => {
    expect(roomAbove(50, [40])).toBe(0);
    expect(MIN_ROOM).toBeGreaterThanOrEqual(96);
  });
  it('is what the model menu uses (its CSS cap is checked by model-menu.spec.ts:57)', () => {
    expect(MODEL_MENU).toContain('roomAbove(');
    expect(MODEL_MENU).toContain("'--model-pop-room'");
  });
});
