import { describe, expect, it, vi } from 'vitest';
import { BRIEF_COPIED, CONTEXT_AUDIT_TEMPLATE, GUIDE_URL, SETUP_CONNECT, SETUP_CONNECT_FILLED, SETUP_COPIED, SETUP_NOTE, SETUP_OWNER_HINT, SETUP_OWNER_PLACEHOLDER, SETUP_REPO_DEFAULT, SETUP_RUN, SETUP_THEN, SETUP_VISIBILITY, githubLogin, prefillRemote, repoNameProblem, setupCard, accessPill, macConnectProblem, macConnectRequest, macContextLead, macStepPill, macTickedAtFirst, blockedByRemovals, canGoLocalOnly, checkButton, checkButtonName, checkPill, homePill, leakModeLine, nextLoaded, orgAccessKey, orgPill, personalPill, projectsLine, remoteProblem, seedHasChanges, seedLines, slugProblem, vaultPill } from './contextLayers';
import type { ContextCheckRepo, ContextLayersStatus, MacContextOffer, ContextRootStatus, ContextSyncStatus } from './contextLayersApi';
import { mockContextLayersCommand, resetMockContextLayers } from './mockContextLayers';

const sync = (over: Partial<ContextSyncStatus> = {}): ContextSyncStatus => ({
  root: 'personal', state: 'idle', enabled: true, remote: 'git@github.com:me/personal-context.git', pendingCommits: 0, unsyncedFiles: 0,
  text: 'Up to date', lastSyncAt: null, lastError: null, blockedBy: null, clashCopies: [], ...over,
});
const root = (over: Partial<ContextRootStatus> = {}): ContextRootStatus => ({
  layer: 'org', path: '/Users/me/Hollow/Context/org', configured: true, exists: true, gitClone: true, lastCommitAt: null, problem: null, ...over,
});

