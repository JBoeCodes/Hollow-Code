import { describe, expect, it } from 'vitest';
import type { SelfHostedEntry, SelfHostedHeldJob, SelfHostedJobRow, SelfHostedJobs, SelfHostedTest, SelfHostedWaiting } from './generated/ipc';
import { API_STYLE_OPTIONS, AS_NOW, CODEX_LEVELS, JOBS_CLAUDE_ONLY, JOBS_DAMAGED, TEST_RUNNING, apiStyleOf, draftFrom, emptyDraft, entryFrom, entrySummary, formatArgs, formatMs, harnessOf, heldDetail, heldLine, invalidText, jobOptions, levelsFor, parseArgs, removeBody, retryResult, testKey, testSummary, waitingDetail, waitingLine, windowText, withApiStyle } from './selfHostedModels';
import PANEL from './settings/SelfHostedModelsPanel.tsx?raw';
import SETTINGS_PAGE from './pages/Settings.tsx?raw';
import SETTINGS from './pages/Settings.tsx?raw';

// The preset as `self_hosted_registry::preset` returns it.
const QWEN: SelfHostedEntry = {
  id: 'qwen', name: 'Qwen3.8 27B', baseUrl: 'https://qwen-27b-x6jmsblvea-uc.a.run.app', apiStyle: 'anthropic',
  servedModel: 'qwen3.8-27b', contextWindow: 131072, effortLevels: ['low', 'medium', 'xhigh'],
  auth: { kind: 'tokenCommand', program: 'gcloud', args: ['auth', 'print-identity-token'] },
  healthPath: '/health', enabled: true,
};

function result(fields: Partial<SelfHostedTest> = {}): SelfHostedTest {
  return {
    ok: true, healthStatus: 200, healthMs: 272_000, modelsStatus: 200, modelListed: true, reportedWindow: 131072,
    windowMismatch: false, firstByteMs: 812, status: 200, error: null, ...fields,
  };
}

describe('self-hosted models form (Q4)', () => {
  it('fills the form from the preset and turns it back into the same entry', () => {
    const draft = draftFrom(QWEN);
    expect(draft.args).toBe('auth print-identity-token');
    expect(draft.contextWindow).toBe('131072');
    expect(entryFrom(draft)).toEqual({ entry: QWEN });
  });

  it('reads a window typed with separators and refuses one that is not a number', () => {
    expect(entryFrom({ ...draftFrom(QWEN), contextWindow: '131,072' })).toEqual({ entry: QWEN });
    expect(entryFrom({ ...draftFrom(QWEN), contextWindow: '128k' })).toEqual({ error: 'The context window must be a whole number of tokens.' });
    expect(entryFrom({ ...draftFrom(QWEN), contextWindow: '' })).toEqual({ error: 'The context window must be a whole number of tokens.' });
  });

  it('keeps arguments with spaces and quotes whole on the one line', () => {
    const args = ['-c', 'echo "tok en"', 'a\\b', ''];
    const line = formatArgs(args);
    expect(line).toBe('-c "echo \\"tok en\\"" "a\\\\b" ""');
    expect(parseArgs(line)).toEqual({ args });
    expect(parseArgs('  auth   print-identity-token ')).toEqual({ args: ['auth', 'print-identity-token'] });
    expect(parseArgs('')).toEqual({ args: [] });
    expect(parseArgs('"open')).toEqual({ error: 'A quote in the arguments is not closed.' });
  });

  it('refuses a window outside the list\'s limits before sending it (Q4 review)', () => {
    const says = "The context window must be between 110,000 and 10,000,000 tokens: Claude Code compacts at 100,000 tokens at the earliest, and a Hollow chat turn's first call is about 52,000.";
    expect(entryFrom({ ...draftFrom(QWEN), contextWindow: '109999' })).toEqual({ error: says });
    expect(entryFrom({ ...draftFrom(QWEN), contextWindow: '99999999999999999999' })).toEqual({ error: says });
    expect(entryFrom({ ...draftFrom(QWEN), contextWindow: '110,000' })).toEqual({ entry: { ...QWEN, contextWindow: 110000 } });
    // The list's own limits win.
    expect(entryFrom(draftFrom(QWEN), { min: 200_000, max: 400_000 })).toEqual({
      error: "The context window must be between 200,000 and 400,000 tokens: Claude Code compacts at 100,000 tokens at the earliest, and a Hollow chat turn's first call is about 52,000.",
    });
  });

  it('sends no token command for an open endpoint', () => {
    const open = { ...draftFrom(QWEN), authKind: 'none' as const, baseUrl: 'http://127.0.0.1:8000' };
    expect(entryFrom(open)).toEqual({ entry: { ...QWEN, baseUrl: 'http://127.0.0.1:8000', auth: { kind: 'none' } } });
    expect(entrySummary({ ...QWEN, auth: { kind: 'none' } })).toBe('qwen3.8-27b · 131,072 tokens · no sign-in · runs on Claude');
  });

  it('starts an empty form with the health path and every field blank', () => {
    const draft = emptyDraft();
    expect(draft.healthPath).toBe('/health');
    expect(draft.authKind).toBe('tokenCommand');
    expect(draft.apiStyle).toBe('anthropic');
    expect(draft.enabled).toBe(true);
    expect([draft.id, draft.name, draft.baseUrl, draft.servedModel, draft.contextWindow]).toEqual(['', '', '', '', '']);
  });
});

