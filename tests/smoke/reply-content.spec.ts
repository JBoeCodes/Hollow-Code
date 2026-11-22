import { expect, test } from './fixtures/test';

declare global { interface Window { replyFixture: ReturnType<typeof import('./fixtures/reply-content').mount>; copiedReply: string[] } }
async function mount(page: import('@playwright/test').Page, kind: 'reply' | 'bash' | 'media', text = '') {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await page.evaluate(async ({ kind, text }) => {
    window.copiedReply = [];
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async (value: string) => { window.copiedReply.push(value); } } });
    const w = window as unknown as { __TAURI_INTERNALS__?: { invoke: (command: string, args?: unknown) => Promise<unknown> } };
    const prior = w.__TAURI_INTERNALS__?.invoke;
    w.__TAURI_INTERNALS__ = { ...w.__TAURI_INTERNALS__, invoke: (command, args) => command === 'plugin:path|resolve_directory'
      ? Promise.resolve('/Users/mock') : prior ? prior(command, args) : Promise.reject(new Error('No native bridge')) };
    const path = '/tests/smoke/fixtures/reply-content.tsx';
    window.replyFixture = (await import(/* @vite-ignore */ path)).mount(kind, text);
  }, { kind, text });
}

// #1668 (n20/t1-images): a picture's button is named "Expand <file>" and opens the
// zoom dialog; "Open in Preview" there is what reaches the side pane.
async function openInPreview(page: import('@playwright/test').Page) {
  const zoom = page.getByRole('dialog', { name: /^Expanded image:/ });
  await expect(zoom).toBeVisible();
  await zoom.getByRole('button', { name: 'Open in Preview', exact: true }).click();
  await expect(zoom).toHaveCount(0);
}

test('400-line code folds, copies all source and stays expanded during streaming', async ({ page }) => {
  const source = Array.from({ length: 400 }, (_, i) => `line ${i + 1}`).join('\n');
  await mount(page, 'reply', '```ts\n' + source + '\n```');
  const study = page.locator('#reply-study');
  const pre = study.locator('.chat-codeblock-pre');
  await expect(pre).not.toContainText('line 25\n');
  await study.getByRole('button', { name: 'Copy ts block' }).click();
  expect(await page.evaluate(() => window.copiedReply)).toEqual([source]);
  const expand = study.getByRole('button', { name: 'Show all 400 lines' });
  await expand.focus(); await page.keyboard.press('Enter');
  await expect(pre).toContainText('line 400');
  await page.evaluate(t => window.replyFixture.text(t), '```ts\n' + source + '\nline 401\n```');
  await expect(pre).toContainText('line 401');
  await study.getByRole('button', { name: 'Copy ts block' }).click();
  expect((await page.evaluate(() => window.copiedReply)).at(-1)).toBe(source + '\nline 401');
});

test('Bash command and output copy exactly without closing the disclosure', async ({ page }) => {
  await mount(page, 'bash');
  const study = page.locator('#reply-study');
  await study.getByRole('button', { name: 'Copy Bash command' }).click();
  await study.getByRole('button', { name: 'Copy Bash output' }).click();
  expect(await page.evaluate(() => window.copiedReply)).toEqual(['printf "hello\\n"', 'hello\n']);
  await expect(study.locator('.toolcall-toggle')).toHaveAttribute('aria-expanded', 'true');
});

test('local reply images load and open by keyboard; missing files are named', async ({ page }) => {
  await mount(page, 'reply', '![local](outputs/ok.png)\n\n![missing](/tmp/missing.png)');
  const study = page.locator('#reply-study');
  const image = study.getByRole('button', { name: 'Expand ok.png', exact: true });
  await expect(image.locator('img')).toHaveAttribute('src', /^data:image\/png/);
  await expect(study).toContainText('Image not found: missing.png');
  await image.focus(); await page.keyboard.press('Enter');
  await openInPreview(page);
  await expect.poll(() => page.evaluate(() => window.replyFixture.state().preview)).toBe('/vault/workspaces/reply/outputs/ok.png');
});

