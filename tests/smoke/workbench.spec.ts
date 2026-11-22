import { openTasks } from './choose';
import { test, expect, type Page } from './fixtures/test';
import { choose, chooseNew, closeProjectFiles, openProjectFiles, paneCovers, previewPane, projectFilesButton, sidebar } from './choose';
import { openLibraryView } from './libraryView';

async function createWorkspace(page: Page, name: string) {
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', {name: 'Project instructions'}).fill(`Produce a checked ${name} report.`);
  await page.getByRole('textbox', {name: 'Project name'}).fill(name);
  await page.getByRole('button', {name: 'Create project', exact: true}).click();
  await expect(page.locator('.work-header h1')).toHaveText(name);
}

test.beforeEach(async ({page}) => { await page.goto('/?mock=1'); });

// COO rebuild L4 (2026-11-10): the workspace panel and its sections are gone.
// The top bar's Project files opens the project's notes in the Preview pane,
// beside the conversation where there is room and over it where there isn't.
test('Project files keeps the conversation’s draft, and WORKSPACE.md reads back after a reload', async ({page}) => {
  await createWorkspace(page, 'Quarter review');
  const composer = page.getByLabel('Chat message');
  await composer.fill('Keep this draft while I check context.');
  await openProjectFiles(page);
  const pane = previewPane(page);
  await expect(pane.getByRole('tab', {name:'WORKSPACE.md', exact:true})).toHaveAttribute('aria-selected', 'true');
  if (await paneCovers(page)) await expect(page.getByRole('region', {name:'Workspace conversation'})).toBeHidden();
  else await expect(composer).toHaveValue('Keep this draft while I check context.');
  // Project files, pressed, returns to the same conversation, its draft as left.
  await closeProjectFiles(page);
  await expect(composer).toHaveValue('Keep this draft while I check context.');
  await page.reload();
  await expect(page.locator('.work-header h1')).toHaveText('Quarter review');
  await openProjectFiles(page);
  await expect(pane).toContainText('Quarter review');
});

// Overnight 11-04 U4: the conversation stays mounted behind the other pages,
// leaves the top bar and focus to the page on screen, and its sidebar row
// brings the same conversation back, draft and all.
test('the workspace conversation stays mounted behind Tasks and its sidebar row brings it back as left', async ({page}, info) => {
  test.skip(info.project.name.includes('narrow'), 'Workspace rows intentionally collapse on narrow screens');
  await page.goto('/?mock=1&demo=1');
  // Opened and brought back by its sidebar row, which opens the project's
  // lead (L8, 11-10); Home's card opens its newest chat, an earlier one here.
  const projectRow = sidebar(page).getByRole('button', { name: /^Q3 regional operations review/ });
  await projectRow.click();
  const composer = page.getByLabel('Chat message');
  await composer.fill('A draft that waits behind Tasks');
  // A remount would lose this mark on the live conversation element.
  await composer.evaluate(el => el.setAttribute('data-retained-probe', 'u4'));
  await openTasks(page);
  await expect(page.locator('textarea[data-retained-probe="u4"]')).toBeAttached();
  await expect(page.locator('textarea[data-retained-probe="u4"]')).toBeHidden();
  // Hidden, the workspace puts nothing in the top bar and takes no focus.
  await expect(projectFilesButton(page)).toHaveCount(0);
  await expect(page.locator('.top-bar .work-topbar-actions')).toHaveCount(0);
  await page.evaluate(async () => {
    const path = '/src/store/chat.ts';
    const chat = await import(/* @vite-ignore */ path);
    chat.requestChatFocus();
  });
  await expect(composer).not.toBeFocused();
  // One click on its sidebar row: the same element, the draft as left. (Home's
  // "Open …" row is gone once a project is open, so it is no way back.)
  await sidebar(page).getByRole('button', { name: /^Q3 regional operations review/ }).click();
  await expect(page.locator('textarea[data-retained-probe="u4"]')).toBeVisible();
  await expect(composer).toHaveValue('A draft that waits behind Tasks');
  await expect(projectFilesButton(page)).toBeVisible();
});

test('Library Conversations takes sole composer ownership and returns it to the workspace', async ({page}, info) => {
  test.skip(info.project.name.includes('narrow'), 'Workspace rows intentionally collapse on narrow screens');
  await page.goto('/?mock=1&demo=1');
  // The project's sidebar row opens its lead and is the way back once
  // another page shows (L8, 11-10: Home's card opens its newest chat, an
  // earlier one here, and is gone once a project is open).
  const back = sidebar(page).getByRole('button', { name: /^Q3 regional operations review/ });
  await back.click();
  const composers = page.locator('textarea[aria-label="Chat message"]');
  await expect(composers).toHaveCount(1);
  await composers.fill('Same shared draft through Library history');
  await page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'Conversations', exact: true }).click();
  await expect(page.locator('.library-conversations textarea[aria-label="Chat message"]')).toBeVisible();
  await expect(composers).toHaveCount(1);
  await expect(page.locator('.work-conversation textarea')).toHaveCount(0);
  await back.click();
  await expect(page.locator('.work-conversation textarea[aria-label="Chat message"]')).toBeVisible();
  await expect(composers).toHaveCount(1);
  await expect(composers).toHaveValue('Same shared draft through Library history');
  // The old duplicate composer cleared the shared canvas draft registry on
  // disposal, leaving the visible workspace unable to receive canvas drafts.
  await expect.poll(() => page.evaluate(async () => {
    const path = '/src/components/ChatCanvas.tsx';
    const canvas = await import(/* @vite-ignore */ path);
    return typeof canvas.composerFill();
  })).toBe('function');
});

