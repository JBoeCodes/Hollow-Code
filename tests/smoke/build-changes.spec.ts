import { test, expect, type Page } from './fixtures/test';
import { choose, chooseNew, expectConversationWithPane, filesPicker, optionLabels, previewPane, projectFilesButton } from './choose';

// The Changes pane of a build workspace (build programme PR B2). The mock
// (src/mockBuild.ts) stands in for git and gh; mutations are recorded in
// localStorage `mockBuildLog`.

async function createBuildWorkspace(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Fix the About page layout.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('About page');
  await page.locator('.project-create-options > summary').click();
  await page.getByRole('radio', { name: 'Repository worktree' }).click();
  await choose(page.getByRole('combobox', { name: 'Repository' }), '/Users/fixture/Projects/hollow');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-branch')).toHaveText('hollow/about-page');
}

/** The build workspace's Changes, in the Preview pane from the top bar's
 *  branch (COO rebuild L4; it was the workspace panel's Changes section). */
async function openChanges(page: Page) {
  await createBuildWorkspace(page);
  await page.getByRole('button', { name: 'Changes on hollow/about-page', exact: true }).click();
  await expect(previewPane(page).locator('.build-changes')).toBeVisible();
}

const log = (page: Page) => page.evaluate(() => JSON.parse(localStorage.getItem('mockBuildLog') ?? '[]') as { command: string; args: Record<string, unknown> }[]);

// P1-46: one delivery truth. Patch the build workspace's mock git state or app
// build and tell the pane, as the backend's events do.
const patchBuild = (page: Page, patch: Record<string, unknown>) => page.evaluate(p => {
  const id = JSON.parse(localStorage.getItem('mockWorkspaces')!).find((w: { build?: unknown }) => w.build).id;
  const key = `mockBuild:${id}`;
  const cur = JSON.parse(localStorage.getItem(key) ?? 'null') ?? { files: [], commits: [{ sha: 'a1b2c3d', subject: 'Start the About page', when: '' }], upstream: null, unpushed: 0, pr: null };
  localStorage.setItem(key, JSON.stringify({ ...cur, ...p }));
  window.__mockEmit('build-changed', id);
}, patch);
const patchAppBuild = (page: Page, patch: Record<string, unknown>) => page.evaluate(p => {
  const id = JSON.parse(localStorage.getItem('mockWorkspaces')!).find((w: { build?: unknown }) => w.build).id;
  const key = `mockAppBuild:${id}`;
  localStorage.setItem(key, JSON.stringify({ ...JSON.parse(localStorage.getItem(key) ?? '{}'), ...p }));
  window.__mockEmit('self-install-event', { workspaceId: id, state: p.state });
}, patch);
const step = (page: Page, label: string) => page.getByRole('list', { name: 'Delivery record' }).locator('li').filter({ has: page.locator('strong', { hasText: new RegExp(`^${label}$`) }) });

