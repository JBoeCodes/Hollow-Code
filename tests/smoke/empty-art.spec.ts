import { expect, test, type Page } from './fixtures/test';
import AxeBuilder from '@axe-core/playwright';
import { join } from 'node:path';
async function boot(page: Page) { await page.goto('/?mock=1'); await expect(page.locator('.top-bar')).toBeVisible(); }
async function study(page: Page, theme='light') {
  await page.addInitScript(t=>{localStorage.setItem('theme',t==='light'?'light':'dark');localStorage.setItem('glass',t==='glass'?'on':'off');},theme);
  await page.goto('/?mock=1&window=smoke-artifact&empty-art=1');
  await expect(page.locator('html')).toHaveAttribute('data-theme',theme==='light'?'light':'dark');
  if(theme==='glass') await expect(page.locator('html')).toHaveAttribute('data-glass','on');
  if(theme==='glass') await page.addStyleTag({content:'html[data-glass] { background:var(--canvas); }'});
  await expect(page.locator('.empty-art')).toHaveCount(9);
}
for(const theme of ['light','dark','glass']) test(`empty art ${theme}: coherent responsive family`,async({page},info)=>{
  await study(page,theme);
  for(const kind of ['inbox','commitments','results','sources','mission','search','waiting','tasks','automations']) {
    const art=page.locator(`[data-scene="${kind}"]`);await expect(art).toBeVisible();await expect(art).toHaveAttribute('aria-hidden','true');
    expect(await art.locator('svg').evaluate(el=>el.querySelectorAll('[tabindex],a,button').length)).toBe(0);
  }
  expect(await page.locator('main').evaluate(el=>el.scrollWidth>el.clientWidth+1)).toBe(false);
  expect((await new AxeBuilder({page}).include('main').withTags(['wcag2a','wcag2aa']).analyze()).violations).toEqual([]);
  if(process.env.HOLLOW_LANE_SHOTS) await page.locator('main').screenshot({animations:'disabled',path:join(process.env.HOLLOW_LANE_SHOTS,`v7-${theme}-${info.project.name}.png`)});
});
test('static stipple yields to texture and accessibility policy without hiding the drawing',async({page})=>{
  await study(page);const art=page.locator('[data-scene="inbox"]');const accent=art.locator('.empty-art-accent');
  await expect(accent).toHaveCSS('animation-name','none');
  const scene=art.locator('.empty-art-scene');
  await page.evaluate(()=>{document.documentElement.dataset.textureActive='true';});
  await expect(scene).not.toHaveCSS('stroke-dasharray','none');
  await page.evaluate(()=>{document.documentElement.dataset.textureActive='false';});
  await expect(scene).toHaveCSS('stroke-dasharray','none');
  await page.evaluate(()=>{document.documentElement.dataset.textureActive='true';document.documentElement.dataset.motion='reduced';});
  await expect(scene).toHaveCSS('stroke-dasharray','none');
  await page.evaluate(()=>delete document.documentElement.dataset.motion);
  await page.emulateMedia({reducedMotion:'reduce'});await expect(scene).toHaveCSS('stroke-dasharray','none');
  await page.emulateMedia({reducedMotion:'no-preference',forcedColors:'active'});await expect(scene).toHaveCSS('stroke-dasharray','none');
  await expect(art.locator('svg')).toBeVisible();
});
test('Waiting on you never celebrates an unavailable source',async({page})=>{
  await boot(page);await page.evaluate(async()=>{const path='/src/api.ts';(await import(/* @vite-ignore */ path)).api.listNeedsYou=async()=>({items:[],total:0,nextCursor:null,unavailable:[]});const store='/src/store/waitingOnYou.ts';const s=await import(/* @vite-ignore */ store);s.openWaitingOnYou();await s.waitingList.load();});
  const dialog=page.getByRole('region',{name:'Waiting on you',exact:true});await expect(dialog.locator('[data-scene="waiting"]')).toBeVisible();
  await page.evaluate(async()=>{const path='/src/api.ts';(await import(/* @vite-ignore */ path)).api.listNeedsYou=async()=>({items:[],total:0,nextCursor:null,unavailable:['synthetic source']});const store='/src/store/waitingOnYou.ts';await (await import(/* @vite-ignore */ store)).waitingList.load();});
  await expect(dialog).toContainText('Nothing waiting in the sources that loaded.');await expect(dialog.locator('.empty-art')).toHaveCount(0);
});
test('Mission Control idle and failed refresh are distinct',async({page})=>{
  await boot(page);await page.evaluate(async()=>{const path='/src/api.ts';(await import(/* @vite-ignore */ path)).api.listRunQueue=async()=>[];const store='/src/store/missionControl.ts';(await import(/* @vite-ignore */ store)).openMissionControl();});
  const dialog=page.getByRole('region',{name:'Running',exact:true});
  for(const mode of ['Tracks','Orbit','List']) { await dialog.getByRole('button',{name:mode,exact:true}).click();await expect(dialog.locator('[data-scene="mission"]')).toBeVisible(); }
  await page.evaluate(async()=>{const path='/src/api.ts';(await import(/* @vite-ignore */ path)).api.listRunQueue=async()=>{throw new Error('synthetic queue failure');};const store='/src/store/runQueue.ts';(await import(/* @vite-ignore */ store)).refreshRunQueue(0);});await expect(dialog).toContainText('Couldn\'t refresh the run list.');await expect(dialog).toContainText('The run list is unavailable.');await expect(dialog.locator('.empty-art')).toHaveCount(0);
});
test('Palette no matches keeps the search input focused',async({page})=>{
  await boot(page);await page.keyboard.press('Meta+k');const input=page.locator('.palette-input');await input.fill('zzqvnoexist98765');await expect(page.locator('.palette-empty [data-scene="search"]')).toBeVisible();await expect(input).toBeFocused();await expect(page.locator('.palette-empty')).toContainText('No matches');
});