describe('Settings, Context (CX17)', () => {
  it('words the personal pill from the sync status, Local only without a remote', () => {
    expect(personalPill(undefined).text).toBe('Checking…');
    expect(personalPill(sync({ remote: null, text: 'Not set up', state: 'off', enabled: false }))).toEqual({ text: 'Not set up' });
    expect(personalPill(sync({ remote: null, text: 'Sync is off', state: 'off', enabled: false }))).toEqual({ text: 'Local only, not backed up', state: 'warn' });
    expect(personalPill(sync())).toEqual({ text: 'Up to date', state: 'ok' });
    expect(personalPill(sync({ pendingCommits: 2, text: '2 changes not synced' })).state).toBe('warn');
    expect(personalPill(sync({ state: 'blocked', text: 'Not synced: me/x.md' })).state).toBe('error');
    expect(personalPill(sync({ state: 'off', enabled: false, text: 'Sync is off' })).state).toBe('warn');
    expect(personalPill(sync({ state: 'syncing', text: 'Syncing' }))).toEqual({ text: 'Syncing' });
  });

  it('never accepts a remote with credentials in it', () => {
    expect(remoteProblem('')).toMatch(/Paste/);
    expect(remoteProblem('https://ghp_token@github.com/me/repo.git')).toMatch(/sign-in/);
    expect(remoteProblem('https://me:pw@github.com/me/repo.git')).toMatch(/sign-in/);
    expect(remoteProblem('ssh://me:pw@github.com/me/repo.git')).toMatch(/password/);
    expect(remoteProblem('http://github.com/me/repo.git')).toMatch(/https/);
    expect(remoteProblem('ext::sh -c x')).not.toBeNull();
    expect(remoteProblem('-u x')).not.toBeNull();
    expect(remoteProblem('https://')).toMatch(/no host/);
    for (const ok of ['https://github.com/me/repo.git', 'ssh://git@github.com/me/repo.git', 'git@github.com:me/repo.git', '/Users/me/repo.git', 'file:///tmp/r.git']) {
      expect(remoteProblem(ok), ok).toBeNull();
    }
    expect(remoteProblem('github.com/me/repo')).not.toBeNull();
  });

  it('checks a machine name by the backend rule', () => {
    expect(slugProblem('')).not.toBeNull();
    expect(slugProblem('my-macbook')).toBeNull();
    expect(slugProblem('My Mac')).not.toBeNull();
    expect(slugProblem('-mac')).not.toBeNull();
    expect(slugProblem('mac--mini')).not.toBeNull();
    expect(slugProblem('a'.repeat(41))).not.toBeNull();
  });

  it('words the org clone and its access check', () => {
    expect(orgPill(root({ configured: false }))).toEqual({ text: 'Not connected' });
    expect(orgPill(root({ problem: 'a symlink' })).state).toBe('error');
    expect(orgPill(root({ gitClone: false })).state).toBe('warn');
    expect(orgPill(root())).toEqual({ text: 'Read only', state: 'ok' });
    expect(accessPill({ state: 'ok', text: '' }).state).toBe('ok');
    expect(accessPill({ state: 'denied', text: '' }).text).toBe('No access');
    expect(accessPill({ state: 'error', text: '' }).state).toBe('error');
  });

  it('words home, projects and the vault', () => {
    const home: ContextLayersStatus['home'] = { slug: null, suggestedSlug: 'mac', file: null, fileExists: false, root: root({ layer: 'personal' }) };
    expect(homePill(home).text).toBe('Name this Mac');
    expect(homePill({ ...home, slug: 'mac', root: root({ layer: 'personal', configured: false }) }).state).toBe('warn');
    expect(homePill({ ...home, slug: 'mac', fileExists: true })).toEqual({ text: 'Home file found', state: 'ok' });
    expect(projectsLine({ withContext: [{ title: 'a', path: '/a', files: ['CLAUDE.md'], git: true }], total: 3, capped: false, error: null }))
      .toBe('1 of 3 project folders carry a CLAUDE.md or AGENTS.md.');
    expect(projectsLine({ withContext: [], total: 0, capped: false, error: null })).toBe('No project folders yet.');
    expect(projectsLine({ withContext: [], total: 90, capped: true, error: null })).toContain('first 60 of 90');
    expect(vaultPill({ setUp: false, enabled: false, remote: null, text: 'Not set up' })).toEqual({ text: 'Not set up' });
    expect(vaultPill({ setUp: true, enabled: true, remote: 'x', text: 'Remote saved; the first push is held' }).state).toBe('warn');
  });

  it('keeps the last rows when a refresh fails, and shows the error', () => {
    const words = (e: unknown) => String(e);
    const first = nextLoaded({ value: undefined, error: '' }, { ok: true, value: 1 }, words);
    expect(first).toEqual({ value: 1, error: '' });
    const failed = nextLoaded(first, { ok: false, error: 'offline' }, words);
    expect(failed).toEqual({ value: 1, error: 'offline' });
    expect(nextLoaded(failed, { ok: true, value: 2 }, words)).toEqual({ value: 2, error: '' });
    expect(nextLoaded({ value: undefined, error: '' }, { ok: false, error: 'x' }, words).value).toBeUndefined();
  });

  it('waits with Local only while a sync runs, and offers removals only when they blocked it', () => {
    expect(canGoLocalOnly(sync())).toBe(true);
    expect(canGoLocalOnly(sync({ state: 'syncing' }))).toBe(false);
    expect(canGoLocalOnly(sync({ remote: null }))).toBe(false);
    expect(blockedByRemovals(sync({ state: 'blocked', blockedBy: '25 of 30 synced files are gone from the vault; if it was meant, use Sync now and allow removals (one sync only)' }))).toBe(true);
    expect(blockedByRemovals(sync({ state: 'blocked', blockedBy: 'me/x.md: a secret' }))).toBe(false);
    expect(blockedByRemovals(sync({ text: 'allow removals' }))).toBe(false);
  });

  it('clears an access result when the org folder or its clone state changes', () => {
    const status = (over: Partial<ContextRootStatus>) => ({ org: { root: root(over) } }) as unknown as ContextLayersStatus;
    expect(orgAccessKey(status({}))).toBe(orgAccessKey(status({})));
    expect(orgAccessKey(status({}))).not.toBe(orgAccessKey(status({ path: '/other' })));
    expect(orgAccessKey(status({}))).not.toBe(orgAccessKey(status({ gitClone: false })));
    expect(orgAccessKey(undefined)).toBe('');
  });

  it('offers Install context check or Remove per repo, never over a hook of its own (CX10)', () => {
    const repo = (state: ContextCheckRepo['state']): ContextCheckRepo => ({ title: 'p', path: '/p', layer: 'project', state, problem: null });
    expect(checkButton(repo('not_installed'))).toBe('Install context check');
    expect(checkButton(repo('installed'))).toBe('Remove');
    expect(checkButton(repo('other_hook'))).toBeNull();
    expect(checkButton(repo('unavailable'))).toBeNull();
    expect(checkButtonName('Install context check', 'hollow')).toBe('Install context check for hollow');
    expect(checkButtonName('Remove', 'hollow')).toBe('Remove context check for hollow');
    expect(checkPill(repo('installed')).state).toBe('ok');
    expect(checkPill(repo('other_hook')).state).toBe('warn');
    expect(leakModeLine(undefined)).toBe('Checking…');
    expect(leakModeLine(undefined, 'boom')).toBe('Couldn’t read the context checks: boom');
    expect(leakModeLine({ mode: 'flag', repos: [] }, 'boom')).toMatch(/^Couldn’t read/);
    expect(leakModeLine({ mode: 'flag', repos: [] })).toMatch(/flag only.*secret is always refused/);
    expect(leakModeLine({ mode: 'block', repos: [] })).toMatch(/refused/);
  });

  it('the mock installs and removes a check the way the backend answers (CX10)', () => {
    vi.stubGlobal('localStorage', { getItem: () => null });
    resetMockContextLayers();
    const status = mockContextLayersCommand('context_check_status', {}) as { mode: string; repos: ContextCheckRepo[] };
    expect(status.mode).toBe('flag');
    const path = status.repos[0].path;
    expect((mockContextLayersCommand('context_install_check', { path, apply: true }) as { text: string }).text).toMatch(/Installed/);
    expect((mockContextLayersCommand('context_check_status', {}) as { repos: ContextCheckRepo[] }).repos[0].state).toBe('installed');
    mockContextLayersCommand('context_remove_check', { path, apply: true });
    expect((mockContextLayersCommand('context_check_status', {}) as { repos: ContextCheckRepo[] }).repos[0].state).toBe('not_installed');
    expect(() => mockContextLayersCommand('context_install_check', { path: '/elsewhere' })).toThrow(/Nothing changed/);
    vi.unstubAllGlobals();
  });

  it('ships the Monthly context audit as a template that runs the audit and reports in Hollow (CX10)', () => {
    expect(CONTEXT_AUDIT_TEMPLATE.label).toBe('Monthly context audit');
    expect(CONTEXT_AUDIT_TEMPLATE.schedule).toBe('0 0 9 1 * * *');
    for (const word of ['context.audit (no arguments)', 'context.leak_flags with days 31', 'outputs/', 'notifications.create_notification', 'title "Monthly context audit"', 'body ', 'kind "info"', 'rule id', 'skipped', 'do not create a tracker issue']) {
      expect(CONTEXT_AUDIT_TEMPLATE.prompt).toContain(word);
    }
    expect(CONTEXT_AUDIT_TEMPLATE.prompt).not.toMatch(/not only in a tracker/);
    expect(CONTEXT_AUDIT_TEMPLATE.prompt).toMatch(/Never copy a matched value/);
  });

  it('has no dash used as punctuation in its words', async () => {
    const words = await import('./contextLayers.ts?raw');
    expect(String(words.default)).not.toMatch(/[–—]/);
  });
});

