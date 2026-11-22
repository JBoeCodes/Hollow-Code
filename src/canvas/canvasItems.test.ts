import { describe, it, expect } from 'vitest';
import { itemRows, itemKey } from './canvasItems';
import { parseChildMessage } from '../canvasBridge';

describe('saved canvas item identities', () => {
  it('keeps five checklist rows ordered and removes status labels', () => {
    const result = itemRows([{ kind: 'list', ordered: false, items: ['Not done: First', 'Done: Second', 'Third', 'Fourth', 'Fifth'] }]);
    expect(result.rows.map(r => r.text)).toEqual(['First', 'Second', 'Third', 'Fourth', 'Fifth']);
    expect(result.rows.map(r => r.index)).toEqual([0, 1, 2, 3, 4]);
    expect(result.omitted).toBe(0);
  });
  it('bounds rows without recycling indices and reports omitted items', () => {
    const result = itemRows([{ kind: 'list', ordered: false, items: ['', ...Array.from({ length: 52 }, (_, n) => `Item ${n}`)] }]);
    expect(result.rows).toHaveLength(50);
    expect(result.rows[0].index).toBe(1);
    expect(result.omitted).toBe(2);
    expect(itemKey('w', 'c', 2, 1)).not.toBe(itemKey('w', 'c', 1, 1));
  });
  it('never gives canvas scripts an import or dialog message', () => {
    for (const kind of ['canvas-items', 'add-items', 'import-tasks', 'import-plan']) {
      expect(parseChildMessage({ __hc: 1, kind })).toEqual({ ok: false, reason: 'unknown-kind' });
    }
  });
  it('copies table data rows without importing the header', () => {
    expect(itemRows([{ kind: 'table', caption: 'Tasks', head: ['Task', 'Owner'], rows: [['Review', 'Pat']] }]).rows.map(r => r.text)).toEqual(['Review · Pat']);
  });
});
