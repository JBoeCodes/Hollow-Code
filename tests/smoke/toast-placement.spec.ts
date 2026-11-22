import { expect, test, type Page } from './fixtures/test';
import { settled } from './settle';

async function installEventBridge(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<string, number[]>();
    let next = 1;
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {
      transformCallback(callback: (event: unknown) => void) {
        const id = next++;
        callbacks.set(id, callback);
        return id;
      },
      invoke(command: string, args: { event?: string; handler?: number }) {
        if (command === 'plugin:event|listen' && args?.event && args.handler) {
          listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]);
          return Promise.resolve(next++);
        }
        if (command === 'plugin:event|unlisten') return Promise.resolve();
        return Promise.reject(new Error(`not in Tauri: ${command}`));
      },
    };
    w.__emitTauriEvent = (event: string, payload: unknown) => {
      for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload });
    };
  });
}

const views = [
  ['Mission Control', 'missionControl', 'openMissionControl', 'closeMissionControl', '.mc-card'],
] as const;

async function action(page: Page, module: string, method: string) {
  await page.evaluate(async ([module, method]) => {
    const path = `/src/store/${module}.ts`;
    (await import(/* @vite-ignore */path))[method]();
  }, [module, method]);
}
async function notice(page: Page, count = 1, ttl = 30000) {
  await page.evaluate(async ([count, ttl]) => {
    const path = '/src/store/runtime.ts';
    const store = await import(/* @vite-ignore */path);
    for (let i = 0; i < count; i++) store.flashWithUndo({ message: `Saved fixture ${i}`, ttlMs: ttl,
      undo: async () => { document.body.dataset.undoCount = String(Number(document.body.dataset.undoCount ?? 0) + 1); },
    });
  }, [count, ttl]);
}
for (const [name, module, open, close, card] of views) {
  test(`${name}: notices occupy space inside its focus area`, async ({ page }) => {
    await installEventBridge(page);
    await page.goto('/?mock=1&demo=studio');
    // Let the first screen settle: Review → Incoming is a page navigation, and
    // the launch's own restore of the last page would otherwise win the race.
    await expect(page.locator('.studio-home')).toBeVisible();
    await action(page, module, open);
    await expect(page.locator(card)).toBeVisible();
    await notice(page);
    await page.evaluate(() => (window as unknown as { __emitTauriEvent: (event: string, data: unknown) => void })
      .__emitTauriEvent('notification-event', { id: 'fixture-toast', title: 'Fixture notification', body: 'A local operation finished.',
        source: 'run-guard', kind: 'success', created: new Date().toISOString(), read: false }));
    await expect(page.locator(`${card} .notif-toast`)).toBeVisible();
    await expect(page.locator(`${card} .snackbar`)).toBeVisible();
    // Measure the shelf against the header only once both stopped moving, and
    // wait for the answer rather than reading it once.
    await settled(page.locator(card), '.view-toast-region, header, .today-close, .wr-close');
    await expect.poll(() => page.locator(card).evaluate(el => {
      const a = el.querySelector('.view-toast-region')!.getBoundingClientRect();
      return [...el.querySelectorAll('header, .today-close, .wr-close')].some(node => {
        const b = node.getBoundingClientRect();
        return a.width > 0 && b.width > 0 && a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
      });
    })).toBe(false);
    await page.locator(`${card} .snackbar-action`).focus();
    await page.keyboard.press('Tab');
    await expect(page.locator(`${card} .snackbar-close`)).toBeFocused();
    await page.keyboard.press('Shift+Tab');
    await expect(page.locator(`${card} .snackbar-action`)).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(page.locator('body')).toHaveAttribute('data-undo-count', '1');
    await notice(page);
    await action(page, module, close);
    await expect(page.locator(card)).toHaveCount(0);
    await expect(page.locator('.snackbar')).toHaveCount(1);
    // The one notice left is not inside a modal (web-first: waits, never samples).
    await expect(page.locator('[aria-modal="true"] .snackbar')).toHaveCount(0);
  });
}

test('a notice follows Running, the view that stayed after Incoming and Commitments were cut', async ({ page }) => {
  await page.goto('/?mock=1&demo=studio');
  await expect(page.locator('.studio-home')).toBeVisible();
  await action(page, 'missionControl', 'openMissionControl');
  await expect(page.locator('.mc-card')).toBeVisible();
  await notice(page, 1, 30000);
  await expect(page.locator('.mc-card .snackbar')).toHaveCount(1);
});


