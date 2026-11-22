// Executable runbook recipes (BI7, 2026-11-18; idea from beancount-io, whose
// guides' recipe blocks run in CI against fixtures).
//
// A doc marks a fenced block as a recipe with an HTML comment on the line
// right before it:
//
//   <!-- recipe: some-id -->
//   ```sh
//   grep -n "const LOCAL_COMMANDS" src/components/chat/slashCommands.ts
//   ```
//
// `runbook-recipes.test.mjs` finds every recipe in AGENTS.md and docs/ and
// runs each line, with this checkout as the fixture, so a runbook whose
// command stops matching the code fails the test instead of rotting. Only
// read-only commands run: the command must be on ALLOWED, no argument may
// hold a shell metacharacter, an absolute path or `..`, no flag may carry a
// path (attached or after `=`), flags that read a named file or walk a tree
// are refused (FORBIDDEN), an operand that exists must resolve inside the
// checkout with symlinks followed, and no shell is involved (spawnSync
// without `shell`). A recipe that breaks those rules is
// refused, never run, and the refusal fails the test too.

import { spawnSync } from 'node:child_process';
import { lstatSync, readdirSync, readFileSync, realpathSync } from 'node:fs';
import { join, relative, resolve, sep } from 'node:path';

/** Commands a recipe may run: each only reads. */
export const ALLOWED = new Set(['grep', 'ls', 'test', 'head', 'wc']);
/**
 * Flags refused per command: ones that read a file named in their value
 * (`grep -f`, `--exclude-from`, `wc --files0-from`) or walk a tree, where a
 * symlink could lead out of the checkout (`grep -r`/`-R`/`-d recurse`,
 * `ls -R`/`-L`). Short letters match anywhere in a cluster such as `-rn`.
 */
const FORBIDDEN = new Map([
  ['grep', { short: 'frRdD', long: ['--file', '--exclude-from', '--recursive', '--dereference-recursive', '--directories', '--devices'] }],
  ['wc', { short: '', long: ['--files0-from'] }],
  ['ls', { short: 'RL', long: ['--recursive', '--dereference'] }],
]);

const MARKER = /^<!--\s*recipe:\s*([a-z0-9][a-z0-9-]*)\s*-->\s*$/;

/** Recipes in Markdown `text`: `{ id, line, commands }`, `line` 1-based. */
export function extract(text) {
  const lines = text.split('\n');
  const out = [];
  for (let i = 0; i < lines.length; i += 1) {
    const m = MARKER.exec(lines[i].trim());
    if (!m) continue;
    const fence = lines[i + 1]?.trim() ?? '';
    if (!/^```(sh|bash|shell)?$/.test(fence)) {
      out.push({ id: m[1], line: i + 1, commands: [], error: 'the line after the marker must open a ```sh block' });
      continue;
    }
    const commands = [];
    let j = i + 2;
    for (; j < lines.length && lines[j].trim() !== '```'; j += 1) {
      const cmd = lines[j].trim();
      if (cmd && !cmd.startsWith('#')) commands.push(cmd);
    }
    out.push({ id: m[1], line: i + 1, commands, error: j >= lines.length ? 'the block is never closed' : null });
    i = j;
  }
  return out;
}

/** Split a command into words: double or single quotes group, no escapes. */
export function words(cmd) {
  const out = [];
  let cur = '';
  let quote = null;
  let started = false;
  for (const ch of cmd) {
    if (quote) {
      if (ch === quote) quote = null;
      else cur += ch;
    } else if (ch === '"' || ch === "'") {
      quote = ch;
      started = true;
    } else if (ch === ' ' || ch === '\t') {
      if (started || cur) out.push(cur);
      cur = '';
      started = false;
    } else {
      cur += ch;
      started = true;
    }
  }
  if (quote) return null;
  if (started || cur) out.push(cur);
  return out;
}

const leaves = (a) => a.startsWith('/') || a.startsWith('~') || a.split('/').includes('..');

/**
 * Why `cmd` may not run, or null when it is a safe read. With `root`, an
 * operand that exists must also resolve (symlinks followed) inside it.
 */
