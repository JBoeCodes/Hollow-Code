import { openTasks } from './choose';
// A creation journey runs at desktop, minimum width and Largest text.
import { test, expect, type Page } from './fixtures/test';
import { chooseNew } from './choose';

async function form(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Journey project');
  return page.locator('.work-create');
}

test('new workspace explains the first blocker (P1-36)', async ({ page }, info) => {
  const editor = await form(page);
  await editor.getByRole('textbox', { name: 'Project name' }).fill('');
  await expect(editor.getByText('Give the project a name.', { exact: true })).toBeVisible();
  const bounds = await editor.boundingBox();
  expect(bounds!.y).toBeGreaterThanOrEqual(0);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(page.viewportSize()!.height);
  await expect(editor.getByRole('button', { name: 'Cancel', exact: true })).toBeInViewport();
  await page.screenshot({ path: info.outputPath('project-dialog.png') });
});

test('new workspace rejects an untouched starter (P1-36)', async ({ page }) => {
  const editor = await form(page);
  await editor.locator('.project-create-options > summary').click();
  await editor.getByRole('button', { name: 'Research a question', exact: true }).click();
  await expect(editor.getByText('Replace the … first', { exact: true })).toBeVisible();
  await expect(editor.getByRole('button', { name: 'Create project', exact: true })).toBeDisabled();
  await expect(editor.getByRole('button', { name: 'Create and start', exact: true })).toBeDisabled();
  await editor.getByRole('textbox', { name: 'Project instructions' }).fill('Compare the two local plans.');
  await expect(editor.getByRole('button', { name: 'Create project', exact: true })).toBeEnabled();
});

test('Command Enter creates once without starting a chat (P1-36)', async ({ page }) => {
  const editor = await form(page);
  await editor.getByRole('textbox', { name: 'Project instructions' }).fill('Prepare the quarterly review.');
  await editor.getByRole('textbox', { name: 'Project name' }).fill('Quarterly review');
  await editor.getByRole('textbox', { name: 'Project instructions' }).press('Meta+Enter');
  await expect(page.locator('.work-header h1')).toHaveText('Quarterly review');
  await expect(page.getByRole('textbox', { name: 'Chat message', exact: true })).toBeFocused();
  const calls = await page.evaluate(() => window.__mockApiCalls ?? {});
  expect(calls.create_workspace).toBe(1);
  expect(calls.chat_send ?? 0).toBe(0);
});

test('Escape closes the new workspace form without creating (P1-36)', async ({ page }) => {
  const editor = await form(page);
  await editor.getByRole('textbox', { name: 'Project instructions' }).fill('A draft only');
  await editor.getByRole('textbox', { name: 'Project instructions' }).press('Escape');
  await expect(editor).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockApiCalls?.create_workspace ?? 0)).toBe(0);
});

test('repository discovery explains missing configuration and retries in place (P1-36)', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockRepoCandidatesError', 'No Projects folder is set'));
  const editor = await form(page);
  await editor.getByRole('textbox', { name: 'Project instructions' }).fill('Fix the layout.');
  await editor.locator('.project-create-options > summary').click();
  await editor.getByRole('radio', { name: 'Repository worktree' }).click();
  await expect(editor).toContainText('No Projects folder is set');
  await expect(editor.getByRole('group', { name: 'Start from' }).getByRole('button', { name: 'Build in a repository' })).toHaveCount(0);
  await expect(editor.locator('label').filter({ has: page.getByRole('textbox', { name: 'Base branch' }) })).toContainText('Base branch');
  await page.evaluate(() => localStorage.removeItem('mockRepoCandidatesError'));
  await editor.getByRole('button', { name: 'Retry repositories', exact: true }).click();
  await expect(editor.getByRole('combobox', { name: 'Repository', exact: true })).toBeVisible();
  await expect(editor.getByRole('textbox', { name: 'Project instructions' })).toHaveValue('Fix the layout.');
});

