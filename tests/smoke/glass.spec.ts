import { showWorkspaceList } from './choose';
import { expect, test, type Page } from './fixtures/test';

/**
 * Liquid Glass only looks like glass when the surfaces between the window and
 * the content let the native vibrancy through: the NSVisualEffectView sits
 * behind the transparent webview, so one opaque full-bleed container hides it
 * everywhere (the themes then look like plain dark). The browser has no
 * vibrancy, so this measures the CSS side: at points in the rail, the title
 * strip and the conversation, the backgrounds stacked above the window must
 * stay translucent. Composited opacity = 1 − Π(1 − alphaᵢ) over every element
 * under the point.
 */
type Probe = { name: string; x: number; y: number; opacity: number; opaque: string[] };

async function probe(page: Page, points: Record<string, [number, number]>): Promise<Probe[]> {
  return page.evaluate((pts) => {
    const alphaOf = (c: string) => {
      const m = c.match(/rgba?\(([^)]+)\)/);
      if (!m) return 0;
      const parts = m[1].split(/[\s,/]+/).filter(Boolean);
      return parts.length >= 4 ? Number(parts[3]) : 1;
    };
    const label = (el: Element) => `${el.tagName.toLowerCase()}${typeof el.className === 'string' && el.className ? '.' + el.className.trim().split(/\s+/).slice(0, 3).join('.') : ''}`;
    return Object.entries(pts).map(([name, [fx, fy]]) => {
      const x = Math.round(innerWidth * fx), y = Math.round(innerHeight * fy);
      let clear = 1;
      const opaque: string[] = [];
      for (const el of document.elementsFromPoint(x, y)) {
        const cs = getComputedStyle(el);
        let a = alphaOf(cs.backgroundColor) * Number(cs.opacity);
        if (cs.backgroundImage && cs.backgroundImage !== 'none' && !cs.backgroundImage.startsWith('url(')) a = Math.max(a, 0.5);
        if (a > 0) clear *= 1 - a;
        if (a >= 0.9) opaque.push(`${label(el)} ${cs.backgroundColor}`);
      }
      return { name, x, y, opacity: 1 - clear, opaque };
    });
  }, points);
}

for (const theme of ['dark', 'light'] as const) {
  test(`glass ${theme}: the shell lets the window's vibrancy through`, async ({ page }, info) => {
    test.skip(info.project.name === 'chromium-narrow', 'desktop layout: rail, title strip and conversation side by side');
    await page.addInitScript((t) => {
      localStorage.setItem('theme', t);
      localStorage.setItem('glass', 'on');
    }, theme);
    await page.goto('/?mock=1');
    await expect(page.getByLabel('Main navigation')).toBeVisible();
    await expect(page.locator('html')).toHaveAttribute('data-glass', 'on');
    // Rail middle, title strip, and an empty stretch of the page / conversation.
    const probes = await probe(page, {
      rail: [0.05, 0.6],
      'title strip': [0.5, 0.02],
      page: [0.55, 0.85],
    });
    const problems = probes
      .filter(p => p.opaque.length || p.opacity >= 0.9)
      .map(p => `${p.name} (${p.x},${p.y}) composited ${p.opacity.toFixed(2)}: ${p.opaque.join(' | ') || 'stacked tints'}`);
    expect(problems).toEqual([]);
  });
}

test('glass dark: a workspace conversation and Settings stay translucent', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'desktop layout with the side panel');
  await page.addInitScript(() => {
    localStorage.setItem('theme', 'dark');
    localStorage.setItem('glass', 'on');
    localStorage.setItem('mockWorkspaces', JSON.stringify([
      { id: 'alpha-1', title: 'Alpha', status: 'active', path: 'workspaces/alpha-1', cwd: '/Users/fixture/vault/workspaces/alpha-1', modified: '2026-09-26T12:00:00Z' },
    ]));
  });
  await page.goto('/?mock=1');
  await showWorkspaceList(page);
  await page.locator('.work-card', { hasText: 'Alpha' }).click();
  await expect(page.getByLabel('Chat message')).toBeVisible();
  // Since #1492 the summary card floats over the chat's top right and the
  // composer runs the pane's full width (side-panel.css), so the old side
  // panel point (0.9, 0.6) now lands on the composer: a Glass reading well at
  // 0.97 on purpose (docs/DESIGN_SYSTEM.md, Materials and depth; --glass-well in
  // tokens.css). Probe the frame on the right, below the card, above the box.
  const chat = await probe(page, { transcript: [0.4, 0.35], 'right of the conversation': [0.9, 0.3], 'title strip': [0.6, 0.02] });
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.locator('.settings-section, .settings-page').first()).toBeVisible();
  const settings = await probe(page, { 'settings page': [0.6, 0.7] });
  const problems = [...chat, ...settings]
    .filter(p => p.opaque.length || p.opacity >= 0.9)
    .map(p => `${p.name} (${p.x},${p.y}) composited ${p.opacity.toFixed(2)}: ${p.opaque.join(' | ') || 'stacked tints'}`);
  expect(problems).toEqual([]);
});

test('glass dark: the collapsed rail joins a translucent title strip (overlay chrome)', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'desktop layout');
  await page.addInitScript(() => { localStorage.setItem('theme', 'dark'); localStorage.setItem('glass', 'on'); });
  await page.goto('/?mock=1&chrome=overlay');
  const rail = page.locator('.work-rail');
  await expect(rail).toBeVisible();
  await page.keyboard.press('Meta+b');
  await expect(rail).toHaveClass(/collapsed/);
  // The strip over the traffic lights and the rail below it.
  const [strip, below] = await probe(page, { 'collapsed rail strip': [0.015, 0.02], 'collapsed rail': [0.015, 0.5] });
  const problems = [strip, below]
    .filter(p => p.opaque.length || p.opacity >= 0.9)
    .map(p => `${p.name} (${p.x},${p.y}) composited ${p.opacity.toFixed(2)}: ${p.opaque.join(' | ') || 'stacked tints'}`);
  expect(problems).toEqual([]);
  // The strip keeps the top bar's tint, so the lights sit on one toolbar.
  const tints = await page.evaluate(() => [getComputedStyle(document.querySelector('.work-rail')!).backgroundColor, getComputedStyle(document.querySelector('.top-bar')!).backgroundColor]);
  expect(tints[0]).toBe(tints[1]);
});