test('remote bulk loading counts only remote images and keeps partial failure feedback', async ({ page }) => {
  await mount(page, 'reply', '![local](/tmp/ok.png)\n\n![one](https://reply.test/one.png)\n\n![two](https://reply.test/fail.png)');
  const study = page.locator('#reply-study');
  await expect(study.getByRole('button', { name: 'Load 2 images', exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.replyFixture.state().remote)).toEqual([]);
  await study.getByRole('button', { name: 'Load 2 images', exact: true }).click();
  await expect(study).toContainText('1 image could not load');
  await expect(study.locator('.md-remote-img')).toHaveCount(1);
  await expect(study.getByRole('button', { name: 'Load 1 image', exact: true })).toBeVisible();
});

test('cold reply fallback blocks raw image requests and link navigation', async ({ page }) => {
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/src/components/ReplyMarkdown.tsx*', async route => { await held; await route.continue(); });
  const requests: string[] = [];
  await page.route('**/outputs/ok.png', route => { requests.push(route.request().url()); return route.abort(); });
  await page.route('https://reply.test/**', route => { requests.push(route.request().url()); return route.abort(); });
  await mount(page, 'reply', '![local](outputs/ok.png) ![remote](https://reply.test/a.png) [web](https://reply.test/page)');
  const pending = page.locator('#reply-study .reply-loading');
  await expect(pending).toContainText('local');
  await expect(pending.locator('img')).toHaveCount(0);
  await pending.getByRole('link', { name: 'web', exact: true }).click();
  await expect(page).toHaveURL(/\?mock=1/);
  expect(requests).toEqual([]);
  release();
  await expect(page.locator('#reply-study').getByRole('button', { name: 'Expand ok.png', exact: true })).toBeEnabled();
});

test('local image reads share a two-read limit across reply segments', async ({ page }) => {
  const text = Array.from({ length: 5 }, (_, i) => `![held](held-${i}.png)\n\n\`\`\`txt\nx\n\`\`\``).join('\n\n');
  await mount(page, 'reply', text);
  await expect.poll(() => page.evaluate(() => window.replyFixture.state().reads.length)).toBeGreaterThanOrEqual(2);
  // All five segments mounted, but none of the held reads has finished.
  await expect(page.locator('#reply-study .reply-local-image')).toHaveCount(5);
  expect(await page.evaluate(() => window.replyFixture.state().reads.length)).toBe(2);
  await page.evaluate(() => window.replyFixture.releaseHeldImages());
  await expect(page.locator('#reply-study .reply-local-image:not(:disabled)')).toHaveCount(5);
});

// Overnight 11-07, D1 (gap 8): a reply's view now lives across its flushes, so a
// read still waiting for a slot outlives the pass that asked for it.
test('a picture waiting for a read slot still loads when its reply grows meanwhile', async ({ page }) => {
  const piece = (i: number, more = '') => `![held](held-${i}.png)${more}\n\n\`\`\`txt\nx\n\`\`\``;
  const text = (more = '') => Array.from({ length: 5 }, (_, i) => piece(i, i === 4 ? more : '')).join('\n\n');
  await mount(page, 'reply', text());
  await expect.poll(() => page.evaluate(() => window.replyFixture.state().reads.length)).toBeGreaterThanOrEqual(2);
  // Two reads hold the limit; the last picture's read waits behind them.
  expect(await page.evaluate(() => window.replyFixture.state().reads.length)).toBe(2);
  // Its reply grows now: a second pass of that view starts while the read waits.
  await page.evaluate(t => window.replyFixture.text(t), text(' and a caption'));
  await expect(page.locator('#reply-study')).toContainText('and a caption');
  await page.evaluate(() => window.replyFixture.releaseHeldImages());
  await expect(page.locator('#reply-study .reply-local-image:not(:disabled)')).toHaveCount(5);
  await expect(page.locator('#reply-study')).not.toContainText('Image unavailable');
});

test('existing but unsupported images say unavailable, not missing', async ({ page }) => {
  await mount(page, 'reply', '![large](large.png)');
  await expect(page.locator('#reply-study')).toContainText('Image unavailable: large.png');
});

test('a late image read cannot replace the new conversation image or its opener', async ({ page }) => {
  await mount(page, 'reply', '![slow](slow.png)');
  await expect.poll(() => page.evaluate(() => window.replyFixture.state().reads)).toContain('/vault/workspaces/reply/slow.png');
  await page.evaluate(() => window.replyFixture.move('/vault/workspaces/new'));
  const image = page.locator('#reply-study').getByRole('button', { name: 'Expand slow.png', exact: true });
  await expect(image).toBeEnabled();
  await page.evaluate(() => window.replyFixture.releaseImage());
  await image.click();
  await openInPreview(page);
  await expect.poll(() => page.evaluate(() => window.replyFixture.state().preview)).toBe('/vault/workspaces/new/slow.png');
});

// Overnight 11-07, D1 (re-review of gap 8): a picture kept through a change of
// conversation with the reply still on screen. The reset effect and the view's
// decoration both run on the change and Solid doesn't order them, so the
// button must not depend on either order: an agent switch or a mode change
// that starts fresh left the old pass's click on it, dead until the next flush.
test('a loaded picture still opens after a new session starts with its reply on screen', async ({ page }) => {
  await mount(page, 'reply', '![local](outputs/ok.png)\n\nfirst');
  const image = page.locator('#reply-study').getByRole('button', { name: 'Expand ok.png', exact: true });
  await expect(image.locator('img')).toHaveAttribute('src', /^data:image\/png/);
  await page.evaluate(() => window.replyFixture.restart());
  // The new pass fills it in again a moment after the change: click until it opens.
  await expect(async () => {
    await image.click();
    await openInPreview(page);
    expect(await page.evaluate(() => window.replyFixture.state().preview)).toBe('/vault/workspaces/reply/outputs/ok.png');
  }).toPass({ timeout: 5000 });
});

test('a loaded picture that is gone from the new folder stops looking openable', async ({ page }) => {
  await mount(page, 'reply', '![x](x.png)\n\nFirst');
  const study = page.locator('#reply-study');
  await expect(study.getByRole('button', { name: 'Expand x.png', exact: true }).locator('img')).toHaveAttribute('src', /^data:image\/png/);
  const before = await page.evaluate(() => window.replyFixture.state().preview);
  await page.evaluate(() => window.replyFixture.move('/vault/workspaces/gone'));
  await expect(study).toContainText('Image not found: x.png');
  const failed = study.locator('.reply-local-image');
  await expect(failed).toBeDisabled();
  await expect(failed).not.toHaveAttribute('aria-label', /.+/);
  await failed.click({ force: true });
  expect(await page.evaluate(() => window.replyFixture.state().preview)).toBe(before);
});

// A failure keeps no stamp, so the folder that had the picture fills it again
// when it is back (the folder alone changes: the version does not).
test('a picture that failed in another folder loads again when its own folder is back', async ({ page }) => {
  await mount(page, 'reply', '![x](x.png)\n\nFirst');
  const study = page.locator('#reply-study');
  const image = study.getByRole('button', { name: 'Expand x.png', exact: true });
  await expect(image.locator('img')).toHaveAttribute('src', /^data:image\/png/);
  await page.evaluate(() => window.replyFixture.cd('/vault/workspaces/gone'));
  await expect(study).toContainText('Image not found: x.png');
  await page.evaluate(() => window.replyFixture.cd('/vault/workspaces/reply'));
  await expect(image.locator('img')).toHaveAttribute('src', /^data:image\/png/);
  await expect(image).toBeEnabled();
});

// A read queued behind the two slot holders belongs to the version that asked
// for it; a new session's pass asks again instead of reusing it, which would
// reject when its turn comes ("Image owner changed") and mark the picture
// unavailable.
test('pictures waiting for a read slot still load after a new session starts', async ({ page }) => {
  const piece = (i: number) => `![held](held-${i}.png)\n\n\`\`\`txt\nx\n\`\`\``;
  await mount(page, 'reply', Array.from({ length: 5 }, (_, i) => piece(i)).join('\n\n'));
  await expect.poll(() => page.evaluate(() => window.replyFixture.state().reads.length)).toBe(2);
  await page.evaluate(() => window.replyFixture.restart());
  await page.evaluate(() => window.replyFixture.releaseHeldImages());
  await expect(page.locator('#reply-study .reply-local-image:not(:disabled)')).toHaveCount(5);
  await expect(page.locator('#reply-study')).not.toContainText('Image unavailable');
});

// The same when only the folder changes (/cwd) and the paths are absolute, so
// the file names are the same in both folders: the read is the folder's too.
test('pictures waiting for a read slot still load after the folder alone changes', async ({ page }) => {
  const piece = (i: number) => `![held](/tmp/held-${i}.png)\n\n\`\`\`txt\nx\n\`\`\``;
  await mount(page, 'reply', Array.from({ length: 5 }, (_, i) => piece(i)).join('\n\n'));
  await expect.poll(() => page.evaluate(() => window.replyFixture.state().reads.length)).toBe(2);
  await page.evaluate(() => window.replyFixture.cd('/vault/workspaces/other'));
  await page.evaluate(() => window.replyFixture.releaseHeldImages());
  await expect(page.locator('#reply-study .reply-local-image:not(:disabled)')).toHaveCount(5);
  await expect(page.locator('#reply-study')).not.toContainText('Image unavailable');
});

test('home links use the OS home; unsupported schemes stay in Hollow; web links explain command-click', async ({ page }) => {
  await mount(page, 'reply', '[home](~/notes.md) and `~/My Notes/a.md:12` and [call](tel:123) and [mail](mailto:a@example.test) and [custom](widget:thing) and [web](https://example.test/page)');
  const study = page.locator('#reply-study');
  // A local file link is a chip named with its words and path (2026-11-10).
  await study.getByRole('link', { name: 'Open home, ~/notes.md', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.replyFixture.state().preview)).toBe('/Users/mock/notes.md');
  await study.locator('code.file-ref').click();
  await expect.poll(() => page.evaluate(() => window.replyFixture.state().preview)).toBe('/Users/mock/My Notes/a.md');
  await study.getByRole('link', { name: 'call', exact: true }).click();
  await expect(page.getByText("Hollow can't open 'tel:' links", { exact: true })).toBeVisible();
  await study.getByRole('link', { name: 'mail', exact: true }).click();
  await expect(page.getByText("Hollow can't open 'mailto:' links", { exact: true })).toBeVisible();
  await study.getByRole('link', { name: 'custom', exact: true }).click();
  await expect(page.getByText("Hollow can't open 'widget:' links", { exact: true })).toBeVisible();
  await expect(page).toHaveURL(/\?mock=1/);
  await expect(study.getByRole('link', { name: 'web', exact: true })).toHaveAttribute('title', /https:\/\/example.test\/page.*\n⌘-click/);
  await expect(study.getByRole('link', { name: 'web', exact: true })).toHaveCSS('text-decoration-line', 'underline');
});

test('image-map links cannot navigate the app to an unsupported scheme', async ({ page }) => {
  await mount(page, 'reply', '<map name="example"><area href="widget:thing" alt="custom" shape="rect" coords="0,0,10,10"></map>');
  const area = page.locator('#reply-study area');
  await expect(area).toHaveAttribute('href', 'widget:thing');
  expect(await area.evaluate(el => !el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true })))).toBe(true);
  await expect(page.getByText("Hollow can't open 'widget:' links", { exact: true })).toBeVisible();
});

