import { openTasks } from './choose';
import { test, expect, type Page } from './fixtures/test';
import { choose } from './choose';

const nav = (page: Page, name: string) => page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name, exact: true });
/** Below 1100 px an open editor takes the list's place; "All automations" is the way back. */
async function showAutomationList(page: Page) {
  const back = page.getByRole('button', { name: '← All automations', exact: true });
  if (await back.isVisible()) await back.click();
}
async function editor(page: Page, loadedWorkspace = false) {
  await page.addInitScript(() => localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'draft-fixture', title: 'Draft fixture', status: 'active', path: 'workspaces/draft-fixture', cwd: '/fixture/workspaces/draft-fixture', modified: '2026-10-22T12:00:00Z' }])));
  await page.goto('/?mock=1');
  if (loadedWorkspace) expect(await page.evaluate(async () => {
    const path = '/src/store/workspaces.ts';
    return await (await import(/* @vite-ignore */ path)).selectWorkspaceOpened('draft-fixture');
  })).toBe(true);
  await page.evaluate(async () => {
    const path = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ path);
    const original = api.listWorkflows;
    api.listWorkflows = async () => {
      const rows = await original();
      return [...rows, { ...rows[0], slug: 'second-automation', title: 'Second automation' }];
    };
  });
  await nav(page, 'Automations').click();
  await page.getByRole('button', { name: 'Open Morning Brief', exact: true }).click();
  await page.getByRole('button', { name: 'Automation options' }).click();
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  await expect(page.locator('.workflow-title-input')).toHaveValue('Morning Brief');
}

test('typing the name preserves Daily 9am through Create', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'draft-fixture', title: 'Draft fixture', status: 'active', path: 'workspaces/draft-fixture', cwd: '/fixture/workspaces/draft-fixture', modified: '2026-10-22T12:00:00Z' }])));
  await page.goto('/?mock=1');
  await nav(page, 'Automations').click();
  await page.getByRole('button', { name: 'New automation', exact: true }).click();
  await choose(page.getByRole('combobox', { name: 'Automation workspace' }), 'draft-fixture');
  await page.getByRole('button', { name: 'Daily', exact: true }).click();
  await choose(page.getByRole('combobox', { name: 'Hour', exact: true }), '9');
  await page.getByPlaceholder('e.g. Weekly evidence review').fill('My daily review');
  await page.locator('.workflow-new textarea').fill('Synthetic procedure');
  await page.evaluate(async () => {
    const path = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ path);
    const original = api.writeWorkflow;
    api.writeWorkflow = async (input: { schedule: string }, slug: string | null) => {
      localStorage.setItem('draftTestSchedule', input.schedule);
      return original(input, slug);
    };
  });
  await page.getByRole('button', { name: 'Create automation', exact: true }).click();
  await expect.poll(() => page.evaluate(() => localStorage.getItem('draftTestSchedule'))).toBe('0 0 9 * * * *');
});

test('row context Edit opens the editable form', async ({ page }) => {
  await page.goto('/?mock=1'); await nav(page, 'Automations').click();
  await page.getByRole('button', { name: 'Open Morning Brief', exact: true }).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  await expect(page.locator('.workflow-title-input')).toHaveValue('Morning Brief');
});