test('a failed workspace list has failure copy and a working retry (P1-36)', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockWorkspaceListError', '1'));
  await page.goto('/?mock=1');
  await expect(page.getByText("Couldn't load your workspaces.", { exact: true })).toBeVisible();
  await expect(page.getByText('Loading your work…', { exact: true })).toHaveCount(0);
  await page.evaluate(() => localStorage.removeItem('mockWorkspaceListError'));
  await page.locator('.studio-loading').getByRole('button', { name: 'Retry loading workspaces', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Projects', exact: true }).getByRole('button', { name: 'New project', exact: true })).toBeVisible();
});

test('Create and start sends exactly the goal in the created workspace (P1-36)', async ({ page }) => {
  const editor = await form(page);
  const goal = 'Compare our two local plans and save a recommendation.';
  await editor.getByRole('textbox', { name: 'Project instructions' }).fill(goal);
  await editor.getByRole('textbox', { name: 'Project name' }).fill('Plan comparison');
  await page.evaluate(() => {
    const w=window as unknown as { releaseCreate?: () => void };
    window.__mockHoldOnce={command:'create_workspace',release:new Promise<void>(resolve=>{w.releaseCreate=resolve;})};
  });
  await editor.getByRole('button', { name: 'Create and start', exact: true }).evaluate(button=>{
    (button as HTMLButtonElement).click();(button as HTMLButtonElement).click();
  });
  await expect.poll(()=>page.evaluate(()=>window.__mockApiCalls?.create_workspace??0)).toBe(1);
  await page.evaluate(()=>{
    const made=JSON.parse(localStorage.getItem('mockWorkspaces')??'[]').find((w:{title:string})=>w.title==='Plan comparison');
    localStorage.setItem(`hollow.draft.${made.id}`,'Keep this separate draft');
    (window as unknown as { releaseCreate:()=>void }).releaseCreate();
  });
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(1);
  const result = await page.evaluate(() => ({ args: JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'), workspaces: JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') }));
  expect(result.args.prompt.split('</hollow-chat-intent>\n\n')[1]).toBe(goal);
  expect(result.args.cwd).toBe(result.workspaces.find((w: {title: string}) => w.title === 'Plan comparison').cwd);
  expect(await page.evaluate(() => window.__mockApiCalls?.create_workspace ?? 0)).toBe(1);
  // A legacy new-chat draft stays in its own slot; the main agent sends only the goal.
  expect(result.args.sessionKey).toMatch(/^s-project-[a-f0-9]{48}$/);
  await page.keyboard.press('Meta+n');
  await expect(page.getByRole('textbox',{name:'Chat message',exact:true})).toHaveValue('Keep this separate draft');
});

test('creation completing after navigation does not take the page or foreign draft (P1-36)', async ({ page }) => {
  const editor = await form(page);
  await editor.getByRole('textbox', { name: 'Project instructions' }).fill('Keep this in its own workspace.');
  await page.evaluate(() => {
    const w = window as unknown as { releaseCreate?: () => void };
    window.__mockHoldOnce = { command: 'create_workspace', release: new Promise<void>(resolve => { w.releaseCreate = resolve; }) };
  });
  await editor.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.create_workspace ?? 0)).toBe(1);
  // A keyboard route or external navigation may leave while creation is pending.
  await page.evaluate(async moduleUrl => (await import(moduleUrl)).setActivePage('tasks'), '/src/store.ts');
  // The view-transition callback applies navigation after the click returns.
  await expect.poll(() => page.evaluate(async moduleUrl => (await import(moduleUrl)).activePage(), '/src/store.ts')).toBe('tasks');
  await page.evaluate(async moduleUrl => {
    const store = await import(moduleUrl);
    store.setChatInput('A foreign draft to preserve');
    (window as unknown as { releaseCreate: () => void }).releaseCreate();
  }, '/src/store.ts');
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.list_workspaces ?? 0)).toBeGreaterThan(1);
  await expect(page.locator('.work-create')).toHaveCount(0);
  expect(await page.evaluate(async moduleUrl => {
    const store = await import(moduleUrl);
    return { page: store.activePage(), draft: store.chatInput(), sent: window.__mockApiCalls?.chat_send ?? 0 };
  }, '/src/store.ts')).toEqual({ page: 'tasks', draft: 'A foreign draft to preserve', sent: 0 });
  await chooseNew(page, 'New project');
  await expect(page.locator('.work-create').getByRole('button', { name: 'Open and start', exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.__mockApiCalls?.create_workspace ?? 0)).toBe(1);
});

