#!/usr/bin/env node
// Copy `tools/ctx-scan/` from an org-context checkout into vendor/ctx-scan/
// and record where it came from.
//
//   node scripts/vendor-ctx-scan.mjs <path to an org-context checkout>
//
// The org-context repo owns scan-rules.json, the standalone ctx-scan and the
// shared fixture corpus. Hollow reads the vendored copy (scan_rules.rs
// include_str!s the rules and its tests read the fixtures);
// vendor/ctx-scan/VENDORED.md records the source commit and each file's
// SHA-256, and `scan_rules::tests::vendored_copy_matches_its_record` fails
// when a vendored file no longer matches its line, so the copy can't be edited
// here by accident. Change the rules upstream, then run this.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { cpSync, existsSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const dest = join(root, 'vendor', 'ctx-scan');

function files(dir, base = dir) {
  const out = [];
  for (const name of readdirSync(dir).sort()) {
    if (name === '__pycache__' || name.endsWith('.pyc') || name === 'VENDORED.md') continue;
    const full = join(dir, name);
    if (statSync(full).isDirectory()) out.push(...files(full, base));
    else out.push(relative(base, full).split('\\').join('/'));
  }
  return out;
}

const checkout = process.argv[2];
if (!checkout) {
  console.error('usage: node scripts/vendor-ctx-scan.mjs <org-context checkout>');
  process.exit(2);
}
const src = join(checkout, 'tools', 'ctx-scan');
if (!existsSync(join(src, 'scan-rules.json'))) {
  console.error(`no tools/ctx-scan/scan-rules.json under ${checkout}`);
  process.exit(2);
}
const commit = execFileSync('git', ['-C', checkout, 'rev-parse', 'HEAD']).toString().trim();
const dirty = execFileSync('git', ['-C', checkout, 'status', '--porcelain', '--', 'tools/ctx-scan']).toString().trim();
if (dirty) {
  console.error('tools/ctx-scan has uncommitted changes in that checkout; commit them first');
  process.exit(2);
}

rmSync(dest, { recursive: true, force: true });
cpSync(src, dest, {
  recursive: true,
  filter: p => !p.includes('__pycache__') && !p.endsWith('.pyc'),
});
const lines = files(dest).map(f => {
  const hash = createHash('sha256').update(readFileSync(join(dest, f))).digest('hex');
  return `${hash}  ${f}`;
});
writeFileSync(
  join(dest, 'VENDORED.md'),
  `# Vendored: ctx-scan

A copy of \`tools/ctx-scan/\` from your organization context repository, commit
\`${commit}\`. Don't edit these files here: change them there by pull request,
then run \`node scripts/vendor-ctx-scan.mjs <checkout>\`. Hollow's
\`scan_rules.rs\` reads \`scan-rules.json\` and its tests read \`fixtures/\`;
\`scan_rules::tests::vendored_copy_matches_its_record\` checks every hash below.

\`\`\`
${lines.join('\n')}
\`\`\`
`,
);
console.log(`vendored ${lines.length} files from ${commit}`);
