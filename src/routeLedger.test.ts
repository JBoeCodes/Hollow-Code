import { describe, expect, it } from 'vitest';
import paletteSource from './components/CommandPalette.tsx?raw';
import ledgerDoc from '../docs/ROUTE_LEDGER.md?raw';
import actionsDoc from '../docs/ACTIONS.md?raw';
import { COMMANDS } from './shortcuts';
import { GO_TARGETS } from './keyboardLeader';
import { ACTION_DEFS, CONDITIONS } from './actionDefs';
import { RIGHT_CLICK, ROUTES, ROUTE_BY_ID, accelLanding, acceleratorsOf, renderActions, renderRouteLedger, routesFor, type Target } from './routeLedger';

// Consolidation D (2026-10-13), plan completion L4 (N4–N8): no capability is
// shortcut- or palette-only. Every accelerator is a shared action whose row
// has a visible click path; every row's path is executable (walked by
// tests/smoke/route-ledger.journey.spec.ts) and says when it is offered.
const specs = import.meta.glob('../tests/smoke/*.spec.ts', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
const spec = (file: string) => specs[`../tests/smoke/${file}`];
const steps = (x: Target): Target[] => [x, ...(x.in ? steps(x.in) : [])];

describe('route ledger', () => {
  it('walks waiting filters on Needs you', () => {
    for (const [id, label] of [['review.decision', 'Decisions'], ['review.results', 'Results'], ['review.failures', 'Failures']]) {
      expect(ROUTE_BY_ID[id].walk.at(-1)).toMatchObject({ name: label, in: { role: 'navigation', name: 'Waiting filters' } });
    }
    expect(ROUTE_BY_ID['review.incoming']).toBeUndefined();
  });
  it('finds the registries it checks', () => {
    // 36 since COO rebuild L4 took the workspace section and canvas-view actions.
    expect(ACTION_DEFS.filter(d => d.palette).length).toBeGreaterThan(34);
    expect(GO_TARGETS.length).toBe(14);
    expect(COMMANDS.length).toBeGreaterThan(20);
  });

  it.each([
    ['go', GO_TARGETS.map(g => g.id)],
    ['keys', COMMANDS.map(c => c.id)],
  ] as const)('every %s accelerator is a shared action reaching a capability with a visible path', (kind, ids) => {
    expect(ids.filter(id => routesFor(kind, id).length !== 1)).toEqual([]);
  });

  it('every palette action is a shared action reaching a capability with a visible path', () => {
    // The palette builds its fixed rows from ACTION_DEFS; no row of its own.
    const literal = [...paletteSource.matchAll(/id: '(act:[a-z0-9-]+)'/g)].map(m => m[1]);
    expect(literal).toEqual([]);
    expect(ACTION_DEFS.filter(d => d.palette && routesFor('palette', d.palette).length !== 1).map(d => d.id)).toEqual([]);
  });

  it('every shared action names a real row', () => {
    expect(ACTION_DEFS.filter(d => !ROUTE_BY_ID[d.route]).map(d => `${d.id} → ${d.route}`)).toEqual([]);
  });

  it('has unique ids, and paths made of clicks', () => {
    expect(new Set(ROUTES.map(r => r.id)).size).toBe(ROUTES.length);
    // (Settings' own "Keyboard shortcuts" section is a place, not a key.)
    const keyish = ROUTES.filter(r => /palette|⌘|⌥|⇧|\bg [a-z]\b|shortcut|Cmd|Ctrl/i.test(r.path.replace('Settings → Keyboard shortcuts', 'Settings → (keys section)'))).map(r => r.id);
    expect(keyish).toEqual([]);
  });

  it('every row has an executable walk: named controls, then a landmark', () => {
    const bad = ROUTES.filter(r => !r.walk.length || ![...r.walk, r.lands].flatMap(steps).every(x => x.role && x.name.trim())).map(r => r.id);
    expect(bad).toEqual([]);
    // A walk is clicks (and typing into a field): no key presses, no palette.
    const viaPalette = ROUTES.filter(r => r.walk.flatMap(steps).some(x => /Command palette|Search anything/.test(x.name)) && r.id !== 'find.search').map(r => r.id);
    expect(viaPalette).toEqual([]);
  });

  it('a conditional row says what the person sees while it is unavailable', () => {
    expect(ROUTES.filter(r => !!r.when !== !!r.unavailable).map(r => r.id)).toEqual([]);
    // Every condition a shared action waits for has its words.
    for (const d of ACTION_DEFS.filter(d => d.when)) expect(CONDITIONS[d.when!].unavailable, d.id).toMatch(/\S/);
  });

  it('a row landing elsewhere for an accelerator names a real accelerator of its own', () => {
    for (const r of ROUTES) {
      if (!r.accelLands || 'role' in r.accelLands) continue;
      const mine = Object.values(acceleratorsOf(r.id)).flat();
      for (const id of Object.keys(r.accelLands)) expect(mine, `${r.id}: ${id}`).toContain(id);
      expect(accelLanding(r, 'nothing')).toBe(r.lands);
    }
  });

  it('palette rows that run their own variant are exactly the ones whose action says why', () => {
    // CommandPalette.tsx PALETTE_OWN: `'act:x': { run: …` entries.
    const own = [...paletteSource.matchAll(/'(act:[a-z0-9-]+)': \{ run:/g)].map(m => m[1]).sort();
    expect(own).toEqual(ACTION_DEFS.filter(d => d.paletteOwn).map(d => d.palette!).sort());
  });

  it('names a real click-only test for every capability', () => {
    const missing = ROUTES.flatMap(r => r.verified
      .filter(v => !spec(v.spec)?.includes(`test('${v.test}`))
      .map(v => `${r.id}: ${v.spec} “${v.test}”`));
    expect(missing).toEqual([]);
  });

  it('every right-click menu in src/ is listed with its visible route and parity test', () => {
    const sources = import.meta.glob('./**/*.tsx', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
    // Sites that open a menu on right-click (not ContextMenu itself, which
    // only swallows the browser's, nor ui.tsx's ListRow, which passes it on).
    const sites = Object.entries(sources).filter(([f, text]) => /onContextMenu=\{[^}]*(openContextMenu|Menu\(|menu\b)/.test(text) && !/ContextMenu\.tsx$|\/ui\.tsx$/.test(f))
      .map(([f]) => `src/${f.slice(2)}`).sort();
    expect(sites).toEqual([...new Set(RIGHT_CLICK.map(r => r.file))].sort());
    for (const r of RIGHT_CLICK) expect(spec('context-menus.spec.ts'), r.test).toContain(`test('${r.test}`);
  });

  it('the walking spec walks every row', () => {
    const walker = spec('route-ledger.journey.spec.ts');
    expect(walker).toContain('for (const r of ROUTES)');
    expect(walker).toContain('await walk(page, r.walk)');
  });

  it('docs/ROUTE_LEDGER.md and docs/ACTIONS.md are current (node scripts/route-ledger.mjs)', () => {
    expect(ledgerDoc).toBe(renderRouteLedger());
    expect(actionsDoc).toBe(renderActions());
  });
});
