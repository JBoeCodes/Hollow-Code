import { test, expect, type Page } from './fixtures/test';
import AxeBuilder from '@axe-core/playwright';
import { join } from 'node:path';
async function boot(page: Page, connected = true) {
  await page.goto('/?mock=1');await expect(page.locator('.app')).toBeVisible();
  await page.evaluate(async connected=>{
    const {api}=await import('/src/api.ts');
    const f=(window as any).companionFixture={connected,fail:false,hold:false,saves:0,scopeSaves:0,reads:0};
    const status=()=>({url:f.connected?'https://companion.example.test':null,connected:f.connected,lastContactAt:new Date().toISOString(),lastError:null,mirrorError:null,mirroredTasks:177,lastPushAt:new Date().toISOString(),mirroredDocs:12,docsError:null,docsSupported:true,scope:'personal',scopeError:null});
    api.companionBackendStatus=async()=>{f.reads++;if(f.fail)throw new Error('fixture connection failure');const s=status();if(f.hold)await new Promise(r=>{f.release=r;});return s;};
    api.companionBackendSet=async(url:string)=>{f.saves++;if(f.holdSave)await new Promise(r=>{f.saveRelease=r;});f.connected=!!url;return status();};
    api.companionBackendClaim=async()=>{f.saves++;if(f.fail)throw new Error('fixture wrong code');f.connected=true;return status();};
    const oldScope=api.companionScopeGet;api.companionScopeSet=async(work:boolean,ids:string[])=>{f.scopeSaves++;await new Promise(r=>{f.scopeRelease=r;});const scope=await oldScope();return {...scope,includeWorkData:work,workspaces:scope.workspaces.map((w:any)=>({...w,personal:ids.includes(w.id)}))};};
    (await import('/src/store/runtime.ts')).requestOpenSettingsSection('companions');
  },connected);
  await page.getByRole('button',{name:'Settings',exact:true}).click();
  await expect(page.locator('.companion-backend')).toBeVisible();
}

test('connected backend leads with status and keeps setup secondary',async({page})=>{
 await boot(page);const panel=page.locator('.companion-backend');await expect(panel.getByRole('heading',{name:'Backend for the web app'})).toBeVisible();await expect(panel.getByText('Connected', {exact:true})).toBeVisible();await expect(panel.getByLabel('Code from its setup page')).not.toBeVisible();await panel.getByText('Change connection', {exact:true}).click();await expect(panel.getByLabel('Code from its setup page')).toBeVisible();await expect(panel).toContainText('only while no Hollow is connected');
});

test('status errors are not shown as off or zero workspaces',async({page})=>{
 await page.goto('/?mock=1');await expect(page.locator('.app')).toBeVisible();await page.evaluate(async()=>{const {api}=await import('/src/api.ts');api.companionBackendStatus=async()=>{throw new Error('unavailable');};(await import('/src/store/runtime.ts')).requestOpenSettingsSection('companions');});await page.getByRole('button',{name:'Settings',exact:true}).click();const panel=page.locator('.companion-backend');await expect(panel.getByRole('alert')).toContainText('Could not read');await expect(panel).not.toContainText('Off.');await expect(panel.getByRole('button',{name:'Retry status'})).toBeVisible();
});

for(const theme of ['light','dark','glass'])test(`companion settings are legible and usable: ${theme}`,async({page},info)=>{
 await page.addInitScript(theme=>{localStorage.setItem('theme',theme==='light'?'light':'dark');localStorage.setItem('glass',theme==='glass'?'on':'off');},theme);await page.emulateMedia({reducedMotion:'reduce'});await boot(page);if(theme==='glass')await page.addStyleTag({content:'html[data-glass] { background:var(--canvas); }'});const panel=page.locator('.companion-devices');await page.getByText(/What the backend copies ·/).click();await expect(page.getByRole('checkbox',{name:'jbmine active'})).toBeVisible();expect((await new AxeBuilder({page}).include('.companion-devices').analyze()).violations).toEqual([]);if(process.env.V5_CAPTURE_DIR)await panel.screenshot({path:join(process.env.V5_CAPTURE_DIR,`v5-${theme}-${info.project.name}.png`)});
});


