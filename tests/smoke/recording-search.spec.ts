import { expect, test } from './fixtures/test';
import { openLibraryView } from './libraryView';

test('transcript search marks literal text and speakers, counts segments and clears locally', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.top-bar')).toBeVisible();
  await page.evaluate(async () => {
    const path = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ path);
    const read = api.readRecording;
    (window as any).__searchCalls = [];
    api.readRecording = async (id: string) => {
      (window as any).__searchCalls.push('read');
      return { ...await read(id), transcript: [
        { start: '00:01', end: '00:02', speaker: 'Plan team', text: 'PLAN and plan. Use [ and .* literally.' },
        { start: '00:03', end: '00:04', speaker: 'Dana', text: '<img src=x onerror=alert(1)> No matching word.' },
        { start: '00:05', end: '00:06', speaker: null, text: 'Keep this source unchanged.' },
      ] };
    };
    for (const method of ['recordingAudioUrl', 'createTask', 'keepRecordingTranscript']) {
      api[method] = async () => { (window as any).__searchCalls.push(method); throw new Error('Unexpected search side effect'); };
    }
  });
  const library = page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true });
  if (!await library.isVisible()) await page.getByRole('button', { name: 'Show sidebar', exact: true }).click();
  await library.click();
  await openLibraryView(page, 'Recordings');
  const transcript = page.getByRole('region', { name: 'Transcript', exact: true });
  const input = transcript.getByRole('searchbox', { name: 'Find in transcript' });
  await expect(transcript.locator('.recording-transcript > li')).toHaveCount(3);
  const original = await transcript.locator('.recording-transcript').innerText();
  for (const [query, marks] of [['pLaN', ['Plan', 'PLAN', 'plan']], ['Dana', ['Dana']], ['[', ['[']], ['.*', ['.*']]] as const) {
    await input.fill(query);
    await expect(transcript.getByRole('status')).toHaveText('1 matching segment of 3');
    await expect(transcript.locator('mark')).toHaveText([...marks]);
  }
  await input.fill('absent');
  await expect(transcript.getByRole('status')).toHaveText('0 matching segments of 3');
  await expect(transcript).toContainText('No line matches.');
  await transcript.getByRole('button', { name: 'Clear search', exact: true }).click();
  await expect(input).toBeFocused();
  await expect(input).toHaveValue('');
  expect(await transcript.locator('.recording-transcript').innerText()).toBe(original);
  await expect(transcript.locator('img')).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__searchCalls)).toEqual(['read']);
});
