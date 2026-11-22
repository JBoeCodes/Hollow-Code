/**
 * A table in a reply, as the clipboard wants it (overnight 11-07, D1). The
 * "Copy table" button puts two things on it: tab-separated text, which Numbers
 * and Sheets paste as cells, and the table's HTML, which keeps the header and
 * the alignment for Pages, Docs and mail. A cell's tabs and line breaks become
 * single spaces, so one cell can never split into two. Reply-only: the caller
 * is the lazy `ReplyMarkdown`.
 */

/** One cell as a single line of text. */
export const cellText = (text: string | null | undefined): string => (text ?? '').replace(/\s+/g, ' ').trim();

/** Rows of cells as tab-separated lines (no trailing newline). */
export function tableToTsv(rows: readonly (readonly string[])[]): string {
  return rows.map(row => row.map(cellText).join('\t')).join('\n');
}

/** The elements that start a new line inside a cell, besides `<br>`. */
const BLOCKS = 'div, p, li, ul, ol, blockquote, pre, h1, h2, h3, h4, h5, h6, section';

/** A cell's text with each line break (`<br>`) and each block's start and end
 *  as a space: `textContent` alone joins the lines ("first<br>second" would
 *  read "firstsecond", and so would two `<div>`s, the items of a list, or the
 *  words before a nested list and its first item). `cellText` collapses the
 *  spaces this leaves. */
function cellContent(cell: HTMLTableCellElement): string {
  const copy = cell.cloneNode(true) as HTMLElement;
  for (const br of copy.querySelectorAll('br')) br.replaceWith(' ');
  for (const block of copy.querySelectorAll(BLOCKS)) { block.prepend(' '); block.append(' '); }
  return copy.textContent ?? '';
}

/** Every row of the table, header first, as cell text. */
export function tableRows(table: HTMLTableElement): string[][] {
  return [...table.rows].map(row => [...row.cells].map(cellContent));
}

const watched = new WeakSet<Element>();

/** Keep each table of `root` scrolled where the reader left it when the markup
 *  is rebuilt (a growing table's is, about every 80 ms): `saved` holds the
 *  offsets, put back now and kept current as a box scrolls. The boxes are
 *  matched by order, and a streaming reply only ever adds tables after these.
 *  A box this already watches was kept as it was (`keepBlocks`), so it keeps
 *  its own place and gets no second listener. */
export function keepTableScroll(root: ParentNode, saved: number[]): void {
  root.querySelectorAll<HTMLElement>('.reply-table-scroll').forEach((box, i) => {
    if (watched.has(box)) return;
    watched.add(box);
    if (saved[i]) box.scrollLeft = saved[i];
    box.addEventListener('scroll', () => { saved[i] = box.scrollLeft; }, { passive: true });
  });
}

/** The label for the nth of several tables ("Copy table 2 of 3"), or the plain one. */
export const tableLabel = (verb: string, index: number, total: number): string =>
  total > 1 ? `${verb} ${index + 1} of ${total}` : verb;

/** Put the table on the clipboard as text and HTML. False when the clipboard refused both. */
export async function copyTable(table: HTMLTableElement): Promise<boolean> {
  const text = tableToTsv(tableRows(table));
  try {
    if (typeof ClipboardItem !== 'undefined' && navigator.clipboard.write) {
      await navigator.clipboard.write([new ClipboardItem({
        'text/plain': new Blob([text], { type: 'text/plain' }),
        'text/html': new Blob([table.outerHTML], { type: 'text/html' }),
      })]);
      return true;
    }
  } catch { /* fall through: some webviews allow only plain text */ }
  try { await navigator.clipboard.writeText(text); return true; } catch { return false; }
}
