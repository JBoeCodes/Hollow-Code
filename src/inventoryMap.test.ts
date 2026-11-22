import { describe, expect, it } from 'vitest';
import reconciliationDoc from '../docs/INVENTORY_RECONCILIATION.md?raw';
import slashSource from './components/chat/slashCommands.ts?raw';
// @ts-expect-error: a Node script without types
import { reconcile, renderReconciliation } from '../scripts/inventory-reconcile.mjs';
import { COMMAND_ONLY, FILES } from './inventoryMap';

// Plan completion L4 (review V02, matrix N1): the review's control and
// action inventory, re-scanned on today's source and reconciled. Every file
// with controls or actions is classified, every baseline action is still
// there, a shared action, or explained, and every command-only way in names
// its visible route.
const root = decodeURIComponent(new URL('..', import.meta.url).pathname);

describe('inventory reconciliation', async () => {
  const d = await reconcile(root);

  it('scans about as much as the review did', () => {
    expect(d.scan.controls.length).toBeGreaterThan(1400);
    expect(d.fileRows.length).toBeGreaterThan(150);
  });

  it('classifies every file the scan finds, and lists none that is gone', () => {
    expect(d.fileRows.filter((r: { class: string }) => r.class === 'UNCLASSIFIED').map((r: { file: string }) => r.file)).toEqual([]);
    expect(d.stale).toEqual([]);
  });

  it('a capability file names real ledger rows; others say what they are', () => {
    expect(d.routesMissing).toEqual([]);
    for (const [file, c] of Object.entries(FILES)) {
      if (c.class === 'capability') expect(c.routes?.length, file).toBeGreaterThan(0);
      else expect(c.note, file).toMatch(/\S/);
    }
  });

  it('accounts for every action object of the review', () => {
    expect(d.actionRows.filter((a: { status: string }) => a.status === 'UNRECONCILED').map((a: { id: string }) => a.id)).toEqual([]);
    expect(d.actionRows.length).toBe(215);
  });

  it('explains every file removed since the review', () => {
    expect(d.gone.filter((g: { reason: string | null }) => !g.reason).map((g: { file: string }) => g.file)).toEqual([]);
  });

  it('gives every slash command a visible route', () => {
    // LOCAL_COMMANDS (components/chat/slashCommands.ts, moved out of ChatBody.tsx
    // by plan L3 X17), by name: a new command fails here until placed.
    const block = slashSource.slice(slashSource.indexOf('const LOCAL_COMMANDS'), slashSource.indexOf('];', slashSource.indexOf('const LOCAL_COMMANDS')));
    const names = [...block.matchAll(/name: '(\/[a-z]+)'/g)].map(m => m[1]);
    expect(names.length).toBeGreaterThan(8);
    for (const cmd of names) {
      expect(COMMAND_ONLY.some(c => c.surface === 'Composer' && c.command.split(/,\s*/).includes(cmd)), cmd).toBe(true);
    }
    for (const c of COMMAND_ONLY) expect(c.visible, c.command).toMatch(/\S/);
  });

  it('docs/INVENTORY_RECONCILIATION.md is current (node scripts/inventory-reconcile.mjs)', () => {
    expect(reconciliationDoc).toBe(renderReconciliation(d));
  });
});