test('a finished build moves the delivery record off "Building now" (P1-46)', async ({ page }) => {
  await openChanges(page);
  const pane = page.locator('.build-changes');
  await pane.getByRole('button', { name: 'Build app', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Build app', exact: true }).click();
  await expect(step(page, 'Built')).toContainText('Building now');
  await patchAppBuild(page, { state: 'built', commit: 'a1b2c3d', receipt: { buildId: 'b-1' } });
  await expect(step(page, 'Built')).toHaveAttribute('data-state', 'done');
  await expect(step(page, 'Built')).toContainText('Built from a1b2c3d');
});

test('building shows how long it has run and when it stops; a failed read clears on the next good one (P1-46)', async ({ page }) => {
  await page.clock.install({ time: new Date('2026-10-25T18:30:00Z') });
  await openChanges(page);
  const app = page.locator('.bc-app');
  await page.evaluate(() => localStorage.setItem('mockAppBuildStatusError', 'The build status could not be read'));
  await patchAppBuild(page, { state: 'building', startedAt: '2026-10-25T18:24:00Z', branch: 'hollow/about-page', commit: 'a1b2c3d' });
  await expect(app).toContainText('The build status could not be read');
  await page.evaluate(() => localStorage.removeItem('mockAppBuildStatusError'));
  await patchAppBuild(page, { state: 'building' });
  await expect(app).toContainText('Building · 6 min (stops at 45 min)');
  await expect(app).not.toContainText('The build status could not be read');
  await page.clock.fastForward(60_000);
  await expect(app).toContainText('Building · 7 min (stops at 45 min)');
});

test('a merged PR counts as committed and hides Open PR… (P1-46)', async ({ page }) => {
  await openChanges(page);
  const pane = page.locator('.build-changes');
  await patchBuild(page, { commits: [], files: [], upstream: 'origin/hollow/about-page', pr: { url: 'https://github.com/fixture/site/pull/42', state: 'merged', number: 42 } });
  await expect(pane.locator('.bc-pr')).toHaveText('PR #42 · merged');
  await expect(pane.getByRole('button', { name: 'Open PR…' })).toHaveCount(0);
  await expect(step(page, 'Committed')).toHaveAttribute('data-state', 'done');
  await expect(step(page, 'Committed')).toContainText('Merged into origin/main with PR #42');
});

test('an unreadable install receipt says so and Retry reads it again (P1-46)', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockSelfInstallReceiptError', 'last-result.json is not valid JSON'));
  await openChanges(page);
  await expect(step(page, 'Installed')).toHaveAttribute('data-state', 'unknown');
  await expect(step(page, 'Installed')).toContainText("The install record couldn't be read: last-result.json is not valid JSON");
  await expect(step(page, 'Installed')).not.toContainText('No install recorded');
  await page.evaluate(() => localStorage.removeItem('mockSelfInstallReceiptError'));
  await step(page, 'Installed').getByRole('button', { name: 'Retry' }).click();
  await expect(step(page, 'Installed')).toContainText('No install recorded for this branch');
});

test('Build app names how far behind the branch is, and Update from origin/main catches up (P1-46)', async ({ page }) => {
  await openChanges(page);
  const pane = page.locator('.build-changes');
  await patchBuild(page, { files: [], behind: 3 });
  await expect(pane.locator('.bc-counts')).toContainText('3 behind');
  await pane.getByRole('button', { name: 'Build app', exact: true }).click();
  await expect(page.getByRole('dialog')).toContainText('This branch is 3 commits behind origin/main');
  await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click();
  const update = pane.getByRole('button', { name: 'Update from origin/main…' });
  await page.evaluate(() => localStorage.setItem('mockBuildBlocked', 'Wait for this workspace’s running conversations to finish.'));
  await page.evaluate(() => window.__mockEmit('build-changed', JSON.parse(localStorage.getItem('mockWorkspaces')!).find((w: { build?: unknown }) => w.build).id));
  await expect(update).toBeDisabled();
  await page.evaluate(() => localStorage.removeItem('mockBuildBlocked'));
  await page.evaluate(() => window.__mockEmit('build-changed', JSON.parse(localStorage.getItem('mockWorkspaces')!).find((w: { build?: unknown }) => w.build).id));
  await update.click();
  await expect(page.getByRole('dialog')).toContainText('Fetches origin/main and replays this branch’s 1 commit on top');
  await page.getByRole('dialog').getByRole('button', { name: 'Update', exact: true }).click();
  await expect(pane.locator('.bc-counts')).not.toContainText('behind');
  await expect(update).toHaveCount(0);
  expect((await log(page)).map(e => e.command)).toEqual(['build_update_from_base']);
});

test('the Services empty state says how long a dev server keeps running (P1-46)', async ({ page }) => {
  await openChanges(page);
  const services = page.locator('.build-changes').getByText(/^Nothing running\./);
  await expect(services).toHaveText('Nothing running. Ask the agent to start a dev server here. It keeps running after the turn ends, until you stop it here or quit Hollow; archiving the chat that started it stops it too, unless that chat runs in Bypass all.');
});

test('background work blocks Commit before confirmation and finishing refreshes it', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('mockBuildBlocked', 'Wait for Polish lane to finish');
    localStorage.setItem('mockAppBuildInstant', '1');
    localStorage.setItem('mockRunQueue', JSON.stringify([{id:900,source:'chat',sourceId:'background-lane',label:'Polish lane',status:'running',priority:0,attempt:1,coalescedCount:0,requestedAt:'2026-10-22T20:00:00Z',cancelRequested:false}]));
  });
  await openChanges(page);
  const pane = page.locator('.build-changes');
  await pane.getByRole('textbox', {name:'Commit message'}).fill('Do not race the lane');
  await expect(pane.getByRole('button',{name:'Commit',exact:true})).toBeDisabled();
  await expect(pane.locator('.bc-actions')).toContainText('Wait for Polish lane');
  const before = await page.evaluate(() => window.__mockApiCalls?.build_status ?? 0);
  await page.evaluate(() => {
    localStorage.removeItem('mockBuildBlocked');
    localStorage.setItem('mockRunQueue','[]');
    window.__mockEmit('run-queue-event', {kind:'finished',id:900});
  });
  await expect(pane.getByRole('button',{name:'Commit',exact:true})).toBeEnabled();
  expect(await page.evaluate(() => window.__mockApiCalls?.build_status ?? 0)).toBeGreaterThan(before);
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('a newly blocked worktree is reread before opening Commit confirmation', async ({page}) => {
  await openChanges(page);
  const pane = page.locator('.build-changes');
  await pane.getByRole('textbox',{name:'Commit message'}).fill('Check live work first');
  await page.evaluate(() => localStorage.setItem('mockBuildBlocked','Wait for a new lane to finish'));
  await pane.getByRole('button',{name:'Commit',exact:true}).click();
  await expect(pane).toContainText('Wait for a new lane');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(await log(page)).toEqual([]);
});

