import { describe, expect, it } from 'vitest';
import whatsNew from '../../docs/WHATS_NEW.md?raw';
import { WHERE_THINGS_WENT, WHERE_THINGS_WENT_UNTIL, whereThingsWentShown } from './whereThingsWent';

describe('the Where things went header link', () => {
  it('shows through the last day and goes after it (injected clock)', () => {
    expect(WHERE_THINGS_WENT_UNTIL).toBe('2026-11-26');
    expect(whereThingsWentShown(new Date(2026, 10, 12, 9, 0))).toBe(true);
    expect(whereThingsWentShown(new Date(2026, 10, 26, 23, 59))).toBe(true);
    expect(whereThingsWentShown(new Date(2026, 10, 27, 0, 0))).toBe(false);
    expect(whereThingsWentShown(new Date(2027, 0, 5))).toBe(false);
  });

  it('has a short hint and no dashes in the list', () => {
    const src = import.meta.glob('../pages/Settings.tsx', { query: '?raw', import: 'default', eager: true });
    const text = Object.values(src)[0] as string;
    const hint = /title="(Where every old setting is now)"/.exec(text)?.[1] ?? '';
    expect(hint.length).toBeGreaterThan(0);
    expect(hint.length).toBeLessThanOrEqual(90);
    expect(JSON.stringify(WHERE_THINGS_WENT)).not.toMatch(/[–—]/);
  });

  it('matches the What’s new entry, row for row', () => {
    const at = whatsNew.indexOf('### Where everything in Settings went');
    expect(at).toBeGreaterThan(-1);
    const entry = whatsNew.slice(at, whatsNew.indexOf('\n### ', at + 5));
    for (const g of WHERE_THINGS_WENT) {
      expect(entry, g.from).toContain(`- ${g.from}`);
      for (const r of g.rows) expect(entry, r.what).toContain(`${r.what}: ${r.now}`);
    }
  });
});
