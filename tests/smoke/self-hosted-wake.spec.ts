import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

/**
 * Self-hosted models Q6 (2026-11-03): a chat on `self-hosted:qwen` shows the
 * start chip above its composer, and only typing, a send or Start now wakes
 * the model (`wake_self_hosted_model`, counted by the mock). Opening the
 * chat, focusing or scrolling, a restored draft, switching chats and typing
 * a `/` command send nothing. The mock's wake takes 1.5 s; a held answer
 * shows "Checking" and then the start with its time so far.
 */
const QWEN = {
  id: 'qwen', name: 'Qwen3.8 27B', baseUrl: 'https://qwen-27b-x6jmsblvea-uc.a.run.app', apiStyle: 'anthropic',
  servedModel: 'qwen3.8-27b', contextWindow: 131072, effortLevels: ['low', 'medium', 'xhigh'],
  auth: { kind: 'tokenCommand', program: 'gcloud', args: ['auth', 'print-identity-token'] },
  healthPath: '/health', enabled: true,
};

const wakes = (page: Page) => page.evaluate(() => window.__mockApiCalls?.wake_self_hosted_model ?? 0);

async function setup(page: Page) {
  await page.addInitScript(entry => {
    localStorage.setItem('chatAgent', 'claude');
    localStorage.setItem('chatModel', 'self-hosted:qwen');
    localStorage.setItem('mockAgentSettings', JSON.stringify({ defaultAgent: 'claude', models: { claude: 'self-hosted:qwen' } }));
    localStorage.setItem('mockSelfHosted', JSON.stringify([entry]));
  }, QWEN);
  await page.goto('/?mock=1');
}

async function newProject(page: Page, name: string) {
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill(`Work on ${name}.`);
  await page.getByRole('textbox', { name: 'Project name' }).fill(name);
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText(name);
}

test('opening, focusing, a restored draft and switching chats wake nothing; typing wakes once', async ({ page }) => {
  await setup(page);
  await newProject(page, 'Qwen first');
  const chip = page.locator('.self-hosted-status');
  await expect(chip).toHaveAttribute('data-state', 'idle');
  await expect(chip.getByRole('status')).toHaveText('Qwen3.8 27B sleeps when idle');
  await expect(chip.getByRole('button', { name: 'Start now', exact: true })).toBeEnabled();

  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.focus();
  await composer.click();
  await page.mouse.wheel(0, 400);
  // A restored draft lands in the box without a keystroke.
  await page.evaluate(async () => {
    const path = '/src/store.ts';
    const store = await import(/* @vite-ignore */ path);
    store.setChatInput('A draft from before');
  });
  await expect(composer).toHaveValue('A draft from before');
  // Another chat on the same model shows the same idle chip.
  await newProject(page, 'Qwen second');
  await expect(chip).toHaveAttribute('data-state', 'idle');
  // A / command is Hollow's own until it is sent.
  await composer.fill('/model');
  expect(await wakes(page)).toBe(0);

  // Hold the wake's answer, so the start shows.
  await page.evaluate(() => {
    let release: () => void = () => {};
    window.__mockHoldOnce = { command: 'wake_self_hosted_model', release: new Promise<void>(done => { release = done; }) };
    (window as unknown as { __releaseWake: () => void }).__releaseWake = () => release();
  });
  await composer.fill('');
  await composer.pressSequentially('Hello');
  await expect.poll(() => wakes(page)).toBe(1);
  await expect(chip).toHaveAttribute('data-state', /checking|starting/);
  await expect(chip).toHaveAttribute('data-state', 'starting', { timeout: 10_000 });
  await expect(chip.getByRole('status')).toHaveText('Qwen3.8 27B is starting, about 4 minutes');
  await expect(chip.locator('.self-hosted-status-elapsed')).toContainText('so far');
  await expect(chip.getByRole('button', { name: 'Start now', exact: true })).toHaveCount(0);
  // More keystrokes join the start.
  await composer.pressSequentially(' there');
  expect(await wakes(page)).toBe(1);

  await page.evaluate(() => (window as unknown as { __releaseWake: () => void }).__releaseWake());
  await expect(chip).toHaveAttribute('data-state', 'ready');
  await expect(chip.getByRole('status')).toHaveText('Qwen3.8 27B is ready');
  expect(await wakes(page)).toBe(1);
});

test('Start now: a refusal in plain words, then a start that ends Ready', async ({ page }) => {
  await setup(page);
  await newProject(page, 'Qwen start');
  const chip = page.locator('.self-hosted-status');
  await expect(chip.getByRole('status')).toHaveText('Qwen3.8 27B sleeps when idle');
  await page.evaluate(() => { window.__mockFailCommandOnce = 'wake_self_hosted_model'; });
  await chip.getByRole('button', { name: 'Start now', exact: true }).click();
  await expect(chip).toHaveAttribute('data-state', 'error');
  await expect(chip.getByRole('status')).toHaveText("Qwen3.8 27B didn't start. mock wake_self_hosted_model failure");
  // A refusal woke nothing, so Start now works at once.
  const startNow = chip.getByRole('button', { name: 'Start now', exact: true });
  await expect(startNow).toBeEnabled();
  await startNow.click();
  await expect(chip).toHaveAttribute('data-state', 'ready');
  await expect(chip.locator('.self-hosted-status-note')).toContainText('Answered in');
  expect(await wakes(page)).toBe(2);
});
