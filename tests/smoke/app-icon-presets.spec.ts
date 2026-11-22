import { expect, test, type Page } from './fixtures/test';
import { openSettingsIcons, openSettingsSection } from './choose';
import AxeBuilder from '@axe-core/playwright';

async function openAppearance(page:Page) {
  await page.getByRole('navigation',{name:'Shared tools'}).getByRole('button',{name:'Settings',exact:true}).click();
  await openSettingsSection(page, 'General');
  await openSettingsIcons(page);
  const row=page.getByRole('region',{name:'App icon',exact:true});
  await expect(row.getByRole('status')).toContainText('Selected:');
  return row;
}
for(const theme of ['light','dark']) test(`five presets, keyboard, persistence and default in ${theme}`,async({page},info)=>{
  await page.addInitScript(t=>localStorage.setItem('theme',t),theme);
  await page.goto('/?mock=1');
  let row=await openAppearance(page);
  const mark=page.locator('.work-brand .hollow-mark');
  const stock=await mark.getAttribute('src');
  await expect(row.getByRole('button',{name:'Fold (default)',exact:true})).toHaveAttribute('aria-pressed','true');
  await expect(page.getByRole('region',{name:'COO icon',exact:true}).getByRole('group',{name:'App icon presets'})).toHaveCount(0);
  await expect(row.getByRole('group',{name:'App icon presets'}).getByRole('button')).toHaveCount(5);
  await row.screenshot({path:info.outputPath(`picker-${theme}.png`)});
  expect((await new AxeBuilder({page}).include('.custom-icon-rows').withTags(['wcag2a','wcag2aa']).analyze()).violations).toEqual([]);
  for(const [name,id] of [['Cut Glass','cut-glass'],['Threshold','threshold'],['Aurora','aurora'],['Monolith','monolith']]) {
    const button=row.getByRole('button',{name,exact:true});
    await button.focus();await page.keyboard.press('Space');
    await expect(row.getByRole('status')).toHaveText(`Selected: ${name}`);
    await expect(button).toHaveAttribute('aria-pressed','true');await expect(button).toBeFocused();
    await expect(mark).toHaveAttribute('src',/^data:image\/png;base64,/);
    const saved=await mark.getAttribute('src');
    await expect(page.locator('link[rel="icon"]')).toHaveAttribute('href',saved!);
    expect(await page.evaluate(()=>JSON.parse(localStorage.getItem('mockCustomIcon:app')!).preset)).toBe(id);
    expect(await page.evaluate(()=>localStorage.getItem('mockCustomIcon:mainAgent'))).toBeNull();
    await page.reload();row=await openAppearance(page);
    await expect(row.getByRole('status')).toHaveText(`Selected: ${name}`);
    await expect(mark).toHaveAttribute('src',saved!);
  }
  await row.getByRole('button',{name:'Fold (default)',exact:true}).click();
  await expect(row.getByRole('status')).toHaveText('Selected: Fold (default)');
  await expect(mark).toHaveAttribute('src',stock!);
  expect(await page.evaluate(()=>JSON.parse(localStorage.getItem('mockCustomIcon:app')!))).toBeNull();
  await expect(page.locator('link[rel="icon"]')).toHaveAttribute('href','/hollow-32.png');
  await row.getByRole('button',{name:'Threshold',exact:true}).click();
  await expect(row.getByRole('status')).toHaveText('Selected: Threshold');
  await row.getByRole('button',{name:'Reset to Fold',exact:true}).click();
  await expect(row.getByRole('status')).toHaveText('Selected: Fold (default)');
});

