import { expect, test, type Page } from './fixtures/test';
import { choose, openAdvancedGroup } from './choose';

async function openSettingsSection(page: Page, label: string, value: string) {
  const mobileSelect = page.getByRole('combobox', { name: 'Settings section' });
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(mobileSelect, value);
    await expect(mobileSelect).toHaveAttribute('data-value', value);
    return;
  }
  // Scope to the index nav — merged sections put buttons with matching
  // labels (e.g. "Open Connectors") into section bodies.
  await page.locator('.settings-index').getByRole('button', { name: new RegExp(label, 'i') }).click();
}

test('capability doctor keeps an explicit decline across reloads', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: /Settings/ }).first().click();
  // The doctor lives in Advanced's This Mac group since Settings PR 3.
  await openSettingsSection(page, 'Advanced', 'advanced');
  await openAdvancedGroup(page, 'this-mac');
  await expect(page.getByRole('heading', { name: 'Capability doctor' })).toBeVisible();

  const plaud = page.locator('.capability-doctor-row').filter({ hasText: 'Plaud' });
  await page.getByRole('button', { name: 'Run all checks' }).click();
  // The backup diagnostic remains intentionally partial in the mock: running
  // connector checks must not turn a cloud-looking path into protection.
  await expect(page.locator('.capability-doctor-summary')).toContainText('4 working · 1 needs attention');
  await plaud.getByRole('button', { name: 'Not using' }).click();
  await expect(plaud).toContainText('declined');

  await page.reload();
  await page.getByRole('button', { name: /Settings/ }).first().click();
  await openSettingsSection(page, 'Advanced', 'advanced');
  await openAdvancedGroup(page, 'this-mac');
  const restoredPlaud = page.locator('.capability-doctor-row').filter({ hasText: 'Plaud' });
  await expect(restoredPlaud).toContainText('declined');
  await expect(restoredPlaud.getByRole('button', { name: 'Use this' })).toBeVisible();
});

test('operator context review is an explicit freshness action', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: /Settings/ }).first().click();
  await openSettingsSection(page, 'Personalization', 'personalization');

  const identity = page.locator('.about-you-note').filter({ hasText: 'Identity' });
  await expect(identity).toContainText('review overdue');
  await identity.getByRole('button', { name: 'Mark reviewed' }).click();
  await expect(identity).toContainText('current');
});

test('about you fills itself from what agents know and asks for the rest', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: /Settings/ }).first().click();
  await openSettingsSection(page, 'Personalization', 'personalization');

  // First visit reads on its own (the mock store starts never scanned).
  await expect(page.locator('.about-you-sources')).toContainText('12 Claude memories');
  await expect(page.locator('.about-you-meta')).toContainText('Last looked just now');
  // The questions and suggestions are You items in Library, Learnings since
  // Settings PR 4; Personalization's one line opens them.
  await page.getByRole('button', { name: /^Waiting for you/ }).click();
  // Suggestions come one at a time (audit H15); the full list stays one click away.
  await expect(page.locator('.about-you-focus')).toContainText(/Suggestion 1 of \d+/);
  await page.locator('.about-you-all > summary').click();
  const role = page.locator('.about-you-suggestion').filter({ hasText: 'Runs payroll' });
  await expect(role).toBeVisible();
  await expect(role).toContainText('from Claude memories · Identity · Role');

  await role.getByRole('button', { name: /^Add to profile/ }).click();
  await expect(role).toHaveCount(0);

  // Edit before adding.
  const answerFirst = page.locator('.about-you-suggestion[data-id="pf-answer-first"]');
  await answerFirst.getByRole('button', { name: 'Edit before adding' }).click();
  await page.getByRole('textbox', { name: 'Edit before adding' }).fill('Answer first, reasoning after');
  await answerFirst.getByRole('button', { name: /^Add to profile/ }).click();

  // Questions file the answer under their label.
  await page.getByLabel('What timezone do you work in?').fill('US Central');
  await page.locator('.about-you-question').getByRole('button', { name: 'Save' }).click();
  await expect(page.locator('.about-you-question')).toHaveCount(0);

  // The notes are still in Personalization, with what was added.
  await page.getByRole('button', { name: /Settings/ }).first().click();
  await openSettingsSection(page, 'Personalization', 'personalization');
  await expect(page.locator('.about-you-note').filter({ hasText: 'Identity' })).toContainText('Role: Runs payroll and HR systems at Acme');
  await expect(page.locator('.about-you-note').filter({ hasText: 'Communication' })).toContainText('Answer first, reasoning after');
  await expect(page.locator('.about-you-note').filter({ hasText: 'Identity' })).toContainText('Timezone: US Central');
  await expect(page.getByText(/You’re caught up/)).toBeVisible();

  await page.getByRole('switch', { name: 'Keep learning' }).click();
  await expect(page.getByText('Only when you ask')).toBeVisible();
});

