import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { extract, refusal, runRecipes, words } from './runbook-recipes.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

test('every recipe in AGENTS.md and docs/ runs and passes against this checkout', () => {
  const { problems, ran } = runRecipes(root);
  assert.deepEqual(problems, []);
  assert.ok(ran.length >= 1, 'docs/reference/common-tasks.md carries at least one recipe');
});

test('a marker needs a fenced block right after it, and comments are skipped', () => {
  const text = [
    '<!-- recipe: one -->',
    '```sh',
    '# a comment',
    'grep -n x a.md',
    '',
    '```',
    '<!-- recipe: two -->',
    'prose',
  ].join('\n');
  const [one, two] = extract(text);
  assert.deepEqual(one, { id: 'one', line: 1, commands: ['grep -n x a.md'], error: null });
  assert.equal(two.id, 'two');
  assert.match(two.error, /must open/);
});

test('only allowlisted read-only commands with plain arguments are run', () => {
  assert.equal(refusal('grep -n "const X" src/a.ts'), null);
  assert.equal(refusal('test -f CLAUDE.md'), null);
  assert.match(refusal('rm -rf src'), /not an allowed/);
  assert.match(refusal('grep x a > b'), /shell syntax/);
  assert.match(refusal('grep x a | sh'), /shell syntax/);
  assert.match(refusal('ls $(echo /)'), /shell syntax/);
  assert.match(refusal('ls /etc'), /leaves the checkout/);
  assert.match(refusal('head ../x'), /leaves the checkout/);
  assert.match(refusal('grep "x'), /unbalanced/);
  // A metacharacter inside quotes is part of a pattern, not shell syntax.
  assert.equal(refusal('grep -n "a|b" CLAUDE.md'), null);
  assert.deepEqual(words('grep -n "two words" f'), ['grep', '-n', 'two words', 'f']);
});

test('a rotted recipe and a refused one both fail, and the refused one never runs', () => {
  const dir = mkdtempSync(join(tmpdir(), 'recipes-'));
  writeFileSync(join(dir, 'a.md'), 'present\n');
  writeFileSync(
    join(dir, 'doc.md'),
    [
      '<!-- recipe: ok -->',
      '```sh',
      'grep -n present a.md',
      '```',
      '<!-- recipe: rotted -->',
      '```sh',
      'grep -n gone a.md',
      '```',
      '<!-- recipe: unsafe -->',
      '```sh',
      'touch made.txt',
      '```',
    ].join('\n'),
  );
  const { problems, ran } = runRecipes(dir, ['doc.md']);
  assert.equal(ran.length, 2);
  assert.equal(problems.length, 2, problems.join('\n'));
  assert.match(problems[0], /recipe rotted: `grep -n gone a.md` exited 1/);
  assert.match(problems[1], /recipe unsafe: refused `touch made.txt`/);
  assert.throws(() => readFileSync(join(dir, 'made.txt')));
});

test('flag values with paths, file-reading and recursive flags are refused', () => {
  assert.match(refusal('grep --file=/etc/passwd x f'), /may not carry a path/);
  assert.match(refusal('grep -f../../secret x f'), /may not carry a path/);
  assert.match(refusal('wc --files0-from=/etc/x'), /may not carry a path/);
  assert.match(refusal('wc --files0-from=list'), /not allowed for wc/);
  assert.match(refusal('grep -f patterns x f'), /not allowed for grep/);
  assert.match(refusal('grep --file=patterns x f'), /not allowed for grep/);
  assert.match(refusal('grep --exclude-from=x y f'), /not allowed for grep/);
  assert.match(refusal('grep -R x .'), /not allowed for grep/);
  assert.match(refusal('grep -rn x .'), /not allowed for grep/);
  assert.match(refusal('grep --recursive x .'), /not allowed for grep/);
  assert.match(refusal('grep -d recurse x .'), /not allowed for grep/);
  assert.match(refusal('ls -R docs'), /not allowed for ls/);
  assert.match(refusal('ls -L x'), /not allowed for ls/);
  assert.match(refusal('head -n1 -- /etc/passwd'), /leaves the checkout/);
  // `test -f` is an operator, not grep's file flag.
  assert.equal(refusal('test -f CLAUDE.md'), null);
  assert.equal(refusal('grep -n -A2 x f'), null);
});

test('an operand that resolves outside the checkout through a symlink is refused', () => {
  const outside = mkdtempSync(join(tmpdir(), 'recipes-outside-'));
  writeFileSync(join(outside, 'secret.txt'), 'secret\n');
  const dir = mkdtempSync(join(tmpdir(), 'recipes-'));
  mkdirSync(join(dir, 'docs'));
  writeFileSync(join(dir, 'inside.txt'), 'ok\n');
  symlinkSync(join(outside, 'secret.txt'), join(dir, 'link'));
  symlinkSync(outside, join(dir, 'linkdir'));
  assert.match(refusal('head link', dir), /resolves outside the checkout/);
  assert.match(refusal('grep -n secret linkdir/secret.txt', dir), /resolves outside the checkout/);
  assert.equal(refusal('head inside.txt', dir), null);
  // A symlinked docs folder or doc is skipped, never followed.
  symlinkSync(outside, join(dir, 'docs', 'linked'));
  writeFileSync(join(outside, 'x.md'), '<!-- recipe: x -->\n```sh\nhead secret.txt\n```\n');
  symlinkSync(join(outside, 'x.md'), join(dir, 'docs', 'x.md'));
  writeFileSync(join(dir, 'AGENTS.md'), '# c\n');
  const { problems, ran } = runRecipes(dir);
  assert.deepEqual(problems, []);
  assert.equal(ran.length, 0);
});
