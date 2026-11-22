import { expect, test, type Page } from './fixtures/test';

// SRI repair (2026-10-05). Under CSP3 a hash-source also allows an EXTERNAL
// script whose `integrity` carries that hash, and the browser requests it
// before the integrity check fails, so the URL alone leaks data. The runtime
// and kit hashes are public. Outside Tauri the artifact renders from srcdoc
// under its own <meta> policy (childCsp), the same table artifacts.rs serves;
// the native served frame is the bed row canvas-sri-script.
//
// The fixture (SmokeHarness `SRI_EXFIL`) holds two external scripts carrying
// the runtime and kit hashes, then `#sri-end`: once that is in the DOM the
// parser has passed both tags, so any request has been issued.

const LEAK = 'https://sri-exfil.invalid/';
const SURFACES = [
  { name: 'HtmlArtifact', index: 0, title: 'Rendered HTML artifact' },
  { name: 'HtmlDoc', index: 1, title: 'Rendered brief' },
] as const;

async function open(page: Page) {
  const reached: string[] = [];
  await page.route(`${LEAK}**`, route => {
    reached.push(route.request().url());
    return route.fulfill({ status: 200, contentType: 'text/javascript', body: '' });
  });
  await page.goto('/?mock=1&window=smoke-artifact');
  return { reached, region: page.getByRole('region', { name: 'SRI exfil fixture' }) };
}

// 10-11 B1: no mode buttons. Each surface opens locked (Offline); the
// refused remote scripts raise the permission line, first for scripts (Run
// scripts → Canvas), then for the network (Allow → Interactive). The chat
// artifact's ⋯ menu also reaches HTTPS (Allow internet, scripts off).
for (const surface of SURFACES) {
  test(`${surface.name}: an external script with the runtime or kit hash as integrity makes no request outside Interactive`, async ({ page }) => {
    const { reached, region } = await open(page);
    const host = region.locator(surface.index === 0 ? '.chat-artifact' : '.html-doc').first();
    const line = host.locator('.artifact-ask');
    const frame = region.frameLocator(`iframe[title="${surface.title}"]`);
    // Chat artifact frames load lazily: bring each new frame into view.
    const shown = () => region.locator(`iframe[title="${surface.title}"]`).scrollIntoViewIfNeeded();
    async function quiet(mode: string) {
      await expect(host).toHaveAttribute('data-sandbox', mode);
      await shown();
      await expect(frame.locator('#sri-end')).toBeAttached();
      // The host's own scripts still run: the kit registered itself.
      await expect.poll(() => frame.locator('body').evaluate(() =>
        (window as unknown as { HollowKit?: { version: string } }).HollowKit?.version ?? null)).toBe('kit@1');
      expect(reached, `${mode}: no external script request`).toEqual([]);
    }

    await quiet('offline');
    if (surface.index === 0) {
      await host.locator('summary[aria-label="HTML actions"]').click();
      await host.getByRole('menuitem', { name: 'Allow internet for this HTML' }).click();
      await quiet('https');
      await host.locator('summary[aria-label="HTML actions"]').click();
      await host.getByRole('menuitem', { name: 'Turn internet off for this HTML' }).click();
      await quiet('offline');
    }
    await expect(line).toContainText('Scripts in this');
    await line.getByRole('button', { name: 'Run scripts' }).click();
    await quiet('canvas');

    // Interactive allows https: scripts by its documented policy: the same
    // tags now reach the route, which proves the fixture is live.
    await expect(line).toContainText('tried to connect to sri-exfil.invalid. Allowing lets it load from and send to any website.');
    await line.getByRole('button', { name: 'Allow', exact: true }).click();
    await expect(host).toHaveAttribute('data-sandbox', 'interactive');
    await shown();
    await expect(frame.locator('#sri-end')).toBeAttached();
    await expect.poll(() => reached.length).toBeGreaterThanOrEqual(1);
    expect(reached.every(url => url.startsWith(`${LEAK}leak?script=`))).toBe(true);
  });
}
