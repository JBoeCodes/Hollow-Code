import { expect, test } from './fixtures/test';
import AxeBuilder from '@axe-core/playwright';
import { join } from 'node:path';
for (const theme of ['light', 'dark', 'glass'])
    test(`flow studio in ${theme}`, async ({ page }, info) => {
        await page.addInitScript(t => { localStorage.setItem('theme', t === 'light' ? 'light' : 'dark'); localStorage.setItem('glass', t === 'glass' ? 'on' : 'off'); }, theme);
        await page.goto('/?mock=1&window=smoke-artifact');
        const region = page.getByRole('region', { name: 'Canvas kit gallery', exact: true });
        await region.getByRole('button', { name: 'Expand artifact' }).click();
        // Expand can leave this lower gallery offscreen. Bring the host frame into view
        // before asking its throttled animation frame loop to prove an inner box stable.
        await region.locator('iframe').scrollIntoViewIfNeeded();
        const frame = region.frameLocator('iframe'), studio = frame.getByRole('region', { name: 'Flow studio' });
        await studio.scrollIntoViewIfNeeded();
        await expect(studio.locator('hc-flow')).toHaveAttribute('data-hc-ready', '');
        await expect(studio.getByRole('button', { name: 'Fit diagram' })).toBeVisible();
        expect(await studio.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
        const result = await new AxeBuilder({ page }).include('[aria-label="Canvas kit gallery"] iframe').withTags(['wcag2a', 'wcag2aa']).analyze();
        expect(result.violations.filter(v => v.impact === 'critical' || v.impact === 'serious')).toEqual([]);
        if (process.env.HOLLOW_LANE_SHOTS)
            await studio.screenshot({ path: join(process.env.HOLLOW_LANE_SHOTS, `v2-${theme}-${info.project.name}.png`) });
    });
