import { test, expect, type Page } from './fixtures/test';
import AxeBuilder from '@axe-core/playwright';
import { join } from 'node:path';
import { settled } from './settle';

// Synthetic confirms only; no file, task, external write or native approval.
const request = (page: Page, title: string) => page.evaluate(async title => {
  const store = await import('/src/store/runtime.ts');
  return Promise.race([store.showConfirm({title}), new Promise(resolve => setTimeout(() => resolve('pending'), 1200))]);
}, title);

test('a tall confirmation retains its checklist viewport at Largest text', async ({page}) => {
  await page.setViewportSize({width:720,height:480});
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale','125'));
  await page.goto('/?mock=1');
  await page.evaluate(async () => {
    const s = await import('/src/store/runtime.ts');
    void s.showConfirm({title:'Review the selected local items before continuing',
      summary:'Keep the original source. Review each item before deciding what belongs in the draft. '.repeat(4),
      body:'Synthetic evidence for a tall confirmation. No local changes will be made. '.repeat(5),
      checklist:[{id:'a',label:'Project summary'},{id:'b',label:'Source notes'},{id:'c',label:'Task owners'}],
      confirmLabel:'Keep selected'});
  });
  const checklist = page.locator('.confirm-checklist');
  await expect(checklist).toBeVisible();
  expect((await checklist.boundingBox())!.height).toBeGreaterThan(32);
  const check = page.getByRole('checkbox',{name:'Source notes',exact:true});
  await check.check();
  await expect(check).toBeChecked();
  await page.getByRole('button',{name:'Cancel',exact:true}).click();
  await expect(page.locator('.confirm-dialog')).toHaveCount(0);
});

test('a failed confirm chunk cancels every later confirm instead of leaving its promise pending', async ({page}) => {
  await page.route('**/src/components/ConfirmDialog.tsx*', route => route.abort());
  await page.goto('/?mock=1');
  await expect(page.locator('.app')).toBeVisible();
  expect(await request(page, 'First synthetic confirmation')).toBe(false);
  expect(await request(page, 'Second synthetic confirmation')).toBe(false);
  expect(await request(page, 'Third synthetic confirmation')).toBe(false);
  expect(await page.evaluate(async () => {
    const s = await import('/src/store/runtime.ts');
    return s.showPrompt({title:'Synthetic name',input:{value:'old',label:'Name'}});
  })).toBe(null);
  expect(await page.evaluate(async () => {
    const s = await import('/src/store/runtime.ts');
    return s.showChecklist({title:'Synthetic selection',checklist:[{id:'one',label:'One'}]});
  })).toBe(null);
});

for (const theme of ['light','dark','glass']) test(`a pending confirm load owns the keyboard and can be cancelled: ${theme}`, async ({page}, info) => {
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.addInitScript(theme => { localStorage.setItem('theme',theme==='light'?'light':'dark'); localStorage.setItem('glass',theme==='glass'?'on':'off'); },theme);
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/src/components/ConfirmDialog.tsx*', async route => { await held; await route.continue().catch(() => {}); });
  await page.goto('/?mock=1');
  await expect(page.locator('.app')).toBeVisible();
  await page.evaluate(async () => {
    const store = await import('/src/store/runtime.ts');
    const trigger = document.createElement('button'); trigger.id = 'synthetic-confirm-trigger'; trigger.textContent = 'Synthetic trigger'; document.body.append(trigger); trigger.focus();
    (window as unknown as {confirmResult: unknown}).confirmResult = 'pending';
    void store.showConfirm({title:'Loading synthetic confirmation'}).then(value => { (window as unknown as {confirmResult: unknown}).confirmResult = value; });
  });
  try {
    await expect(page.getByRole('dialog', {name:'Loading synthetic confirmation'})).toBeVisible();
    expect(await page.evaluate(async () => (await import('/src/modalGuard.ts')).modalOpen())).toBe(true);
    await expect(page.locator('html')).toHaveAttribute('data-theme',theme==='light'?'light':'dark');
    if (theme==='glass') { await expect(page.locator('html')).toHaveAttribute('data-glass','on'); await page.addStyleTag({content:'html[data-glass] { background:var(--canvas); }'}); }
    await settled(page.getByRole('dialog', {name:'Loading synthetic confirmation'}));
    expect((await new AxeBuilder({page}).include('.confirm-backdrop').analyze()).violations).toEqual([]);
    if (process.env.S1_CAPTURE_DIR) await page.getByRole('dialog').screenshot({path:join(process.env.S1_CAPTURE_DIR,`s1-confirm-${theme}-${info.project.name}.png`)});
    await page.keyboard.press('Tab');
    await expect(page.getByRole('button',{name:'Cancel',exact:true})).toBeFocused();
    await page.keyboard.press('Escape');
    await expect.poll(() => page.evaluate(() => (window as unknown as {confirmResult: unknown}).confirmResult)).toBe(false);
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(page.locator('#synthetic-confirm-trigger')).toBeFocused();
  } finally { release(); }
});


