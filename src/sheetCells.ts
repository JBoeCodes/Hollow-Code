/** UN1: pure helpers for the Preview pane's live sheet (`components/SheetGrid.tsx`). */
const NUMBER = /^-?\d[\d,]*(\.\d+)?([eE][+-]?\d+)?%?$/;

/** A cell that reads as a number, so the grid aligns it right. */
export const isNumericCell = (text: string): boolean => text !== '' && NUMBER.test(text);

/** The formula bar's text for a cell: `=B1*2` when it has a formula, else its value. */
export function formulaBarText(value: string, formula: string | undefined): string {
  return formula ? `=${formula}` : value;
}