describe('self-hosted models API style (Q9)', () => {
  it('turns an OpenAI-style draft into an entry that runs on Codex', () => {
    const codex = { ...QWEN, id: 'qwen-codex', apiStyle: 'openai' };
    const draft = draftFrom(codex);
    expect(draft.apiStyle).toBe('openai');
    expect(entryFrom(draft)).toEqual({ entry: codex });
    expect(harnessOf('openai')).toBe('Codex');
    expect(harnessOf('anthropic')).toBe('Claude');
    expect(apiStyleOf('something else')).toBe('anthropic');
  });

  it('offers Codex\'s levels only for an OpenAI-style model, and drops max on switching', () => {
    const all = ['low', 'medium', 'high', 'xhigh', 'max'];
    expect(levelsFor('openai', all)).toEqual(CODEX_LEVELS);
    expect(levelsFor('anthropic', all)).toEqual(all);
    const draft = { ...draftFrom(QWEN), effortLevels: ['medium', 'max'] };
    const switched = withApiStyle(draft, 'openai');
    expect(switched.apiStyle).toBe('openai');
    expect(switched.effortLevels).toEqual(['medium']);
    expect(withApiStyle(switched, 'anthropic').effortLevels).toEqual(['medium']);
  });

  it('says Remove keeps an OpenAI-style model\'s Codex home and its conversations (Q9 review)', () => {
    expect(removeBody(QWEN)).toBe('Hollow forgets this endpoint and stops its local forwarder. The model and its server are not changed.');
    const codex = removeBody({ ...QWEN, id: 'qwen-codex', apiStyle: 'openai' });
    expect(codex).toContain("Its Codex home, with this model's Codex conversations, stays in Hollow's app data (self-hosted-codex/qwen-codex)");
    expect(PANEL).toContain('body: removeBody(entry)');
  });

  it('shows the style as a visible field with plain help, and no key of its own', () => {
    expect(API_STYLE_OPTIONS.map(o => o.value)).toEqual(['anthropic', 'openai']);
    expect(PANEL).toContain('label="API style"');
    expect(PANEL).toContain('OpenAI style runs it on Codex');
    expect(PANEL).toContain('withApiStyle(');
    for (const text of [PANEL, ...API_STYLE_OPTIONS.map(o => o.label)]) {
      expect(text).not.toMatch(/[\u2013\u2014]/);
    }
  });
});

