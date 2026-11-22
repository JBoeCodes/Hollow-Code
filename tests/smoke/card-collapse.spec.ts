import { expect, test } from './fixtures/test';
import { seedSubagents } from './subagentsFixture';

for (const width of [1440, 800]) for (const theme of ['light', 'dark']) {
  test(`compact card ${width} ${theme}`, async ({ page }, info) => {
    await page.setViewportSize({ width, height: 900 });
    await page.addInitScript(t => localStorage.setItem('theme', t), theme);
    await seedSubagents(page);
    await page.goto('/?mock=1&demo=1');
    await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
    await expect(page.locator('.chat-opening')).toHaveCount(0);
    await expect(page.getByRole('textbox', { name: 'Chat message', exact: true })).toBeVisible();
    await page.evaluate(async () => {
      const path = '/src/store/chatRuntime.ts';
      const runtime = await import(/* @vite-ignore */ path);
      runtime.setActiveChatRecall(Array.from({ length: 8 }, (_, i) => ({
        subject: `Project decision ${i + 1}`, statement: 'Keep the visible routes and original project data. '.repeat(8),
        path: `learnings/decision-${i}.md`, targetPath: '*', score: 8,
      })));
    });
    const card = page.locator('#side-panel-card');
    await expect(card).toContainText('Recall');
    const revision = process.env.CARD_CAPTURE_REV ?? 'after';
    await page.screenshot({ path: info.outputPath(`${revision}-${width}-${theme}.png`) });
    if (revision === 'before') return;
    const headers = card.locator('.side-card-toggle');
    await expect(headers).toHaveCount(4);
    for (const header of await headers.all()) await expect(header).toHaveAttribute('aria-expanded', 'false');
    expect((await card.boundingBox())!.height).toBeLessThan(300);
    const recall = card.getByRole('button', { name: 'Recall 8', exact: true });
    await recall.focus();
    await page.keyboard.press('Enter');
    await expect(recall).toHaveAttribute('aria-expanded', 'true');
    await expect(card.getByRole('button', { name: 'Not relevant', exact: true })).toHaveCount(0);
    const item = card.getByRole('button', { name: 'Project decision 1', exact: true });
    await item.focus();
    await page.keyboard.press('Space');
    await expect(item).toHaveAttribute('aria-expanded', 'true');
    await expect(card.getByRole('button', { name: 'Outdated', exact: true })).toBeVisible();
    await card.getByRole('button', { name: /^Outputs \d/ }).click();
    await expect(recall).toHaveAttribute('aria-expanded', 'true');
    await expect(card.locator('#side-card-outputs-body .side-card-file').last()).toBeInViewport();
    const body = card.locator('#side-card-recall-body');
    expect(await body.evaluate(el => el.scrollHeight > el.clientHeight)).toBe(true);
    expect((await card.boundingBox())!.height).toBeLessThan(650);
    await page.screenshot({ path: info.outputPath(`expanded-${width}-${theme}.png`) });
    await page.getByRole('button', { name: 'Project files', exact: true }).click();
    await page.getByRole('complementary', { name: 'Preview', exact: true }).getByRole('button', { name: /Close.*[Pp]review/ }).click();
    await expect(recall).toHaveAttribute('aria-expanded', 'true');
    await expect(card.getByRole('button', { name: /^Outputs \d/ })).toHaveAttribute('aria-expanded', 'true');
    await expect(card.getByRole('button', { name: /^Sources \d/ })).toHaveAttribute('aria-expanded', 'false');
    await recall.click();
    await expect(recall).toBeFocused();
    await page.setViewportSize({ width: 720, height: 480 });
    for (const header of await headers.all()) {
      await header.scrollIntoViewIfNeeded();
      await expect(header).toBeInViewport();
    }
    await page.setViewportSize({ width, height: 900 });
    await page.reload();
    await expect(card.getByRole('button', { name: /^Outputs \d/ })).toHaveAttribute('aria-expanded', 'true');
  });
}
