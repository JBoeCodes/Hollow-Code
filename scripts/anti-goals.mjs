#!/usr/bin/env node
// Anti-goals (BI6, 2026-11-17): planners run candidate items past the dated
// user decisions in docs/DO_NOT_DO.md and drop a conflicting one by naming the
// rule, instead of proposing refused work again.
//
//   node scripts/anti-goals.mjs "Add BankB export to the Money tab"
//   node scripts/anti-goals.mjs --file items.txt      one item per line, list markers stripped
//   node scripts/anti-goals.mjs                       only checks that the rules file is well formed
//   --rules <path>                                    another rules file (default docs/DO_NOT_DO.md)
//
// A rule is a `## YYYY-MM-DD · Title` section with a `Cause:` line and a
// `Match:` line. `Match:` is plain and deterministic: alternatives separated by
// commas, and inside one alternative `+` joins words that must all appear
// (`bankb + export, banka + export`). Matching is case-insensitive and a word
// must start at a word boundary, so `export` also finds `exports` but not
// `reexport`. There is no fuzzy or model match: a rule that should catch more
// gets another alternative. Exit 1 on a conflict or a malformed rule.
//
// Method only (a dated list of refused work that planners must read and cite);
// the file format and the checker are this repository's own.
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
export const RULES_FILE = join(root, 'docs/DO_NOT_DO.md');

const HEADING = /^##\s+(\d{4}-\d{1,2}-\d{1,2})\s*·\s*(.+?)\s*$/;
const FIELD = /^\s*[-*]?\s*(Decision|Cause|Source|Match):\s*(.*)$/i;

const realDate = s => {
  const [y, m, d] = s.split('-').map(Number);
  const t = new Date(Date.UTC(y, m - 1, d));
  return t.getUTCFullYear() === y && t.getUTCMonth() === m - 1 && t.getUTCDate() === d;
};

/** The rules in a DO_NOT_DO file: `{ date, title, cause, decision, source, match, line }`; `match` is a list of all-of word lists. */
export function parseRules(markdown) {
  const rules = [];
  let rule = null;
  let last = null;
  const lines = markdown.split('\n');
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const heading = HEADING.exec(line);
    if (heading) {
      rule = { date: heading[1], title: heading[2], cause: '', decision: '', source: '', matchText: '', line: i + 1 };
      rules.push(rule);
      last = null;
      continue;
    }
    if (/^#{1,6}\s/.test(line)) {
      rule = null;
      last = null;
      continue;
    }
    if (!rule) continue;
    const field = FIELD.exec(line);
    if (field) {
      last = field[1].toLowerCase() === 'match' ? 'matchText' : field[1].toLowerCase();
      rule[last] = field[2].trim();
    } else if (last && /^\s{2,}\S/.test(line)) {
      rule[last] += ` ${line.trim()}`;
    } else {
      last = null;
    }
  }
  return rules.map(r => ({
    ...r,
    match: r.matchText
      .split(',')
      .map(alt => alt.split('+').map(w => w.trim().toLowerCase()).filter(Boolean))
      .filter(alt => alt.length),
  }));
}

/** Does the text contain every word of one alternative, each starting at a word boundary? */
export function termMatches(text, words) {
  if (!words.length) return false;
  return words.every(w => {
    const pattern = w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/\s+/g, '\\s+');
    return new RegExp(`(?<![\\p{L}\\p{N}])${pattern}`, 'iu').test(text);
  });
}

/** The rules an item conflicts with, each with the message to print. */
export function conflicts(item, rules) {
  return rules
    .filter(r => r.match.some(alt => termMatches(item, alt)))
    .map(r => ({ rule: r, message: `conflicts with the rule "${r.title}" (${r.date}): ${r.cause}` }));
}

/** Problems with a rules file, one string each. */
export function checkRules(markdown) {
  const rules = parseRules(markdown);
  if (!rules.length) return ['no rules found (a rule is a "## YYYY-MM-DD · Title" section)'];
  const problems = [];
  for (const r of rules) {
    const where = `rule "${r.title}" (line ${r.line})`;
    if (!realDate(r.date)) problems.push(`${where}: ${r.date} is not a real date`);
    if (!r.cause) problems.push(`${where}: no Cause: line (the reason for the decision)`);
    if (!r.match.length) problems.push(`${where}: no Match: line (words that identify a conflicting item)`);
  }
  return problems;
}

function main(argv) {
  let rulesPath = RULES_FILE;
  const items = [];
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--rules') rulesPath = argv[++i];
    else if (argv[i] === '--file') {
      for (const line of readFileSync(argv[++i], 'utf8').split('\n')) {
        const item = line.replace(/^\s*(?:[-*]|\d+[.)])\s+/, '').trim();
        if (item) items.push(item);
      }
    } else items.push(argv[i]);
  }
  const text = readFileSync(rulesPath, 'utf8');
  const problems = checkRules(text);
  if (problems.length) {
    console.error(`anti-goals: ${rulesPath}:\n  ${problems.join('\n  ')}`);
    return 1;
  }
  const rules = parseRules(text);
  let rejected = 0;
  for (const item of items) {
    for (const c of conflicts(item, rules)) {
      rejected++;
      console.error(`anti-goals: rejected "${item}": ${c.message}`);
    }
  }
  if (!rejected) console.log(`anti-goals: ${rules.length} rules, ${items.length} items, no conflict`);
  return rejected ? 1 : 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) process.exit(main(process.argv.slice(2)));