describe('self-hosted models words (Q4)', () => {
  it('names the served model, window and sign-in', () => {
    expect(windowText(131072)).toBe('131,072 tokens');
    expect(entrySummary(QWEN)).toBe('qwen3.8-27b · 131,072 tokens · token from gcloud · runs on Claude');
    expect(entrySummary({ ...QWEN, apiStyle: 'openai' })).toBe('qwen3.8-27b · 131,072 tokens · token from gcloud · runs on Codex');
  });

  it('formats times from milliseconds to minutes', () => {
    expect(formatMs(812)).toBe('812 ms');
    expect(formatMs(4200)).toBe('4.2 s');
    expect(formatMs(272_000)).toBe('4 min 32 s');
    expect(formatMs(300_000)).toBe('5 min');
  });

  it('rounds before it splits, so no "60 s" ever shows (Q4 review)', () => {
    expect(formatMs(299_600)).toBe('5 min');
    expect(formatMs(59_960)).toBe('1 min');
    expect(formatMs(59_940)).toBe('59.9 s');
    expect(formatMs(119_500)).toBe('2 min');
    expect(formatMs(61_000)).toBe('1 min 1 s');
    expect(formatMs(999.6)).toBe('1.0 s');
    const odd: string[] = [];
    for (let ms = 0; ms < 400_000; ms++) {
      const text = formatMs(ms);
      if (/\b60(\.0)? s|1000 ms/.test(text)) odd.push(`${ms}: ${text}`);
    }
    expect(odd).toEqual([]);
  });

  it('says a passing Test with its health and first byte times', () => {
    expect(testSummary(result(), QWEN)).toEqual({
      state: 'ok', chip: 'Passed',
      lines: ['Health check 200 in 4 min 32 s.', 'First byte 200 in 812 ms.'],
    });
  });

  it('warns about a window the endpoint disagrees with, or a model it does not list', () => {
    const mismatch = testSummary(result({ windowMismatch: true, reportedWindow: 65536 }), QWEN);
    expect(mismatch.state).toBe('warn');
    expect(mismatch.lines).toContain('The endpoint reports a window of 65,536 tokens; this entry says 131,072 tokens.');
    const unlisted = testSummary(result({ modelListed: false, reportedWindow: null }), QWEN);
    expect(unlisted.state).toBe('warn');
    expect(unlisted.lines).toContain("The endpoint's model list doesn't name qwen3.8-27b.");
  });

  it('says a model list it could not read, not that it lacks the model (Q4 review)', () => {
    const missing = testSummary(result({ modelsStatus: 404, modelListed: null, reportedWindow: null }), QWEN);
    expect(missing.state).toBe('warn');
    expect(missing.lines).toContain("Couldn't read the model list (404).");
    expect(missing.lines.join(' ')).not.toContain("doesn't name");
    expect(testSummary(result({ modelsStatus: null, modelListed: null, reportedWindow: null }), QWEN).lines).toContain("Couldn't read the model list.");
    expect(testSummary(result({ modelsStatus: 200, modelListed: null, reportedWindow: null }), QWEN).lines).toContain("Couldn't read the model list: the answer isn't a list of models.");
  });

  it('keys a result to the endpoint it tested, not its switch (Q4 review)', () => {
    expect(testKey({ ...QWEN, enabled: false })).toBe(testKey(QWEN));
    expect(testKey({ ...QWEN, baseUrl: 'https://other.example.com' })).not.toBe(testKey(QWEN));
    expect(testKey({ ...QWEN, contextWindow: 200000 })).not.toBe(testKey(QWEN));
  });

  it('shows the error of a failing health check or completion', () => {
    const down = testSummary(result({ ok: false, healthStatus: 503, healthMs: 1200, firstByteMs: null, status: null, modelListed: false, reportedWindow: null, error: 'The health check (/health) answered 503.' }), QWEN);
    expect(down).toEqual({ state: 'error', chip: 'Failed', lines: ['Health check 503 in 1.2 s.', 'The health check (/health) answered 503.'] });
    const refused = testSummary(result({ ok: false, status: 404, error: 'The model answered 404: no model' }), QWEN);
    expect(refused.state).toBe('error');
    expect(refused.lines.at(-1)).toBe('The model answered 404: no model');
  });

  it('uses plain copy: no em dashes, and the wait it names while a Test runs', () => {
    expect(TEST_RUNNING).toBe('Starting the model, this can take about 5 minutes.');
    expect(PANEL).toContain('TEST_RUNNING');
    for (const text of [PANEL, TEST_RUNNING]) expect(text).not.toContain('—');
  });
});

