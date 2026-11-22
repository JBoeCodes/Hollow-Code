import {test,expect} from './fixtures/test';
import { openSettingsIcons, openSettingsSection } from './choose';

test('Save as skill stays disabled for unsaved title and description edits',async({page})=>{
  await page.goto('/?mock=1');
  await page.getByRole('navigation',{name:'Main'}).getByRole('button',{name:'Automations',exact:true}).click();
  await page.getByRole('button',{name:'Open Morning Brief',exact:true}).click();
  await page.getByRole('button',{name:'Automation options'}).click();
  await page.getByRole('menuitem',{name:'Edit',exact:true}).click();
  const save=page.getByRole('button',{name:'Save as skill',exact:true});
  await expect(save).toBeEnabled();
  await page.locator('.workflow-title-input').fill('Unsaved title');
  await expect(save).toBeDisabled();
  await page.locator('.workflow-title-input').fill('Morning Brief');
  await expect(save).toBeEnabled();
  await page.locator('.workflow-description-input').fill('Unsaved description');
  await expect(save).toBeDisabled();
});

test('Save as skill refuses a duplicate and Cancel leaves it alone',async({page})=>{
  await page.goto('/?mock=1');
  await page.evaluate(async()=>{
    const path='/src/api.ts';const {api}=await import(/* @vite-ignore */path);
    await api.writeUserSkill('morning-brief','Original','Keep this');
  });
  await page.getByRole('navigation',{name:'Main'}).getByRole('button',{name:'Automations',exact:true}).click();
  await page.getByRole('button',{name:'Open Morning Brief',exact:true}).click();
  await page.getByRole('button',{name:'Automation options'}).click();
  await page.getByRole('menuitem',{name:'Edit',exact:true}).click();
  await page.getByRole('button',{name:'Save as skill',exact:true}).click();
  const confirm=page.getByRole('dialog',{name:'Replace the skill /morning-brief?'});
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button',{name:'Cancel',exact:true}).click();
  await expect(confirm).toBeHidden();
  const skills=await page.evaluate(async()=>{
    const path='/src/api.ts';const {api}=await import(/* @vite-ignore */path);
    return (await api.listSkills()).filter((s:{name:string})=>s.name==='morning-brief');
  });
  expect(skills).toHaveLength(1);
  expect(skills[0].description).toBe('Original');
});

test('creating a skill with an existing name asks and Cancel keeps the original',async({page})=>{
  await page.goto('/?mock=1');
  await page.getByRole('navigation',{name:'Shared tools'}).getByRole('button',{name:'Settings',exact:true}).click();
  await openSettingsSection(page, 'Connectors');
  await page.getByRole('button', { name: 'Open Library, Skills', exact: true }).click(); // Skills moved to Library in Settings PR 4
  await page.getByRole('button',{name:'New skill',exact:true}).click();
  await page.getByPlaceholder('my-skill').fill('foo');
  await page.getByPlaceholder('When should an agent use this skill?').fill('Keep original');
  await page.getByRole('button',{name:'Create skill',exact:true}).click();
  await page.getByRole('button',{name:'New skill',exact:true}).click();
  await page.getByPlaceholder('my-skill').fill('foo');
  await page.getByPlaceholder('When should an agent use this skill?').fill('Replacement');
  await page.getByRole('button',{name:'Create skill',exact:true}).click();
  const confirm=page.getByRole('dialog');
  await expect(confirm).toContainText('Replace the skill /foo?');
  await expect(confirm).toContainText("History can't undo this");
  await expect(confirm.getByRole('button',{name:'Save under a new name',exact:true})).toBeVisible();
  const calls=await page.evaluate(()=>window.__mockApiCalls?.write_user_skill??0);
  await confirm.getByRole('button',{name:'Cancel',exact:true}).click();
  await expect(confirm).toBeHidden();
  const matches=await page.evaluate(async()=>{
    const path='/src/api.ts';const {api}=await import(/* @vite-ignore */path);
    return (await api.listSkills()).filter((s:{name:string})=>s.name==='foo');
  });
  expect(matches).toHaveLength(1);
  expect(matches[0].description).toBe('Keep original');
  expect(await page.evaluate(()=>window.__mockApiCalls?.write_user_skill??0)).toBe(calls);
});

test('Aurora over a custom icon confirms and Cancel never changes the saved icon',async({page})=>{
  await page.goto('/?mock=1');
  await page.getByRole('navigation',{name:'Shared tools'}).getByRole('button',{name:'Settings',exact:true}).click();
  await openSettingsSection(page, 'General');
  await openSettingsIcons(page);
  const row=page.getByRole('region',{name:'App icon',exact:true});
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const dialog=page.getByRole('dialog',{name:'App icon',exact:true});
  await dialog.getByLabel('Choose icon file').setInputFiles({name:'mine.svg',mimeType:'image/svg+xml',buffer:Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#7050ac"/><circle cx="32" cy="32" r="16" fill="#ffffff"/></svg>')});
  await dialog.getByRole('button',{name:'Apply icon'}).click();
  await expect(row.getByRole('status')).toHaveText('Selected: Custom');
  const before=await page.evaluate(()=>localStorage.getItem('mockCustomIcon:app'));
  const calls=await page.evaluate(()=>window.__mockApiCalls?.select_app_icon_preset??0);
  await row.getByRole('button',{name:'Aurora',exact:true}).click();
  const confirm=page.getByRole('dialog',{name:'Replace your custom icon?'});
  await expect(confirm).toContainText('Its original is deleted');
  await confirm.getByRole('button',{name:'Cancel',exact:true}).click();
  await expect(confirm).toBeHidden();
  expect(await page.evaluate(()=>localStorage.getItem('mockCustomIcon:app'))).toBe(before);
  expect(await page.evaluate(()=>window.__mockApiCalls?.select_app_icon_preset??0)).toBe(calls);
});

test('rewriting an edited prep pack asks before any model request',async({page})=>{
  await page.goto('/?mock=1');
  await page.evaluate(async()=>{
    const apiPath='/src/api.ts',prepPath='/src/store/prepPack.ts';
    const {api}=await import(/* @vite-ignore */apiPath);const prep=await import(/* @vite-ignore */prepPath);
    const w=window as unknown as {rewrites:number};w.rewrites=0;
    api.prepPack=async(id:string,regenerate:boolean)=>{
      if(regenerate)w.rewrites++;
      return {meetingId:id,subject:'Edited planning',start:'2026-10-22T13:00:00Z',path:'outputs/prep/edited.md',workspaceId:null,markdown:'# My edited notes',kept:true,edited:true,contentHash:'exact-edited-content'};
    };
    prep.openPrepPack('edited-meeting','Edited planning');
  });
  const pack=page.getByRole('dialog',{name:'Prep: Edited planning'});
  await expect(pack).toContainText('My edited notes');
  await pack.getByRole('button',{name:'Write it again',exact:true}).click();
  const confirm=page.getByRole('dialog',{name:'Replace your edited note?'});
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button',{name:'Cancel',exact:true}).click();
  await expect(confirm).toBeHidden();
  await expect(pack).toContainText('My edited notes');
  expect(await page.evaluate(()=>(window as unknown as {rewrites:number}).rewrites)).toBe(0);
});
