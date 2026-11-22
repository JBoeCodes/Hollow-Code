#!/usr/bin/env node
// Every repository path the agent manual names must exist (merged plan slice 6
// item 6). The manual is AGENTS.md (the entry point; root CLAUDE.md only
// includes it with `@AGENTS.md`) plus docs/agents/*.md and docs/reference/*.md,
// since 2026-11-21 (ENG-674); before that it was CLAUDE.md plus
// docs/reference/*.md (the 2026-10-13 split, overnight S11).
//
//   node scripts/check-claude-paths.mjs        (also run by npm run test:scripts)
//
// The manual is read by every agent session, so a path it names that no longer
// exists sends agents looking for code that is gone. This finds the paths in
// inline code, fenced blocks and link targets and checks them against the
// repository's files (`git ls-files`, tracked plus untracked-but-not-ignored).
// Inline code and fenced paths are repository-relative wherever they are
// written; a link target is resolved against the folder of the doc it is in:
//
// - A path that starts with a top-level entry of the repository (`src/…`,
//   `docs/…`, `scripts/…`, `README.md`, …) must exist exactly, file or folder.
// - A bare or partial file name with a source extension (`chat.rs`,
//   `components/Rail.tsx`, `styles/ui.css`) must match the end of some file's
//   path. Inside fenced blocks (the layout tree) a token with a source
//   extension or a trailing `/` is matched that way, because the tree indents
//   instead of writing full paths.
// - Anything with a placeholder or glob (`{slug}`, `<id>`, `*`, `$`, `…`), an
//   absolute or home path, a URL, or a runtime file name without a source
//   extension (`budgets.json`, `WORKSPACE.md`) is not a repository path and is
//   skipped.
//
// It also checks that every `AGENTS.md#anchor` mention in README.md, the docs
// under docs/ (not the build log or docs/history/) and the manual names a
// heading AGENTS.md still has, that none of them links `CLAUDE.md#anchor` (that
// file is a one-line include with no headings: link AGENTS.md or the doc that
// holds the section), and that every Markdown link with a `#fragment` into a manual doc,
// from those docs or from the manual itself (`#x` within one doc included),
// names a heading that doc has, so moving a section can't silently break
// those pointers.
//
// Paths the manual names on purpose although they don't exist (deleted
// modules, examples, files in the user's vault) go in ALLOWED with the reason.
// The test fails on an ALLOWED entry that no manual doc names any more or that
// exists, so the list can't go stale.
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync } from 'node:fs';
import { join, posix, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));

export const SOURCE_EXT = /\.(rs|ts|tsx|mjs|js|css|py)$/;
export const ENTRY = 'AGENTS.md';
export const AGENTS_DIR = 'docs/agents';
export const REFERENCE_DIR = 'docs/reference';
/** The entry point is read by every session; house rule: under 15 KB. */
export const ENTRY_MAX_BYTES = 15_000;

/** Not repository paths, named on purpose. Keep each entry's reason. */
export const ALLOWED = new Map([
  ['src-tauri/src/foo.rs', 'example name in Common tasks → Add a new connector'],
  ['src/pages/MyPage.tsx', 'example name in Common tasks → Add a new top-level page'],
  ['.claude/CLAUDE.md', "an agent-instruction file in the user's vault that run_guard flags"],
]);

/** Repository files and folders, repo-relative with `/` separators. */
export function listRepoPaths(dir = root) {
  let files;
  try {
    files = execFileSync('git', ['ls-files', '-z', '--cached', '--others', '--exclude-standard'], {
      cwd: dir,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
    })
      .split('\0')
      .filter(Boolean);
  } catch {
    files = walk(dir, dir);
  }
  const dirs = new Set();
  for (const f of files) {
    const parts = f.split('/');
    for (let i = 1; i < parts.length; i++) dirs.add(parts.slice(0, i).join('/'));
  }
  return { files: new Set(files), dirs };
}

const SKIP_DIRS = new Set(['.git', 'node_modules', 'target', 'dist', 'gen', 'test-results', 'playwright-report']);
function walk(dir, base, out = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (SKIP_DIRS.has(entry.name)) continue;
    const full = join(dir, entry.name);
    if (entry.isDirectory()) walk(full, base, out);
    else out.push(relative(base, full).split(sep).join('/'));
  }
  return out;
}

