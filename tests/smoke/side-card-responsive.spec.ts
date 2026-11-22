import { expect, test } from './fixtures/test';
import { seedSubagents } from './subagentsFixture';

// U5 audit (overnight 11-17): the persistent side card keeps every control
// reachable and inside the chat pane from the window minimum (720 x 480) up,
// at Largest text, light and dark, with every section open (the worst case).
// The transcript and composer stay full width beside it.
const SIZES = [
  { width: 720, height: 480 }, { width: 720, height: 700 }, { width: 900, height: 480 },
  { width: 1100, height: 600 }, { width: 1440, height: 900 },
];

for (const size of SIZES) for (const scale of ['100', '125']) for (const theme of ['light', 'dark']) {
  test(`side card fits ${size.width}x${size.height} text ${scale} ${theme}`, async ({ page }, info) => {
    await page.setViewportSize(size);
    await page.addInitScript(([t, s]) => { localStorage.setItem('theme', t); localStorage.setItem('hollow.studio.textScale', s); }, [theme, scale]);
    await seedSubagents(page);
    await page.goto('/?mock=1&demo=1');
    await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
    await expect(page.locator('.chat-opening')).toHaveCount(0);
    await expect(page.getByRole('textbox', { name: 'Chat message', exact: true })).toBeVisible();
    await page.evaluate(async () => {
      const path = '/src/store/chatRuntime.ts';
      const runtime = await import(/* @vite-ignore */ path);
      runtime.setActiveChatRecall(Array.from({ length: 6 }, (_, i) => ({
        subject: `Project decision ${i + 1}`, statement: 'Keep the visible routes and original project data. '.repeat(6),
        path: `learnings/decision-${i}.md`, targetPath: '*', score: 8,
      })));
    });
    const card = page.locator('#side-panel-card');
    await expect(card).toContainText('Recall');
    const headers = card.locator('.side-card-toggle');
    // Open every section, the worst case for height.
    for (const header of await headers.all()) if ((await header.getAttribute('aria-expanded')) === 'false') await header.click();
    const measure = () => page.evaluate(() => {
      const r = (el: Element | null) => el ? el.getBoundingClientRect() : null;
      const card = document.querySelector<HTMLElement>('#side-panel-card')!;
      const inner = card.querySelector<HTMLElement>('.side-card')!;
      const main = card.closest('.chat-main')!;
      const scroll = document.querySelector<HTMLElement>('.chat-scroll')!;
      const cr = r(card)!; const mr = r(main)!; const sr = r(scroll)!;
      const composer = document.querySelector('[aria-label="Chat message"]');
      return {
        card: { l: cr.left, t: cr.top, r: cr.right, b: cr.bottom }, main: { l: mr.left, t: mr.top, r: mr.right, b: mr.bottom },
        scroll: { w: sr.width }, composer: r(composer),
        innerHScroll: inner.scrollWidth - inner.clientWidth, innerVScroll: inner.scrollHeight - inner.clientHeight,
        vw: window.innerWidth, vh: window.innerHeight, docH: document.documentElement.scrollWidth - window.innerWidth,
        bodies: [...card.querySelectorAll<HTMLElement>('.side-card-body')].map(b => ({ id: b.id, over: b.scrollHeight - b.clientHeight, h: b.clientHeight })),
        subagentText: (() => { const e = card.querySelector('.side-card-subagents > span:last-child'); return e ? Math.round(e.getBoundingClientRect().width) : null; })(),
        toggles: [...card.querySelectorAll<HTMLElement>('.side-card-toggle')].map(t => ({
          name: t.textContent, clippedX: t.scrollWidth - t.clientWidth,
          title: (() => { const s = t.querySelector('span')!.getBoundingClientRect(); const ir = inner.getBoundingClientRect(); return s.right <= ir.right + 1 && s.left >= ir.left - 1; })(),
        })),
      };
    });
    const m = await measure();
    info.annotations.push({ type: 'measure', description: JSON.stringify(m) });
    // Inside the chat pane, no page-level horizontal scroll.
    expect(m.card.l).toBeGreaterThanOrEqual(m.main.l - 1);
    expect(m.card.r).toBeLessThanOrEqual(m.main.r + 1);
    expect(m.card.t).toBeGreaterThanOrEqual(m.main.t - 1);
    expect(m.card.b).toBeLessThanOrEqual(m.main.b + 1);
    expect(m.docH).toBeLessThanOrEqual(1);
    // The card scrolls vertically, never sideways; every header title shows whole.
    expect(m.innerHScroll).toBeLessThanOrEqual(1);
    for (const t of m.toggles) { expect(t.clippedX, `${t.name} clipped`).toBeLessThanOrEqual(1); expect(t.title, `${t.name} title inside card`).toBe(true); }
    // The transcript keeps a usable width and the composer stays on screen.
    expect(m.scroll.w).toBeGreaterThan(m.main.r - m.main.l - 8);
    expect(m.composer!.bottom).toBeLessThanOrEqual(m.vh + 1);
    // Every header is reachable by scrolling the card, with a visible label and a keyboard route.
    for (const header of await headers.all()) { await header.scrollIntoViewIfNeeded(); await expect(header).toBeInViewport(); }
    await headers.first().focus();
    await page.keyboard.press('Tab');
    await expect(page.locator('#side-panel-card :focus')).toHaveCount(1);
    await page.screenshot({ path: info.outputPath(`card-${size.width}x${size.height}-${scale}-${theme}.png`) });
    // Preview hides the card and its close route restores it.
    await page.getByRole('button', { name: 'Project files', exact: true }).click();
    const preview = page.getByRole('complementary', { name: 'Preview', exact: true });
    await expect(preview).toBeVisible();
    await preview.getByRole('button', { name: /Close.*[Pp]review/ }).click();
    await expect(card).toBeVisible();
  });
}