for (const route of ['back', 'tab', 'page', 'row', 'history', 'workspace']) test(`unsaved prompt survives cancelled ${route} navigation`, async ({ page }) => {
  await editor(page);
  await page.getByRole('textbox', { name: 'Prompt', exact: true }).fill('My unsaved draft');
  const leave = async () => {
    if (route === 'back') await page.getByRole('button', { name: 'Back to latest result', exact: false }).click();
    else if (route === 'tab') await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
    else if (route === 'page') await openTasks(page);
    else if (route === 'history') await page.getByRole('group', { name: 'History', exact: true }).getByRole('button', { name: 'Back', exact: true }).click();
    else if (route === 'workspace') await page.evaluate(async () => {
      const path = '/src/store/workspaces.ts'; const store = await import(/* @vite-ignore */ path);
      const target = store.workspaces().find((w: { id: string }) => w.id !== store.workspaceId());
      if (!target) throw new Error('Missing fixture workspace');
      void store.selectWorkspace(target.id);
    });
    else {
      await showAutomationList(page);
      await page.getByRole('button', { name: 'Open Second automation', exact: true }).click();
    }
  };
  await leave();
  const confirm = page.getByRole('dialog', { name: 'Discard unsaved changes to Morning Brief?' });
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(confirm).toBeHidden();
  // A cancelled row click leaves the list showing; below 1100 px the editor stays mounted behind it.
  await expect(page.getByRole('textbox', { name: 'Prompt', exact: true, includeHidden: true })).toHaveValue('My unsaved draft');
  await leave();
  await confirm.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(confirm).toBeHidden();
  await expect(page.getByRole('textbox', { name: 'Prompt', exact: true })).toHaveCount(0);
});

for (const route of ['result', 'loaded result', 'canvases'] as const) test(`a cancelled ${route} link preserves its destination before and after the draft answer`, async ({ page }) => {
  await editor(page, route === 'loaded result');
  await page.getByRole('textbox', { name: 'Prompt', exact: true }).fill('Keep this automation draft');
  const before = await page.evaluate(async () => {
    const libraryPath = '/src/store/libraryMode.ts', previewPath = '/src/store/preview.ts';
    const library = await import(/* @vite-ignore */ libraryPath);
    const preview = await import(/* @vite-ignore */ previewPath);
    library.setLibraryMode('notes', true); library.setResultsType('files');
    return {
      preview: JSON.stringify(preview.previewTarget() ?? null),
      library: [library.libraryMode(), library.resultsType(), library.canvasStartWorkspace(), library.canvasOpenRequest()],
    };
  });
  const readDestination = () => page.evaluate(async () => {
    const libraryPath = '/src/store/libraryMode.ts', previewPath = '/src/store/preview.ts';
    const library = await import(/* @vite-ignore */ libraryPath);
    const preview = await import(/* @vite-ignore */ previewPath);
    return {
      preview: JSON.stringify(preview.previewTarget() ?? null),
      library: [library.libraryMode(), library.resultsType(), library.canvasStartWorkspace(), library.canvasOpenRequest()],
    };
  });
  const leave = () => page.evaluate(async route => {
    if (route !== 'canvases') {
      const path = '/src/store/workspaces.ts';
      void (await import(/* @vite-ignore */ path)).openWorkspaceResult('draft-fixture', 'outputs/chosen.md');
    } else {
      const path = '/src/store/libraryMode.ts';
      void (await import(/* @vite-ignore */ path)).openLibraryCanvases('draft-fixture');
    }
  }, route);
  const confirm = page.getByRole('dialog', { name: 'Discard unsaved changes to Morning Brief?' });
  expect(await page.evaluate(async () => {
    const path = '/src/store/workspaces.ts';
    const store = await import(/* @vite-ignore */ path);
    return store.workspaceId() === 'draft-fixture' && !!store.workspaceDetail();
  })).toBe(route === 'loaded result');
  await leave();
  await expect(confirm).toBeVisible();
  expect(await readDestination()).toEqual(before);
  await confirm.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(confirm).toBeHidden();
  expect(await readDestination()).toEqual(before);
  await expect(page.getByRole('textbox', { name: 'Prompt', exact: true })).toHaveValue('Keep this automation draft');
  await leave();
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(confirm).toBeHidden();
  await expect(page.getByRole('textbox', { name: 'Prompt', exact: true })).toHaveCount(0);
  if (route !== 'canvases') {
    await expect(page.locator('.top-bar-crumbs')).toContainText('Draft fixture');
    // The Result opens in the Preview pane (COO rebuild L4: it was the
    // workspace's remembered Results view).
    await expect(page.getByRole('complementary', { name: 'Preview', exact: true }).getByRole('tab', { name: 'chosen.md', exact: true })).toHaveAttribute('aria-selected', 'true');
  } else {
    await expect(page.getByRole('navigation', { name: 'Library view' })).toBeVisible();
    await expect.poll(readDestination).not.toEqual(before);
    expect((await readDestination()).library.slice(0, 3)).toEqual(['results', 'canvases', 'draft-fixture']);
  }
  // Notes is All's view (overnight 11-12 L4), so All is what was remembered.
  expect(await page.evaluate(() => localStorage.getItem('hollow.library.mode'))).toBe('all');
});

