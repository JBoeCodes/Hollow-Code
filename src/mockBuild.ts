// Browser-mock fixtures for build workspaces beyond B1 (`?mock=1` only): the
// Changes pane (Build PR B2). State lives in localStorage `mockBuild:<id>`;
// every mutating call appends `{command, args}` to `mockBuildLog` so specs can
// assert what was sent.
import type { BuildChangedFile, BuildStatus } from './workspaceTypes';

interface MockBuild { files: BuildChangedFile[]; commits: { sha: string; subject: string; when: string }[]; upstream: string | null; unpushed: number; pr: BuildStatus['pr']; behind?: number }

const DIFFS: Record<string, string> = {
  'README.md': 'diff --git a/README.md b/README.md\nindex 1111111..2222222 100644\n--- a/README.md\n+++ b/README.md\n@@ -1,4 +1,6 @@ # Site\n # Site\n-Old intro.\n+A clearer intro.\n+\n+See the About page.\n \n Contact us.\n',
  'src/about.ts': 'diff --git a/src/about.ts b/src/about.ts\nnew file mode 100644\n--- /dev/null\n+++ b/src/about.ts\n@@ -0,0 +1,2 @@\n+export const about = "Hollow";\n+export const year = 2026;\n\\ No newline at end of file\n',
};

function initial(): MockBuild {
  return {
    files: [
      { path: 'README.md', status: 'modified', additions: 3, deletions: 1, staged: false, uncommitted: true },
      { path: 'src/about.ts', status: 'untracked', additions: 2, deletions: 0, staged: false, uncommitted: true },
    ],
    commits: [{ sha: 'a1b2c3d', subject: 'Start the About page', when: '2026-09-27T01:00:00-04:00' }],
    upstream: null, unpushed: 0, pr: null,
  };
}

/** Install from Hollow (Build PR B5): `mockAppBuild:<id>` holds the state;
 *  `mockAppBuildInstant` makes a build finish at once; `mockSelfInstallResult`
 *  is what the "relaunched" app reports. P1-46: `mockAppBuildStatusError` and
 *  `mockSelfInstallReceiptError` make those reads fail with their text. */
function appBuildFixture(command: string, args: Record<string, unknown> | undefined): { value: unknown } | null {
  if (command === 'self_install_result') return { value: JSON.parse(localStorage.getItem('mockSelfInstallResult') ?? 'null') };
  if (command === 'self_install_receipt') {
    const unreadable = localStorage.getItem('mockSelfInstallReceiptError');
    if (unreadable) throw unreadable;
    return { value: JSON.parse(localStorage.getItem('mockSelfInstallReceipt') ?? 'null') };
  }
  if (command === 'ack_self_install_result') { localStorage.removeItem('mockSelfInstallResult'); localStorage.setItem('mockSelfInstallAcked', '1'); return { value: undefined }; }
  if (!['app_build_status', 'app_build_brings', 'start_app_build', 'cancel_app_build'].includes(command)) return null;
  const id = String(args?.id ?? '');
  const key = `mockAppBuild:${id}`;
  const workspace = (JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as { id: string; build?: { repo: string; branch: string } }[]).find(w => w.id === id);
  const idle = { workspaceId: id, isHollow: !!workspace?.build?.repo.endsWith('/hollow'), state: 'idle', otherJob: null, log: null, logTail: '', startedAt: null, finishedAt: null, error: null, commit: null, branch: null };
  const current = { ...idle, ...JSON.parse(localStorage.getItem(key) ?? '{}') };
  const save = (patch: Record<string, unknown>) => { const next = { ...current, ...patch }; localStorage.setItem(key, JSON.stringify(next)); const log = JSON.parse(localStorage.getItem('mockBuildLog') ?? '[]'); log.push({ command, args }); localStorage.setItem('mockBuildLog', JSON.stringify(log)); return { value: next }; };
  switch (command) {
    case 'app_build_status': {
      const failed = localStorage.getItem('mockAppBuildStatusError');
      if (failed) throw failed;
      return { value: { ...current, blocked: localStorage.getItem('mockBuildBlocked') } };
    }
    case 'app_build_brings': {
      if (localStorage.getItem('mockAppBuildBringsError')) throw new Error('The build receipt base is unavailable');
      if (current.state !== 'built') throw new Error('No finished build is available');
      return { value: { buildId: current.receipt?.buildId, installed: 'base', installedFrom: 'build receipt', repo: workspace?.build?.repo, target: current.branch, targetCommit: current.commit, fetchedAt: null,
        missing: 2, note: null, commits: [], pullRequests: [{ number: 771, title: 'Name the installed release', merge: 'abc12345', mergedAt: '', untested: ['Not checked by hand in the native app.'], entryFound: true }] } };
    }
    case 'start_app_build': {
      if (!current.isHollow) throw "This worktree is not Hollow's repository";
      const done = localStorage.getItem('mockAppBuildInstant') === '1';
      return save({ state: done ? 'built' : 'building', receipt: done ? { buildId: crypto.randomUUID(), workspaceId: id, worktree: '/fixture/work', bundle: '/fixture/Hollow.app', bundleDigest: 'fixture', sourceFingerprint: 'fixture', commit: 'b5b5b5b5b5b5', dirty: false, baseCommit: 'base', recordedAt: '' } : null, branch: workspace?.build?.branch ?? 'hollow/work', commit: 'b5b5b5b5b5b5', log: '/Users/fixture/Library/Logs/Hollow/self-install/build.log', logTail: done ? '[self-install] built Hollow.app' : '[self-install] npm run tauri:bundle\n   Compiling hollow v0.5.3', error: null });
    }
    case 'cancel_app_build': {
      if (current.state !== 'building') throw 'No app build is running for this workspace';
      save({ state: 'failed', error: 'Stopped' });
      return { value: undefined };
    }
  }
  return null;
}

