#!/usr/bin/env node
// Orchestrator facts and an outcome check (BI2, 2026-11-17).
//
//   node scripts/lane-outcome.mjs check [--repo dir] [--pr N] [--branch b] [--sha s]
//        [--remote origin] [--fetch] [--json]
//   node scripts/lane-outcome.mjs facts <board.md> --now <iso> [--json]
//
// `check`: a worker's summary is a claim, not evidence. Before a row is marked
// done, git must agree. The row stays OPEN (exit 1, one line per reason) when:
//   - a claimed sha, or the tip of the claimed branch, is not an ancestor of
//     `<remote>/main`;
//   - a claimed PR number has no commit on `<remote>/main` whose SUBJECT ends
//     `(#N)` (squash merge) or reads `Merge pull request #N` (a body mention does
//     not count; with a sha too, that commit's own subject must name the PR);
//   - an input is malformed (pr digits only, sha 7 to 40 lowercase hex, plain
//     branch and remote names), so `HEAD` or `--flag` never reaches git;
//   - the claimed branch holds commits no remote has (unpushed);
//   - the working tree is not clean.
// It reads the refs as last fetched; `--fetch` runs `git fetch <remote> main`
// first (the only network step). The git runner is injectable (`checkOutcome`).
//
// `facts`: one JSON of the rows of a board table, so the orchestrator never
// reads the board: `{ ready, waiting, blocked, due, next, done }`, each a list
// of row ids. The table needs a `Status` column (first column is the id);
// optional `Depends` (ids or `none`), `Due` (ISO date) and `Unblock`.
//   status ready, open, todo, queued -> ready, unless a Depends id is not done
//          (then waiting); waiting -> waiting; blocked -> blocked;
//   done, merged, closed, dropped -> done (never due).
// `due` lists every not-done row whose `Due` is on or before `--now` (a row can
// be both ready and due). `next` is the first ready row.
// No model; time is the `--now` argument. Method only (beancount-io's board and
// outcome commands); code is ours.
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

/** The real git runner: `(args) => { code, stdout }`. */
export const gitRunner = dir => args => {
  const r = spawnSync('git', ['-C', dir, ...args], { encoding: 'utf8' });
  return { code: r.status ?? 1, stdout: r.stdout ?? '' };
};

const prMerged = (subject, n) => subject.endsWith(`(#${n})`) || new RegExp(`^Merge pull request #${n}\\b`).test(subject);