test('external commits refresh within 15 seconds and show when status was read', async ({page}) => {
  await page.clock.install({time:new Date('2026-10-23T01:30:50Z')});
  await openChanges(page);
  const pane = page.locator('.build-changes');
  const read = pane.getByText(/^Read /);
  await expect(read).toBeVisible();
  const old = await read.textContent();
  const before = await page.evaluate(() => window.__mockApiCalls?.build_status ?? 0);
  await page.evaluate(() => {
    const id = JSON.parse(localStorage.getItem('mockWorkspaces')!).find((w:{build?:unknown}) => w.build).id;
    localStorage.setItem(`mockBuild:${id}`, JSON.stringify({files:[],commits:[{sha:'fresh123',subject:'Committed from another terminal',when:new Date().toISOString()},{sha:'old123',subject:'Earlier',when:''}],upstream:null,unpushed:0,pr:null}));
  });
  await page.clock.fastForward(16000);
  await expect(pane.locator('.bc-counts')).toContainText('2 ahead');
  await expect(read).not.toHaveText(old!);
  expect(await page.evaluate(() => window.__mockApiCalls?.build_status ?? 0)).toBeGreaterThan(before);
});

// Installing moved to the top bar's Update button (row U10), whose
// Restart to update runs the restart checks itself.
for (const action of ['Push', 'Open pull request']) {
  test(`${action} rereads a new busy guard before confirmation`, async ({page}) => {
    await page.addInitScript(() => localStorage.setItem('mockAppBuildInstant', '1'));
    await openChanges(page);
    const pane = page.locator('.build-changes');
    if (action === 'Open pull request') await pane.getByRole('button', {name:'Open PR…'}).click();
    const button = pane.getByRole('button', {name:action,exact:true});
    await expect(button).toBeEnabled();
    await page.evaluate(() => {
      localStorage.setItem('mockBuildBlocked','Wait for another conversation to finish');
      localStorage.setItem('mockBuildLog','[]');
    });
    await button.click();
    await expect(pane).toContainText('Wait for another conversation');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    expect(await log(page)).toEqual([]);
  });
}