describe('self-hosted models part (Q4)', () => {
  it('sits in Models, after the agent form and before Accounts, lazy, with every action a visible button', () => {
    const at = (needle: string) => SETTINGS.indexOf(needle);
    expect(at('id="selfhosted"')).toBeGreaterThan(at('id="models"'));
    expect(at('id="selfhosted"')).toBeLessThan(at('id="accounts"'));
    for (const label of ['Add a self-hosted model', 'Use the Qwen on Cloud Run preset', '>Test<', '>Edit<', '>Remove<', '>Save<', '>Cancel<', '>Check again<']) {
      expect(PANEL, label).toContain(label);
    }
    // Q4 review: each row's buttons are named for their model, and the
    // form's switch says what it does.
    for (const name of ['aria-label={`Test ${entry.name}`}', 'aria-label={`Edit ${entry.name}`}', 'aria-label={`Remove ${entry.name}`}', 'aria-label="Add a self-hosted model"', 'label="Use this model"']) {
      expect(PANEL, name).toContain(name);
    }
    // Every capability is a click; the part adds no key of its own.
    expect(PANEL).not.toMatch(/onKeyDown|addEventListener\('keydown'/);
  });

  it('says chats offer the models, and a change drops the cached Claude list (Q5)', () => {
    expect(SETTINGS).not.toContain("Chats don't offer them yet");
    expect(SETTINGS).toContain("Each one switched on is in a chat's model menu.");
    // After a save, and after a switch or a removal (both through `change`);
    // Q9: Codex's list too, which holds the OpenAI-style entries.
    expect(PANEL.match(/forgetCatalog\('claude'\)/g)).toHaveLength(2);
    expect(PANEL.match(/forgetCatalog\('codex'\)/g)).toHaveLength(2);
  });

  it('marks an entry a hand edit broke, with the reason chats give (Q5 delta review)', () => {
    expect(invalidText('This model id is one Claude Code uses for its own models; use the endpoint\'s own id. '))
      .toBe('Chats can\'t use it as saved: This model id is one Claude Code uses for its own models; use the endpoint\'s own id. Edit it to fix this.');
    expect(PANEL).toContain('v().invalid[entry.id]');
    expect(PANEL).toContain('invalidText(why())');
  });
});

describe('background jobs (Q10)', () => {
  const row = (fields: Partial<SelfHostedJobRow> = {}): SelfHostedJobRow => ({
    kind: 'extraction', label: 'Recording extraction', about: 'Promises found in a kept recording.', entry: null, problem: null, ...fields,
  });
  const waiting = (fields: Partial<SelfHostedWaiting> = {}): SelfHostedWaiting => ({
    entry: 'qwen', name: 'Qwen3.8 27B', count: 3, oldestAt: '2026-11-03T12:00:00Z', draining: false, lastError: null, ...fields,
  });
  const OFF: SelfHostedEntry = { ...QWEN, id: 'spare', name: 'Spare GPU', enabled: false };
  const OPENAI: SelfHostedEntry = { ...QWEN, id: 'oai', name: 'OpenAI style', apiStyle: 'openai' };

  it('offers As now and every switched-on model Hollow can run', () => {
    expect(AS_NOW).toBe('');
    expect(jobOptions(row(), [QWEN, OFF, OPENAI])).toEqual([
      { value: '', label: 'As now' },
      { value: 'qwen', label: 'Qwen3.8 27B' },
    ]);
    expect(jobOptions(row(), [])).toEqual([{ value: '', label: 'As now' }]);
  });

  it('keeps the saved model in the list when it cannot run now', () => {
    expect(jobOptions(row({ entry: 'spare' }), [QWEN, OFF])).toEqual([
      { value: '', label: 'As now' },
      { value: 'qwen', label: 'Qwen3.8 27B' },
      { value: 'spare', label: "Spare GPU (can't run now)" },
    ]);
    expect(jobOptions(row({ entry: 'gone' }), [QWEN]).at(-1)).toEqual({ value: 'gone', label: "gone (can't run now)" });
    // The saved one that can run is listed once.
    expect(jobOptions(row({ entry: 'qwen' }), [QWEN])).toHaveLength(2);
  });

  it('offers only Anthropic-style models, since jobs run on Claude, and shows a saved one that became OpenAI-style as held (Q9 with Q10)', () => {
    expect(jobOptions(row({ entry: 'oai' }), [QWEN, OPENAI])).toEqual([
      { value: '', label: 'As now' },
      { value: 'qwen', label: 'Qwen3.8 27B' },
      { value: 'oai', label: "OpenAI style (can't run now)" },
    ]);
    expect(JOBS_CLAUDE_ONLY).toBe("They run on Claude, so only models with Anthropic's Messages API are offered here; an OpenAI-style model runs on Codex, in chats only.");
    expect(PANEL).toContain('{JOBS_CLAUDE_ONLY}');
    expect(JOBS_CLAUDE_ONLY).not.toMatch(/[\u2013\u2014]/);
  });

  it('counts what waits for each model, and says what happens next', () => {
    expect(waitingLine(waiting())).toBe('3 jobs waiting for Qwen3.8 27B');
    expect(waitingLine(waiting({ count: 1, name: null }))).toBe('1 job waiting for qwen');
    const now = new Date('2026-11-03T12:12:30Z');
    expect(waitingDetail(waiting(), now)).toBe('The oldest has waited 12 min. Hollow wakes the model once one has waited 30 minutes, or right after the model answers a chat.');
    expect(waitingDetail(waiting({ oldestAt: '2026-11-03T12:12:00Z' }), now)).toMatch(/^The oldest came in just now\./);
    expect(waitingDetail(waiting({ lastError: "Qwen3.8 27B didn't wake: its health check answered 503" }), now))
      .toBe("The oldest has waited 12 min. The last try stopped: Qwen3.8 27B didn't wake: its health check answered 503");
    expect(waitingDetail(waiting({ draining: true }), now)).toBe(`Waking Qwen3.8 27B and running them. ${TEST_RUNNING}`);
  });

  it('says what a held job waits for, and names only the buttons its row shows (review round 2)', () => {
    const held = (fields: Partial<SelfHostedHeldJob> = {}): SelfHostedHeldJob => ({
      id: 'j1', kind: 'extraction', label: 'Recording extraction', entry: 'qwen', name: 'Qwen3.8 27B',
      queuedAt: '2026-11-03T12:00:00Z', attempts: 0, lastError: null, gaveUp: false, running: false,
      canRunAsNow: true, canTryAgain: false, onlyOn: null, ...fields,
    });
    const now = new Date('2026-11-03T12:40:00Z');
    expect(heldLine(held())).toBe('Recording extraction job held');
    expect(heldDetail(held(), now)).toBe('Queued 40 min ago for Qwen3.8 27B, and its kind runs As now since. Run as now sends it there, or Discard drops it.');
    // Gave up while its kind still goes to the model: Try again, never "Run them now" (that row is gone).
    const failed = held({ gaveUp: true, attempts: 3, lastError: 'the model answered 500', canRunAsNow: false, canTryAgain: true });
    expect(heldLine(failed)).toBe('Recording extraction job gave up after 3 tries');
    expect(heldDetail(failed, now)).toBe('Queued 40 min ago for Qwen3.8 27B. It failed 3 times: Try again runs it on the model once more (after a run of it already under way), or Discard drops it. Last error: the model answered 500');
    expect(heldDetail(failed, now)).not.toContain('Run them now');
    // Gave up and its kind back to As now: Run as now.
    expect(heldDetail(held({ gaveUp: true, attempts: 3 }), now)).toMatch(/Run as now sends it there, or Discard drops it\.$/);
    // A self-hosted chat's own job never runs as now.
    const own = held({ kind: 'titles', label: 'Chat and session titles', canRunAsNow: false, onlyOn: 'qwen' });
    expect(heldDetail(own, now)).toBe('Queued 40 min ago for Qwen3.8 27B. It belongs to a chat on that model, so it runs only there: send its kind to that model again, or Discard drops it.');
    expect(heldDetail(held({ canRunAsNow: false }), now)).toBe('Queued 40 min ago for Qwen3.8 27B. Discard drops it.');
    expect(heldDetail(held({ running: true, name: null }), now)).toBe('Queued 40 min ago for qwen. Hollow is running it now.');
    expect(JOBS_DAMAGED).toBe('Background jobs are held until this is fixed or the file is moved aside:');
  });

  it('says whether Try again started a run now or the next run takes the job (review round 4)', () => {
    const jobs: SelfHostedJobs = { kinds: [], waiting: [], held: [], error: null, queueError: null };
    const qwen = { entry: 'qwen', name: 'Qwen3.8 27B', jobs };
    expect(retryResult({ ...qwen, started: true, underWay: false })).toBe('✓ Waking Qwen3.8 27B to run it again');
    expect(retryResult({ ...qwen, started: false, underWay: true })).toBe('✓ Back in line: a run of Qwen3.8 27B is under way, and the next one runs it');
    // Q10 follow-up: a drain lock Hollow couldn't probe says nothing is under way.
    const unknown = retryResult({ ...qwen, started: false, underWay: false });
    expect(unknown).toBe('✓ Back in line: the next run of Qwen3.8 27B runs it');
    expect(unknown).not.toContain('under way');
    // The model the job waits for now, not the one it gave up on; its id when it left the list.
    expect(retryResult({ entry: 'llama', name: 'Llama 4', jobs, started: true, underWay: false })).toBe('✓ Waking Llama 4 to run it again');
    expect(retryResult({ entry: 'llama', name: null, jobs, started: false, underWay: true })).toContain('a run of llama is under way');
    expect(PANEL).toContain('const retry = await api.retrySelfHostedJob(h.id);');
    expect(PANEL).toContain('flashFooter(retryResult(retry), 5000);');
    expect(PANEL).not.toContain('retryResult(retry, h.name');
  });

  it('shows the block outside the list, so a list that fails to load hides nothing (review)', () => {
    // S5b: Background jobs lives in Advanced (Hollow's own model calls), mounted as `part="jobs"`: it renders
    // alone, before and apart from the list, so a list that fails to load can hide nothing of it.
    expect(PANEL).toContain('<BackgroundJobs entries={view()?.entries ?? []} />');
    const alone = PANEL.indexOf("if (!shows('list')) return");
    const list = PANEL.indexOf('<Show when={view()} fallback=');
    expect(alone).toBeGreaterThan(0);
    expect(PANEL.indexOf('<BackgroundJobs entries', alone)).toBeLessThan(list);
    // The whole panel (no `part`) still shows it after the list's own Show closes.
    const close = PANEL.indexOf('</Show>', PANEL.indexOf('<Form />'));
    expect(PANEL.lastIndexOf('<BackgroundJobs entries')).toBeGreaterThan(close);
    for (const text of ['>Run as now<', '>Try again<', '>Discard<', 'aria-label={`Run the held ${h.label} job as now`}', 'aria-label={`Try the ${h.label} job again`}', 'aria-label={`Discard the held ${h.label} job`}', '<Show when={h.canRunAsNow}>', '<Show when={h.canTryAgain}>', 'api.retrySelfHostedJob(h.id);']) {
      expect(PANEL, text).toContain(text);
    }
    expect(PANEL).not.toContain('<Show when={!h.gaveUp}>');
    expect(PANEL).not.toContain('Every kind runs as now until this is fixed');
  });

  it('has a select per kind and Run them now, all clicks, and sits in Advanced, Hollow\'s own model calls', () => {
    expect(SETTINGS_PAGE).toMatch(/<SettingsGroup id="model-calls"[\s\S]*<SelfHostedModelsPanel part="jobs" \/>/);
    expect(SETTINGS_PAGE).toMatch(/<SelfHostedModelsPanel part="list" \/>/);
    for (const text of ['>Background jobs<', '>Run them now<', 'label={`${row.label} model`}', 'aria-label={`Run the jobs waiting for ${w.name ?? w.entry} now`}', "onNative('self-hosted-jobs-changed'"]) {
      expect(PANEL, text).toContain(text);
    }
    for (const text of [PANEL]) expect(text).not.toContain('—');
  });
});