/** Strip the punctuation and suffixes that surround a path in prose. */
export function cleanToken(raw) {
  let t = raw.trim();
  t = t.replace(/^[("'[]+/, '').replace(/[)"'\],;:.]+$/, '');
  t = t.replace(/'s$/, '');
  t = t.split('::')[0];
  t = t.split('#')[0];
  t = t.replace(/:\d+(-\d+)?$/, '');
  if (t.startsWith('./')) t = t.slice(2);
  return t;
}

const PLACEHOLDER = /[{}<>*$~…|\\]/;

/** A link target as a repository path: resolved against `file`'s folder, fragment kept. */
export function resolveLink(target, file = ENTRY) {
  if (/^[a-z]+:/i.test(target) || target.startsWith('<') || target.startsWith('/')) return target;
  const hash = target.indexOf('#');
  const path = hash < 0 ? target : target.slice(0, hash);
  if (!path) return target;
  let resolved = posix.normalize(posix.join(posix.dirname(file), path));
  if (path.endsWith('/') && !resolved.endsWith('/')) resolved += '/';
  return resolved + (hash < 0 ? '' : target.slice(hash));
}

/**
 * Path candidates in a Markdown text: `{ path, fenced, line }` for every token
 * that looks like a repository path. Rules in the header comment. `file` is
 * where the text lives, for resolving link targets.
 */
export function extractPaths(markdown, topLevel, file = ENTRY) {
  const found = [];
  const consider = (raw, fenced, line) => {
    if (/^[a-z]+:\/\//i.test(raw.trim()) || /^(mailto|tel):/i.test(raw.trim())) return;
    if (PLACEHOLDER.test(raw)) return;
    const t = cleanToken(raw);
    if (!t || t.startsWith('/') || t.startsWith('-') || !/^[\w.@/-]+$/.test(t)) return;
    if (!/[A-Za-z]/.test(t)) return;
    const first = t.split('/')[0];
    const rooted = topLevel.has(first) && (t.includes('/') || topLevel.has(t));
    if (fenced) {
      if (SOURCE_EXT.test(t) || raw.replace(/[),;:.]+$/, '').endsWith('/')) {
        found.push({ path: t, fenced: true, line });
      }
      return;
    }
    if (rooted || SOURCE_EXT.test(t)) found.push({ path: t, fenced: false, line });
  };
  const lines = markdown.split('\n');
  let fence = false;
  lines.forEach((text, i) => {
    const line = i + 1;
    if (text.trim().startsWith('```')) {
      fence = !fence;
      return;
    }
    if (fence) {
      for (const tok of text.split(/\s+/)) consider(tok, true, line);
      return;
    }
    for (const m of text.matchAll(/`([^`]+)`/g)) {
      for (const tok of m[1].split(/\s+/)) consider(tok, false, line);
    }
    for (const m of text.matchAll(/\]\(([^)\s]+)\)/g)) consider(resolveLink(m[1], file), false, line);
  });
  return found;
}

function resolves(candidate, { files, dirs }, topLevel) {
  const p = candidate.path.replace(/\/$/, '');
  const first = p.split('/')[0];
  if (!candidate.fenced && topLevel.has(first)) return files.has(p) || dirs.has(p);
  if (files.has(p) || dirs.has(p)) return true;
  const tail = `/${p}`;
  for (const f of files) if (f.endsWith(tail)) return true;
  for (const d of dirs) if (d.endsWith(tail)) return true;
  return false;
}

/** Paths in one manual doc that don't exist (ALLOWED ones skipped). */
export function pathProblems({ markdown, repo, allowed = ALLOWED, file = ENTRY }) {
  const topLevel = new Set([...repo.files, ...repo.dirs].map((p) => p.split('/')[0]));
  const problems = [];
  const seen = new Set();
  for (const c of extractPaths(markdown, topLevel, file)) {
    if (allowed.has(c.path)) continue;
    if (resolves(c, repo, topLevel)) continue;
    const key = `${c.path}@${c.line}`;
    if (seen.has(key)) continue;
    seen.add(key);
    problems.push(`${file}:${c.line}: \`${c.path}\` does not exist in the repository`);
  }
  return problems;
}

/** ALLOWED entries that `text` (the whole manual) no longer names, or that exist now. */
export function allowedProblems({ text, repo, allowed = ALLOWED }) {
  const topLevel = new Set([...repo.files, ...repo.dirs].map((p) => p.split('/')[0]));
  const problems = [];
  for (const [path, reason] of allowed) {
    if (!text.includes(path)) problems.push(`ALLOWED entry \`${path}\` (${reason}) is no longer named in AGENTS.md, docs/agents/ or docs/reference/; remove it`);
    else if (resolves({ path, fenced: false }, repo, topLevel)) {
      problems.push(`ALLOWED entry \`${path}\` (${reason}) exists now; remove it from ALLOWED`);
    }
  }
  return problems;
}

/** Problems with one doc's paths, or with the ALLOWED list itself. */
export function checkClaudePaths({ markdown, repo, allowed = ALLOWED, file = ENTRY }) {
  return [...pathProblems({ markdown, repo, allowed, file }), ...allowedProblems({ text: markdown, repo, allowed })];
}

/** The same over the whole manual: `docs` maps repo path → text. */
export function checkManualPaths({ docs, repo, allowed = ALLOWED }) {
  const problems = [];
  for (const [file, markdown] of Object.entries(docs)) problems.push(...pathProblems({ markdown, repo, allowed, file }));
  problems.push(...allowedProblems({ text: Object.values(docs).join('\n'), repo, allowed }));
  return problems;
}

/** GitHub's heading anchors for a Markdown text (duplicates get -1, -2, …). */
export function headingAnchors(markdown) {
  const anchors = new Set();
  const seen = new Map();
  let fence = false;
  for (const line of markdown.split('\n')) {
    if (line.trim().startsWith('```')) fence = !fence;
    const m = !fence && /^#{1,6} (.*)$/.exec(line);
    if (!m) continue;
    const slug = m[1].trim().toLowerCase().replace(/[^\p{L}\p{N}\s_-]/gu, '').replace(/ /g, '-');
    const n = seen.get(slug) ?? 0;
    anchors.add(n ? `${slug}-${n}` : slug);
    seen.set(slug, n + 1);
  }
  return anchors;
}

/**
 * Mentions of the form `AGENTS.md#anchor` in `sources` whose anchor the entry
 * point (`markdown`) lacks, and every `CLAUDE.md#anchor` (CLAUDE.md is a
 * one-line `@AGENTS.md` include with no headings of its own).
 */
export function checkClaudeAnchors({ markdown, sources }) {
  const anchors = headingAnchors(markdown);
  const problems = [];
  for (const [file, text] of Object.entries(sources)) {
    text.split('\n').forEach((line, i) => {
      for (const m of line.matchAll(/AGENTS\.md#([\w-]+)/g)) {
        if (!anchors.has(m[1])) problems.push(`${file}:${i + 1}: AGENTS.md has no heading for #${m[1]}`);
      }
      for (const m of line.matchAll(/CLAUDE\.md#([\w-]+)/g)) {
        problems.push(`${file}:${i + 1}: CLAUDE.md#${m[1]}: CLAUDE.md only includes AGENTS.md; link AGENTS.md or the doc that holds the section`);
      }
    });
  }
  return problems;
}

/**
 * Markdown links with a fragment into a manual doc (`docs`, by repo path)
 * whose fragment that doc has no heading for. `sources` (repo path → text)
 * are the files whose links are checked; `#x` alone points into the source
 * itself, which counts only when it is a manual doc.
 */
export function checkManualAnchors({ docs, sources }) {
  const anchors = new Map(Object.entries(docs).map(([f, t]) => [f, headingAnchors(t)]));
  const problems = [];
  for (const [file, text] of Object.entries(sources)) {
    let fence = false;
    text.split('\n').forEach((line, i) => {
      if (line.trim().startsWith('```')) fence = !fence;
      if (fence) return;
      for (const m of line.matchAll(/\]\(([^)\s]*#[^)\s]+)\)/g)) {
        const target = resolveLink(m[1], file);
        const hash = target.indexOf('#');
        const path = hash === 0 ? file : target.slice(0, hash);
        const frag = target.slice(hash + 1);
        const have = anchors.get(path);
        if (have && !have.has(frag)) problems.push(`${file}:${i + 1}: ${path} has no heading for #${frag}`);
      }
    });
  }
  return problems;
}

/** The manual: AGENTS.md, every docs/agents/*.md and every docs/reference/*.md, by repo path. */
export function manualDocs(dir = root) {
  const out = { [ENTRY]: readFileSync(join(dir, ENTRY), 'utf8') };
  for (const sub of [AGENTS_DIR, REFERENCE_DIR]) {
    let names = [];
    try {
      names = readdirSync(join(dir, sub)).sort();
    } catch {
      names = [];
    }
    for (const f of names) if (f.endsWith('.md')) out[`${sub}/${f}`] = readFileSync(join(dir, sub, f), 'utf8');
  }
  return out;
}

/**
 * The docs whose links into the manual must resolve: AGENTS.md, README.md and
 * every Markdown file under docs/ except the build log and docs/history/.
 */
export function linkingDocs(dir = root) {
  const out = {};
  for (const f of ['AGENTS.md', 'README.md']) out[f] = readFileSync(join(dir, f), 'utf8');
  const walkDocs = (rel) => {
    for (const e of readdirSync(join(dir, rel), { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const path = `${rel}/${e.name}`;
      if (e.isDirectory()) {
        if (path !== 'docs/history') walkDocs(path);
      } else if (e.isFile() && e.name.endsWith('.md') && path !== 'docs/BUILD_LOG.md') {
        out[path] = readFileSync(join(dir, path), 'utf8');
      }
    }
  };
  walkDocs('docs');
  return out;
}

function main() {
  const docs = manualDocs(root);
  const markdown = docs[ENTRY];
  const sources = linkingDocs(root);
  const repo = listRepoPaths(root);
  const problems = [
    ...new Set([
      ...checkManualPaths({ docs, repo }),
      ...checkClaudeAnchors({ markdown, sources: { ...sources, ...docs } }),
      ...checkManualAnchors({ docs, sources: { ...sources, ...docs } }),
    ]),
  ];
  if (problems.length) {
    for (const p of problems) console.error(p);
    console.error(`${problems.length} path problem(s) in the agent manual`);
    process.exitCode = 1;
  } else {
    const topLevel = new Set([...repo.files, ...repo.dirs].map((p) => p.split('/')[0]));
    const n = Object.entries(docs).reduce((sum, [f, t]) => sum + extractPaths(t, topLevel, f).length, 0);
    console.log(`${ENTRY} + ${Object.keys(docs).length - 1} docs/agents and docs/reference docs: ${n} repository paths checked, all exist; every link to a manual heading from README.md, docs/ (not the build log or history) and the manual resolves`);
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