test('a build change event refreshes the visible folder without waiting for the poll', async ({page}) => {
  await openChanges(page);
  const before = await page.evaluate(() => window.__mockApiCalls?.build_status ?? 0);
  await page.evaluate(() => {
    const id = JSON.parse(localStorage.getItem('mockWorkspaces')!).find((w:{build?:unknown}) => w.build).id;
    localStorage.setItem('mockBuildBlocked','Wait for the changed folder');
    window.__mockEmit('build-changed', id);
  });
  await expect(page.locator('.bc-actions')).toContainText('Wait for the changed folder');
  expect(await page.evaluate(() => window.__mockApiCalls?.build_status ?? 0)).toBeGreaterThan(before);
});

test('a queue event cannot silently discard a pending Commit check', async ({page}) => {
  await openChanges(page);
  await page.getByRole('textbox',{name:'Commit message'}).fill('Keep my click');
  await page.evaluate(() => {
    const w = window as unknown as { releaseStatus: () => void };
    window.__mockHoldOnce = {command:'build_status', release:new Promise<void>(resolve => {w.releaseStatus = resolve;})};
  });
  await page.getByRole('button',{name:'Commit',exact:true}).click();
  await expect(page.getByRole('button',{name:'Committing…',exact:true})).toBeDisabled();
  await page.evaluate(() => {
    window.__mockEmit('run-queue-event',{kind:'finished',id:901});
    (window as unknown as {releaseStatus: () => void}).releaseStatus();
  });
  await expect(page.getByRole('dialog')).toContainText('Commit 2 changes?');
  await page.getByRole('dialog').getByRole('button',{name:'Cancel',exact:true}).click();
  expect(await log(page)).toEqual([]);
});