test('reply decoration keeps executable markup and automatic remote requests out', async ({ page }) => {
  const requests: string[] = [];
  await page.route('https://reply.test/**', route => { requests.push(route.request().url()); return route.abort(); });
  await mount(page, 'reply', '<script>window.badReply = true</script><a href="javascript:window.badReply=true">unsafe</a><img src="https://reply.test/a.png" onerror="window.badReply=true">');
  const study = page.locator('#reply-study');
  await expect(study.locator('script, [onerror], a[href^="javascript:"]')).toHaveCount(0);
  await expect(study.locator('.md-remote-img')).toHaveCount(1);
  expect(requests).toEqual([]);
  expect(await page.evaluate(() => (window as unknown as { badReply?: boolean }).badReply)).toBeUndefined();
});

test('failed media offers retry and Preview pane, then releases on session change', async ({ page }) => {
  await page.route('**/__media__/*', route => route.fulfill({ status: 200, body: '', contentType: 'video/mp4' }));
  await mount(page, 'media');
  const study = page.locator('#reply-study');
  await expect(study).toContainText('Couldn’t load this video');
  await study.getByRole('button', { name: 'Open in Preview pane', exact: true }).click();
  expect(await page.evaluate(() => window.replyFixture.state().opens)).toBe(1);
  await page.evaluate(() => window.replyFixture.allowMedia());
  await study.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.replyFixture.state().grants)).toBeGreaterThan(1);
  await page.evaluate(() => window.replyFixture.session('second'));
  await expect.poll(() => page.evaluate(() => window.replyFixture.state().released.length)).toBeGreaterThan(0);
});

test('a late retry grant is released after the media session changes', async ({ page }) => {
  await page.route('**/__media__/*', route => route.fulfill({ status: 200, body: '', contentType: 'video/mp4' }));
  await mount(page, 'media');
  const study = page.locator('#reply-study');
  await expect(study).toContainText('Couldn’t load this video');
  await page.evaluate(() => { window.replyFixture.allowMedia(); window.replyFixture.holdNextGrant(); });
  await study.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.replyFixture.state().grants)).toBe(2);
  await page.evaluate(() => window.replyFixture.session('second'));
  await expect.poll(() => page.evaluate(() => window.replyFixture.state().grants)).toBeGreaterThan(2);
  await page.evaluate(() => window.replyFixture.releaseGrant());
  await expect.poll(() => page.evaluate(() => window.replyFixture.state().released)).toContain('2'.padStart(32, '0'));
});
