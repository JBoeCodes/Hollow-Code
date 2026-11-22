// Release notes for a channel build (updates design section 5, step 4, row
// U2): what a build brings over the previous one. A port of the parts of
// src-tauri/src/install_brings.rs the release builder needs, so the notes a
// Nightly carries read like the install card's list:
//
//   - `git log --first-parent <from>..<to>`: merges titled `Merge #N: ...` or
//     `Merge pull request #N ...` are pull requests (title from the subject or
//     the merge body), everything else a direct commit (`prOf`, as `pr_of`).
//   - each PR's "not checked by hand" sentences from its docs/BUILD_LOG.md
//     entry at <to>, matched by `(overnight X)` tag or title (`notesFor`, as
//     `notes_for` and `untested_lines`).
//   - plus what install_brings doesn't read: the docs/WHATS_NEW.md entries
//     (`### Title` with its `**What:**` line) present at <to> and not at
//     <from>.
//
// Pure apart from the injected `git(args)`, which returns stdout or throws.

/** Rows kept, newest first (install_brings LIMIT). */
export const LIMIT = 120;
/** Untested notes kept per PR (install_brings NOTES). */
export const NOTES = 3;
/** First Nightly (no previous build known): this many newest changes. */
export const FIRST_RUN_COUNT = 20;
/** The manifest's `notes` string is cut to this many characters. */
export const MAX_TEXT = 4000;

/** `Merge #451: Title` / `Merge pull request #451 from o/b` (title in the body). */
export function prOf(subject, body = '') {
  if (subject.startsWith('Merge #')) {
    const rest = subject.slice('Merge #'.length);
    const colon = rest.indexOf(':');
    if (colon < 0) return null;
    const n = rest.slice(0, colon).trim();
    if (!/^\d+$/.test(n)) return null;
    return { number: Number(n), title: rest.slice(colon + 1).trim() };
  }
  if (!subject.startsWith('Merge pull request #')) return null;
  const n = subject.slice('Merge pull request #'.length).split(/\s+/)[0];
  if (!/^\d+$/.test(n ?? '')) return null;
  const title = body.split('\n').map(l => l.trim()).find(l => l !== '') ?? subject;
  return { number: Number(n), title };
}