export function refusal(cmd, root = null) {
  if (/[|;&<>`$\\\n(){}*?]/.test(cmd.replace(/"[^"]*"|'[^']*'/g, ''))) {
    return 'shell syntax (pipes, redirects, globs, substitutions) is not allowed';
  }
  const argv = words(cmd);
  if (!argv || argv.length === 0) return 'unbalanced quotes or an empty command';
  const [bin, ...args] = argv;
  if (!ALLOWED.has(bin)) return `${bin} is not an allowed read-only command (${[...ALLOWED].join(', ')})`;
  const forbidden = FORBIDDEN.get(bin) ?? { short: '', long: [] };
  let operands = false;
  for (const a of args) {
    if (!operands && a === '--') {
      operands = true;
      continue;
    }
    if (!operands && a.startsWith('-') && a.length > 1) {
      // A flag: no path in its value, attached (`-f../x`) or after `=`.
      if (a.includes('/') || a.includes('~') || a.includes('..')) {
        return `${a}: a flag may not carry a path`;
      }
      if (a.startsWith('--')) {
        if (forbidden.long.some((f) => a === f || a.startsWith(`${f}=`))) return `${a} is not allowed for ${bin}`;
      } else if ([...a.slice(1)].some((c) => forbidden.short.includes(c))) {
        return `${a} is not allowed for ${bin} (no -${forbidden.short.split('').join(', -')})`;
      }
      continue;
    }
    if (leaves(a)) return `${a} leaves the checkout`;
    if (root) {
      const path = resolve(root, a);
      if (existsLstat(path)) {
        const real = realpathSync(path);
        const top = realpathSync(root);
        if (real !== top && !real.startsWith(top + sep)) return `${a} resolves outside the checkout (${real})`;
      }
    }
  }
  return null;
}

function existsLstat(path) {
  try {
    lstatSync(path);
    return true;
  } catch {
    return false;
  }
}

/** Markdown files to scan under `root`: AGENTS.md (the manual entry point) and docs/, history left out. */
export function recipeDocs(root) {
  const out = ['AGENTS.md'];
  // lstat: a symlinked folder or doc is skipped, never followed.
  const walk = (dir) => {
    for (const name of readdirSync(join(root, dir))) {
      const rel = `${dir}/${name}`;
      if (rel === 'docs/history') continue;
      const st = lstatSync(join(root, rel));
      if (st.isDirectory()) walk(rel);
      else if (st.isFile() && name.endsWith('.md')) out.push(rel);
    }
  };
  if (lstatSync(join(root, 'docs')).isDirectory()) walk('docs');
  return out;
}

/** Run every recipe in `docs` under `root`; one problem string per failure. */
export function runRecipes(root, docs = recipeDocs(root)) {
  const problems = [];
  const ran = [];
  for (const doc of docs) {
    const text = readFileSync(join(root, doc), 'utf8');
    if (!text.includes('<!-- recipe:')) continue;
    for (const r of extract(text)) {
      const where = `${relative(root, join(root, doc))}:${r.line} recipe ${r.id}`;
      if (r.error) {
        problems.push(`${where}: ${r.error}`);
        continue;
      }
      if (r.commands.length === 0) {
        problems.push(`${where}: no command`);
        continue;
      }
      // Check every line before running any, so a refused recipe runs nothing.
      const refused = r.commands.map((cmd) => [cmd, refusal(cmd, root)]).filter(([, why]) => why);
      if (refused.length > 0) {
        for (const [cmd, why] of refused) problems.push(`${where}: refused \`${cmd}\`: ${why}`);
        continue;
      }
      for (const cmd of r.commands) {
        const [bin, ...args] = words(cmd);
        const res = spawnSync(bin, args, { cwd: root, encoding: 'utf8', timeout: 10_000, shell: false });
        if (res.status !== 0) {
          problems.push(`${where}: \`${cmd}\` exited ${res.status ?? res.signal}: ${(res.stderr || res.stdout || '').trim().split('\n')[0] ?? ''}`);
        }
      }
      ran.push(where);
    }
  }
  return { problems, ran };
}