test('turning off ignores an older status read and protects credential edits while saving',async({page})=>{
 await boot(page);await page.getByText('Change connection',{exact:true}).click();await page.getByLabel('Code from its setup page').fill('MY-DRAFT');await page.evaluate(()=>{(window as any).companionFixture.hold=true;});await expect.poll(()=>page.evaluate(()=>!!(window as any).companionFixture.release),{timeout:7000}).toBe(true);
 await expect(page.getByLabel('Code from its setup page')).toHaveValue('MY-DRAFT');await page.evaluate(()=>{(window as any).companionFixture.holdSave=true;});await page.getByRole('button',{name:'Turn off',exact:true}).click();await expect(page.getByRole('button',{name:'Turn off',exact:true})).toBeDisabled();await page.evaluate(()=>{(window as any).companionFixture.saveRelease();});await expect(page.getByText('Not connected',{exact:true})).toBeVisible();await page.evaluate(()=>{(window as any).companionFixture.release();});await expect(page.getByText('Not connected',{exact:true})).toBeVisible();expect(await page.evaluate(()=>(window as any).companionFixture.saves)).toBe(1);
});

test('scope selections cannot change during a save and restore on its result',async({page})=>{
 await boot(page);await page.getByText(/What the backend copies ·/).click();const option=page.getByRole('checkbox',{name:'jbmine active'});await option.check();await page.getByRole('button',{name:'Save what the backend copies',exact:true}).click();await expect(option).toBeDisabled();await expect(page.getByRole('checkbox',{name:/Include work data/})).toBeDisabled();await page.evaluate(()=>{(window as any).companionFixture.scopeRelease();});await expect(option).toBeChecked();await expect(option).toBeEnabled();await expect(page.getByText('Not saved yet',{exact:false})).toHaveCount(0);
});


test('a slow status poll completes instead of being replaced by every tick',async({page})=>{
 await boot(page);await page.evaluate(()=>{(window as any).companionFixture.hold=true;});await expect.poll(()=>page.evaluate(()=>!!(window as any).companionFixture.release),{timeout:7000}).toBe(true);const reads=await page.evaluate(()=>(window as any).companionFixture.reads);await page.waitForTimeout(5200);expect(await page.evaluate(()=>(window as any).companionFixture.reads)).toBe(reads);await page.evaluate(()=>{(window as any).companionFixture.release();});await expect(page.getByText('Connected',{exact:true})).toBeVisible();
});

test('claim failures retain the actionable cause and entered code',async({page})=>{
 await boot(page,false);await page.getByText('Connect to the homelab',{exact:true}).click();await page.getByRole('form',{name:'Connect to the homelab',exact:true}).getByLabel('Backend URL').fill('https://companion.example.test');await page.getByLabel('Code from its setup page').fill('KEEP-CODE');await page.evaluate(()=>{(window as any).companionFixture.fail=true;});await page.getByRole('button',{name:'Connect',exact:true}).click();await expect(page.locator('.companion-backend').getByRole('alert')).toContainText('fixture wrong code');await expect(page.getByLabel('Code from its setup page')).toHaveValue('KEEP-CODE');
});

test('Test connection checks the saved or the typed connection, saves nothing and answers plainly (ST5)',async({page})=>{
 await boot(page);
 await page.evaluate(async()=>{const {api}=await import('/src/api.ts');const f=(window as any).companionFixture;f.tests=[];
  api.companionBackendTest=async(url:string,token:string)=>{f.tests.push([url,token]);return url?{ok:false,message:'The backend at typed.example.test answered but refused the applier token. Paste it again, or connect with a new code.',version:null}:{ok:true,message:'Connected: the backend at companion.example.test accepted the token. It has 1 paired device and 177 shared tasks.',version:'0.4.2'};};});
 const panel=page.locator('.companion-backend');
 const saved=panel.locator('.companion-backend-overview .companion-test-result'), typed=panel.locator('details.companion-scope .companion-test-result');
 // Both answer lines are always there (live regions), empty until a test.
 await expect(saved).toHaveText('');await expect(saved).toHaveAttribute('role','status');
 await panel.locator('.companion-backend-overview').getByRole('button',{name:'Test connection',exact:true}).click();
 await expect(saved).toHaveText('Connected: the backend at companion.example.test accepted the token. It has 1 paired device and 177 shared tasks. Backend 0.4.2.');
 await page.getByText('Change connection',{exact:true}).click();await page.getByText('Advanced: paste an applier token',{exact:true}).click();
 const form=page.getByRole('form',{name:'Connect with an applier token'});
 await form.getByLabel('Backend URL').fill('https://typed.example.test');await form.getByLabel('Applier token').fill('tok-typed');
 await expect(saved).toHaveText('');await expect(typed).toHaveText('');
 await form.getByRole('button',{name:'Test connection',exact:true}).click();
 await expect(typed).toContainText('refused the applier token');await expect(saved).toHaveText('');
 expect(await page.evaluate(()=>(window as any).companionFixture.tests)).toEqual([['',''],['https://typed.example.test','tok-typed']]);
 expect(await page.evaluate(()=>(window as any).companionFixture.saves)).toBe(0);
 await expect(form.getByLabel('Applier token')).toHaveValue('tok-typed');
 await form.getByLabel('Applier token').fill('tok-other');await expect(typed).toHaveText('');
 // A save clears an answer about the connection it replaced.
 await form.getByRole('button',{name:'Test connection',exact:true}).click();await expect(typed).toContainText('refused');
 await form.getByRole('button',{name:'Save',exact:true}).click();
 await expect(typed).toHaveText('');expect(await page.evaluate(()=>(window as any).companionFixture.saves)).toBe(1);
});