for (const theme of ['light', 'dark', 'glass']) {
  test(`${theme}: bounded stacks preserve controls in every view`, async ({ page }, info) => {
    await page.addInitScript(theme => {
      localStorage.setItem('theme', theme === 'light' ? 'light' : 'dark');
      localStorage.setItem('glass', theme === 'glass' ? 'on' : 'off');
    }, theme);
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/?mock=1&demo=studio');
    // As above: the launch's restore of the last page must land before a view opens.
    await expect(page.locator('.studio-home')).toBeVisible();
    await notice(page, 5);
    for (const [name, module, open, close, card] of views) {
      await action(page, module, open);
      const region = page.locator(`${card} .view-toast-region`);
      await expect(region.locator('.snackbar')).toHaveCount(5);
      // Bounded: taller than one bar, no taller than min(28vh, 220px), and no
      // sideways overflow. One poll over one reading, and only a height that
      // held still across two frames counts: separate polls could each pass
      // on a different moment of a shelf still growing as the bars mount.
      const max = Math.min(220, page.viewportSize()!.height * .28) + 1;
      await expect.poll(() => region.evaluate(async (el, max) => {
        const first = el.getBoundingClientRect().height;
        await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        const height = el.getBoundingClientRect().height;
        const overflow = el.scrollWidth - el.clientWidth;
        return { steady: height === first, tall: height > 40, bounded: height <= max, fits: overflow <= 1, height, overflow };
      }, max), { message: `${name}: the five-notice shelf is bounded (height in (40, ${max}], no sideways overflow)` })
        .toMatchObject({ steady: true, tall: true, bounded: true, fits: true });
      for (const bar of await region.locator('.snackbar').all()) await expect(bar).toHaveCSS('opacity', '1');
      await page.screenshot({ path: info.outputPath(`${theme}-${name.replaceAll(' ', '-')}.png`) });
      // Today, Activity and Weekly review are views of Work since
      // consolidation D: leave them the way a person does, by its Workspaces tab.
      if (['Today', 'Mission Control', 'Weekly review'].includes(name)) await page.getByRole('navigation', { name: 'Projects views' }).getByRole('button', { name: 'Home' }).click();
      else if (name === 'Inbox') await page.getByRole('navigation', { name: 'Review views' }).getByRole('button', { name: /^Waiting/ }).click();
      else await action(page, module, close);
      await expect(page.locator(card)).toHaveCount(0);
    }
  });
}

for (const [name, module, open, , card] of views.filter(view => ['Today', 'Commitments', 'Weekly review'].includes(view[0]))) {
  test(`${name}: notices remain reachable while reading the bottom`, async ({ page }) => {
    await page.goto('/?mock=1&demo=studio');
    await expect(page.locator('.studio-home')).toBeVisible();
    await action(page, module, open);
    await expect(page.locator(card)).toBeVisible();
    await page.locator(card).evaluate(el => {
      const content = el.querySelector('.view-toast-content') ?? el;
      const spacer = document.createElement('div');
      spacer.style.height = '1600px'; spacer.style.flexShrink = '0';
      spacer.textContent = 'Synthetic long content'; content.append(spacer);
      const scroller = el.querySelector('.view-toast-content') ?? el.parentElement!;
      scroller.scrollTop = scroller.scrollHeight;
    });
    await notice(page);
    // The bar springs in from 14px below its resting place: measure it at rest,
    // and wait for the position instead of reading one boundingBox().
    const bar = page.locator(`${card} .snackbar`);
    await expect(bar).toBeVisible();
    await settled(bar);
    await expect.poll(async () => (await bar.boundingBox())?.y ?? -1).toBeGreaterThanOrEqual(0);
    await expect.poll(async () => {
      const bounds = await bar.boundingBox();
      return bounds ? bounds.y + bounds.height : Infinity;
    }).toBeLessThanOrEqual(page.viewportSize()!.height);
    await page.locator(`${card} .snackbar-action`).click();
    await expect(page.locator('body')).toHaveAttribute('data-undo-count', '1');
  });
}

for (const [name, module, open, , card] of views.filter(view => ['Today', 'Weekly review'].includes(view[0]))) {
  test(`${name}: keyboard reading starts in the scrollable content`, async ({ page }) => {
    await page.goto('/?mock=1&demo=studio');
    await expect(page.locator('.studio-home')).toBeVisible();
    await action(page, module, open);
    const content = page.locator(`${card} .view-toast-content`);
    await expect(content).toBeFocused();
    await content.evaluate(el => {
      const spacer = document.createElement('div'); spacer.style.height = '1600px'; spacer.style.flexShrink = '0'; el.append(spacer);
    });
    await page.keyboard.press('PageDown');
    await expect.poll(() => content.evaluate(el => el.scrollTop)).toBeGreaterThan(0);
    // A view of Work since consolidation D: Escape does not leave it; the
    // Work tabs do.
    await page.getByRole('navigation', { name: 'Projects views' }).getByRole('button', { name: 'Home' }).click();
    await expect(page.locator(card)).toHaveCount(0);
  });
}
