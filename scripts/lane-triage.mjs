#!/usr/bin/env node
// Triage before work (BI3, 2026-11-17): every lane ledger row carries one of
// four outcomes, and each outcome carries what a reader needs to trust it.
//
//   node scripts/lane-triage.mjs <ledger.md>... [--allow-placeholders]
//
//   work   do the row; nothing more is needed here.
//   close  the row is already done; needs `Evidence:` (a sha, a test, a path).
//   block  the row waits on something that is not another row; needs
//          `Unblock: <a condition someone can check>` and `Owner: <who>`, so the
//          next pass can re-check it instead of rereading the whole row.
//   drop   the row is not going to be done; needs `Why:` (the decision, with its
//          date, or the finding).
//
// The rule behind it: never delete a row on suspicion. Block it with the
// question. A row is read from two places in a ledger (docs/LANE_LEDGER_TEMPLATE.md):
// a table row under a header that has a `Triage` column, and a list item under a
// `Held ...` heading or carrying `Triage: <outcome>`; an indented line under a
// list item belongs to it. Exit 1 lists every row that fails. A fresh template
// has unfilled `<...>` cells: `--allow-placeholders` skips those rows.
//
// Method only (a worker writes evidence for one of four outcomes before it
// works); the wording and the checker are this repository's own.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

export const OUTCOMES = new Set(['work', 'close', 'block', 'drop']);

const cells = line => line.trim().replace(/^\|/, '').replace(/\|\s*$/, '').split('|').map(c => c.trim());
const isSeparator = line => /^\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/.test(line.trim());
const isPlaceholder = s => !s || /^<[^>]*>$/.test(s.trim());

/** The rows of a ledger: `{ id, outcome, text, line, kind }`; `outcome` is '' when unfilled. */
export function parseRecords(markdown) {
  const lines = markdown.split('\n');
  const out = [];
  let held = false;
  let triageCol = -1;
  let current = null;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (current && /^\s{2,}\S/.test(line)) {
      current.text += ` ${line.trim()}`;
      continue;
    }
    current = null;
    if (/^#{1,6}\s/.test(line)) {
      held = /^#{1,6}\s*held/i.test(line);
      triageCol = -1;
      continue;
    }
    if (line.trim().startsWith('|')) {
      if (isSeparator(line)) continue;
      const row = cells(line);
      const header = row.findIndex(c => c.toLowerCase() === 'triage');
      if (header >= 0 && triageCol < 0) {
        triageCol = header;
        continue;
      }
      if (triageCol < 0) continue;
      const cell = row[triageCol] ?? '';
      const word = /^[a-z]+/i.exec(cell)?.[0].toLowerCase() ?? '';
      out.push({ id: row[0] || '(no id)', outcome: word, text: line, line: i + 1, kind: 'table' });
      continue;
    }
    triageCol = -1;
    const item = /^[-*]\s+(.*)$/.exec(line);
    if (!item) continue;
    const triage = /\bTriage:\s*([a-z]+)/i.exec(item[1]);
    if (!held && !triage) continue;
    const id = item[1].split(':')[0].trim() || '(no id)';
    current = { id, outcome: triage ? triage[1].toLowerCase() : '', text: item[1], line: i + 1, kind: 'list' };
    out.push(current);
  }
  return out;
}

/** The text after `Label:` up to the next label, table bar or full stop. */
function field(text, label) {
  const at = text.search(new RegExp(`\\b${label}:`, 'i'));
  if (at < 0) return null;
  const rest = text.slice(at + label.length + 1);
  const end = rest.search(/\b(?:Unblock|Owner|Evidence|Why|Triage):|\||;/i);
  const value = (end < 0 ? rest : rest.slice(0, end)).trim();
  // A full stop that ends the value (not one inside a path or a sha).
  return value.replace(/\.(\s.*)?$/, '').replace(/[;,]\s*$/, '').trim();
}

/** Problems with a ledger's rows, one string each. */
export function checkLedger(markdown, { allowPlaceholders = false } = {}) {
  const problems = [];
  for (const r of parseRecords(markdown)) {
    const where = `${r.id} (line ${r.line})`;
    if (!r.outcome) {
      if (!allowPlaceholders) problems.push(`${where}: no triage outcome (work, close, block or drop)`);
      continue;
    }
    if (!OUTCOMES.has(r.outcome)) {
      problems.push(`${where}: unknown triage outcome "${r.outcome}" (work, close, block or drop)`);
      continue;
    }
    if (r.outcome === 'block') {
      const condition = field(r.text, 'Unblock');
      const owner = field(r.text, 'Owner');
      if (condition === null) problems.push(`${where}: blocked with no Unblock line (Unblock: <a condition someone can check>. Owner: <who>)`);
      else {
        if (isPlaceholder(condition)) problems.push(`${where}: Unblock has no condition someone can check`);
        if (isPlaceholder(owner ?? '')) problems.push(`${where}: Unblock has no Owner`);
      }
    }
    if (r.outcome === 'close' && isPlaceholder(field(r.text, 'Evidence') ?? '')) {
      problems.push(`${where}: closed as already done with no Evidence: (a sha, a test or a path)`);
    }
    if (r.outcome === 'drop' && isPlaceholder(field(r.text, 'Why') ?? '')) {
      problems.push(`${where}: dropped with no Why: (the dated decision, or the finding)`);
    }
  }
  return problems;
}

function main(argv) {
  const allowPlaceholders = argv.includes('--allow-placeholders');
  const files = argv.filter(a => !a.startsWith('--'));
  if (!files.length) {
    console.error('usage: node scripts/lane-triage.mjs <ledger.md>... [--allow-placeholders]');
    return 2;
  }
  let bad = 0;
  for (const file of files) {
    const text = readFileSync(file, 'utf8');
    const problems = checkLedger(text, { allowPlaceholders });
    if (problems.length) {
      bad += problems.length;
      console.error(`lane-triage: ${file}:\n  ${problems.join('\n  ')}`);
    } else {
      console.log(`lane-triage: ${file}: ${parseRecords(text).length} rows, all triaged`);
    }
  }
  return bad ? 1 : 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) process.exit(main(process.argv.slice(2)));
