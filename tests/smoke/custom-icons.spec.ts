import { expect, test, type Locator, type Page } from './fixtures/test';
import { openSettingsIcons, openSettingsSection } from './choose';
import AxeBuilder from '@axe-core/playwright';

// Upload an icon and Generate a logo (overhaul H1, H2, 2026-11-12) replaced the icon maker.
const SVG='<svg xmlns="http://www.w3.org/2000/svg" width="128" height="64"><rect width="128" height="64" fill="#7050ac"/><circle cx="64" cy="32" r="24" fill="#ffffff"/></svg>';
const OTHER='<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" fill="red"/></svg>';
async function choose(dialog:Locator,svg=SVG,name='sample.svg'){
  await dialog.getByLabel('Choose icon file').setInputFiles({name,mimeType:'image/svg+xml',buffer:Buffer.from(svg)});
}

test('P1-56 retry after a conflict read preserves the draft and rechecks CAS at confirmation',async({page})=>{
  const row=await appearance(page);
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'COO icon',exact:true});
  await choose(editor);
  await editor.getByRole('button',{name:'Apply icon'}).click();await expect(editor).toHaveCount(0);
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  await choose(editor,OTHER,'other.svg');
  await page.evaluate(()=>{
    const key='mockCustomIcon:mainAgent',r=JSON.parse(localStorage.getItem(key)!);
    r.revision='first-change';localStorage.setItem(key,JSON.stringify(r));
    window.__mockFailCommandOnce='read_custom_icon';
  });
  await editor.getByRole('button',{name:'Apply icon'}).click();
  await expect(editor.getByRole('button',{name:'Apply icon'})).toBeDisabled();
  await editor.getByRole('button',{name:'Try again',exact:true}).click();
  await expect(editor.getByRole('img',{name:'COO icon preview',exact:true})).toBeVisible();
  await editor.getByRole('button',{name:'Apply icon'}).click();
  const confirm=page.getByRole('dialog',{name:'Replace the updated icon?'});
  await expect(confirm).toBeVisible();
  await page.evaluate(()=>{
    const key='mockCustomIcon:mainAgent',r=JSON.parse(localStorage.getItem(key)!);
    r.revision='second-change';localStorage.setItem(key,JSON.stringify(r));
  });
  await confirm.getByRole('button',{name:'Replace icon',exact:true}).click();
  await expect(editor.getByText(/saved icon changed/i)).toBeVisible();
  expect(await page.evaluate(()=>JSON.parse(localStorage.getItem('mockCustomIcon:mainAgent')!).revision)).toBe('second-change');
  await expect(editor.getByRole('img',{name:'COO icon preview',exact:true})).toBeVisible();
});

test('P1-56 row distinguishes damage recovery from a retryable read',async({page})=>{
  await page.addInitScript(()=>localStorage.setItem('mockCustomIconReadError:app','Permission denied (os error 13)'));
  await appearance(page);
  const row=page.getByRole('region',{name:'App icon',exact:true});
  await expect(row.getByRole('alert')).toBeVisible();
  await expect(row.getByText(/keeps a copy/)).toHaveCount(0);
  await page.evaluate(()=>localStorage.setItem('mockCustomIconReadError:app','ICON_DAMAGED: damaged fixture'));
  await row.getByRole('button',{name:'Try again',exact:true}).click();
  await expect(row.getByText(/Reset to Fold keeps a copy/)).toBeVisible();
  await expect(row.getByRole('button',{name:'Try again',exact:true})).toHaveCount(0);
});

test('P1-56 preparation failure keeps Apply off until a good image is chosen',async({page})=>{
  const row=await appearance(page);
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'COO icon',exact:true});
  await page.evaluate(()=>{window.__mockFailCommandOnce='prepare_custom_icon';});
  await choose(editor,OTHER,'safe.svg');
  await expect(editor.getByRole('alert')).toBeVisible();
  await expect(editor.getByRole('button',{name:'Apply icon'})).toBeDisabled();
  await choose(editor,OTHER,'safe.svg');
  await expect(editor.getByRole('button',{name:'Apply icon'})).toBeEnabled();
});