test('an unreachable tailnet backend says so once, with the Tailscale hint and when it was last reached (B05)',async({page})=>{
 // The bridge's words for no answer from a tailnet name (companion_bridge.rs `unreachable_words`).
 const words="Hollow couldn't reach homelab.tail1234.ts.net. Check the address, that the backend is running and that this Mac can reach it. If the backend is on your tailnet, check that Tailscale is running and connected on this Mac.";
 await page.goto('/?mock=1');await expect(page.locator('.app')).toBeVisible();
 await page.evaluate(async words=>{const {api}=await import('/src/api.ts');const at=new Date(Date.now()-5*60_000).toISOString();
  // A mirror push that failed against the same backend repeats the words: still shown once.
  api.companionBackendStatus=async()=>({url:'https://homelab.tail1234.ts.net',connected:false,lastContactAt:at,lastError:words,mirrorError:words,mirroredTasks:177,lastPushAt:at,mirroredDocs:12,docsError:null,docsSupported:true,scope:'personal',scopeError:null});
  (await import('/src/store/runtime.ts')).requestOpenSettingsSection('companions');},words);
 await page.getByRole('button',{name:'Settings',exact:true}).click();
 const panel=page.locator('.companion-backend');await expect(panel).toBeVisible();
 await expect(panel.locator('.companion-connection-state')).toHaveText("Can't reach the backend");
 await expect(panel.getByRole('alert')).toHaveText(words);
 await expect(panel.getByText('check that Tailscale is running and connected on this Mac')).toHaveCount(1);
 await expect(panel.locator('.companion-last-contact')).toHaveText(/^Last reached \d+m ago$/);
 await expect(panel.getByText('Connection or sharing needs attention')).toHaveCount(0);
 await expect(panel.locator('.companion-backend-overview').getByRole('button',{name:'Test connection',exact:true})).toBeVisible();
 expect((await new AxeBuilder({page}).include('.companion-backend').analyze()).violations).toEqual([]);
});

test('a backend that answered with an error needs attention rather than reading as unreachable (B05 review)',async({page})=>{
 const refused='The backend refused the applier token. Check the token in Settings, or connect again with a code.';
 await page.goto('/?mock=1');await expect(page.locator('.app')).toBeVisible();
 await page.evaluate(async refused=>{const {api}=await import('/src/api.ts');const at=new Date(Date.now()-5*60_000).toISOString();
  api.companionBackendStatus=async()=>({url:'https://homelab.tail1234.ts.net',connected:false,lastContactAt:at,lastError:refused,mirrorError:null,mirroredTasks:177,lastPushAt:at,mirroredDocs:12,docsError:null,docsSupported:true,scope:'personal',scopeError:null});
  (await import('/src/store/runtime.ts')).requestOpenSettingsSection('companions');},refused);
 await page.getByRole('button',{name:'Settings',exact:true}).click();
 const panel=page.locator('.companion-backend');await expect(panel).toBeVisible();
 await expect(panel.locator('.companion-connection-state')).toHaveText('Backend needs attention');
 await expect(panel.getByRole('alert')).toHaveText(refused);
 await expect(panel.getByText('check that Tailscale is running')).toHaveCount(0);
});