test('workspace composer drafts stay separate across switch and reload', async ({page}, info) => {
  test.skip(info.project.name.includes('narrow'), 'Workspace rows intentionally collapse on narrow screens');
  await createWorkspace(page,'Alpha');
  await page.getByLabel('Chat message').fill('Alpha private draft');
  await createWorkspace(page,'Beta');
  await expect(page.getByLabel('Chat message')).toHaveValue('');
  await page.getByLabel('Chat message').fill('Beta private draft');
  // A new project opens its main agent (10-30), and is in the rail, where its
  // row opens that lead (L8, 11-10); each draft is that conversation's own
  // (Sam's per-chat drafts, 10-27).
  const open = async (title: string) => { await page.getByRole('navigation',{name:'Project navigation'}).getByRole('button',{name:title,exact:true}).click(); await expect(page.locator('.work-header h1')).toHaveText(title); };
  await open('Alpha');
  await expect(page.getByLabel('Chat message')).toHaveValue('Alpha private draft');
  await page.reload();
  await expect(page.getByLabel('Chat message')).toHaveValue('Alpha private draft');
  await open('Beta');
  await expect(page.getByLabel('Chat message')).toHaveValue('Beta private draft');
});

test('primary shell omits retired tabs and avoids legacy data startup', async ({page}) => {
  await expect(page.getByRole('heading',{name:'Projects',exact:true})).toBeVisible();
  const nav=page.getByRole('complementary',{name:'Main navigation'});
  // The rail is there, so the absences below aren't vacuous.
  await expect(nav).toBeVisible();
  // Retired destinations: none is a page row. Projects is one again since
  // L1 (11-12), as Work's new name; the retired Projects page was another.
  const pages=page.getByRole('navigation',{name:'Main'});
  for (const name of ['Inbox','Approvals','Graph','Calendar','People','Chat','Work']) {
    await expect(pages.getByRole('button',{name,exact:true})).toHaveCount(0);
  }
  const calls=await page.evaluate(()=>(window as any).__mockApiCalls??{});
  for(const command of ['projects','tasks','vault_graph','list_agent_sessions','vault_tree','seed_routines']) expect(calls[command]??0,command).toBe(0);
  const overflow=await page.evaluate(()=>document.documentElement.scrollWidth>window.innerWidth);
  expect(overflow).toBe(false);
  await openTasks(page);
  await expect(page.getByRole('heading',{name:'Tasks',exact:true})).toBeVisible();
  // Tasks is a sidebar row again (overnight 11-04 S1), and it is the page on screen.
  await expect(nav.getByRole('button',{name:'Tasks',exact:true})).toHaveAttribute('aria-current','page');
  await page.getByRole('button',{name:'Library',exact:true}).click();
  await expect(page.getByRole('heading',{name:'Library',exact:true})).toBeVisible();
  // Library opens on All, one searchable list (overnight 11-12 L3).
  await expect(page.getByLabel('Search your Library')).toBeVisible();
});

test('library groups notes under folder headings that fold', async ({page}) => {
  await page.getByRole('button',{name:'Library',exact:true}).click();
  await openLibraryView(page, 'Notes');
  await choose(page.getByLabel('Library folder'), 'all');
  const projects=page.locator('.library-group-head button').filter({hasText:'Projects'});
  await expect(projects).toHaveAttribute('aria-expanded','true');
  await projects.click();
  await expect(projects).toHaveAttribute('aria-expanded','false');
});

test('reopen restores exact provider session and retained conversation', async ({page}) => {
  await createWorkspace(page,'Resume check');
  await page.evaluate(()=>{
    const w=JSON.parse(localStorage.getItem('mockWorkspaces')!)[0];
    const summary={key:'s-20260923-abcd0001',title:'Check the report',agent:'codex',cwd:w.cwd,harnessSessionId:'native-codex-thread',resumable:'native',createdAt:new Date().toISOString(),lastActivityAt:new Date().toISOString(),endedAt:null,turnCount:1,parentKey:null,forkKind:null,adopted:false,stints:[]};
    localStorage.setItem('mockWorkspaceSessions',JSON.stringify([summary]));
    localStorage.setItem('mockWorkspaceConversations',JSON.stringify({[summary.key]:{summary,eventsRetained:false,turns:[{id:'t1',role:'assistant',text:'The report is checked against the attached source.',tools:[],agent:'codex',runId:'run-1',startedAt:summary.createdAt,status:'success',receipt:null,transcriptIncomplete:false}]}}));
    // It was the conversation on screen: a new project opens its main agent
    // (10-30), which launch now reopens too (overnight 11-03 R4).
    localStorage.setItem('hollow.lastConversation',JSON.stringify({workspaceId:w.id,key:summary.key}));
  });
  await page.reload();
  await expect(page.getByRole('region',{name:'Workspace conversation'})).toContainText('The report is checked against the attached source.');
  await expect(page.getByRole('button',{name:/^Model:/})).toHaveAttribute('aria-label',/^Model: Codex/);
  await page.getByLabel('Chat message').fill('Continue the verified result.');
  await page.reload();
  await expect(page.getByLabel('Chat message')).toHaveValue('Continue the verified result.');
  await expect(page.getByRole('region',{name:'Workspace conversation'})).toContainText('The report is checked against the attached source.');
});