test('P1-56 an I/O read failure offers retry without a damage remedy',async({page})=>{
  await appearance(page);
  const row=page.getByRole('region',{name:'App icon',exact:true});
  await page.evaluate(()=>localStorage.setItem('mockCustomIconReadError:app','Permission denied (os error 13)'));
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'App icon',exact:true});
  await expect(editor.getByRole('alert')).toBeVisible();
  await expect(editor.getByRole('button',{name:'Apply icon'})).toBeDisabled();
  await expect(editor.getByText(/keeps a copy/)).toHaveCount(0);
  await page.evaluate(()=>localStorage.removeItem('mockCustomIconReadError:app'));
  await editor.getByRole('button',{name:'Try again',exact:true}).click();
  await choose(editor);
  await expect(editor.getByRole('button',{name:'Apply icon'})).toBeEnabled();
});

test('P1-56 backdrop clicks require a backdrop pointer origin',async({page})=>{
  const row=await appearance(page);
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'COO icon',exact:true});
  await choose(editor);
  await expect(editor.getByRole('button',{name:'Apply icon'})).toBeEnabled();
  await editor.dispatchEvent('pointerdown');
  await page.locator('.rbk-backdrop').dispatchEvent('click');
  await expect(editor).toBeVisible();
});

test('P1-56 a conflict rereads the revision and keeps the choice until replacement is confirmed',async({page})=>{
  const row=await appearance(page);
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'COO icon',exact:true});
  await choose(editor);
  await editor.getByRole('button',{name:'Apply icon'}).click();
  await expect(editor).toHaveCount(0);
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  await choose(editor,OTHER,'other.svg');
  const reads=await page.evaluate(()=>{
    const key='mockCustomIcon:mainAgent', record=JSON.parse(localStorage.getItem(key)!);
    record.revision='external-change';localStorage.setItem(key,JSON.stringify(record));
    return window.__mockApiCalls?.read_custom_icon??0;
  });
  await editor.getByRole('button',{name:'Apply icon'}).click();
  await expect(editor.getByText(/saved icon changed/i)).toBeVisible();
  await expect.poll(()=>page.evaluate(()=>window.__mockApiCalls?.read_custom_icon??0)).toBeGreaterThan(reads);
  await expect(editor.getByRole('alert')).toHaveCount(0);
  await editor.getByRole('button',{name:'Apply icon'}).click();
  const confirm=page.getByRole('dialog',{name:'Replace the updated icon?'});
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button',{name:'Cancel',exact:true}).click();
  expect(await page.evaluate(()=>JSON.parse(localStorage.getItem('mockCustomIcon:mainAgent')!).revision)).toBe('external-change');
});

test('P1-56 a damaged icon names its reset and cannot apply',async({page})=>{
  await appearance(page);
  const row=page.getByRole('region',{name:'App icon',exact:true});
  await page.evaluate(()=>localStorage.setItem('mockCustomIconReadError:app','ICON_DAMAGED: Saved icon is damaged; left unchanged: EOF while parsing a value at line 1 column 1'));
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'App icon',exact:true});
  await expect(editor.getByText(/Reset to Fold keeps a copy/)).toBeVisible();
  await expect(editor.getByRole('button',{name:'Apply icon'})).toBeDisabled();
  await expect(editor.getByRole('button',{name:'Reset to Fold'})).toBeEnabled();
  await expect(editor.getByRole('alert')).toContainText(/saved icon.*damaged/i);
});

test('P1-56 Escape with a chosen image confirms and a cancelled confirmation keeps it',async({page})=>{
  const row=await appearance(page);
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'COO icon',exact:true});
  await choose(editor);
  await expect(editor.getByRole('img',{name:'COO icon preview',exact:true})).toBeVisible();
  await page.keyboard.press('Escape');
  const confirm=page.getByRole('dialog',{name:'Discard this icon?'});
  await expect(confirm).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(confirm).toHaveCount(0);
  await expect(editor.getByRole('img',{name:'COO icon preview',exact:true})).toBeVisible();
  await editor.getByRole('button',{name:'Cancel',exact:true}).click();
  await confirm.getByRole('button',{name:'Discard',exact:true}).click();
  await expect(editor).toHaveCount(0);
  await expect(row.getByRole('button',{name:'Upload an icon',exact:true})).toBeFocused();
});

