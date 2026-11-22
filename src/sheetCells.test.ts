import { describe, expect, it } from 'vitest';
import { formulaBarText, isNumericCell } from './sheetCells';

describe('sheet cells (UN1)', () => {
  it('reads numbers, thousands, decimals, exponents and percents as numeric', () => {
    for (const n of ['1200', '-3', '1,200.50', '0.18', '1.5E+20', '12%']) expect(isNumericCell(n)).toBe(true);
    for (const t of ['', 'North', '12 units', '2026-11-19', '#DIV/0!']) expect(isNumericCell(t)).toBe(false);
  });
  it('shows a formula with its equals sign, else the value', () => {
    expect(formulaBarText('85', 'B1*2')).toBe('=B1*2');
    expect(formulaBarText('Region', undefined)).toBe('Region');
    expect(formulaBarText('Region', '')).toBe('Region');
  });
});
