import { expect, test, type Page } from './fixtures/test';

const pane = (page: Page) => page.getByRole('complementary', { name: 'Preview', exact: true });
async function open(page: Page, path: string) {
  await page.evaluate(async path => {
    const module = '/src/store/preview.ts';
    (await import(/* @vite-ignore */ module)).openPreview(path);
  }, path);
}

test('mixed tabs keep PDF position, expose keyboard selection and close independently', async ({ page }) => {
  await page.goto('/?mock=1');
  await open(page, '/mock/vault/notes/report.pdf');
  const pdf = pane(page).getByRole('region', { name: 'PDF preview: report.pdf' });
  await expect(pdf.getByRole('button', { name: 'Next PDF page' })).toBeEnabled();
  await pdf.getByRole('button', { name: 'Next PDF page' }).click();
  await expect(pdf.getByRole('status', { name: 'PDF page count' })).toHaveText('Page 2 of 2');
  await pdf.getByRole('button', { name: 'Zoom in PDF' }).click();
  await pdf.getByRole('button', { name: 'Refresh PDF preview' }).click();
  await expect(pdf.getByRole('status', { name: 'PDF page count' })).toHaveText('Page 2 of 2');
  await expect(pdf.getByRole('button', { name: '125% · Fit PDF to width' })).toHaveText('125%');
  await open(page, '/mock/vault/notes/other.md');
  await open(page, 'https://example.com');
  const tabs = pane(page).getByRole('tablist', { name: 'Preview tabs' });
  await expect(tabs.getByRole('tab')).toHaveCount(3);
  await expect(pane(page).getByRole('tablist')).toHaveCount(1);
  await tabs.getByRole('tab', { name: 'report.pdf' }).click();
  await expect(pdf.getByRole('status', { name: 'PDF page count' })).toHaveText('Page 2 of 2');
  await expect(pdf.getByRole('button', { name: '125% · Fit PDF to width' })).toHaveText('125%');
  await tabs.getByRole('tab', { name: 'report.pdf' }).press('ArrowRight');
  await expect(tabs.getByRole('tab', { name: 'other.md' })).toBeFocused();
  await expect(tabs.getByRole('tab', { name: 'report.pdf' })).toHaveAttribute('aria-selected', 'true');
  await tabs.getByRole('tab', { name: 'other.md' }).press('Enter');
  await pane(page).getByRole('button', { name: 'Close other.md', exact: true }).click();
  await expect(tabs.getByRole('tab')).toHaveCount(2);
  await expect(tabs.getByRole('tab', { name: 'example.com' })).toBeFocused();
  await tabs.getByRole('tab', { name: 'example.com' }).press('Home');
  await expect(tabs.getByRole('tab', { name: 'report.pdf' })).toBeFocused();
  await tabs.getByRole('tab', { name: 'report.pdf' }).press('Enter');
  await expect(pdf.getByRole('status', { name: 'PDF page count' })).toHaveText('Page 2 of 2');
});

test('a shortened PDF retries the same page and offers explicit first-page recovery', async ({ page }) => {
  await page.goto('/?mock=1');
  const path = '/mock/vault/notes/shortened.pdf';
  await open(page, path);
  const pdf = pane(page).getByRole('region', { name: 'PDF preview: shortened.pdf' });
  await pdf.getByRole('button', { name: 'Next PDF page' }).click();
  await expect(pdf.getByRole('status', { name: 'PDF page count' })).toHaveText('Page 2 of 2');
  await page.evaluate(path => localStorage.setItem('mockPdfPreviews', JSON.stringify({ [path]: { pages: 1 } })), path);
  await pdf.getByRole('button', { name: 'Refresh PDF preview' }).click();
  await expect(pdf.getByRole('alert')).toContainText('no longer available');
  await pdf.getByRole('button', { name: 'Retry PDF preview' }).click();
  await expect(pdf.getByRole('alert')).toContainText('no longer available');
  await pdf.getByRole('button', { name: 'Go to first PDF page' }).click();
  await expect(pdf.getByRole('status', { name: 'PDF page count' })).toHaveText('Page 1 of 1');
});