test('P1-56 dragging from the card to the backdrop does not dismiss it',async({page})=>{
  const row=await appearance(page);
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'COO icon',exact:true});
  await choose(editor);
  const box=(await editor.locator('.custom-icon-upload').boundingBox())!;
  await page.mouse.move(box.x+box.width/2,box.y+box.height/2);
  await page.mouse.down();await page.mouse.move(2,2);await page.mouse.up();
  await expect(editor).toBeVisible();
  await expect(page.getByRole('dialog',{name:'Discard this icon?'})).toHaveCount(0);
});

test('P1-56 unreadable-copy hint belongs to reads, not writes',async({page})=>{
  const row=await appearance(page);
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'COO icon',exact:true});
  await choose(editor);
  await editor.getByRole('button',{name:'Apply icon'}).click();
  await page.evaluate(()=>{window.__mockFailCommandOnce='reset_custom_icon';});
  await row.getByRole('button',{name:'Reset to default'}).click();
  await page.getByRole('dialog',{name:'Replace your custom icon?'}).getByRole('button',{name:'Reset to default',exact:true}).click();
  await expect(row.getByRole('alert')).toBeVisible();
  await expect(row.getByText(/keeps a copy/)).toHaveCount(0);
  await page.evaluate(()=>localStorage.setItem('mockCustomIconReadError:mainAgent','ICON_DAMAGED: damaged fixture'));
  await row.getByRole('button',{name:'Try again',exact:true}).click();
  await expect(row.getByText(/Reset to default keeps a copy/)).toBeVisible();
});

