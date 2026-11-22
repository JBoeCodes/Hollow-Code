#!/usr/bin/env node
// Lane reports → one morning summary (overnight D8). Reads every
// `lane-*.md` in an overnight folder and writes one short summary: merged
// PRs one line each, what wasn't merged, decisions taken, open review
// findings and what nobody tried by hand. Reads and writes local files only.
//
//   node scripts/overnight-summary.mjs <folder> [--out <file>]
//
// Without --out it prints the summary. The reports are free-form Markdown;
// an item is a `##`/`###` heading that starts with its id (`A1`, `S5 + S6`,
// `S7, S8`, `B6 · Side-pane chat — skipped`), and a label is a bold or
// bulleted phrase ending in a colon, from the lanes' own words: **Decisions…:**,
// **Left:**, "Documented, not fixed:", **Not built:**, **For Owner…:**,
// **Untested by hand:** and the like.
import { existsSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from 'node:fs';
import { basename, join } from 'node:path';
import { fileURLToPath } from 'node:url';

// ITEM and PR are shared with scripts/agent-eval/morning-report.mjs (OF21).
export const ITEM = /^#{2,3}\s+([A-Z]\d+(?:\s*[+,]\s*[A-Z]\d+)*)(?:\s*[·:]\s*|\s+)(.+?)\s*$/;
export const PR = /\[#(\d+)\]\(https:\/\/github\.com\/[^)]+\/pull\/\d+\)/g;
const COMMIT = /main\s+\**`([0-9a-f]{7,40})`/;
const KINDS = [
  // Not "Decision-free and next".
  ['decisions', /^decisions?(?![-\w])|\(decision\)/i],
  ['alex', /^(for|needs) owner\b/i],
  ['open', /^(left|documented|not fixed|accepted|not done|not built|open)(?![-\w])/i],
  ['untested', /^untested\b/i],
];
const MAX = 300;

const indent = (line) => line.length - line.trimStart().length;
const clean = (text) => {
  const flat = text.replace(/\*\*/g, '').replace(/\s+/g, ' ').trim();
  return flat.length > MAX ? `${flat.slice(0, MAX - 1).trimEnd()}…` : flat;
};

/** A label line: `- **Left:** text`, `**Decisions taken (no questions):**`,
 *  `- Documented, not fixed: text`. → { depth, label, rest } */
export function labelOf(line) {
  // The colon makes it a label: "**Open workspace map** opens…" isn't one.
  const bold = /^(\s*)(?:-\s+)?\*\*([^*]+?:|[^*]+?\*\*:)\**\s*(.*)$/.exec(line);
  const plain = /^(\s*)-\s+([A-Z][A-Za-z ,]+?):\s+(.*)$/.exec(line);
  const m = bold ?? plain;
  if (!m) return null;
  const label = m[2].replace(/\*\*:?$/, '').replace(/:$/, '').trim();
  const kind = KINDS.find(([, re]) => re.test(label))?.[0];
  return kind ? { depth: m[1].length, kind, rest: m[3] } : null;
}

/** The entries under a label line at `start`: each direct sub-bullet with its
 *  continuation lines, or the label's own text when it has none. */
function block(lines, start, depth, rest) {
  const body = [];
  let end = start;
  for (let i = start + 1; i < lines.length; i++) {
    const l = lines[i];
    if (!l.trim()) {
      // A blank line ends the block unless the next line still nests under it.
      if (i + 1 < lines.length && lines[i + 1].trim() && indent(lines[i + 1]) > depth) continue;
      break;
    }
    if (indent(l) <= depth) break;
    body.push(l);
    end = i;
  }
  return { end, entries: entries(body, depth, rest) };
}

function entries(body, depth, rest) {
  const subDepth = Math.min(...body.filter((l) => /^\s*- /.test(l)).map(indent));
  const entries = [];
  for (const l of body) {
    if (/^\s*- /.test(l) && indent(l) === subDepth) entries.push(l.trim().replace(/^- /, ''));
    else if (entries.length) entries[entries.length - 1] += ` ${l.trim().replace(/^- /, '')}`;
    else rest += ` ${l.trim()}`;
  }
  if (!entries.length) return rest.trim() ? [clean(rest)] : [];
  // "It skips:" with a short list under it reads as one line; a long one
  // keeps its rows, so nothing is cut off.
  const joined = `${rest} ${entries.join(' ')}`.replace(/\s+/g, ' ').trim();
  if (rest.trim().endsWith(':') && joined.length <= MAX) return [clean(joined)];
  const out = entries.map(clean).filter(Boolean);
  return rest.trim() && !/^(listed|these|as follows)\b.*:$/i.test(rest.trim()) ? [clean(rest), ...out] : out;
}

/** Every item in one report: { lane, id, title, status, prs, commit,
 *  decisions, open, untested }. */
export function parseReport(name, text) {
  const lines = text.split('\n');
  const items = [];
  let cur = null;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const head = /^#{2,3}\s/.test(line) ? ITEM.exec(line) : null;
    if (/^#{1,3}\s/.test(line)) {
      cur = null;
      const item = head && heading(head[1], head[2]);
      if (item) {
        cur = { lane: name, ...item, prs: [], commit: null, decisions: [], alex: [], open: [], untested: [], body: '' };
        items.push(cur);
      }
      continue;
    }
    if (!cur) continue;
    cur.body += `${line}\n`;
    const label = labelOf(line);
    if (label) {
      const { end, entries: found } = block(lines, i, label.depth, label.rest);
      cur[label.kind].push(...found);
      // A label inside this one's list is part of it, not a second entry.
      for (let k = i + 1; k <= end; k++) cur.body += `${lines[k]}\n`;
      i = end;
    }
  }
  for (const it of items) {
    // The item's own PR: from its **PR:** or "merged with a merge commit"
    // line when it has one, so a PR the prose mentions isn't credited.
    const own = it.body.split('\n').filter((l) => /\*\*PR:?\*\*|merged with a merge commit/i.test(l)).join('\n');
    for (const m of (own || it.body).matchAll(PR)) if (!it.prs.includes(m[1])) it.prs.push(m[1]);
    // "main `abc`" may wrap: "as main\n`abc`".
    it.commit = COMMIT.exec(it.body)?.[1]?.slice(0, 8) ?? null;
    delete it.body;
  }
  return items;
}

/** An item heading's id, title and status, or null for a section that
 *  isn't one ("A4 notes (for the successor)"). */
export function heading(id, rest) {
  const at = rest.lastIndexOf('—');
  let title = (at >= 0 ? rest.slice(0, at) : rest).trim();
  let status = at >= 0 ? rest.slice(at + 1).trim() : '';
  if (!status) {
    // "S1 · part 2 merged (N4: …)": the status is in the title.
    if (/\bmerged\b/i.test(title)) status = 'merged';
    else if (/^[a-z]/.test(title)) return null;
  }
  return { id: id.replace(/\s+/g, ' '), title: title || id, status };
}

const merged = (it) => /\bmerged\b/i.test(it.status) && it.prs.length > 0;

/** The summary Markdown for `reports` ({ name, text }[]), dated `day`. */
export function summarize(reports, day, now = new Date()) {
  const items = reports.flatMap((r) => parseReport(r.name, r.text));
  // Both lanes may use one id (an S5 each): the lane letter tells them apart.
  const lane = (it) => /^lane-(\w+)\.md$/.exec(it.lane)?.[1]?.toUpperCase() ?? it.lane;
  const ref = (it) => `**${it.id}** (${lane(it)})${it.title === it.id ? '' : ` ${it.title}`}`;
  const out = [
    `# Overnight ${day}: morning summary`,
    '',
    `From ${reports.map((r) => r.name).join(', ')} at ${now.toISOString().slice(0, 16).replace('T', ' ')} UTC: ${items.filter(merged).length} merged, ${items.filter((it) => !merged(it)).length} other.`,
    '',
    `## Merged (${items.filter(merged).length})`,
    '',
  ];
  for (const it of items.filter(merged)) {
    out.push(`- ${ref(it)}: ${it.prs.map((n) => `#${n}`).join(', ')}${it.commit ? ` \`${it.commit}\`` : ''}`);
  }
  const other = items.filter((it) => !merged(it));
  if (other.length) {
    out.push('', '## Not merged', '');
    for (const it of other) out.push(`- ${ref(it)}: ${it.status || 'no status'}`);
  }
  for (const [kind, title] of [
    ['alex', 'For Owner'],
    ['decisions', 'Decisions taken'],
    ['open', 'Open review findings (not fixed)'],
    ['untested', 'Untested by hand'],
  ]) {
    const rows = items.flatMap((it) => it[kind].map((e) => `- **${it.id}** (${lane(it)}): ${e}`));
    out.push('', `## ${title} (${rows.length})`, '');
    out.push(...(rows.length ? rows : ['- None recorded.']));
  }
  return `${out.join('\n')}\n`;
}

/** The `lane-*.md` reports in `dir`, by name. */
export function readReports(dir) {
  return readdirSync(dir)
    .filter((f) => /^lane-.*\.md$/.test(f))
    .sort()
    .map((f) => ({ name: f, text: readFileSync(join(dir, f), 'utf8') }));
}

/** Whether this file is the one node was asked to run (through a symlink too). */
function isMain() {
  try {
    return Boolean(process.argv[1]) && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
}

if (isMain()) {
  const args = process.argv.slice(2);
  const outAt = args.indexOf('--out');
  const out = outAt >= 0 ? args[outAt + 1] : undefined;
  const dir = args.find((a, i) => !a.startsWith('--') && !(outAt >= 0 && i === outAt + 1));
  const usage = 'usage: node scripts/overnight-summary.mjs <overnight folder> [--out <file>]';
  if (!dir || (outAt >= 0 && !out)) {
    console.error(usage);
    process.exit(2);
  }
  if (!existsSync(dir) || !statSync(dir).isDirectory()) {
    console.error(`${dir} is not a folder.\n${usage}`);
    process.exit(2);
  }
  const reports = readReports(dir);
  if (!reports.length) {
    console.error(`No lane-*.md in ${dir}.`);
    process.exit(1);
  }
  const day = /overnight-(\d{4}-\d{2}-\d{2})/.exec(basename(dir))?.[1] ?? basename(dir);
  const text = summarize(reports, day);
  if (out) writeFileSync(out, text);
  else process.stdout.write(text);
}