const STARTS = /^(not (yet )?(checked|tried|tested|run|verified|exercised|measured)|untested|nobody (tried|checked)|never (tried|run|tested)|no (dev server|test bed|live screen reader))\b/i;
const NEGATED = /\b(wasn't|weren't|isn't|aren't|didn't|hasn't been|haven't been|was not|were not) (run|checked|tried|tested|exercised|measured)\b/i;
const LABEL = /^[A-Z][A-Za-z ]{0,24}:\s+/;

/** Sentences of a build-log entry that say something wasn't checked or run. */
export function untestedLines(entry) {
  const items = [];
  for (const line of entry.split('\n').slice(1)) {
    const t = line.trim();
    if (t === '' || t.startsWith('#')) { items.push(''); continue; }
    const bullet = t.startsWith('- ') || t.startsWith('* ');
    const text = t.replace(/^[-*]+/, '').trim().replaceAll('**', '');
    const last = items.length ? items[items.length - 1] : undefined;
    if (!bullet && last !== undefined && last !== '') items[items.length - 1] = `${last} ${text}`;
    else items.push(text);
  }
  const out = [];
  for (const item of items.filter(i => i !== '')) {
    // Rust's split_inclusive(". "): each piece keeps its ". ".
    const sentences = item.split(/(?<=\. )/).map(s => s.trim());
    for (const sentence of sentences) {
      const body = sentence.replace(LABEL, '');
      if (STARTS.test(body) || NEGATED.test(sentence)) {
        const chars = [...sentence];
        out.push(chars.length > 240 ? `${chars.slice(0, 239).join('')}\u2026` : sentence);
        if (out.length === NOTES) return out;
      }
    }
  }
  return out;
}

/** `S1`, `N4`... in a text's last parenthetical, when it names an overnight item. */
function tags(text) {
  const open = text.lastIndexOf('(');
  if (open < 0) return [];
  const inside = text.slice(open);
  if (!inside.includes('overnight')) return [];
  return inside.split(/[^A-Za-z0-9]/).filter(w => w.length >= 2 && w.length <= 3 && /^[A-Z][0-9]+$/.test(w));
}

function loose(text) {
  return text.toLowerCase().replace(/[\u2014\u2013]/g, ':').split(/\s+/).filter(Boolean).join(' ').replaceAll(' :', ':');
}

/** The build-log entry for a PR: `{ found, untested }`. */
export function notesFor(entries, title) {
  const want = tags(title);
  const key = [...loose(title)].slice(0, 30).join('');
  const hit = entries.find(e => {
    const head = e.split('\n')[0] ?? '';
    if (want.length) return tags(head).some(h => want.includes(h));
    const h = loose(head);
    const dot = h.indexOf(' \u00b7 ');
    const rest = dot >= 0 ? h.slice(dot + 3) : h;
    return key !== '' && rest.startsWith(key);
  });
  return hit === undefined ? { found: false, untested: [] } : { found: true, untested: untestedLines(hit) };
}

/** `### Title` entries of a WHATS_NEW.md text with their `**What:**` line. */
export function whatsNewEntries(text) {
  const out = [];
  let current = null;
  for (const line of (text ?? '').split('\n')) {
    const heading = line.match(/^###\s+(.+?)\s*$/);
    if (heading) { current = { title: heading[1], what: '' }; out.push(current); continue; }
    if (/^#{1,2}\s/.test(line)) { current = null; continue; }
    const what = line.match(/^\s*-\s+\*\*What:\*\*\s*(.*)$/);
    if (current && what && current.what === '') current.what = what[1].trim();
  }
  return out;
}

const show = (git, rev, path) => {
  try { return git(['show', `${rev}:${path}`]); } catch { return ''; }
};

/**
 * What `to` brings over `from` (null: the first build, so the newest
 * FIRST_RUN_COUNT first-parent changes). `git` runs in the release checkout.
 */
export function collectNotes({ git, from, to }) {
  const notes = { from: from ?? null, to, pullRequests: [], commits: [], whatsNew: [], note: null };
  let base = from;
  if (base) {
    let ancestor = true;
    try { git(['merge-base', '--is-ancestor', base, to]); } catch { ancestor = false; }
    if (!ancestor) {
      notes.note = `The previous build's commit ${base.slice(0, 8)} isn't on this one's history, so the newest ${FIRST_RUN_COUNT} changes are listed.`;
      base = null;
    }
  } else {
    notes.note = `No previous build is known, so the newest ${FIRST_RUN_COUNT} changes are listed.`;
  }
  const args = base
    ? ['log', '--first-parent', `--max-count=${LIMIT + 1}`, '--format=%H%x1f%s%x1f%cI%x1f%b%x1e', `${base}..${to}`, '--']
    : ['log', '--first-parent', `--max-count=${FIRST_RUN_COUNT}`, '--format=%H%x1f%s%x1f%cI%x1f%b%x1e', to, '--'];
  const records = git(args).split('\x1e').map(s => s.trim()).filter(Boolean);
  if (base && records.length > LIMIT) notes.note = `Only the newest ${LIMIT} are listed; there are more.`;
  const entries = show(git, to, 'docs/BUILD_LOG.md').split('\n## ');
  for (const rec of records.slice(0, LIMIT)) {
    const [sha = '', subject = '', at = '', body = ''] = rec.split('\x1f');
    const pr = prOf(subject, body);
    if (pr) {
      const { found, untested } = notesFor(entries, pr.title);
      notes.pullRequests.push({ number: pr.number, title: pr.title, merge: sha.slice(0, 8), mergedAt: at, untested, entryFound: found });
    } else {
      notes.commits.push({ sha: sha.slice(0, 8), subject, at });
    }
  }
  const before = new Set(base ? whatsNewEntries(show(git, base, 'docs/WHATS_NEW.md')).map(e => `${e.title}\n${e.what}`) : []);
  if (base) notes.whatsNew = whatsNewEntries(show(git, to, 'docs/WHATS_NEW.md')).filter(e => !before.has(`${e.title}\n${e.what}`));
  return notes;
}

/** The notes as plain text for the manifest's `notes`, at most `max` characters. */
export function notesText(notes, max = MAX_TEXT) {
  const lines = [];
  if (notes.whatsNew.length) {
    lines.push("What's new:");
    for (const e of notes.whatsNew) lines.push(`- ${e.title}${e.what ? `: ${e.what}` : ''}`);
  }
  if (notes.pullRequests.length) {
    if (lines.length) lines.push('');
    lines.push('Changes:');
    for (const pr of notes.pullRequests) lines.push(`- #${pr.number} ${pr.title}`);
  }
  if (notes.commits.length) {
    if (lines.length) lines.push('');
    lines.push('Direct commits:');
    for (const c of notes.commits) lines.push(`- ${c.subject}`);
  }
  if (!lines.length) lines.push('No changes listed.');
  const text = lines.join('\n');
  if ([...text].length <= max) return text;
  const tail = '\n(more in notes.json)';
  return `${[...text].slice(0, max - tail.length - 1).join('')}\u2026${tail}`;
}
