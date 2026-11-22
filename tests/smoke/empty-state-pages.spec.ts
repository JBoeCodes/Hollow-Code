import { choose, openTasks } from './choose';
import { expect, test, type Page } from './fixtures/test';

async function prepare(page:Page, view:'Tasks'|'Automations') {
  await page.goto('/?mock=1'); await expect(page.locator('.top-bar')).toBeVisible();
  await page.evaluate(async()=>{
    const p='/src/api.ts';const {api}=await import(/* @vite-ignore */p);
    const base=await api.tasks(); api.tasks=async()=>({...base,items:[],syncError:null});
    api.listWorkflows=async()=>[];
    const s='/src/store.ts'; await (await import(/* @vite-ignore */s)).refetchTasks();
  });
  await page.getByRole('navigation',{name:'Main'}).getByRole('button',{name:view,exact:true}).click();
}

test('Automations artwork follows ready empty data without masking refresh failures or filters',async({page})=>{
  await prepare(page,'Automations');
  const empty=page.locator('.workflow-empty');
  await expect(page.getByRole('heading',{name:'No automations yet',exact:true})).toHaveCount(1);
  await expect(empty).toBeVisible();
  await expect(empty.locator('[data-scene="automations"]')).toHaveCount(1);
  await expect(page.getByRole('button',{name:'New automation',exact:true})).toBeVisible();
  await page.getByRole('textbox',{name:'Find an automation'}).fill('missing');
  await expect(page.locator('.workflow-empty-list .empty-art')).toHaveCount(0);
  await expect(page.getByRole('heading',{name:'No matching automations'})).toBeVisible();
  await expect(page.getByRole('button',{name:'Clear search',exact:true})).toBeVisible();
  await page.getByRole('textbox',{name:'Find an automation'}).fill('');
  await expect(empty.locator('[data-scene="automations"]')).toHaveCount(1);
  for(const mode of ['loading','failed','ready']) {
    await page.evaluate(async mode=>{
      const p='/src/api.ts';const {api}=await import(/* @vite-ignore */p);
      api.listWorkflows=()=>mode==='loading'?new Promise(()=>{}):mode==='failed'?Promise.reject(new Error('synthetic workflow failure')):Promise.resolve([]);
      const s='/src/store.ts';void (await import(/* @vite-ignore */s)).refetchWorkflows()?.catch(()=>{});
    },mode);
    if(mode==='ready') await expect(empty.locator('[data-scene="automations"]')).toHaveCount(1);
    else {
      await expect(empty.locator('.empty-art')).toHaveCount(0);
      await expect(empty.locator('.ui-empty-icon')).toHaveCount(1);
      await expect(empty.getByRole('heading',{name:'No automations yet'})).toHaveCount(1);
      await expect(page.getByRole('button',{name:'New automation',exact:true})).toBeVisible();
    }
    if(mode==='failed') await expect(page.getByRole('status',{name:'Couldn’t refresh automations'})).toBeVisible();
  }
});

for(const width of [1440,760]) test(`an open automation draft survives an empty list refresh at ${width}px`,async({page})=>{
  await page.setViewportSize({width,height:900});
  await page.goto('/?mock=1');
  await page.getByRole('navigation',{name:'Main'}).getByRole('button',{name:'Automations',exact:true}).click();
  await page.getByRole('button',{name:'Open Morning Brief',exact:true}).click();
  await page.getByRole('button',{name:'Automation options'}).click();
  await page.getByRole('menuitem',{name:'Edit',exact:true}).click();
  const prompt=page.getByRole('textbox',{name:'Prompt',exact:true});
  await prompt.fill('Keep the only open draft');
  await page.evaluate(async()=>{
    const p='/src/api.ts';const {api}=await import(/* @vite-ignore */p);
    api.listWorkflows=async()=>[];
    const s='/src/store.ts';await (await import(/* @vite-ignore */s)).refetchWorkflows();
  });
  await expect(page.locator('.workflow-row')).toHaveCount(0);
  await expect(prompt).toBeVisible();
  await expect(prompt).toHaveValue('Keep the only open draft');
  await openTasks(page);
  const confirm=page.getByRole('dialog',{name:'Discard unsaved changes to Morning Brief?'});
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button',{name:'Keep editing',exact:true}).click();
  await expect(confirm).toBeHidden();
  await expect(prompt).toBeVisible();
  await expect(prompt).toHaveValue('Keep the only open draft');
});

test('the first created automation stays visible when its list refresh fails',async({page})=>{
  await page.addInitScript(()=>localStorage.setItem('mockWorkspaces',JSON.stringify([{id:'created-fixture',title:'Created fixture',status:'active',path:'workspaces/created-fixture',cwd:'/fixture/workspaces/created-fixture',modified:'2026-11-04T10:00:00Z'}])));
  await prepare(page,'Automations');
  await page.getByRole('button',{name:'New automation',exact:true}).click();
  await choose(page.getByRole('combobox',{name:'Automation workspace'}),'created-fixture');
  await page.getByPlaceholder('e.g. Weekly evidence review').fill('First automation');
  await page.locator('.workflow-new textarea').fill('A checked synthetic procedure');
  await page.evaluate(async()=>{
    const p='/src/api.ts';const {api}=await import(/* @vite-ignore */p);
    const write=api.writeWorkflow;
    api.writeWorkflow=async(...args:Parameters<typeof write>)=>{
      const result=await write(...args);
      api.listWorkflows=async()=>{throw new Error('Synthetic post-create refresh failure');};
      return result;
    };
  });
  await page.getByRole('button',{name:'Create automation',exact:true}).click();
  await expect(page.getByRole('status',{name:'Couldn’t refresh automations'})).toBeVisible();
  await expect(page.locator('.workflow-review').getByRole('heading',{name:'First automation',exact:true})).toBeVisible();
});

test('empty Automations keeps one explanation and a reachable creator at narrow widths',async({page})=>{
  await page.setViewportSize({width:760,height:700});
  await prepare(page,'Automations');
  const empty=page.locator('.workflow-empty');
  await expect(empty.getByRole('heading',{name:'No automations yet'})).toBeVisible();
  await expect(empty.getByRole('heading',{name:'No automations yet'})).toBeInViewport();
  await expect(page.getByRole('heading',{name:'No automations yet',exact:true})).toHaveCount(1);
  await expect(empty).toContainText('Start one with New automation');
  await page.getByRole('button',{name:'New automation',exact:true}).click();
  await expect(page.locator('.workflow-new')).toBeVisible();
  await page.locator('.workflow-new').getByRole('button',{name:'Cancel',exact:true}).click();
  await expect(empty).toBeVisible();
});

