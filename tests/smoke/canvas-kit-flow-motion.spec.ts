import { readFileSync } from 'node:fs';
import { expect, test } from './fixtures/test';
const kit = () => readFileSync('src/canvas/kit/generated/kit.min.js', 'utf8');
const css = () => readFileSync('src/canvas/kit/generated/kit.min.css', 'utf8');
const content = `<hc-flow label="Delivery" interactive>[Build]\na: Build {start}\nb: Check\n[Review]\nc: Review {decision}\nd: Revise\n[Ship]\ne: Merge {success}\nf: Ready {end}\n[]\na -> b -> c\nc -> d: findings\nd -> b: retry\nc -> e: green {active}\ne -> f\na -> f: fast path</hc-flow>`;
test('flow zoom, keyboard pan, fit and cleanup', async ({ page }) => {
    await page.setContent(`<style>${css()}</style>${content}<script>${kit()}</script>`);
    const flow = page.locator('hc-flow'), viewport = flow.getByRole('region', { name: 'Delivery diagram controls' });
    await expect(viewport).toBeVisible();
    const svg = flow.locator('svg');
    const initial = await svg.getAttribute('viewBox');
    await flow.getByRole('button', { name: 'Zoom in', exact: true }).click();
    await expect(svg).not.toHaveAttribute('viewBox', initial!);
    const zoomed = await svg.getAttribute('viewBox');
    await viewport.focus();
    await page.keyboard.press('ArrowRight');
    await expect(svg).not.toHaveAttribute('viewBox', zoomed!);
    await flow.getByRole('button', { name: 'Fit diagram', exact: true }).click();
    await expect(svg).toHaveAttribute('viewBox', initial!);
    await expect(flow.locator('.hc-flow-link[data-active=true]')).toHaveCount(1);
    await flow.evaluate(el => el.remove());
    expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
});
test('flow motion is transform-only, pauses out of view and settles under both motion gates', async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    await page.setContent(`<style>${css()}hc-flow{margin-top:1600px}</style>${content}<script>${kit()}</script>`);
    await expect(page.locator('hc-flow')).toHaveAttribute('data-hc-ready', '');
    expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
    await page.locator('hc-flow').scrollIntoViewIfNeeded();
    await expect.poll(() => page.evaluate(() => document.getAnimations().length)).toBeGreaterThan(0);
    const frames = await page.evaluate(() => document.getAnimations().flatMap(a => (a.effect as KeyframeEffect).getKeyframes()));
    for (const f of frames)
        expect(Object.keys(f).filter(k => !['offset', 'computedOffset', 'easing', 'composite', 'transform', 'opacity'].includes(k))).toEqual([]);
    await page.evaluate(() => window.scrollTo(0, 0));
    await expect.poll(() => page.evaluate(() => document.getAnimations().every(a => a.playState === 'paused'))).toBe(true);
    await page.evaluate(() => { document.documentElement.dataset.hcMotion = 'reduce'; });
    await expect.poll(() => page.evaluate(() => document.getAnimations().length)).toBe(0);
    await page.locator('hc-flow').scrollIntoViewIfNeeded();
    await page.evaluate(() => { delete document.documentElement.dataset.hcMotion; });
    await expect.poll(() => page.evaluate(() => document.getAnimations().length)).toBeGreaterThan(0);
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await expect.poll(() => page.evaluate(() => document.getAnimations().length)).toBe(0);
});
test('hover and keyboard focus animate an inactive edge, hidden document pauses it', async ({ page }) => {
    await page.setContent(`<style>${css()}</style>${content}<script>${kit()}</script>`);
    const link = page.locator('.hc-flow-link').first();
    await expect(link).toHaveAttribute('tabindex', '0');
    await link.focus();
    await expect.poll(() => link.evaluate(el => el.getAnimations({ subtree: true }).filter(a => a.playState === 'running').length)).toBeGreaterThan(0);
    await page.evaluate(() => { Object.defineProperty(document, 'hidden', { configurable: true, value: true }); document.dispatchEvent(new Event('visibilitychange')); });
    await expect.poll(() => page.evaluate(() => document.getAnimations().every(a => a.playState === 'paused'))).toBe(true);
    await page.locator('hc-flow').evaluate(el => el.remove());
    expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
});
test('roving edge focus stays visible and zoom limits preserve button focus', async ({ page }) => {
    await page.setContent(`<style>${css()}</style>${content}<script>${kit()}</script>`);
    const flow = page.locator('hc-flow'), first = flow.locator('.hc-flow-link').first();
    await expect(flow.locator('.hc-flow-link[tabindex="0"]')).toHaveCount(1);
    const svg = flow.locator('svg'), initial = await svg.getAttribute('viewBox');
    await flow.getByRole('button', { name: 'Zoom in', exact: true }).click();
    await first.focus();
    await page.keyboard.press('ArrowRight');
    await expect(flow.locator('.hc-flow-link').nth(1)).toBeFocused();
    await expect(flow.locator('.hc-flow-link[tabindex="0"]')).toHaveCount(1);
    await flow.getByRole('button', { name: 'Fit diagram' }).click();
    const out = flow.getByRole('button', { name: 'Zoom out', exact: true });
    await out.focus();
    await page.keyboard.press('Enter');
    await expect(out).toBeFocused();
    await expect(svg).toHaveAttribute('viewBox', initial!);
});
test('visible active particles are bounded and hidden viewport links pause', async ({ page }) => {
    const rows = Array.from({ length: 12 }, (_, i) => `a -> n${i} {active}`).join('\n');
    await page.setContent(`<style>${css()}</style><hc-flow interactive>${rows}</hc-flow><script>${kit()}</script>`);
    await expect.poll(() => page.evaluate(() => document.getAnimations().filter(a => (a.effect as KeyframeEffect).target?.classList.contains('hc-flow-particle')).length)).toBeGreaterThan(0);
    expect(await page.evaluate(() => document.getAnimations().filter(a => (a.effect as KeyframeEffect).target?.classList.contains('hc-flow-particle') && a.playState === 'running').length)).toBeLessThanOrEqual(8);
    await page.locator('hc-flow').evaluate(el => { (el as HTMLElement).style.display = 'none'; });
    await expect.poll(() => page.evaluate(() => document.getAnimations().every(a => a.playState === 'paused'))).toBe(true);
});