/** Services (Build PR B6): `mockServices` holds the list; stop/log calls are
 *  appended to `mockBuildLog`. */
function servicesFixture(command: string, args: Record<string, unknown> | undefined): { value: unknown } | null {
  if (!['list_services', 'stop_service', 'service_log'].includes(command)) return null;
  const all = JSON.parse(localStorage.getItem('mockServices') ?? '[]') as { cwd: string; scope: string; name: string }[];
  const scope = args?.cwd as string | null;
  if (command === 'list_services') return { value: all.filter(s => !scope || s.scope === scope) };
  const log = JSON.parse(localStorage.getItem('mockBuildLog') ?? '[]'); log.push({ command, args }); localStorage.setItem('mockBuildLog', JSON.stringify(log));
  if (command === 'stop_service') {
    localStorage.setItem('mockServices', JSON.stringify(all.filter(s => !(s.scope === scope && s.name === args?.name))));
    return { value: undefined };
  }
  return { value: `  VITE v5.4.0  ready in 312 ms\n  ➜  Local:   http://localhost:5173/` };
}

export function buildFixture(command: string, args: Record<string, unknown> | undefined): { value: unknown } | null {
  const app = appBuildFixture(command, args);
  if (app) return app;
  const services = servicesFixture(command, args);
  if (services) return services;
  if (!command.startsWith('build_') || command === 'build_repo_candidates') return null;
  const id = String(args?.id ?? '');
  const key = `mockBuild:${id}`;
  const load = (): MockBuild => JSON.parse(localStorage.getItem(key) ?? 'null') ?? initial();
  const save = (b: MockBuild) => localStorage.setItem(key, JSON.stringify(b));
  const log = () => { const l = JSON.parse(localStorage.getItem('mockBuildLog') ?? '[]'); l.push({ command, args }); localStorage.setItem('mockBuildLog', JSON.stringify(l)); };
  const workspace = (JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as { id: string; title: string; build?: { branch: string; base: string; worktree: string } }[]).find(w => w.id === id);
  const status = (b: MockBuild): BuildStatus => ({
    blocked: localStorage.getItem('mockBuildBlocked'),
    worktree: workspace?.build?.worktree ?? '/Users/fixture/Hollow/Worktrees/site/work', branch: workspace?.build?.branch ?? 'hollow/work', base: workspace?.build?.base ?? 'origin/main',
    head: b.commits[0]?.sha ?? '0000000', ahead: b.commits.length, behind: b.behind ?? 0, upstream: b.upstream, unpushed: b.unpushed,
    files: b.files, moreFiles: 0, commits: b.commits, pr: b.pr, goal: 'Fix the About page layout.',
  });
  switch (command) {
    // The Mac mini runner (2026-11-01): Runs on, kept in mockWorkspaces' build.
    case 'build_set_runs_on': {
      log();
      const all = JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as { id: string; build?: Record<string, unknown> }[];
      const row = all.find(w => w.id === id);
      if (!row?.build) throw `${id} is not a build workspace`;
      if (args?.runsOn) row.build.runsOn = args.runsOn; else delete row.build.runsOn;
      localStorage.setItem('mockWorkspaces', JSON.stringify(all));
      return { value: row.build };
    }
    case 'build_status': return { value: { ...status(load()), blocked: localStorage.getItem('mockBuildBlocked') } };
    case 'build_diff': {
      const path = args?.path as string | null;
      const files = load().files.filter(f => !path || f.path === path);
      return { value: { diff: files.map(f => DIFFS[f.path] ?? '').join(''), truncated: false, truncatedFiles: [] } };
    }
    case 'build_commit': {
      log();
      const b = load();
      if (!b.files.some(f => f.uncommitted)) throw 'Nothing to commit: the worktree has no changes';
      b.files = b.files.map(f => ({ ...f, uncommitted: false, staged: false, status: f.status === 'untracked' ? 'added' : f.status }));
      b.commits = [{ sha: `c${b.commits.length}0ffee`, subject: String(args?.message), when: new Date().toISOString() }, ...b.commits];
      if (b.upstream) b.unpushed += 1;
      save(b); return { value: status(b) };
    }
    case 'build_update_from_base': {
      log();
      const blocked = localStorage.getItem('mockBuildBlocked');
      if (blocked) throw blocked;
      const b = load(); b.behind = 0; save(b); return { value: status(b) };
    }
    case 'build_push': {
      log();
      const b = load(); b.upstream = `origin/${status(b).branch}`; b.unpushed = 0; save(b); return { value: status(b) };
    }
    case 'build_open_pr': {
      log();
      const b = load(); b.upstream ??= `origin/${status(b).branch}`; b.unpushed = 0;
      b.pr = { url: 'https://github.com/fixture/site/pull/42', state: 'open', number: 42 }; save(b);
      return { value: b.pr.url };
    }
  }
  return null;
}