test('first automation read failure offers Retry without a false empty state', async ({ page }) => {
  await page.addInitScript(() => { window.__mockFailCommandOnce = 'list_workflows'; });
  await page.goto('/?mock=1'); await nav(page, 'Automations').click();
  await expect(page.getByText("Couldn't read your automations").first()).toBeVisible();
  await expect(page.getByText('No automations yet')).toHaveCount(0);
  await page.getByRole('button', { name: 'Retry', exact: true }).first().click();
  await expect(page.getByRole('button', { name: 'Open Morning Brief', exact: true })).toBeVisible();
});

test('clean Back is immediate but title-only changes require a discard choice', async ({ page }) => {
  await editor(page);
  await page.getByRole('button', { name: 'Back to latest result', exact: false }).click();
  await expect(page.locator('.workflow-review')).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await page.getByRole('button', { name: 'Automation options' }).click();
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  await page.locator('.workflow-title-input').fill('Renamed draft');
  await openTasks(page);
  const confirm = page.getByRole('dialog', { name: 'Discard unsaved changes to Renamed draft?' });
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(confirm).toBeHidden();
  await expect(page.locator('.workflow-title-input')).toHaveValue('Renamed draft');
});

test('a new automation draft survives Cancel and page navigation refusal', async ({ page }) => {
  await page.goto('/?mock=1'); await nav(page, 'Automations').click();
  await page.getByRole('button', { name: 'New automation', exact: true }).click();
  await page.getByPlaceholder('e.g. Weekly evidence review').fill('New draft');
  await page.locator('.workflow-new textarea').fill('Keep this unsaved procedure');
  await page.locator('.workflow-new').getByRole('button', { name: 'Cancel', exact: true }).click();
  const confirm = page.getByRole('dialog', { name: 'Discard unsaved changes to New draft?' });
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(confirm).toBeHidden();
  await openTasks(page);
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(confirm).toBeHidden();
  await expect(page.locator('.workflow-new textarea')).toHaveValue('Keep this unsaved procedure');
});

