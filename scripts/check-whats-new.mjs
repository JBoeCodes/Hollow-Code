#!/usr/bin/env node
// What's new never says a release is not in the installed app (overnight
// 2026-10-15 H11; polish plan P0-05, 2026-10-23).
//
//   node scripts/check-whats-new.mjs        (also run by npm run test:scripts)
//
// Each release section of docs/WHATS_NEW.md opened with "None of it is
// installed yet; it arrives with the next install", and nobody reworded it at
// the install, so by 10-15 seven releases that were in the installed app still
// said they weren't. Its replacement, "Not in the installed app until the next
// install.", went stale the same way: by 10-22 five intros said it, including
// releases the 10-19 install certainly had. Install state lives in the build
// log and the install receipts, and What's new's header names the running
// build; a release intro says what it added, never that it isn't installed
// (naming a past install, "Installed with `abc1234` on 10-14", stays true).
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

export const WHATS_NEW = fileURLToPath(new URL('../docs/WHATS_NEW.md', import.meta.url));

const INSTEAD = 'say what the release added; the header names the running build, and the build log records installs';

/** [phrase, what to write instead]. Matched across a line wrap. */
export const REFUSED = [
  [/\binstalled\s+yet\b/i, INSTEAD],
  [/\byet\s+installed\b/i, INSTEAD],
  [/\b(?:not|isn['’]t)\s+(?:yet\s+)?in\s+the\s+installed\s+app\b/i, INSTEAD],
];

/** Problems in What's new's text: `line N: "phrase" (write: …)`. */
export function staleInstallWords(md) {
  const out = [];
  for (const [phrase, instead] of REFUSED) {
    for (const m of md.matchAll(new RegExp(phrase.source, `${phrase.flags}g`))) {
      const line = md.slice(0, m.index).split('\n').length;
      out.push({ line, text: `line ${line}: "${m[0].replace(/\s+/g, ' ')}" (write: ${instead})` });
    }
  }
  return out.sort((a, b) => a.line - b.line).map(p => p.text);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const problems = staleInstallWords(readFileSync(WHATS_NEW, 'utf8'));
  if (problems.length) {
    console.error(`docs/WHATS_NEW.md says a release isn't installed ${problems.length} time(s); it goes stale at the next install:`);
    for (const p of problems) console.error(`  ${p}`);
    process.exit(1);
  }
  console.log('check-whats-new: no release says it isn\'t installed');
}
