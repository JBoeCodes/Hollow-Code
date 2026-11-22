import AxeBuilder from '@axe-core/playwright';
import { chooseNew } from './choose';
import { expect, test, type Page } from './fixtures/test';

test.beforeEach(({}, info) => { test.skip(info.config.configFile?.endsWith('playwright.static.config.ts') ?? false, 'V8 fixture uses the dev module graph and a controlled scroll frame'); });

/** V8: real chat surfaces driven by synthetic Tauri events. No external effects. */

/** Tauri event delivery on top of the `?mock=1` IPC (as canvas-inline.spec.ts). */
async function installEventBridge(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<string, number[]>();
    let next = 1;
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {
      transformCallback(callback: (event: unknown) => void) { const id = next++; callbacks.set(id, callback); return id; },
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

const emit = (page: Page, payload: Record<string, unknown>) => page.evaluate(event => {
  (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent('chat-event', event);
}, payload);
const calls = (page: Page, command: string) => page.evaluate(c => window.__mockApiCalls?.[c] ?? 0, command);

const PROSE = (n: number, word: string) => Array.from({ length: n }, (_, i) => `${word} paragraph ${i + 1} of the closing notes.`).join('\n\n');
const CANVAS = '```hollow-canvas\n<hc-canvas id="release-status" title="Release status" height="300">\n  <p>29 checks passed</p>\n</hc-canvas>\n```';

test('cold reply chunk keeps the actual transcript at the bottom after prose arrives', async ({ page }) => {
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/src/components/ReplyMarkdown.tsx*', async route => { await held; await route.continue(); });
  await installEventBridge(page);
  await openWorkspace(page);
  await ask(page, 'Read the long report', 'cold-reply');
  await emit(page, { type: 'assistantDelta', text: PROSE(60, 'Report') });
  await emit(page, { type: 'done', sessionId: null });
  await expect(page.locator('.work-conversation .reply-loading')).toContainText('Report paragraph 60');
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(4);
  release();
  await expect(page.locator('.work-conversation .reply-loading')).toHaveCount(0);
  await expect(page.locator('.work-conversation .chat-msg')).toContainText('Report paragraph 60');
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(4);
});

async function openWorkspace(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Produce a checked release report.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Release');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Release');
  // As in WebKit (the app's web view): no browser scroll anchoring to keep a
  // canvas in place on its own, so these tests see Hollow's re-anchoring.
  await page.addStyleTag({ content: '.chat-scroll, .chat-scroll * { overflow-anchor: none !important; }' });
}

async function ask(page: Page, prompt: string, runId: string) {
  const sends = await calls(page, 'chat_send');
  const composer = page.getByLabel('Chat message');
  await composer.fill(prompt);
  await composer.press('Meta+Enter');
  await expect.poll(() => calls(page, 'chat_send')).toBe(sends + 1);
  const clientTurnId = await page.evaluate(async()=>{const path='/src/store.ts';return (await import(/* @vite-ignore */ path)).chatQueuedTurn()?.clientTurnId;});
  await emit(page, { type: 'runStarted', asked: prompt, runId, clientTurnId, recall: [] });
}

/** Where the last canvas sits in the transcript's scroller, and how far the
 *  scroller is from its bottom. */
const geometry = (page: Page) => page.evaluate(() => {
  const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;
  const c = [...s.querySelectorAll<HTMLElement>('.chat-canvas')].pop();
  return {
    canvasTop: c ? Math.round(c.getBoundingClientRect().top - s.getBoundingClientRect().top) : null,
    gap: Math.round(s.scrollHeight - s.scrollTop - s.clientHeight),
  };
});

/** A canvas reply long enough that its closing prose pushes the canvas off
 *  screen when the chat ends at the bottom. */
async function canvasTurn(page: Page, runId: string) {
  await ask(page, 'Where is the release?', runId);
  await emit(page, { type: 'assistantDelta', text: `Here is where the release stands.\n\n${CANVAS}\n\n` });
  // The frame mounts mid-turn and settles its height before more text comes.
  await expect(page.locator('.work-conversation iframe[title="Canvas: Release status"]')).toBeVisible();
  await page.waitForTimeout(200);
  await emit(page, { type: 'assistantDelta', text: PROSE(40, 'Closing') });
  // Mid-stream: the canvas is not snapped to while the reply streams.
  await expect.poll(async () => (await geometry(page)).canvasTop).not.toBe(16);
  await emit(page, { type: 'done', sessionId: null });
}


// V8: public chat events drive actual components; existing focus assertions stay unchanged.
for (const theme of ['light', 'dark', 'glass']) test(`chat motion ${theme}: timeline, loading and working`, async ({ page }, info) => {
  await page.addInitScript(t => { localStorage.setItem('theme', t === 'light' ? 'light' : 'dark'); localStorage.setItem('glass', t === 'glass' ? 'on' : 'off'); }, theme);
  await installEventBridge(page); await openWorkspace(page);
  if(theme==='glass') await page.addStyleTag({content:'html[data-glass] { background:var(--canvas); }'});
  await ask(page, 'Read the plan and check the result', 'motion-tools');
  await emit(page, {type:'toolUse', id:'read', name:'Read', input:{file_path:'PLAN.md'}});
  await expect(page.locator('.chat-working-mark svg')).toBeVisible();
  await emit(page, {type:'toolResult', id:'read', content:'Plan inspected', isError:false});
  await emit(page, {type:'toolUse', id:'check', name:'Bash', input:{command:'npm test'}});
  await emit(page, {type:'toolResult', id:'check', content:'Check failed', isError:true});
  await emit(page, {type:'assistantDelta', text:'I found one check to repair.'});
  await emit(page, {type:'done', sessionId:null});
  const fold=page.locator('.chat-run-fold-head'); await expect(fold).toContainText('2 steps');
  await fold.focus(); await page.keyboard.press('Enter'); await expect(fold).toHaveAttribute('aria-expanded','true');
  await expect(page.locator('.chat-run-timeline')).toBeVisible();
  await expect(page.locator('.chat-run-timeline .toolcall-icon')).toHaveCount(2);
  await expect(page.locator('.chat-run-timeline .toolcall.st-error')).toHaveCount(1);
  // Full tool details remain available; disclosure keeps keyboard focus.
  // The row's toggle, not its middle: a narrow row's middle is the file chip,
  // which opens the file (2026-11-04).
  await page.locator('.chat-run-timeline .toolcall-toggle').first().click();
  await expect(page.locator('.chat-run-timeline')).toContainText('Plan inspected');
  await expect(page.locator('.chat-run-timeline .toolcall-body').first()).toHaveCSS('opacity','1');
  expect((await new AxeBuilder({page}).include('.work-conversation').withTags(['wcag2a','wcag2aa']).analyze()).violations).toEqual([]);
  if(process.env.HOLLOW_LANE_SHOTS) await page.locator('.work-conversation').screenshot({animations:'disabled',path:`${process.env.HOLLOW_LANE_SHOTS}/v8-timeline-${theme}-${info.project.name}.png`});
  await ask(page, 'Draw the checked plan', 'motion-canvas');
  await emit(page,{type:'assistantDelta',text:'```hollow-canvas\n<hc-canvas id="motion" title="Checked plan" height="340">\n'});
  const skeleton=page.locator('.chat-preparation'); await expect(skeleton).toBeVisible();
  await expect(skeleton).toHaveCSS('height','340px'); await expect(skeleton).toContainText('Drawing');
  await expect(page.locator('.chat-working-mark')).toBeVisible();
  await expect.poll(async()=>(await geometry(page)).gap).toBeLessThan(80);
  expect(await page.locator('.chat-scroll').evaluate(el => el.scrollWidth > el.clientWidth + 1)).toBe(false);
  if(process.env.HOLLOW_LANE_SHOTS) await page.locator('.work-conversation').screenshot({animations:'disabled',path:`${process.env.HOLLOW_LANE_SHOTS}/v8-${theme}-${info.project.name}.png`});
  await emit(page,{type:'assistantDelta',text:'<p>One check needs repair.</p>\n</hc-canvas>\n```'});
  await expect(page.getByTitle('Canvas: Checked plan')).toBeVisible();
  await expect(skeleton).toHaveCount(0);
});
test('motion is paused unseen and away, with static content under either reduced-motion gate',async({page})=>{
  await installEventBridge(page); await openWorkspace(page);await ask(page,'Draw the plan','motion-gates');
  await emit(page,{type:'toolUse',id:'gates',name:'Read',input:{file_path:'PLAN.md'}});
  await emit(page,{type:'assistantDelta',text:'```hollow-canvas\n<hc-canvas id="motion" title="Checked plan" height="240">\n'});
  const mark=page.locator('.chat-working-mark'), orbit=mark.locator('.chat-working-orbit');
  const skeleton=page.locator('.chat-preparation'), sheen=skeleton.locator('.chat-preparation-sheen');
  await expect(orbit).toHaveCSS('animation-play-state','running');await expect(sheen).toHaveCSS('animation-play-state','running');
  await mark.evaluate(el=>(el as HTMLElement).style.transform='translateX(5000px)');await expect(orbit).toHaveCSS('animation-play-state','paused');
  await mark.evaluate(el=>(el as HTMLElement).style.transform='');
  await page.evaluate(()=>{Object.defineProperty(document,'hidden',{configurable:true,value:true});document.dispatchEvent(new Event('visibilitychange'));});
  await expect(orbit).toHaveCSS('animation-play-state','paused');await expect(sheen).toHaveCSS('animation-play-state','paused');
  await page.evaluate(()=>{Object.defineProperty(document,'hidden',{configurable:true,value:false});document.dispatchEvent(new Event('visibilitychange'));document.body.classList.add('is-away');});
  await expect(sheen).toHaveCSS('animation-play-state','paused');
  await page.evaluate(()=>{document.body.classList.remove('is-away');document.documentElement.dataset.motion='reduced';});
  await expect(orbit).toHaveCSS('animation-name','none');await expect(sheen).toBeHidden();await expect(skeleton).toContainText('Checked plan');
  await page.evaluate(()=>delete document.documentElement.dataset.motion);await page.emulateMedia({reducedMotion:'reduce'});
  await expect(orbit).toHaveCSS('animation-name','none');await expect(sheen).toBeHidden();await expect(mark.locator('svg')).toBeVisible();
});

for(const intent of ['none','scroll','unmount']) test(`pending follow: ${intent}`, async ({page},info)=>{
  test.skip(info.project.name.includes('narrow'),'desktop scroll ordering');
  await page.emulateMedia({reducedMotion:'reduce'});await installEventBridge(page);await openWorkspace(page);
  await canvasTurn(page,'coalesce-canvas');
  await expect.poll(async()=>{const t=(await geometry(page)).canvasTop;return t!==null&&t>=0&&t<=40;}).toBe(true);
  await page.getByLabel('Chat message').fill('And the risks?');
  await page.evaluate(async()=>{
    const s=document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;
    // Return to following first; a real streamed update schedules ordinary follow.
    // With End, as a person would: the turn's end holds its canvas and moves
    // to it, and until that move has landed (its scroll event, or 700 ms) a
    // bare scroll doesn't let go of the hold, so the push below didn't follow
    // (2 to 3 runs in 45 on main, 11-04, the hold still on at the push).
    s.dispatchEvent(new KeyboardEvent('keydown',{key:'End'}));
    s.scrollTop=s.scrollHeight;s.dispatchEvent(new Event('scroll'));
    const w=window as any, raf=window.requestAnimationFrame.bind(window);
    w.__heldFollow=new Map();let nextHeld=1000000000;const cancel=window.cancelAnimationFrame.bind(window);
    window.requestAnimationFrame=(callback:FrameRequestCallback)=>{
      if(callback.toString().includes('setAtBottom(true)')) { const id=nextHeld++;w.__heldFollow.set(id,callback);return id; }
      return raf(callback);
    };
    window.cancelAnimationFrame=(id:number)=>{if(!w.__heldFollow.delete(id))cancel(id);};
    w.__restoreFollow=()=>{window.requestAnimationFrame=raf;window.cancelAnimationFrame=cancel;for(const cb of w.__heldFollow.values())raf(cb);w.__heldFollow.clear();};
    // Back at the end, the view asked for a follow frame before the hook, and
    // the push's follow would join it unheld: let it run. From here every one
    // is held.
    await new Promise<void>(resolve=>raf(()=>raf(()=>resolve())));
    const path='/src/store.ts';const store=await import(/* @vite-ignore */ path);
    store.pushChatTurn({role:'assistant',text:'A late retained note.',tools:[]});
  });
  await expect.poll(()=>page.evaluate(()=>(window as any).__heldFollow.size)).toBe(1);
  // Browser layout/scroll notification occurs before the coalesced frame, as a
  // late canvas size and scroll clamp can do. No wheel/key intent occurred: the explicit send wins.
  await page.evaluate(()=>{const s=document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;const pad=document.createElement('div');pad.style.height='400px';s.append(pad);s.scrollTop-=10;s.dispatchEvent(new Event('scroll'));});
  // A Find jump explicitly releases following without a physical scroll gesture.
  // The newer send must still promote the already pending ordinary frame.
  // Find lives in the conversation menu since the top-bar polish (#1492).
  await page.getByRole('button',{name:'Actions for this conversation',exact:true}).click();
  await page.getByRole('menu').getByRole('menuitem',{name:'Find in this conversation'}).click();
  await page.getByRole('searchbox',{name:'Find in this conversation'}).fill('Where is the release?');
  await expect.poll(async()=>(await geometry(page)).gap).toBeGreaterThan(200);
  await page.keyboard.press('Escape');
  const before=await calls(page,'chat_send');
  await page.getByLabel('Chat message').press('Meta+Enter');
  await expect.poll(()=>calls(page,'chat_send')).toBe(before+1);
  if(intent === 'scroll') {
    await page.evaluate(()=>{const s=document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;s.dispatchEvent(new WheelEvent('wheel',{deltaY:-100}));s.scrollTop=30;s.dispatchEvent(new Event('scroll'));});
    await expect.poll(()=>page.evaluate(()=>(window as any).__heldFollow.size)).toBe(0);
  }
  if(intent === 'unmount') {
    await page.getByRole('navigation',{name:'Main',exact:true}).getByRole('button',{name:'Projects',exact:true}).click();
    await expect(page.locator('.work-conversation .chat-scroll')).toHaveCount(0);
    await expect.poll(()=>page.evaluate(()=>(window as any).__heldFollow.size)).toBe(0);
  }
  await page.evaluate(()=>(window as any).__restoreFollow());
  if(intent === 'scroll') { await page.waitForTimeout(150);expect(await page.locator('.work-conversation .chat-scroll').evaluate(el=>el.scrollTop)).toBe(30); }
  else if(intent === 'none') await expect.poll(async()=>(await geometry(page)).gap).toBeLessThan(4);
});

test('loading reserves height without repeated announcements and rejection resolves to offline fallback',async({page})=>{
  await installEventBridge(page);await openWorkspace(page);await ask(page,'Prepare a canvas','prepare-load');
  await page.evaluate(async()=>{const path='/src/api.ts';const {api}=await import(/* @vite-ignore */ path);api.artifactPut=()=>new Promise((_resolve,reject)=>(window as any).__rejectArtifact=reject);});
  await emit(page,{type:'assistantDelta',text:CANVAS});
  const loading=page.locator('.chat-artifact-placeholder .chat-preparation');
  await expect(loading).toContainText('Loading Release status');await expect(loading).toHaveCSS('height','300px');
  expect(await loading.getAttribute('role')).toBeNull();
  await page.evaluate(()=>(window as any).__rejectArtifact(new Error('synthetic store unavailable')));
  await expect(page.getByTitle('Canvas: Release status')).toBeVisible();await expect(loading).toHaveCount(0);
  await expect(page.locator('.artifact-ask')).toContainText('showing this without them');
});

test('a delayed scroll notification at the same position keeps following late layout',async({page},info)=>{
  test.skip(info.project.name.includes('narrow'),'desktop scroll ordering');
  await page.emulateMedia({reducedMotion:'reduce'});await installEventBridge(page);await openWorkspace(page);
  await ask(page,'A long response','late-scroll');await emit(page,{type:'assistantDelta',text:PROSE(40,'Opening')});
  await expect.poll(async()=>(await geometry(page)).gap).toBeLessThan(4);
  await page.evaluate(()=>{
    const s=document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;
    const turn=s.querySelector<HTMLElement>('.turn.assistant')!;
    const top=s.scrollTop;
    const pad=document.createElement('div');pad.style.height='400px';turn.append(pad);
    // Browser scroll notification for an earlier programmatic follow: the
    // position is unchanged, but a frame/message has just grown below it.
    if(s.scrollTop!==top)throw new Error('fixture moved the reading position');
    s.dispatchEvent(new Event('scroll'));
  });
  await expect.poll(async()=>(await geometry(page)).gap).toBeLessThan(4);
});

for(const input of ['focus','tab','unannounced']) test(`reading wins over ordinary follow: ${input}`,async({page},info)=>{
  test.skip(info.project.name.includes('narrow'),'desktop scroll ordering');
  await page.emulateMedia({reducedMotion:'reduce'});await installEventBridge(page);await openWorkspace(page);
  await ask(page,'Earlier report','reader-first');await emit(page,{type:'assistantDelta',text:'The earlier report.'});await emit(page,{type:'done',sessionId:null});
  await ask(page,'Long response','reader-live');await emit(page,{type:'assistantDelta',text:PROSE(40,'Later')});
  await expect.poll(async()=>(await geometry(page)).gap).toBeLessThan(4);
  await page.locator('.work-conversation .chat-scroll').evaluate(el=>(el as HTMLElement).focus({preventScroll:true}));
  await page.evaluate(async()=>{
    const w=window as any,raf=window.requestAnimationFrame.bind(window),cancel=window.cancelAnimationFrame.bind(window);
    w.__heldFollow=new Map();let id=1000000000;
    window.requestAnimationFrame=cb=>{if(cb.toString().includes('setAtBottom(true)')){const n=id++;w.__heldFollow.set(n,cb);return n;}return raf(cb);};
    window.cancelAnimationFrame=n=>{if(!w.__heldFollow.delete(n))cancel(n);};
    w.__restoreFollow=()=>{window.requestAnimationFrame=raf;window.cancelAnimationFrame=cancel;for(const cb of w.__heldFollow.values())raf(cb);w.__heldFollow.clear();};
    const path='/src/store.ts';(await import(/* @vite-ignore */ path)).pushChatTurn({role:'assistant',text:'More at the bottom.',tools:[]});
  });
  await expect.poll(()=>page.evaluate(()=>(window as any).__heldFollow.size)).toBe(1);
  if(input==='focus') await page.locator('.work-conversation .turn-copy').first().evaluate(el=>(el as HTMLElement).focus());
  else if(input==='tab') await page.keyboard.press('Tab');
  else await page.locator('.work-conversation .chat-scroll').evaluate(el=>{el.scrollTop=30;el.dispatchEvent(new Event('scroll'));});
  await page.evaluate(()=>(window as any).__restoreFollow());
  await page.waitForTimeout(150);
  expect((await geometry(page)).gap).toBeGreaterThan(200);
  await expect(page.getByRole('button',{name:'Jump to latest',exact:true})).toBeVisible();
  if(input!=='unannounced') await expect(page.locator('.work-conversation .turn-copy').first()).toBeFocused();
});
