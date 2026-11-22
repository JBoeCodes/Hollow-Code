import { expect, test, type Locator, type Page } from './fixtures/test';

// K3 review fix: ARTIFACT_STYLES gives an artifact document an opaque default
// background (so contrast inside the frame doesn't depend on the host), and
// it must do that on <body>, never <html>. CSS propagates a body background
// to the viewport only while the root has none, so an html background stops
// an artifact's own `body{background:…}` from filling the frame: a centred
// column shows host --canvas at its sides, and an expanded (≥640px) frame
// shows a strip below short content. These checks read real pixels.

type Rgb = [number, number, number];
const OWN: Rgb = [0x12, 0x34, 0x56];
const REGION = 'Artifact backgrounds';

/** Decode a PNG in a blank page (the harness's own CSP would refuse a data:
 *  image) and read the pixels at the given points, in CSS px of a `cssWidth`
 *  wide element; a negative coordinate counts from the far edge. */
async function pixels(page: Page, png: Buffer, cssWidth: number, points: Array<[number, number]>): Promise<Rgb[]> {
  const blank = await page.context().newPage();
  try {
    return await blank.evaluate(async ({ b64, w, pts }) => {
      const img = new Image();
      img.src = `data:image/png;base64,${b64}`;
      await img.decode();
      const c = document.createElement('canvas');
      c.width = img.naturalWidth;
      c.height = img.naturalHeight;
      const ctx = c.getContext('2d')!;
      ctx.drawImage(img, 0, 0);
      const scale = c.width / w;
      const at = (v: number, size: number) => Math.min(size - 1, Math.max(0, Math.round((v < 0 ? size + v * scale : v * scale))));
      return pts.map(([px, py]) => {
        const x = at(px, c.width);
        const y = at(py, c.height);
        const d = ctx.getImageData(x, y, 1, 1).data;
        return [d[0], d[1], d[2]] as [number, number, number];
      });
    }, { b64: png.toString('base64'), w: cssWidth, pts: points });
  } finally {
    await blank.close();
  }
}

const near = (a: Rgb, b: Rgb) => a.every((v, i) => Math.abs(v - b[i]) <= 3);

/** 4px in from the left and right edges at mid height, and 4px up from the
 *  bottom: left and right of the card's rounded corners, and the middle. */
async function frameEdges(page: Page, iframe: Locator): Promise<Rgb[]> {
  await iframe.scrollIntoViewIfNeeded();
  const box = (await iframe.boundingBox())!;
  const midY = Math.round(box.height / 2);
  return pixels(page, await iframe.screenshot(), box.width, [
    [4, midY], [-4, midY], [20, -4], [Math.round(box.width / 2), -4], [-20, -4],
  ]);
}

async function open(page: Page, theme: 'light' | 'dark') {
  await page.addInitScript((t) => localStorage.setItem('theme', t), theme);
  await page.goto('/?mock=1&window=smoke-artifact');
  const region = page.getByRole('region', { name: REGION, exact: true });
  const artifacts = region.locator('.chat-artifact');
  await expect(artifacts).toHaveCount(2);
  const frames = region.locator('iframe[title="Rendered HTML artifact"]');
  await expect(region.frameLocator('iframe[title="Rendered HTML artifact"]').first().locator('p')).toBeVisible();
  await expect(region.frameLocator('iframe[title="Rendered HTML artifact"]').nth(1).locator('p')).toBeVisible();
  return { region, artifacts, frames };
}

for (const theme of ['light', 'dark'] as const) {
  test(`${theme}: an artifact's own body background fills the whole frame, collapsed and expanded`, async ({ page }) => {
    const { artifacts, frames } = await open(page, theme);
    const own = frames.first();
    const box = await own.boundingBox();
    expect(box!.width, 'the frame is wider than the artifact column').toBeGreaterThan(260);

    const collapsed = await frameEdges(page, own);
    expect(collapsed.every(p => near(p, OWN)), `collapsed edges ${JSON.stringify(collapsed)}`).toBe(true);

    await artifacts.first().getByRole('button', { name: 'Expand artifact' }).click();
    await expect.poll(async () => (await own.boundingBox())!.height).toBeGreaterThanOrEqual(640);
    const expanded = await frameEdges(page, own);
    expect(expanded.every(p => near(p, OWN)), `expanded edges ${JSON.stringify(expanded)}`).toBe(true);
  });

  test(`${theme}: an artifact without a background paints the host canvas colour itself`, async ({ page }) => {
    const { region } = await open(page, theme);
    const frame = region.frameLocator('iframe[title="Rendered HTML artifact"]').nth(1);
    // The document's own background, so contrast checks inside the frame see
    // the colour a person sees (the K3 dark-mode axe fault).
    const painted = await frame.locator('body').evaluate(() => {
      const probe = document.createElement('span');
      probe.style.color = 'var(--app-canvas)';
      document.body.append(probe);
      const canvas = getComputedStyle(probe).color;
      probe.remove();
      return { canvas, body: getComputedStyle(document.body).backgroundColor, html: getComputedStyle(document.documentElement).backgroundColor };
    });
    expect(painted.body).toBe(painted.canvas);
    expect(painted.html, 'html stays transparent so a body background can propagate').toBe('rgba(0, 0, 0, 0)');
  });
}