test('focusing an already-visible route preserves zoom and Escape leaves a small flow',async({page})=>{
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.setContent(`<style>${css()}</style><hc-flow interactive>a -> b</hc-flow><hc-flow label="Small">c -> d</hc-flow><script>${kit()}</script>`);
  const flow=page.locator('hc-flow').first(),svg=flow.locator('svg');
  await flow.getByRole('button',{name:'Zoom in',exact:true}).click();
  const zoomed=await svg.getAttribute('viewBox');
  await flow.locator('.hc-flow-link').focus();
  await expect(svg).toHaveAttribute('viewBox',zoomed!);
  const small=page.locator('hc-flow').nth(1);
  await small.locator('.hc-flow-link').focus();await page.keyboard.press('Escape');
  await expect(small.locator('.hc-flow-box')).toBeFocused();
});


test('drag pans and only modified wheel zooms',async({page})=>{
  await page.setContent(`<style>${css()}</style>${content}<script>${kit()}</script>`);
  const flow=page.locator('hc-flow'),svg=flow.locator('svg');
  await flow.getByRole('button',{name:'Zoom in',exact:true}).click();
  const zoomed=await svg.getAttribute('viewBox'),box=(await svg.boundingBox())!;
  const x=box.x+box.width/2,y=box.y+box.height/2;
  await page.mouse.move(x,y);await page.mouse.down();await page.mouse.move(x-30,y-25);await page.mouse.up();
  await expect(svg).not.toHaveAttribute('viewBox',zoomed!);
  const panned=await svg.getAttribute('viewBox');
  await page.mouse.wheel(0,-80);await expect(svg).toHaveAttribute('viewBox',panned!);
  await page.keyboard.down('Control');await page.mouse.wheel(0,-80);await page.keyboard.up('Control');
  await expect(svg).not.toHaveAttribute('viewBox',panned!);
});

test('crowded labels retain their full connection text in the visible fallback', async ({ page }) => {
    const lines = Array.from({length:8},(_,i)=>`[G${i}]\nn${i}`);
    for(let a=0;a<8;a++) for(let b=0;b<8;b++) if(a!==b) lines.push(`n${a} -> n${b}: Connection ${a} to ${b}`);
    await page.setContent(`<style>${css()}</style><hc-flow label="Dense">${lines.join('\n')}</hc-flow><script>${kit()}</script>`);
    await expect(page.getByText(/link labels are in the connection list/)).toBeVisible();
    await page.getByText('Diagram connections', {exact:true}).click();
    await expect(page.getByRole('list', {name:'Dense as text'})).toBeVisible();
    await expect(page.getByRole('list', {name:'Dense as text'})).toContainText('Connection 0 to 1');
});
