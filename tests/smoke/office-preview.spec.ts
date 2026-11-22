import { expect, test, type Page } from './fixtures/test';

// B1b (ENG-104): Word, Excel and PowerPoint files show their text in the
// Preview pane (office_preview), with a header that says it is the text only
// and an Open in app control, or a clear fallback when the read fails.
const pane = (page: Page) => page.getByRole('complementary', { name: 'Preview', exact: true });
async function open(page: Page, path: string) {
  await page.evaluate(async path => {
    const module = '/src/store/preview.ts';
    (await import(/* @vite-ignore */ module)).openPreview(path);
  }, path);
}

test('a Word file shows its paragraphs under a text-only header', async ({ page }) => {
  await page.goto('/?mock=1');
  await open(page, '/mock/vault/notes/report.docx');
  const doc = pane(page).getByRole('region', { name: 'Word preview: report.docx' });
  await expect(doc.getByText('Revenue grew 12% on the quarter, led by the North region.')).toBeVisible();
  await expect(doc.getByText('Preview of the text only; open it in its app for layout.')).toBeVisible();
  await expect(doc.getByRole('button', { name: 'Open in app' })).toBeVisible();
});

test('an Excel file shows a live sheet with tabs and says what was cut', async ({ page }) => {
  await page.goto('/?mock=1');
  await open(page, '/mock/vault/notes/data.xlsx');
  const book = pane(page).getByRole('region', { name: 'Excel preview: data.xlsx' });
  const q3 = book.getByRole('region', { name: 'Sheet Q3' });
  await expect(q3.getByRole('columnheader', { name: 'B' })).toBeVisible();
  await expect(q3.getByRole('row', { name: '2 North 1200 0.18' })).toBeVisible();
  await expect(book.getByRole('note')).toContainText('first 200 rows with cells only');
  await book.getByRole('tab', { name: 'Notes' }).click();
  await expect(book.getByRole('region', { name: 'Sheet Notes' }).getByRole('cell', { name: 'Figures in thousands' })).toBeVisible();
});

// UN1: selecting a cell shows its formula (or value) in the formula bar; arrow keys move.
test('an Excel cell shows its formula in the formula bar and arrow keys move the pick', async ({ page }) => {
  await page.goto('/?mock=1');
  await open(page, '/mock/vault/notes/data.xlsx');
  const book = pane(page).getByRole('region', { name: 'Excel preview: data.xlsx' });
  const q3 = book.getByRole('region', { name: 'Sheet Q3' });
  await q3.getByRole('cell', { name: '2150' }).click();
  await expect(book.getByLabel('Formula bar')).toHaveText('=SUM(B2:B3)');
  await expect(book.getByLabel('Selected cell')).toHaveText('B4');
  await page.keyboard.press('ArrowUp');
  await expect(book.getByLabel('Formula bar')).toHaveText('950');
  await expect(book.getByLabel('Selected cell')).toHaveText('B3');
});

test('a PowerPoint file shows each slide with its text', async ({ page }) => {
  await page.goto('/?mock=1');
  await open(page, '/mock/vault/notes/deck.pptx');
  const deck = pane(page).getByRole('region', { name: 'PowerPoint preview: deck.pptx' });
  await expect(deck.getByRole('region', { name: 'Slide 1' }).getByText('Launch plan')).toBeVisible();
  await expect(deck.getByRole('region', { name: 'Slide 2' }).getByText('Beta in March, launch in May')).toBeVisible();
});

test('an Office file that cannot be read says why and offers its app', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.evaluate(() => localStorage.setItem('mockOfficePreviewError', JSON.stringify({
    'broken.docx': "Hollow couldn't read the text in this file: not a readable Office file. Open it in its app instead.",
  })));
  await open(page, '/mock/vault/notes/broken.docx');
  const doc = pane(page).getByRole('region', { name: 'Word preview: broken.docx' });
  const alert = doc.getByRole('alert');
  await expect(alert).toContainText('Couldn’t preview this file');
  await expect(alert).toContainText('not a readable Office file');
  await expect(alert.getByRole('button', { name: 'Retry' })).toBeVisible();
  await expect(doc.getByRole('button', { name: 'Open in app' })).toBeVisible();
});