async function appearance(page:Page,theme='light',open=true) {
  await page.addInitScript(t=>{localStorage.setItem('theme',t);localStorage.setItem('mockWorkspaces',JSON.stringify([{id:'main',kind:'main',title:'COO',status:'active',path:'workspaces/main',cwd:'/Users/fixture/vault/workspaces/main',modified:'2026-10-21T12:00:00Z'}]));},theme);
  await page.goto('/?mock=1');
  await page.getByRole('navigation',{name:'Shared tools'}).getByRole('button',{name:'Settings',exact:true}).click();
  await openSettingsSection(page, 'General');
  if(open)await openSettingsIcons(page);
  return page.getByRole('region',{name:'COO icon',exact:true});
}
test('overnight 11-07: the closed icons disclosure says when a row needs attention',async({page})=>{
  await page.addInitScript(()=>localStorage.setItem('mockCustomIconReadError:app','Permission denied (os error 13)'));
  await appearance(page,'light',false);
  const fold=page.locator('.settings-icons');
  await expect(fold).not.toHaveAttribute('open','');
  await expect(fold.locator('summary')).toContainText('Needs attention');
  await fold.locator('summary').click();
  await expect(page.getByRole('region',{name:'App icon',exact:true}).getByRole('alert')).toBeVisible();
  await page.evaluate(()=>localStorage.removeItem('mockCustomIconReadError:app'));
  await page.getByRole('region',{name:'App icon',exact:true}).getByRole('button',{name:'Try again',exact:true}).click();
  await expect(fold.locator('summary')).not.toContainText('Needs attention');
  await expect(fold).toHaveAttribute('open','');
});
test('Settings search reveals icons inside the closed disclosure',async({page})=>{
  await appearance(page);
  const fold=page.locator('.settings-icons');
  await fold.locator('summary').click();
  await expect(fold).not.toHaveAttribute('open','');
  await page.getByRole('searchbox',{name:'Find a setting'}).fill('App icon');
  await page.getByRole('navigation',{name:'Settings sections'}).getByRole('button',{name:'App icon',exact:true}).click();
  await expect(fold).toHaveAttribute('open','');
  await expect(page.getByRole('region',{name:'App icon',exact:true})).toBeVisible();
  await expect(page.getByRole('region',{name:'App icon',exact:true}).getByRole('button',{name:'Fold (default)',exact:true})).toBeFocused();
});
for(const theme of ['light','dark']) test(`upload, status badges and reset in ${theme}`,async({page},info)=>{
  const row=await appearance(page,theme);
  const stockImage=page.locator('.top-bar .main-icon-image');
  await expect(stockImage).toHaveAttribute('href',/main-agent-fold-512/);
  const stockHref=(await stockImage.getAttribute('href'))!;
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'COO icon',exact:true});
  await expect(editor.getByRole('button',{name:'Apply icon'})).toBeDisabled();
  await choose(editor);
  await expect(editor.locator('.main-icon-image')).toHaveCount(4);
  await expect(editor.getByRole('img',{name:'COO · Waiting on you'})).toBeVisible();
  await expect(editor.locator('.main-orb-count')).toHaveText('!');
  // Apply just left its disabled look; axe must not sample the colour mid-transition.
  await page.emulateMedia({reducedMotion:'reduce'});
  await expect(editor.getByRole('button',{name:'Apply icon'})).toBeEnabled();
  await page.evaluate(()=>new Promise<void>(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r()))));
  expect((await new AxeBuilder({page}).include('.custom-icon-editor').withTags(['wcag2a','wcag2aa']).analyze()).violations).toEqual([]);
  await editor.screenshot({path:info.outputPath(`upload-${theme}.png`)});
  await editor.getByRole('button',{name:'Apply icon'}).click();
  await expect(editor).toHaveCount(0);
  await expect(page.locator('.top-bar .main-icon-image')).toHaveCount(1);
  await expect(stockImage).toHaveAttribute('href',/^data:image\/png;base64,/);
  await page.reload();
  await expect(page.locator('.top-bar .main-icon-image')).toHaveCount(1);
  const orb=page.locator('.main-agent-orb');
  await orb.screenshot({path:info.outputPath(`orb-${theme}.png`)});
  await orb.click();
  await page.locator('.top-bar').getByRole('button',{name:'Actions for COO',exact:true}).click();
  await page.getByRole('menuitem',{name:'Change icon…',exact:true}).click();
  await expect(editor.getByRole('button',{name:'Upload an icon',exact:true})).toHaveAttribute('aria-pressed','true');
  await expect(editor.getByRole('button',{name:'Generate a logo',exact:true})).toBeVisible();
  await editor.getByRole('button',{name:'Reset to default',exact:true}).click();
  await page.getByRole('dialog',{name:'Reset to the stock icon?'}).getByRole('button',{name:'Reset to default',exact:true}).click();
  await expect(editor).toHaveCount(0);
  await expect(stockImage).toHaveAttribute('href',stockHref);
});
test('upload previews, keeps the original, and a failed apply keeps the dialog',async({page},info)=>{
  await appearance(page);
  const row=page.getByRole('region',{name:'App icon',exact:true});
  await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'App icon',exact:true});
  await editor.getByLabel('Choose icon file').setInputFiles({name:'photo.gif',mimeType:'image/gif',buffer:Buffer.from('not used')});
  await expect(editor.getByRole('alert')).toContainText('PNG, JPEG or SVG');
  await choose(editor);
  await expect(editor.getByRole('img',{name:'App icon preview',exact:true})).toBeVisible();
  await page.evaluate(()=>{window.__mockFailCommandOnce='save_custom_icon';});
  await editor.getByRole('button',{name:'Apply icon'}).click();
  await expect(editor.getByRole('alert')).toBeVisible();
  await expect(editor.getByRole('img',{name:'App icon preview',exact:true})).toBeVisible();
  await editor.screenshot({path:info.outputPath('upload.png')});
  await editor.getByRole('button',{name:'Apply icon'}).click();
  await expect(editor).toHaveCount(0);
  expect(await page.evaluate(()=>JSON.parse(localStorage.getItem('mockCustomIcon:app')!).original.bytes)).toBe(Buffer.from(SVG).toString('base64'));
  await row.getByRole('button',{name:'Reset to Fold'}).click();
  await page.getByRole('dialog',{name:'Replace your custom icon?'}).getByRole('button',{name:'Reset to Fold',exact:true}).click();
  await expect(row.getByRole('button',{name:'Reset to Fold'})).toBeDisabled();
});
test('cancel and rejected SVG leave the saved choice alone',async({page})=>{
  const row=await appearance(page);await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'COO icon',exact:true});
  await choose(editor,'<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><script>alert(1)</script></svg>','bad.svg');
  await expect(editor.getByRole('alert')).toBeVisible();await expect(editor.getByRole('button',{name:'Apply icon'})).toBeDisabled();
  await editor.getByRole('button',{name:'Cancel'}).click();
  expect(await page.evaluate(()=>localStorage.getItem('mockCustomIcon:mainAgent'))).toBeNull();
});
test('Generate a logo shows four options from the stub agent and applies the picked one',async({page},info)=>{
  const row=await appearance(page);
  await row.getByRole('button',{name:'Generate a logo',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'COO icon',exact:true});
  await editor.getByLabel('Describe the logo (optional)').fill('a lantern');
  await editor.getByRole('button',{name:'Generate four options',exact:true}).click();
  const options=editor.getByRole('radiogroup',{name:'Logo options'}).getByRole('radio');
  await expect(options).toHaveCount(4);
  await expect(editor.getByText('Drawn by Codex.',{exact:false})).toBeVisible();
  await expect(editor.getByRole('button',{name:'Apply icon'})).toBeDisabled();
  // The radio is visually hidden inside its tile; a person clicks the tile.
  await editor.locator('.custom-icon-option').nth(2).click();
  await expect(options.nth(2)).toBeChecked();
  // Picking prepares the image and locks the choice until it is ready.
  await expect(editor.getByRole('img',{name:'COO icon preview',exact:true})).toBeVisible();
  await expect(editor.getByRole('button',{name:'Apply icon'})).toBeEnabled();
  await options.nth(2).focus();
  await page.keyboard.press('ArrowRight');
  await expect(options.nth(3)).toBeChecked();
  await expect(options.nth(2)).not.toBeChecked();
  await expect(editor.getByRole('img',{name:'COO icon preview',exact:true})).toBeVisible();
  // The new pick is checked before Apply comes back: axe read the button mid
  // fade from disabled to red (3.27:1, 11-15 run), so wait for it to settle.
  const apply=editor.getByRole('button',{name:'Apply icon'});
  await expect(apply).toBeEnabled();
  await expect.poll(()=>apply.evaluate(b=>b.getAnimations().length)).toBe(0);
  expect((await new AxeBuilder({page}).include('.custom-icon-editor').withTags(['wcag2a','wcag2aa']).analyze()).violations).toEqual([]);
  await editor.screenshot({path:info.outputPath('generate.png')});
  await editor.getByRole('button',{name:'Apply icon'}).click();
  await expect(editor).toHaveCount(0);
  expect(await page.evaluate(()=>JSON.parse(localStorage.getItem('mockCustomIcon:mainAgent')!).original.format)).toBe('svg');
});
test('Generate a logo failure says so and Upload stays one click away',async({page})=>{
  await page.addInitScript(()=>localStorage.setItem('mockLogoOptionsError','Codex is out of usage'));
  const row=await appearance(page);
  await row.getByRole('button',{name:'Generate a logo',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'COO icon',exact:true});
  await editor.getByRole('button',{name:'Generate four options',exact:true}).click();
  await expect(editor.getByRole('alert')).toContainText('out of usage');
  await editor.getByRole('button',{name:'Upload an icon',exact:true}).click();
  await expect(editor.getByLabel('Choose icon file')).toBeVisible();
});
for(const theme of ['light','dark']) test(`full-bleed artwork leaves every status ring visible in ${theme}`,async({page},info)=>{
  const row=await appearance(page,theme);await row.getByRole('button',{name:'Upload an icon',exact:true}).click();
  const editor=page.getByRole('dialog',{name:'COO icon',exact:true});
  const svg='<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512"><rect width="512" height="512" fill="#ffffff"/><rect x="256" width="256" height="512" fill="#000000"/></svg>';
  await choose(editor,svg,'contrast.svg');
  await expect(editor.locator('.main-icon-image')).toHaveCount(4);
  expect(await editor.locator('.main-indicator').evaluateAll(nodes=>nodes.every(node=>{
    const image=node.querySelector('image')!, ring=node.querySelector('.main-orb-shell')!;
    return !!image.getAttribute('clip-path') && !!(image.compareDocumentPosition(ring)&Node.DOCUMENT_POSITION_FOLLOWING) && node.querySelector('clipPath circle')?.getAttribute('r')==='9.5';
  }))).toBe(true);
  const ids=await editor.locator('clipPath').evaluateAll(nodes=>nodes.map(n=>n.id));expect(new Set(ids).size).toBe(ids.length);
  await editor.locator('.custom-icon-badges').screenshot({path:info.outputPath(`badges-${theme}.png`)});
});