test('existing custom survives opening Settings; an upload over a preset becomes Custom',async({page})=>{
  await page.goto('/?mock=1');let row=await openAppearance(page);
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'App icon',exact:true});
  const svg='<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#7050ac"/><circle cx="32" cy="32" r="16" fill="#ffffff"/></svg>';
  await editor.getByLabel('Choose icon file').setInputFiles({name:'mine.svg',mimeType:'image/svg+xml',buffer:Buffer.from(svg)});
  await editor.getByRole('button',{name:'Apply icon'}).click();
  await expect(editor).toBeHidden();await expect(row.getByRole('status')).toHaveText('Selected: Custom');
  const custom=await page.evaluate(()=>localStorage.getItem('mockCustomIcon:app'));
  await page.reload();row=await openAppearance(page);
  await expect(row.getByRole('status')).toHaveText('Selected: Custom');
  expect(await page.evaluate(()=>localStorage.getItem('mockCustomIcon:app'))).toBe(custom);
  await expect(row.locator('.app-icon-preset[aria-pressed="true"]')).toHaveCount(0);
  await row.getByRole('button',{name:'Aurora',exact:true}).click();
  await page.getByRole('dialog',{name:'Replace your custom icon?'}).getByRole('button',{name:'Replace icon',exact:true}).click();
  await expect(row.getByRole('status')).toHaveText('Selected: Aurora');
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  await editor.getByLabel('Choose icon file').setInputFiles({name:'mine.svg',mimeType:'image/svg+xml',buffer:Buffer.from(svg)});
  await editor.getByRole('button',{name:'Apply icon'}).click();
  await expect(row.getByRole('status')).toHaveText('Selected: Custom');
});

test('failed preset retains prior artwork and can retry',async({page})=>{
  await page.goto('/?mock=1');const row=await openAppearance(page);
  await row.getByRole('button',{name:'Threshold',exact:true}).click();
  await expect(row.getByRole('status')).toHaveText('Selected: Threshold');
  const before=await page.evaluate(()=>localStorage.getItem('mockCustomIcon:app'));
  await page.evaluate(()=>{window.__mockFailCommandOnce='select_app_icon_preset';});
  await row.getByRole('button',{name:'Monolith',exact:true}).click();
  await expect(row.getByRole('alert')).toBeVisible();
  await expect(row.getByRole('status')).toHaveText('Selected: Threshold');
  expect(await page.evaluate(()=>localStorage.getItem('mockCustomIcon:app'))).toBe(before);
  await row.getByRole('button',{name:'Monolith',exact:true}).click();
  await expect(row.getByRole('status')).toHaveText('Selected: Monolith');
});

test('a delayed mutation response cannot replace a newer window selection',async({page})=>{
  await page.goto('/?mock=1');const row=await openAppearance(page);
  await page.evaluate(()=>{
    const w=window as typeof window & {releaseIcon?:()=>void};
    window.__mockHoldOnce={command:'select_app_icon_preset',release:new Promise<void>(resolve=>{w.releaseIcon=resolve;})};
  });
  await row.getByRole('button',{name:'Threshold',exact:true}).click();
  await expect.poll(()=>page.evaluate(()=>JSON.parse(localStorage.getItem('mockCustomIcon:app')??'null')?.preset)).toBe('threshold');
  await page.evaluate(async()=>{
    const modulePath='/src/mockApi.ts';
    const {mockInvoke}=await import(/* @vite-ignore */ modulePath);
    const saved=JSON.parse(localStorage.getItem('mockCustomIcon:app')!);
    await mockInvoke('select_app_icon_preset',{preset:'monolith',expected:saved.revision});
    (window as typeof window & {releaseIcon?:()=>void}).releaseIcon!();
  });
  await expect(row.getByRole('status')).toHaveText('Selected: Monolith');
  const saved=await page.evaluate(()=>JSON.parse(localStorage.getItem('mockCustomIcon:app')!).pngs[128]);
  await expect(page.locator('.work-brand .hollow-mark')).toHaveAttribute('src',`data:image/png;base64,${saved}`);
  await expect(page.locator('link[rel="icon"]')).toHaveAttribute('href',`data:image/png;base64,${saved}`);
});