describe('Set up without Hollow (CX22)', () => {
  it('links the guide in the org repo and says what the copied brief is for', () => {
    expect(GUIDE_URL).toBe('https://github.com/example-org/org-context/blob/main/GUIDE.md');
    expect(BRIEF_COPIED).toMatch(/Follow this brief and set up my context/);
  });
  it('provides a self-contained portable setup brief', async () => {
    const brief = (await import('./contextSetupBrief')).SETUP_BRIEF;
    expect(brief).toMatch(/^## Appendix: templates$/m);
    expect(brief).toContain('gh repo create personal-context --private --add-readme');
  });
});

describe('Connect this Mac’s context (F3)', () => {
  const offer = (over: Partial<MacContextOffer> = {}): MacContextOffer => ({
    pair: 'm-1+runner:mini', otherName: 'mini', otherKind: 'runner', open: true, slug: null, suggestedSlug: 'studio',
    layers: [
      { layer: 'personal', state: 'missing', text: '', target: '/Users/me/Hollow/Context/personal', url: 'git@github.com:me/personal-context.git' },
      { layer: 'org', state: 'missing', text: '', target: '/Users/me/Hollow/Context/org', url: null },
      { layer: 'home', state: 'missing', text: '', target: null, url: null },
      { layer: 'vault', state: 'held', text: '', target: null, url: null },
    ],
    ...over,
  });

  it('ticks every missing row and never a held or connected one', () => {
    expect(macTickedAtFirst(offer())).toEqual(['personal', 'org', 'home']);
    const req = macConnectRequest(offer(), ['personal', 'vault'], { personal: '', org: '' }, '');
    expect(req.layers).toEqual(['personal']);
    expect(req.personalUrl).toBe('git@github.com:me/personal-context.git');
    expect(req.orgUrl).toBeNull();
    expect(req.pair).toBe('m-1+runner:mini');
  });

  it('asks for an address, refuses credentials and checks the name', () => {
    const o = offer();
    expect(macConnectProblem(o, macConnectRequest(o, [], { personal: '', org: '' }, ''))).toMatch(/Tick/);
    expect(macConnectProblem(o, macConnectRequest(o, ['org'], { personal: '', org: '' }, ''))).toMatch(/organization repo/);
    expect(macConnectProblem(o, macConnectRequest(o, ['org'], { personal: '', org: 'https://tok@github.com/o/r.git' }, ''))).toMatch(/sign-in/);
    expect(macConnectProblem(o, macConnectRequest(o, ['org', 'home'], { personal: '', org: 'git@github.com:o/r.git' }, ''))).toBeNull();
    expect(macConnectProblem(o, macConnectRequest(o, ['home'], { personal: '', org: '' }, 'Bad Name'))).toMatch(/Lowercase/);
    expect(macConnectProblem(offer({ suggestedSlug: null }), macConnectRequest(o, ['home'], { personal: '', org: '' }, ''))).toMatch(/name/);
  });

  it('leads with the other Mac, and words each step', () => {
    expect(macContextLead(offer())).toContain('runner mini');
    expect(macContextLead(offer({ otherName: null, otherKind: null }))).toMatch(/^Clone/);
    expect(macStepPill('skipped')).toEqual({ text: 'Already here', state: 'ok' });
    expect(macStepPill('refused').state).toBe('error');
  });

  it('the browser mock offers the card once, and Connect fills the rows', () => {
    vi.stubGlobal('localStorage', { getItem: (k: string) => (k === 'mockMacContext' ? 'offer' : null) });
    resetMockContextLayers();
    try {
      const open = mockContextLayersCommand('mac_context_offer', { any: false }) as MacContextOffer;
      expect(open.open).toBe(true);
      const req = macConnectRequest(open, macTickedAtFirst(open), { personal: 'git@h:me/p.git', org: 'git@h:o/c.git' }, 'studio');
      const report = mockContextLayersCommand('mac_context_connect', { request: req }) as { connected: boolean };
      expect(report.connected).toBe(true);
      expect(mockContextLayersCommand('mac_context_offer', { any: false })).toBeNull();
      const later = mockContextLayersCommand('mac_context_offer', { any: true }) as MacContextOffer;
      expect(later.layers.filter(r => r.state === 'missing')).toEqual([]);
    } finally {
      vi.unstubAllGlobals();
      resetMockContextLayers();
    }
  });
});

describe('the first sync dry run (PX1)', () => {
  const folder = (over: Partial<import('./contextLayersApi').ContextSeedFolder>) => ({ folder: 'learnings', onlyVault: [], onlyClone: [], differ: [], same: 0, capped: false, ...over });
  it('words each folder and kind of change, in order, and says nothing for matching folders', () => {
    const lines = seedLines([
      folder({ folder: 'me', same: 5 }),
      folder({ folder: 'memory', onlyClone: ['a.md', 'b.md'] }),
      folder({ folder: 'learnings', onlyVault: ['x.md'], differ: ['y.md'] }),
    ]);
    expect(lines).toEqual([
      { folder: 'memory/', what: '2 only in the repo: the first sync writes it into the vault', names: 'a.md, b.md' },
      { folder: 'learnings/', what: '1 only in the vault: the first sync sends it to the repo', names: 'x.md' },
      { folder: 'learnings/', what: '1 different in each: the vault’s stays and the repo’s is saved beside it (a clash card)', names: 'y.md' },
    ]);
  });
  it('cuts a long name list with the count left', () => {
    const names = Array.from({ length: 9 }, (_, i) => `n${i}.md`);
    expect(seedLines([folder({ onlyVault: names })])[0].names).toBe('n0.md, n1.md, n2.md, n3.md, n4.md, n5.md and 3 more');
  });
  it('has changes only when a file differs between the sides', () => {
    expect(seedHasChanges([folder({ same: 3 })])).toBe(false);
    expect(seedHasChanges([folder({ differ: ['a.md'] })])).toBe(true);
    expect(seedHasChanges([])).toBe(false);
  });
});

describe('Set it up for me (CX18): a card only', () => {
  it('checks the repo name: letters, digits, dot, dash, underscore', () => {
    expect(repoNameProblem(SETUP_REPO_DEFAULT)).toBeNull();
    expect(repoNameProblem('  my.context_v2-x  ')).toBeNull();
    expect(repoNameProblem('')).toMatch(/Type a name/);
    expect(repoNameProblem('my context')).toMatch(/Letters, digits/);
    expect(repoNameProblem('a/b')).toMatch(/Letters, digits/);
    expect(repoNameProblem('ctx;rm')).toMatch(/Letters, digits/);
    expect(repoNameProblem('naïve')).toMatch(/Letters, digits/);
    expect(repoNameProblem('..')).toMatch(/only dots/);
    expect(repoNameProblem('-x')).toMatch(/Start with/);
    expect(repoNameProblem('a'.repeat(101))).toMatch(/100/);
    expect(repoNameProblem('a'.repeat(100))).toBeNull();
  });

  it('takes only a real GitHub login as the owner', () => {
    expect(githubLogin('octo-cat')).toBe('octo-cat');
    expect(githubLogin(' example-user ')).toBe('example-user');
    for (const bad of [null, undefined, '', '-x', 'x-', 'a--b', 'a b', 'a/b', '<your-github-login>', 'a'.repeat(40)]) expect(githubLogin(bad)).toBeNull();
  });

  it('without a known login: a command gh runs as signed in, the placeholder address, no prefill', () => {
    const card = setupCard(SETUP_REPO_DEFAULT, null);
    expect(card).toEqual({
      name: 'personal-context', problem: null, command: 'gh repo create personal-context --private',
      url: `git@github.com:${SETUP_OWNER_PLACEHOLDER}/personal-context.git`, prefill: null,
    });
    // A placeholder in the command would be a shell redirect: it never goes there.
    expect(card.command).not.toContain('<');
    expect(setupCard('ctx', 'not a login').prefill).toBeNull();
  });

  it('with a login: the owner in the command and the address, which is the prefill', () => {
    const card = setupCard(' my-ctx ', 'octo');
    expect(card.command).toBe('gh repo create octo/my-ctx --private');
    expect(card.url).toBe('git@github.com:octo/my-ctx.git');
    expect(card.prefill).toBe(card.url);
    expect(remoteProblem(card.url!)).toBeNull();
  });

  it('is always private and never creates anything beyond the one command', () => {
    for (const owner of [null, 'octo']) {
      const command = setupCard('personal-context', owner).command!;
      expect(command).toMatch(/ --private$/);
      expect(command).not.toMatch(/--public|--internal|--clone|--push|--source/);
    }
    expect(SETUP_VISIBILITY).toMatch(/^Private/);
    expect(SETUP_NOTE).toBe('Hollow doesn’t create the repo; you run this once.');
  });

  it('shows no command or address while the name has a problem', () => {
    expect(setupCard('bad name', 'octo')).toEqual({ name: 'bad name', problem: 'Letters, digits, dots, dashes and underscores only.', command: null, url: null, prefill: null });
  });

  it('fills the Connect field only while it is empty or holds its own last address', () => {
    const a = 'git@github.com:octo/a.git';
    const b = 'git@github.com:octo/b.git';
    expect(prefillRemote('', null, a)).toBe(a);
    expect(prefillRemote(a, a, b)).toBe(b);
    expect(prefillRemote('git@github.com:me/mine.git', a, b)).toBe('git@github.com:me/mine.git');
    expect(prefillRemote(a, a, null)).toBe('');
    expect(prefillRemote('typed', a, null)).toBe('typed');
    expect(prefillRemote('', null, null)).toBe('');
  });

  it('keeps every card line within the 90 character help budget, with no dashes', () => {
    const lines = [SETUP_NOTE, SETUP_VISIBILITY, SETUP_RUN, SETUP_CONNECT, SETUP_CONNECT_FILLED, SETUP_OWNER_HINT, SETUP_COPIED, ...SETUP_THEN];
    for (const line of lines) {
      expect(line.length, line).toBeLessThanOrEqual(90);
      expect(line, line).not.toMatch(/[\u2013\u2014]/);
    }
  });
});