/** 10-11 B1: the first artifact's head, its ⋯ menu and its permission line. */
const firstArtifact = (page: Page) => page.locator('.chat-artifact').first();
async function artifactMenu(page: Page, item: string) {
  await firstArtifact(page).locator('summary[aria-label="HTML actions"]').click();
  await firstArtifact(page).getByRole('menuitem', { name: item }).click();
}

test('html outside a workspace opens locked, asks only after a refusal, and has no mode buttons', async ({ page }) => {
  await page.goto('/?mock=1&window=smoke-artifact');

  await expect(page.getByRole('heading', { name: 'Smoke harness' })).toBeVisible();
  await expect(page.getByRole('group', { name: 'Artifact security mode' })).toHaveCount(0);
  await expect(firstArtifact(page)).toHaveAttribute('data-sandbox', 'offline');

  const frame = page.frameLocator('iframe[title="Rendered HTML artifact"]').first();
  await expect(frame.getByRole('heading', { name: 'Artifact safety fixture' })).toBeVisible();
  await expect(frame.locator('body')).not.toHaveAttribute('data-script-ran', '1');
  // The refused inline script raises the one permission line.
  const line = firstArtifact(page).locator('.artifact-ask');
  await expect(line).toContainText('Scripts in this HTML are off outside workspaces and Chats.');

  await line.getByRole('button', { name: 'Run scripts' }).click();
  await expect(firstArtifact(page)).toHaveAttribute('data-sandbox', 'canvas');
  // Outside the app Canvas renders without scripts and says so.
  await expect(line).toContainText('Scripts run only in the Hollow app');
  await artifactMenu(page, 'Allow internet for this HTML');
  await expect(firstArtifact(page)).toHaveAttribute('data-sandbox', 'interactive');
  await expect(frame.locator('body')).toHaveAttribute('data-script-ran', '1');
  expect(await frame.locator('body').evaluate(() => typeof RTCPeerConnection)).toBe('undefined');
  expect(await frame.locator('body').evaluate(() => { const child = window.open('about:blank'); const opened = !!child; child?.close(); return opened; })).toBe(false);
  expect(await frame.locator('body').evaluate(() => { const child = document.createElement('iframe'); document.body.append(child); try { return typeof child.contentWindow?.RTCPeerConnection; } catch { return 'blocked'; } finally { child.remove(); } })).toBe('blocked');
  expect(await frame.locator('body').evaluate(() => Object.getOwnPropertyDescriptor(window, 'RTCPeerConnection')?.configurable)).toBe(false);
});

test('a grant is remembered for that HTML, and the menu turns it off again', async ({ page }) => {
  await page.goto('/?mock=1&window=smoke-artifact');
  await artifactMenu(page, 'Run scripts in this HTML');
  await expect(firstArtifact(page)).toHaveAttribute('data-sandbox', 'canvas');
  await page.reload();
  await expect(firstArtifact(page)).toHaveAttribute('data-sandbox', 'canvas');
  await artifactMenu(page, 'Turn scripts off again');
  await expect(firstArtifact(page)).toHaveAttribute('data-sandbox', 'offline');
  await page.reload();
  await expect(firstArtifact(page)).toHaveAttribute('data-sandbox', 'offline');
});

test('View source lives in the menu and on ⌥⌘U', async ({ page }) => {
  await page.goto('/?mock=1&window=smoke-artifact');
  await artifactMenu(page, 'View source');
  const code = firstArtifact(page).locator('pre.chat-artifact-code');
  await expect(code).toContainText('Artifact safety fixture');
  await expect(code).toBeFocused();
  await page.keyboard.press('Meta+Alt+KeyU');
  await expect(code).toHaveCount(0);
  await expect(page.frameLocator('iframe[title="Rendered HTML artifact"]').first().getByRole('heading', { name: 'Artifact safety fixture' })).toBeVisible();
});

test('canvas mode outside the app falls back to Offline and says so', async ({ page }) => {
  await page.goto('/?mock=1&window=smoke-artifact');
  await artifactMenu(page, 'Run scripts in this HTML');
  await expect(firstArtifact(page).locator('.artifact-ask')).toContainText('Scripts run only in the Hollow app; showing this without them.');
  const frame = page.frameLocator('iframe[title="Rendered HTML artifact"]').first();
  await expect(frame.getByRole('heading', { name: 'Artifact safety fixture' })).toBeVisible();
  await expect(frame.locator('body')).not.toHaveAttribute('data-script-ran', '1');
  await expect(frame.locator('meta[http-equiv="x-dns-prefetch-control"]')).toHaveAttribute('content', 'off');
});

