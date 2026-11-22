import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';
import type { UsageBucket, UsageDay, UsageSummary } from '../../src/types';
import { showUsageTab } from './usageTab';
const bucket=(key:string,n:number|null,extra:Partial<UsageBucket>={}):UsageBucket=>({key,label:key==='claude'?'Claude Code':key==='codex'?'Codex':key,runs:1,metered:1,partial:0,unknown:0,notStarted:0,inProgress:0,unpriced:0,inputTokens:1000,cachedInputTokens:0,outputTokens:100,reportedCostUsd:null,estimatedCostUsd:n,...extra});
function fixture():UsageSummary {
 const daily:UsageDay[]=[1,2,3,5,6,7].map((n,i)=>{
  const byAgent=[bucket('claude',[2,5,3,6,4,3][i],{reportedCostUsd:i===2?2:null}),bucket('codex',i===2?null:[1,2,3,2,1,4][i],i===2?{unpriced:1}:{})];
  return {date:`2026-10-0${n}`,byAgent,total:bucket('total',byAgent.reduce((s,b)=>s+(b.estimatedCostUsd??0),0),{runs:2,reportedCostUsd:i===2?2:null,unpriced:i===2?1:0}),nightByAgent:i===1?[bucket('claude',5)]:i===4?[bucket('codex',1)]:[]};
 });
 return {window:'month',since:'2026-10-01T04:00:00.000Z',generatedAt:'2026-10-07T21:00:00.000Z',daily,total:bucket('total',33,{runs:12,reportedCostUsd:2,unpriced:1}),byAgent:[bucket('claude',23,{runs:6,reportedCostUsd:2}),bucket('codex',10,{runs:6,unpriced:1})],bySurface:[bucket('chat',33,{runs:12,reportedCostUsd:2,unpriced:1})],byWorkflow:[],byProject:[],byLead:[],bySubagent:[],byAccount:[],ledgerSince:'2026-10-01T04:00:00.000Z',ledgerError:null,undatedRuns:0,nightAttributionError:null};
}
async function open(page:Page,data=fixture()) {
 await page.addInitScript(d=>localStorage.setItem('mockUsageSummary',JSON.stringify(d)),data);
 await page.goto('/?mock=1');await page.getByRole('button',{name:/Settings/}).first().click();
 if((page.viewportSize()?.width??1000)<=860)await choose(page.getByRole('combobox',{name:'Settings section'}),'usage');
 else await page.locator('.settings-index').getByRole('button',{name:/Usage/}).click();
 await showUsageTab(page,'All agents');
 await expect(page.locator('.usage-instruments')).toBeVisible();
 await page.getByRole('tab',{name:'This month'}).click();
 await expect(page.getByRole('tab',{name:'This month'})).toHaveAttribute('aria-selected','true');
}
for(const theme of ['light','dark','glass'])test(`usage instruments ${theme}: data, keyboard and gauges`,async({page},info)=>{
 await page.addInitScript(t=>{localStorage.setItem('theme',t==='light'?'light':'dark');localStorage.setItem('glass',t==='glass'?'on':'off');},theme);
 await open(page);if(theme==='glass'){await expect(page.locator('html')).toHaveAttribute('data-glass','on');await page.addStyleTag({content:'html[data-glass] { background:var(--canvas); }'});}
 const view=page.locator('.usage-instruments');await view.scrollIntoViewIfNeeded();await expect(view.locator('.usage-area > .usage-entrance')).toHaveCSS('opacity','1');
 await expect(view.locator('.usage-area path')).toHaveCount(4); // Both agents have two pieces: absent day, plus an adjacent unpriced Codex day.
 await view.getByRole('button',{name:'Inspect 2026-10-02'}).focus();await page.keyboard.press('Enter');await expect(view.locator('.usage-day-detail')).toContainText('≈$7.00 estimated');
 await view.getByRole('button',{name:'Reported',exact:true}).click();await expect(view.locator('.usage-area path')).toHaveCount(1);await expect(view.locator('.usage-agent-split')).toContainText('No amount recorded');
 await view.getByRole('button',{name:'Estimated',exact:true}).click();
 expect((await new AxeBuilder({page}).include('.usage-instruments').withTags(['wcag2a','wcag2aa']).analyze()).violations).toEqual([]);
 expect(await view.evaluate(el=>el.scrollWidth>el.clientWidth+1)).toBe(false);
 if(process.env.HOLLOW_LANE_SHOTS) {
  // The narrow Settings scroller clips a whole-card shot. Capture real visible regions separately.
  const capture=info.project.name.includes('narrow')?view.locator('.usage-visual-grid'):view;
  await capture.scrollIntoViewIfNeeded();
  await capture.screenshot({animations:'disabled',path:`${process.env.HOLLOW_LANE_SHOTS}/v9-${theme}-${info.project.name}.png`});
  if(info.project.name.includes('narrow'))await view.locator('.usage-night-band').screenshot({animations:'disabled',path:`${process.env.HOLLOW_LANE_SHOTS}/v9-night-${theme}-${info.project.name}.png`});
 }
 await view.locator('summary').click();await expect(view.getByRole('region',{name:'Daily usage values'})).toContainText('No direct link');
 await showUsageTab(page,'Budgets');const budgets=page.locator('.budgets-panel');await budgets.scrollIntoViewIfNeeded();await expect(budgets.locator('.budget-gauge text')).toHaveText('120%');
 await budgets.getByRole('button',{name:'Edit',exact:true}).click();await expect(budgets.getByLabel('Limit',{exact:true})).toBeVisible();await budgets.getByRole('button',{name:'Cancel',exact:true}).click();
 expect((await new AxeBuilder({page}).include('.budgets-panel').withTags(['wcag2a','wcag2aa']).analyze()).violations).toEqual([]);
 if(process.env.HOLLOW_LANE_SHOTS)await budgets.screenshot({animations:'disabled',path:`${process.env.HOLLOW_LANE_SHOTS}/v9-budgets-${theme}-${info.project.name}.png`});
});
test('unpriced and failed reads stay explicit; refresh recovers',async({page})=>{
 const data=fixture();data.daily=[];data.nightAttributionError='Missing links';data.byAgent=[bucket('codex',null,{unpriced:1})];await open(page,data);
 const view=page.locator('.usage-instruments');await expect(view).toContainText('No estimated amounts recorded');await expect(view).toContainText('Night attribution could not be read');
 await page.evaluate(()=>localStorage.setItem('mockUsageError','yes'));await page.locator('.usage-toolbar').getByRole('button',{name:'Refresh'}).click();
 await expect(page.locator('.usage-panel [role="alert"]')).toContainText('Usage could not be read');await expect(view).toHaveCount(0);
 await page.evaluate(()=>localStorage.removeItem('mockUsageError'));await page.locator('.usage-toolbar').getByRole('button',{name:'Refresh'}).click();await expect(view).toBeVisible();
});
test('visible-only entrance respects hidden, away and both motion gates; leaving Usage unmounts',async({page})=>{
 await open(page);const view=page.locator('.usage-instruments'),motion=view.locator('.usage-area > .usage-entrance');await view.scrollIntoViewIfNeeded();
 await expect(motion).toHaveCSS('animation-play-state','running');
 await view.evaluate(el=>(el as HTMLElement).style.transform='translateX(6000px)');await expect(motion).toHaveCSS('animation-play-state','paused');await view.evaluate(el=>(el as HTMLElement).style.transform='');
 await page.evaluate(()=>{Object.defineProperty(document,'hidden',{configurable:true,value:true});document.dispatchEvent(new Event('visibilitychange'));});await expect(motion).toHaveCSS('animation-play-state','paused');
 await page.evaluate(()=>{Object.defineProperty(document,'hidden',{configurable:true,value:false});document.dispatchEvent(new Event('visibilitychange'));document.body.classList.add('is-away');});await expect(motion).toHaveCSS('animation-play-state','paused');
 await page.evaluate(()=>{document.body.classList.remove('is-away');document.documentElement.dataset.motion='reduced';});await expect(motion).toHaveCSS('animation-name','none');
 await page.evaluate(()=>delete document.documentElement.dataset.motion);await page.emulateMedia({reducedMotion:'reduce'});await expect(motion).toHaveCSS('animation-name','none');await expect(view.locator('.usage-area')).toBeVisible();
 if((page.viewportSize()?.width??1000)<=860)await choose(page.getByRole('combobox',{name:'Settings section'}),'general');else await page.locator('.settings-index').getByRole('button',{name:/^General/}).click();
 await expect(view).toHaveCount(0);
});

test('missing IntersectionObserver renders a static final graphic',async({page})=>{
 await page.addInitScript(()=>{Object.defineProperty(window,'IntersectionObserver',{configurable:true,value:undefined});});
 await open(page);await expect(page.locator('.usage-instruments .usage-area > .usage-entrance')).toHaveCSS('animation-name','none');
 await expect(page.locator('.usage-instruments .usage-area > .usage-entrance')).toHaveCSS('opacity','1');
});