test('a loaded confirm replaces the cancel-only fallback and still resolves the original request', async ({page}) => {
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/src/components/ConfirmDialog.tsx*', async route => { await held; await route.continue().catch(() => {}); });
  await page.goto('/?mock=1');
  await page.evaluate(async () => {
    const s = await import('/src/store/runtime.ts');
    const trigger = document.createElement('button'); trigger.id = 'synthetic-confirm-trigger'; trigger.textContent = 'Synthetic trigger'; document.body.append(trigger); trigger.focus();
    (window as unknown as {confirmResult:unknown}).confirmResult = 'pending';
    void s.showConfirm({title:'Synthetic successful load'}).then(value => { (window as unknown as {confirmResult:unknown}).confirmResult = value; });
  });
  await expect(page.getByText('Opening confirmation…',{exact:true})).toBeVisible();
  release();
  await expect(page.getByRole('button',{name:'Confirm',exact:true})).toBeFocused();
  await page.keyboard.press('Enter');
  await expect.poll(() => page.evaluate(() => (window as unknown as {confirmResult:unknown}).confirmResult)).toBe(true);
  await expect(page.locator('#synthetic-confirm-trigger')).toBeFocused();
});


test('replacing a loading request preserves its original focus return and cancels the old request', async ({page}) => {
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/src/components/ConfirmDialog.tsx*', async route => { await held; await route.continue().catch(() => {}); });
  await page.goto('/?mock=1');
  await page.evaluate(async () => {
    const s = await import('/src/store/runtime.ts');
    const trigger = document.createElement('button'); trigger.id = 'synthetic-confirm-trigger'; trigger.textContent = 'Synthetic trigger'; document.body.append(trigger); trigger.focus();
    (window as unknown as {firstResult:unknown}).firstResult = 'pending';
    void s.showConfirm({title:'First held request'}).then(value => { (window as unknown as {firstResult:unknown}).firstResult = value; });
  });
  try {
    await expect(page.getByRole('button',{name:'Cancel',exact:true})).toBeFocused();
    await page.evaluate(async () => {
      const s = await import('/src/store/runtime.ts');
      (window as unknown as {confirmResult:unknown}).confirmResult = 'pending';
      void s.showConfirm({title:'Replacement request'}).then(value => { (window as unknown as {confirmResult:unknown}).confirmResult = value; });
    });
    await expect.poll(() => page.evaluate(() => (window as unknown as {firstResult:unknown}).firstResult)).toBe(false);
    await expect(page.getByRole('dialog',{name:'Replacement request'})).toBeVisible();
    release();
    await expect(page.getByRole('button',{name:'Confirm',exact:true})).toBeFocused();
    await page.keyboard.press('Escape');
    await expect.poll(() => page.evaluate(() => (window as unknown as {confirmResult:unknown}).confirmResult)).toBe(false);
    await expect(page.locator('#synthetic-confirm-trigger')).toBeFocused();
  } finally { release(); }
});


test('a chunk failing after its loading dialog took focus returns to the original trigger', async ({page}) => {
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/src/components/ConfirmDialog.tsx*', async route => { await held; await route.abort().catch(() => {}); });
  await page.goto('/?mock=1');
  await page.evaluate(async () => {
    const s = await import('/src/store/runtime.ts');
    const trigger = document.createElement('button'); trigger.id = 'synthetic-confirm-trigger'; trigger.textContent = 'Synthetic trigger'; document.body.append(trigger); trigger.focus();
    (window as unknown as {confirmResult:unknown}).confirmResult = 'pending';
    void s.showConfirm({title:'Synthetic failed load'}).then(value => { (window as unknown as {confirmResult:unknown}).confirmResult = value; });
  });
  try {
    await expect(page.getByRole('button',{name:'Cancel',exact:true})).toBeFocused();
    release();
    await expect.poll(() => page.evaluate(() => (window as unknown as {confirmResult:unknown}).confirmResult)).toBe(false);
    await expect(page.locator('#synthetic-confirm-trigger')).toBeFocused();
    expect(await request(page,'Later synthetic request')).toBe(false);
    await expect(page.locator('#synthetic-confirm-trigger')).toBeFocused();
  } finally { release(); }
});