test('the branch opens Changes in the Preview pane', async ({ page }) => {
  await createBuildWorkspace(page);
  const branch = page.getByRole('button', { name: 'Changes on hollow/about-page', exact: true });
  await expect(branch).toBeVisible();
  await branch.click();
  const pane = previewPane(page);
  await expect(pane).toBeVisible();
  await expect(pane.getByRole('tab', { name: 'Changes', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(pane.getByRole('region', { name: 'Changed files' })).toBeVisible();
  await expect(pane.getByRole('region', { name: 'Commit and publish' })).toBeVisible();
  // The conversation stays beside it, and Project files is pressed while
  // Changes shows; pressed, it hides the pane.
  await expectConversationWithPane(page);
  const files = projectFilesButton(page);
  await expect(files).toHaveAttribute('aria-pressed', 'true');
  await files.click();
  await expect(pane).toHaveCount(0);
  // The Files picker's Changed files group ends with All changes, which
  // opens the same tab.
  await files.click();
  await expect(pane.getByRole('tab', { name: 'WORKSPACE.md', exact: true })).toHaveAttribute('aria-selected', 'true');
  const labels = await optionLabels(filesPicker(page));
  expect(labels).toContain('All changes: commit and publish…');
  await choose(filesPicker(page), 'hollow:changes');
  await expect(pane.getByRole('tab', { name: 'Changes', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(pane.locator('.build-changes')).toBeVisible();
});

test('a general workspace has no branch and no Changes', async ({ page }) => {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Memo project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Write a memo.');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Memo project');
  await expect(page.locator('.work-branch')).toHaveCount(0);
  await expect(page.getByRole('button', { name: /^Changes on / })).toHaveCount(0);
  await projectFilesButton(page).click();
  const labels = await optionLabels(filesPicker(page));
  expect(labels).not.toContain('All changes: commit and publish…');
});

test('files, a diff, and Ask about this drafts the hunk without sending', async ({ page }) => {
  await openChanges(page);
  const pane = page.locator('.build-changes');
  await expect(pane.locator('.bc-branch')).toContainText('hollow/about-page');
  await expect(pane.locator('.bc-branch')).toContainText('origin/main');
  await expect(pane.locator('.bc-counts')).toContainText('1 ahead');
  await expect(pane.locator('.bc-counts')).toContainText('not pushed');
  const readme = pane.getByRole('button', { name: /README\.md/ });
  await expect(readme).toContainText('+3');
  await expect(readme).toContainText('−1');
  await expect(pane.getByRole('button', { name: /src\/about\.ts/ }).locator('.bc-letter')).toHaveText('U');
  await readme.click();
  await expect(pane.locator('.bc-line.add').first()).toContainText('A clearer intro.');
  await expect(pane.locator('.bc-line.del')).toContainText('Old intro.');
  const sends = await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0);
  await pane.getByRole('button', { name: 'Ask about this' }).click();
  const composer = page.getByLabel('Chat message');
  await expect(composer).toHaveValue(/About this change in README\.md \(lines 1-6\):/);
  await expect(composer).toHaveValue(/```diff\n@@ -1,4 \+1,6 @@ # Site\n # Site\n-Old intro\.\n\+A clearer intro\./);
  expect(await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(sends);
});

test('commit, push and open a PR each confirm first', async ({ page }) => {
  await openChanges(page);
  const pane = page.locator('.build-changes');
  const commit = pane.getByRole('button', { name: 'Commit', exact: true });
  await expect(commit).toBeDisabled();
  await pane.getByRole('textbox', { name: 'Commit message' }).fill('Rewrite the intro');
  // Cancelling the confirm sends nothing.
  await commit.click();
  await expect(page.locator('.confirm-dialog')).toContainText('Commit 2 changes?');
  await page.locator('.confirm-dialog').getByRole('button', { name: 'Cancel' }).click();
  expect(await log(page)).toEqual([]);
  await commit.click();
  await page.locator('.confirm-dialog').getByRole('button', { name: 'Commit', exact: true }).click();
  await expect(pane.locator('.bc-counts')).toContainText('2 ahead');
  expect((await log(page))[0]).toMatchObject({ command: 'build_commit', args: { message: 'Rewrite the intro' } });
  await expect(pane.getByRole('textbox', { name: 'Commit message' })).toBeDisabled();

  await pane.getByRole('button', { name: 'Push', exact: true }).click();
  await expect(page.locator('.confirm-dialog')).toContainText('Push hollow/about-page to origin?');
  await page.locator('.confirm-dialog').getByRole('button', { name: 'Push', exact: true }).click();
  await expect(pane.locator('.bc-counts')).toContainText('pushed');
  expect((await log(page)).map(e => e.command)).toEqual(['build_commit', 'build_push']);

  await pane.getByRole('button', { name: 'Open PR…' }).click();
  await expect(pane.getByRole('textbox', { name: 'Pull request title' })).toHaveValue('About page');
  await expect(pane.getByRole('textbox', { name: 'Pull request description' })).toHaveValue('Fix the About page layout.');
  await pane.getByRole('textbox', { name: 'Pull request title' }).fill('Fix the About page');
  await pane.getByRole('button', { name: 'Open pull request' }).click();
  await expect(page.locator('.confirm-dialog')).toContainText('gh pr create for hollow/about-page into main');
  await page.locator('.confirm-dialog').getByRole('button', { name: 'Open pull request' }).click();
  await expect(pane.locator('.bc-pr')).toHaveText('PR #42 · open');
  expect((await log(page)).at(-1)).toMatchObject({ command: 'build_open_pr', args: { title: 'Fix the About page', body: 'Fix the About page layout.' } });
  await expect(pane.getByRole('button', { name: 'Open PR…' })).toHaveCount(0);
});