test('failed opening keeps the created workspace and retries without creating twice (P1-36)', async ({ page }) => {
  const editor = await form(page);
  await editor.getByRole('textbox', { name: 'Project instructions' }).fill('Retain the goal after an open failure.');
  await page.evaluate(() => { window.__mockFailCommandOnce = 'read_workspace'; });
  await editor.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(editor.getByRole('textbox', { name: 'Project instructions' })).toHaveValue('Retain the goal after an open failure.');
  await editor.getByRole('button', { name: 'Open and start', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(1);
  expect(await page.evaluate(() => window.__mockApiCalls?.create_workspace ?? 0)).toBe(1);
});

test('a creation rejected after leaving retains its error and draft for retry (P1-36)', async ({ page }) => {
  const editor = await form(page);
  await editor.getByRole('textbox', { name: 'Project instructions' }).fill('Keep my failed creation draft.');
  await editor.getByRole('textbox', { name: 'Project name' }).fill('Recover this draft');
  await page.evaluate(async moduleUrl => {
    const { api } = await import(moduleUrl);
    const original = api.createWorkspace;
    const w = window as unknown as { rejectCreation: () => void; creationStarted: boolean };
    api.createWorkspace = () => {
      w.creationStarted = true;
      return new Promise((_resolve, reject) => { w.rejectCreation = () => {
        api.createWorkspace = original;
        reject('Creation failed: fixture rejection');
      }; });
    };
  }, '/src/api.ts');
  await editor.getByRole('button', { name: 'Create and start', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { creationStarted: boolean }).creationStarted)).toBe(true);
  // A keyboard route or external navigation may leave while creation is pending.
  await page.evaluate(async moduleUrl => (await import(moduleUrl)).setActivePage('tasks'), '/src/store.ts');
  await page.evaluate(() => (window as unknown as { rejectCreation: () => void }).rejectCreation());
  await chooseNew(page, 'New project');
  await expect(editor).toContainText('Creation failed: fixture rejection');
  await expect(editor.getByRole('textbox', { name: 'Project instructions' })).toHaveValue('Keep my failed creation draft.');
  await expect(editor.getByRole('textbox', { name: 'Project name' })).toHaveValue('Recover this draft');
  expect(await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(0);
  await editor.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Recover this draft');
  expect(await page.evaluate(() => window.__mockApiCalls?.create_workspace ?? 0)).toBe(1);
});

test('Create and start does not return or send after leaving a delayed open (P1-36)', async ({ page }) => {
  const editor = await form(page);
  await editor.getByRole('textbox', { name: 'Project instructions' }).fill('A goal owned by the new workspace');
  await page.evaluate(() => {
    const w = window as unknown as { releaseOpen?: () => void; heldOpen?: { delivered?: boolean } };
    const held = { command: 'read_workspace', release: new Promise<void>(resolve => { w.releaseOpen = resolve; }) };
    window.__mockHoldOnce = held; w.heldOpen = held;
  });
  await editor.getByRole('button', { name: 'Create and start', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.read_workspace ?? 0)).toBe(1);
  // A keyboard route or external navigation may leave while creation is pending.
  await page.evaluate(async moduleUrl => (await import(moduleUrl)).setActivePage('tasks'), '/src/store.ts');
  await expect.poll(() => page.evaluate(async moduleUrl => (await import(moduleUrl)).activePage(), '/src/store.ts')).toBe('tasks');
  await page.evaluate(async moduleUrl => {
    const store = await import(moduleUrl);
    store.setChatInput('Another page owns this draft');
    (window as unknown as { releaseOpen: () => void }).releaseOpen();
  }, '/src/store.ts');
  await expect.poll(() => page.evaluate(() => (window as unknown as { heldOpen: { delivered?: boolean } }).heldOpen.delivered)).toBe(true);
  expect(await page.evaluate(async moduleUrl => {
    const store = await import(moduleUrl);
    return { page: store.activePage(), draft: store.chatInput(), sent: window.__mockApiCalls?.chat_send ?? 0 };
  }, '/src/store.ts')).toEqual({ page: 'tasks', draft: 'Another page owns this draft', sent: 0 });
});

test('an unconsumed first goal is cancelled when its conversation leaves (P1-36)', async ({ page }) => {
  let release!: () => void;
  const ready = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/src/components/ChatBody.tsx', async route => { await ready; await route.continue(); });
  try {
    const editor = await form(page);
    await editor.getByRole('textbox', { name: 'Project instructions' }).fill('Never replay this in a later conversation.');
    await editor.getByRole('button', { name: 'Create and start', exact: true }).click();
    await expect.poll(() => page.evaluate(async moduleUrl => (await import(moduleUrl)).pendingChatSend()?.text, '/src/store/runtime.ts')).toBe('Never replay this in a later conversation.');
    await openTasks(page);
    expect(await page.evaluate(async moduleUrl => (await import(moduleUrl)).pendingChatSend(), '/src/store/runtime.ts')).toBeNull();
    expect(await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(0);
  } finally { release(); }
});
