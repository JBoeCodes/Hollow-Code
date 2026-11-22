import { openTasks } from './choose';
import {expect,test,type Page} from './fixtures/test';

// Kept after the issue-list rewrite (11-10): the scope tabs, saved filters,
// row metadata chips and bulk selection are gone; row geometry, a long title
// and the detail pane at scaled widths remain.
async function open(page:Page) {
  await page.goto('/?mock=1&demo=1');
  await openTasks(page);
  await expect(page.locator('.issue-row').first()).toBeVisible();
}
async function geometry(page:Page) {
  const bad=await page.locator('.issue-row').evaluateAll(rows=>rows.flatMap(row=>{
    const b=row.getBoundingClientRect();
    return [...row.querySelectorAll('.issue-row-main')].flatMap(el=>{
      const r=el.getBoundingClientRect();
      return r.left<b.left-1||r.right>b.right+1?[(el as HTMLElement).className]:[];
    });
  }));
  expect(bad).toEqual([]);
  expect(await page.evaluate(()=>document.scrollingElement!.scrollWidth<=innerWidth)).toBe(true);
}

test('long task title keeps its project and detail',async({page})=>{
  await open(page);
  const title='Review the full regional infrastructure proposal and its implementation dependencies';
  await page.evaluate(async title=>{
    const w=window as unknown as {__mockDemoTasks:{title:string;url:string;unresolvedBlockers:string[];blockedBy:string[]}[]};
    const blockers=[w.__mockDemoTasks[0].url];
    w.__mockDemoTasks=w.__mockDemoTasks.map(t=>t.title==='Draft airport pricing options'?{...t,title,unresolvedBlockers:blockers,blockedBy:blockers}:t);
    const store='/src/store.ts';await(await import(/* @vite-ignore */ store)).refetchTasks();
  },title);
  const row=page.locator('.issue-group[data-kind="local"] .issue-row.task-row').filter({hasText:title});
  await expect(row.locator('.issue-project')).toContainText('Airport contract renewals');
  await expect(row.locator('.issue-due')).not.toBeEmpty();
  await geometry(page);
  await row.locator('.issue-row-main').click();
  await expect(page.locator('.task-detail-title')).toHaveText(title);
});

test('selected detail follows scaled widths and keeps readable titles at Largest compact density',async({page})=>{
  await page.addInitScript(()=>{
    localStorage.setItem('hollow.studio.textScale','125');
    localStorage.setItem('hollow.studio.density','compact');
  });
  await page.setViewportSize({width:1100,height:800});
  await open(page);
  await geometry(page);
  await page.locator('.issue-row.task-row').filter({hasText:'Collect vendor security questionnaires'}).locator('.issue-row-main').click();
  await expect(page.locator('.task-detail-title')).toHaveText('Collect vendor security questionnaires');
  // Largest makes 1100px a medium window: details replace the list.
  await expect(page.locator('.task-collection')).toBeHidden();
  await expect(page.locator('.task-detail-title')).toBeInViewport();
  await expect(page.getByRole('button',{name:'Close task details'})).toBeInViewport();
  // A wide window retains both panes and the original row geometry checks.
  await page.setViewportSize({width:1440,height:920});
  await expect(page.locator('.task-collection')).toBeVisible();
  await expect(page.locator('.task-detail-title')).toBeInViewport();
  await geometry(page);
  const title=await page.locator('.issue-row-main .issue-title').first().boundingBox();
  expect(title!.width).toBeGreaterThan(80);
  await page.setViewportSize({width:1100,height:800});
  await expect(page.locator('.task-collection')).toBeHidden();
  await page.getByRole('button',{name:'Close task details'}).click();
  await expect(page.locator('.task-collection')).toBeVisible();
  await geometry(page);
});