test('a changed draft invalidates a pending discard choice', async ({ page }) => {
  await editor(page);
  await page.getByRole('textbox', { name: 'Prompt', exact: true }).fill('First draft');
  await openTasks(page);
  const confirm = page.getByRole('dialog', { name: 'Discard unsaved changes to Morning Brief?' });
  await expect(confirm).toBeVisible();
  // Simulate an editor update arriving while its global choice is open.
  await page.getByRole('textbox', { name: 'Prompt', exact: true }).evaluate((element: HTMLTextAreaElement) => {
    element.value = 'Newer draft'; element.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await confirm.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(confirm).toBeHidden();
  await expect(page.getByRole('textbox', { name: 'Prompt', exact: true })).toHaveValue('Newer draft');
  await expect(page.getByText('The draft changed. Choose where to go again.', { exact: true })).toBeVisible();
  await openTasks(page);
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Prompt', exact: true })).toHaveCount(0);
});

test('handled automation requests do not interrupt a draft when the list refreshes', async ({ page }) => {
  await editor(page);
  await page.getByRole('button', { name: 'Back to latest result', exact: false }).click();
  await page.evaluate(async () => {
    const path = '/src/store/runtime.ts'; (await import(/* @vite-ignore */ path)).requestOpenWorkflow('morning-brief');
  });
  expect(await page.evaluate(async () => {
    const path = '/src/store/runtime.ts'; return (await import(/* @vite-ignore */ path)).workflowOpenRequest();
  })).toBeNull();
  await expect(page.locator('.workflow-review')).toBeVisible();
  await page.getByRole('button', { name: 'Automation options' }).click();
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  await page.getByRole('textbox', { name: 'Prompt', exact: true }).fill('Keep typing');
  await page.evaluate(async () => {
    const path = '/src/store/runtime.ts'; await (await import(/* @vite-ignore */ path)).refetchWorkflows();
    await new Promise(requestAnimationFrame);
  });
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Prompt', exact: true })).toHaveValue('Keep typing');
});

for (const answer of ['Keep editing', 'Discard']) test(`a matching automation request is consumed after ${answer}`, async ({ page }) => {
  await editor(page);
  await page.getByRole('textbox', { name: 'Prompt', exact: true }).fill('A guarded draft');
  await page.evaluate(async () => {
    const path = '/src/store/runtime.ts'; (await import(/* @vite-ignore */ path)).requestOpenWorkflow('second-automation');
  });
  const confirm = page.getByRole('dialog', { name: 'Discard unsaved changes to Morning Brief?' });
  await expect(confirm).toBeVisible();
  expect(await page.evaluate(async () => {
    const path = '/src/store/runtime.ts'; return (await import(/* @vite-ignore */ path)).workflowOpenRequest();
  })).toBeNull();
  await confirm.getByRole('button', { name: answer, exact: true }).click();
  await expect(confirm).toBeHidden();
  await page.evaluate(async () => {
    const path = '/src/store/runtime.ts'; await (await import(/* @vite-ignore */ path)).refetchWorkflows();
  });
  // ConfirmLoading has role=dialog synchronously, before the lazy dialog loads.
  await expect(page.getByRole('dialog')).toHaveCount(0);
  if (answer === 'Keep editing') await expect(page.getByRole('textbox', { name: 'Prompt', exact: true })).toHaveValue('A guarded draft');
  else await expect(page.locator('.workflow-review').getByRole('heading', { name: 'Second automation', exact: true })).toBeVisible();
});

test('an unmatched automation request waits until its row arrives', async ({ page }) => {
  await editor(page);
  expect(await page.evaluate(async () => {
    const path = '/src/store/runtime.ts'; const store = await import(/* @vite-ignore */ path);
    store.requestOpenWorkflow('arriving-automation');
    return store.workflowOpenRequest()?.slug;
  })).toBe('arriving-automation');
  await expect(page.locator('.workflow-title-input')).toHaveValue('Morning Brief');
  await page.evaluate(async () => {
    const apiPath = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ apiPath);
    const original = api.listWorkflows;
    api.listWorkflows = async () => {
      const rows = await original();
      return [...rows, { ...rows[0], slug: 'arriving-automation', title: 'Arriving automation' }];
    };
    const path = '/src/store/runtime.ts'; await (await import(/* @vite-ignore */ path)).refetchWorkflows();
  });
  await expect(page.locator('.workflow-review').getByRole('heading', { name: 'Arriving automation', exact: true })).toBeVisible();
  expect(await page.evaluate(async () => {
    const path = '/src/store/runtime.ts'; return (await import(/* @vite-ignore */ path)).workflowOpenRequest();
  })).toBeNull();
});

for (const target of ['page', 'editor', 'new']) test(`choosing the current page or current form (${target}) does not ask to discard`, async ({ page }) => {
  await editor(page);
  if (target === 'new') {
    await page.getByRole('button', { name: 'New automation', exact: true }).click();
    await page.getByPlaceholder('e.g. Weekly evidence review').fill('Keep this new draft');
    await page.getByRole('button', { name: 'New automation', exact: true }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(page.getByPlaceholder('e.g. Weekly evidence review')).toHaveValue('Keep this new draft');
    return;
  }
  await page.getByRole('textbox', { name: 'Prompt', exact: true }).fill('Keep typing');
  if (target === 'page') await nav(page, 'Automations').click();
  else {
    await showAutomationList(page);
    await page.getByRole('button', { name: 'Open Morning Brief', exact: true }).click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  }
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Prompt', exact: true })).toHaveValue('Keep typing');
});
