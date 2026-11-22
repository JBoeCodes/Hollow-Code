import { afterEach, describe, expect, it, vi } from 'vitest';
const spies = vi.hoisted(() => ({ run: vi.fn(), go: vi.fn(), takes: vi.fn(() => false), modal: vi.fn(() => false), palette: vi.fn((_id: string) => false), runKey: vi.fn((_id: string) => true) }));
vi.mock('./nativeEvents', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock('./modalGuard', () => ({ modalOpen: spies.modal, MODAL_SEL: '.modal:not(.is-closing)' }));
vi.mock('./actionRun', () => ({ runMenuOrKey: spies.runKey }));
vi.mock('./store/browser', () => ({ browserGo: spies.go, browserTakesNav: spies.takes }));
vi.mock('./store/overlays', () => ({ runPaletteCommand: spies.palette }));
const flash = vi.hoisted(() => vi.fn());
vi.mock('./store/snackbars', () => ({ flashFooter: flash }));
import { dialogBlockingAdd, runMenuAction, runNativeMenu } from './appMenu';
import { addSummoned, openQuickCapture } from './modals/addState';

// P1-49: a menu item clicked behind a dialog used to do nothing at all.
describe('a menu item behind a dialog', () => {
  it('says why it did nothing', () => {
    spies.modal.mockReturnValue(true);
    runMenuAction('go-tasks');
    expect(flash).toHaveBeenCalledWith('Close the dialog first', expect.any(Number));
    spies.modal.mockReturnValue(false);
  });
});

// P1-48: behind the open palette (a modal) the menu's Search… and Jump still
// reach it: the palette takes them before the modal guard.
describe('the open palette and the menu', () => {
  it('Search… and Jump go to the palette when it takes them', () => {
    spies.palette.mockImplementation(id => id === 'search' || id === 'jump');
    spies.modal.mockReturnValue(true);
    runMenuAction('search');
    runMenuAction('jump');
    expect(spies.palette.mock.calls).toEqual([['search'], ['jump']]);
    expect(spies.modal).not.toHaveBeenCalled();
  });
});

// S5 review: ⌘⌥⇧T / ⌘⌥⇧I from another app while the Add sheet is open said
// "Close the dialog first" about Add itself; they now reach it (it switches
// kind), and any other dialog is named.
describe('the Add keys behind a dialog', () => {
  type El = { classList: { contains: (c: string) => boolean }; matches: (s: string) => boolean; querySelector: (s: string) => unknown; getAttribute: (a: string) => string | null };
  const el = (cls: string, attrs: Record<string, string> = {}): El => ({
    classList: { contains: c => c === cls }, matches: () => false,
    querySelector: () => ({ getAttribute: (a: string) => attrs[a] ?? null }), getAttribute: () => null,
  });
  const doc = (open: El[], titles: Record<string, string> = {}) =>
    ({ querySelectorAll: () => open, getElementById: (id: string) => (titles[id] ? { textContent: titles[id] } : null) }) as unknown as Document;
  const settle = () => new Promise(r => setTimeout(r, 0));

  it('names nothing when the only dialog is the Add sheet, and names any other', () => {
    expect(dialogBlockingAdd(doc([el('add-sheet')]))).toBeNull();
    expect(dialogBlockingAdd(doc([el('modal', { 'aria-labelledby': 't' })], { t: 'Edit task' }))).toBe('“Edit task”');
    expect(dialogBlockingAdd(doc([el('modal', { 'aria-label': 'Command palette' })]))).toBe('“Command palette”');
    expect(dialogBlockingAdd(doc([el('confirm-backdrop')]))).toBe('the open dialog');
  });

  it('reach an open Add sheet as a summon, and say what to close otherwise', async () => {
    const g = globalThis as { document?: Document };
    spies.modal.mockReturnValue(true);
    g.document = doc([el('add-sheet')]);
    let summoned: boolean | null = null;
    spies.runKey.mockImplementation(() => { openQuickCapture(); summoned = addSummoned(); return true; });
    runMenuAction('quick-capture');
    await settle();
    expect(spies.runKey).toHaveBeenCalledWith('quick-capture');
    expect(summoned, 'the opening knows a system-wide key made it').toBe(true);
    expect(flash).not.toHaveBeenCalled();
    g.document = doc([el('add-sheet'), el('modal', { 'aria-labelledby': 't' })], { t: 'Edit task' });
    runMenuAction('quick-ingest');
    await settle();
    expect(flash).toHaveBeenCalledWith('Close “Edit task” first, then press the key again', expect.any(Number));
    expect(spies.runKey).toHaveBeenCalledTimes(1);
    // Any other item still says the old words.
    runMenuAction('go-tasks');
    expect(flash).toHaveBeenLastCalledWith('Close the dialog first', expect.any(Number));
    delete g.document;
  });
});

afterEach(() => { vi.clearAllMocks(); spies.takes.mockReturnValue(false); spies.modal.mockReturnValue(false); spies.palette.mockImplementation(() => false); });

// P1-47: the menu's Back and Forward (⌘[ ⌘]) reach the menu when the side
// panel's native page has the keys; there they step the page, not Hollow.
describe('the native menu', () => {
  it('Back and Forward step the browser page while it has the keys', () => {
    spies.takes.mockReturnValue(true);
    runNativeMenu('go-back', spies.run);
    runNativeMenu('go-forward', spies.run);
    expect(spies.go.mock.calls).toEqual([['back'], ['forward']]);
    expect(spies.run).not.toHaveBeenCalled();
  });
  it('anywhere else they move Hollow, and other items never go to the page', () => {
    runNativeMenu('go-back', spies.run);
    spies.takes.mockReturnValue(true);
    runNativeMenu('go-tasks', spies.run);
    expect(spies.go).not.toHaveBeenCalled();
    expect(spies.run.mock.calls).toEqual([['go-back'], ['go-tasks']]);
  });
  it('behind a modal the page stays put (runMenuAction refuses the rest)', () => {
    spies.takes.mockReturnValue(true);
    spies.modal.mockReturnValue(true);
    runNativeMenu('go-back', spies.run);
    expect(spies.go).not.toHaveBeenCalled();
    expect(spies.run.mock.calls).toEqual([['go-back']]);
  });
});