export function checkOutcome(claim, git, remote = 'origin') {
  const reasons = [];
  if (claim.pr && !/^\d+$/.test(claim.pr)) reasons.push(`PR "${claim.pr}" is not a number`);
  if (claim.sha && !/^[0-9a-f]{7,40}$/.test(claim.sha)) reasons.push(`sha "${claim.sha}" is not 7 to 40 lowercase hex digits`);
  if (claim.branch && !/^[A-Za-z0-9._/-]+$/.test(claim.branch)) reasons.push(`branch "${claim.branch}" has characters a branch name should not`);
  if (claim.branch && /^-|\.\.|\/\//.test(claim.branch)) reasons.push(`branch "${claim.branch}" is not a plain branch name`);
  if (!/^[A-Za-z0-9._][A-Za-z0-9._-]*$/.test(remote)) reasons.push(`remote "${remote}" is not a plain remote name`);
  if (reasons.length) return { ok: false, reasons };
  const main = `${remote}/main`;
  // `refs/heads/` prefixes the branch and the other values are validated above, so only
  // positions after which no option follows carry `--end-of-options`.
  const E = '--end-of-options';
  const ok = args => git(args).code === 0;
  const out = args => {
    const r = git(args);
    return r.code === 0 ? r.stdout.trim() : '';
  };
  const onMain = sha => ok(['merge-base', '--is-ancestor', E, sha, main]);
  if (!claim.pr && !claim.branch && !claim.sha) reasons.push('no claim to check: give a PR number, branch or sha');
  if (claim.sha) {
    if (!ok(['cat-file', '-e', `${claim.sha}^{commit}`])) reasons.push(`commit ${claim.sha} is unknown here`);
    else {
      if (!onMain(claim.sha)) reasons.push(`commit ${claim.sha} is not on ${main}`);
      if (claim.pr) {
        const subject = out(['show', '-s', '--format=%s', E, claim.sha]);
        if (!prMerged(subject, claim.pr)) reasons.push(`commit ${claim.sha} subject does not name PR #${claim.pr}`);
      }
    }
  } else if (claim.pr) {
    // Only a commit SUBJECT counts: a body line that mentions (#N) is not a merge.
    const log = out(['log', '--format=%H%x09%s', '-F', `--grep=#${claim.pr}`, E, main]);
    const hit = log.split('\n').some(l => prMerged(l.split('\t').slice(1).join('\t'), claim.pr));
    if (!hit) reasons.push(`PR #${claim.pr} has no merge commit on ${main}`);
  }
  if (claim.branch) {
    const local = out(['rev-parse', '--verify', '--quiet', `refs/heads/${claim.branch}`]);
    const remoteTip = out(['rev-parse', '--verify', '--quiet', `refs/remotes/${remote}/${claim.branch}`]);
    const tip = local || remoteTip;
    if (!tip) reasons.push(`branch ${claim.branch} exists neither here nor on ${remote}`);
    else {
      if (!claim.sha && !onMain(tip)) reasons.push(`branch ${claim.branch} (${tip.slice(0, 9)}) is not merged into ${main}`);
      if (local) {
        const n = Number(out(['rev-list', '--count', `refs/heads/${claim.branch}`, '--not', `--remotes=${remote}`]));
        if (n > 0) reasons.push(`branch ${claim.branch} has ${n} unpushed commit${n === 1 ? '' : 's'}`);
      }
    }
  }
  const status = git(['status', '--porcelain']);
  const dirty = (status.code === 0 ? status.stdout : '').split('\n').filter(Boolean);
  if (status.code !== 0) reasons.push('git status failed: the tree state is unknown');
  if (dirty.length) reasons.push(`working tree is not clean (${dirty.length} path${dirty.length === 1 ? '' : 's'}, first: ${dirty[0].slice(3)})`);
  return { ok: reasons.length === 0, reasons };
}

const cells = line => line.trim().replace(/^\|/, '').replace(/\|\s*$/, '').split('|').map(c => c.trim().replace(/^\*\*(.*)\*\*$/, '$1'));
const isSeparator = line => /^\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/.test(line.trim());
const DONE = new Set(['done', 'merged', 'closed', 'dropped']);
const READY = new Set(['ready', 'open', 'todo', 'queued']);

export function boardFacts(markdown, now) {
  const rows = [];
  let col = null;
  for (const line of markdown.split('\n')) {
    if (!line.trim().startsWith('|')) {
      col = null;
      continue;
    }
    if (isSeparator(line)) continue;
    const row = cells(line);
    if (!col) {
      const lower = row.map(c => c.toLowerCase());
      if (lower.includes('status')) col = { status: lower.indexOf('status'), depends: lower.indexOf('depends'), due: lower.indexOf('due') };
      continue;
    }
    const get = i => (i >= 0 ? row[i] ?? '' : '');
    rows.push({ id: row[0], status: get(col.status).toLowerCase().match(/^[a-z]+/)?.[0] ?? '', depends: get(col.depends), due: get(col.due) });
  }
  const done = new Set(rows.filter(r => DONE.has(r.status)).map(r => r.id));
  const facts = { ready: [], waiting: [], blocked: [], due: [], next: null, done: [...done] };
  const nowMs = Date.parse(now);
  for (const r of rows) {
    if (done.has(r.id)) continue;
    const deps = r.depends.split(/[,\s]+/).filter(d => d && !/^(none|-|n\/a)$/i.test(d));
    if (r.status === 'blocked') facts.blocked.push(r.id);
    else if (r.status === 'waiting' || (READY.has(r.status) && deps.some(d => !done.has(d)))) facts.waiting.push(r.id);
    else if (READY.has(r.status)) facts.ready.push(r.id);
    const due = Date.parse(r.due);
    if (Number.isFinite(due) && Number.isFinite(nowMs) && due <= nowMs) facts.due.push(r.id);
  }
  facts.next = facts.ready[0] ?? null;
  return facts;
}

function main(argv) {
  const [cmd, ...rest] = argv;
  const o = { _: [] };
  for (let i = 0; i < rest.length; i++) {
    const a = rest[i];
    if (a === '--fetch' || a === '--json') o[a.slice(2)] = true;
    else if (['--repo', '--pr', '--branch', '--sha', '--remote', '--now'].includes(a)) o[a.slice(2)] = rest[++i];
    else if (a.startsWith('--')) {
      console.error(`unknown argument ${a}`);
      return 2;
    } else o._.push(a);
  }
  if (cmd === 'facts') {
    if (!o._[0] || !Number.isFinite(Date.parse(o.now))) {
      console.error('usage: lane-outcome.mjs facts <board.md> --now <iso> [--json]');
      return 2;
    }
    console.log(JSON.stringify(boardFacts(readFileSync(o._[0], 'utf8'), o.now), null, o.json ? 0 : 2));
    return 0;
  }
  if (cmd === 'check') {
    const git = gitRunner(o.repo ?? '.');
    const remote = o.remote ?? 'origin';
    if (o.fetch && /^[A-Za-z0-9._][A-Za-z0-9._-]*$/.test(remote) && git(['fetch', '--end-of-options', remote, 'main']).code !== 0) console.error(`warning: git fetch ${remote} main failed; refs may be stale`);
    const r = checkOutcome({ pr: o.pr, branch: o.branch, sha: o.sha }, git, remote);
    if (o.json) console.log(JSON.stringify(r));
    else if (r.ok) console.log('ok: the claim matches git');
    else for (const reason of r.reasons) console.log(`OPEN ${reason}`);
    return r.ok ? 0 : 1;
  }
  console.error('usage: lane-outcome.mjs check ... | facts <board.md> --now <iso>');
  return 2;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) process.exit(main(process.argv.slice(2)));