test('a theme switch restyles an already-mounted artifact frame without reloading it', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('theme', 'light'));
  await page.goto('/?mock=1&window=smoke-artifact');
  const iframe = page.locator('iframe[title="Rendered HTML artifact"]').first();
  const frame = page.frameLocator('iframe[title="Rendered HTML artifact"]').first();
  await expect(frame.getByRole('heading', { name: 'Artifact safety fixture' })).toBeVisible();
  const hostInk = () => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--ink').trim());
  const frameInk = () => frame.locator('html').evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--app-ink').trim());
  const lightInk = await hostInk();
  await expect.poll(frameInk).toBe(lightInk);
  // Mark this document: a reload would lose the marker.
  await frame.locator('html').evaluate(() => { (window as unknown as { __mounted: number }).__mounted = 1; });
  await page.evaluate(() => { document.documentElement.dataset.theme = 'dark'; });
  const darkInk = await hostInk();
  expect(darkInk).not.toBe(lightInk);
  await expect.poll(frameInk).toBe(darkInk);
  await expect.poll(() => frame.locator('html').evaluate(() => getComputedStyle(document.documentElement).colorScheme)).toBe('dark');
  expect(await frame.locator('html').evaluate(() => (window as unknown as { __mounted?: number }).__mounted)).toBe(1);
  await expect(iframe).toBeVisible();
});

for (const variant of [
  { name: 'light', theme: 'light', glass: false },
  { name: 'dark', theme: 'dark', glass: false },
  { name: 'glass', theme: 'dark', glass: true },
] as const) {
  test(`screenshot coverage: ${variant.name}`, async ({ page }, testInfo) => {
    await page.addInitScript(({ theme, glass }) => {
      localStorage.setItem('theme', theme);
      if (glass) localStorage.setItem('glass', 'on');
      else localStorage.removeItem('glass');
    }, variant);

    await page.goto('/?mock=1');
    await expect(page.getByLabel('Main navigation')).toBeVisible();
    await testInfo.attach(`hollow-${variant.name}.png`, {
      body: await page.screenshot({ fullPage: true }),
      contentType: 'image/png',
    });
  });
}


test('scripted artifact link needs host consent and Cancel opens nothing', async ({ page }) => {
  const reached: string[] = [];
  page.on('request', request => { if (request.url().includes(':49231/')) reached.push(request.url()); });
  await page.goto('/?mock=1&window=smoke-artifact');
  await artifactMenu(page, 'Run scripts in this HTML');
  await artifactMenu(page, 'Allow internet for this HTML');
  await expect(firstArtifact(page)).toHaveAttribute('data-sandbox', 'interactive');
  const frame = page.frameLocator('iframe[title="Rendered HTML artifact"]').first();
  await frame.getByRole('button', { name: 'Request synthetic link' }).click();
  const dialog = page.getByRole('dialog', { name: 'Open this canvas link?' });
  await expect(dialog).toContainText('http://127.0.0.1:49231/canary?value=synthetic');
  await expect(dialog.getByRole('button', { name: 'Cancel', exact: true })).toBeFocused();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).toBeHidden();
  expect(reached).toEqual([]);
});

test('the host drops malformed, oversized and unsupported bridge messages', async ({ page }) => {
  const reached: string[] = [];
  page.on('request', request => { if (request.url().includes(':49231/')) reached.push(request.url()); });
  await page.goto('/?mock=1&window=smoke-artifact');
  await artifactMenu(page, 'Run scripts in this HTML');
  await artifactMenu(page, 'Allow internet for this HTML');
  await expect(firstArtifact(page)).toHaveAttribute('data-sandbox', 'interactive');
  const frame = page.frameLocator('iframe[title="Rendered HTML artifact"]').first();
  await frame.getByRole('button', { name: 'Send malformed bridge messages' }).click();
  const dialog = page.getByRole('dialog', { name: 'Open this canvas link?' });
  // Only the last, well-formed message reaches the confirmation.
  await expect(dialog).toContainText('http://127.0.0.1:49231/valid');
  for (const path of ['oversize', 'version', 'legacy', 'extra']) await expect(dialog).not.toContainText(`/${path}`);
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).toBeHidden();
  expect(reached).toEqual([]);
});

test('a file view that changes file starts the new file from its own default (review B1 high)', async ({ page }) => {
  await page.goto('/?mock=1&window=smoke-artifact');
  const region = page.getByRole('region', { name: 'Switching file' });
  const doc = region.locator('.html-doc');
  await expect(doc).toHaveAttribute('data-sandbox', 'offline');
  await doc.locator('.artifact-ask').getByRole('button', { name: 'Run scripts' }).click();
  await expect(doc).toHaveAttribute('data-sandbox', 'canvas');
  await region.getByRole('button', { name: 'Show the other file' }).click();
  await expect(region.frameLocator('iframe[title="Rendered brief"]').getByRole('heading', { name: 'Artifact second file' })).toBeVisible();
  await expect(doc).toHaveAttribute('data-sandbox', 'offline');
});
