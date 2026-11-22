// Hashed identifier denylist (BG3, 2026-11-16): keeps client, employee and
// other named identifiers out of the repo's tracked files without publishing
// them. `scripts/identifier-denylist.sha256` holds only SHA-256 values; the
// plaintext names stay in a gitignored local file of the person's own.
//
// How a match works: every one-word and two-word shingle of every tracked text
// file (words read across the whole file, so a name a hard wrap split still
// matches) is normalised (NFKC) and lowercased, its words joined by one space,
// hashed and looked up in the list. A hit prints the file, line, column and the first eight hex digits
// of the hash, never the text. A missing, empty or malformed list fails closed
// (exit 2): a guard with nothing to match must not look like a pass.
//
// Cautions (the idea's own): a short or common name false-hits, and a hash of
// a short name can be brute forced, so this stops casual reading, not an
// attacker. `--hash` refuses a one-word name under 4 characters and anything
// over two words (list the two most distinctive words instead).
//
//   node scripts/identifier-guard.mjs                 scan this repo's tracked files
//   node scripts/identifier-guard.mjs --hash < names.txt >> scripts/identifier-denylist.sha256
//                                                     one name per line in, hashes out
//
// Ideas only from BugHunter's identifier guard; no code taken.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, existsSync, realpathSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO = join(dirname(fileURLToPath(import.meta.url)), '..');
export const DENYLIST = join(REPO, 'scripts', 'identifier-denylist.sha256');
/** Files bigger than this are reported as skipped, never silently passed. */
export const MAX_BYTES = 8 * 1024 * 1024;
/** A one-word shingle shorter than this is not looked up (it would hit common words). */
export const MIN_WORD = 3;

// Letters, combining marks and digits: a mark stays with its letter, so a
// name typed decomposed (e + U+0301) is one word.
const WORD = /[\p{L}\p{M}\p{N}]+/gu;

/** A word folded the way the list is made: compatibility-normalised, lowercase. */
export const fold = word => word.normalize('NFKC').toLowerCase();

export function hashPhrase(phrase) {
  return createHash('sha256').update(phrase, 'utf8').digest('hex');
}

/** The words of a name or phrase, folded. */
export function wordsOf(text) {
  return [...text.matchAll(WORD)].map(m => fold(m[0]));
}

/**
 * The set of hashes in a list file's text: one 64-digit hex value per line,
 * `#` comments and blank lines allowed, a trailing ` # note` allowed. Throws
 * (fail closed) on a malformed line or an empty list.
 */
export function parseDenylist(text) {
  const out = new Set();
  text.split('\n').forEach((raw, i) => {
    const line = raw.replace(/#.*$/, '').trim();
    if (!line) return;
    if (!/^[0-9a-f]{64}$/.test(line)) throw new Error(`denylist line ${i + 1} is not a SHA-256 value`);
    out.add(line);
  });
  if (out.size === 0) throw new Error('the denylist has no entries: an empty list would pass everything, so it fails closed');
  return out;
}

export function loadDenylist(path = DENYLIST) {
  if (!existsSync(path)) throw new Error(`the denylist ${path} is missing: a missing list fails closed`);
  return parseDenylist(readFileSync(path, 'utf8'));
}

/**
 * Hits of `hashes` in `text` as `{ line, column, hash }` (1-based; the column
 * is the shingle's first character in UTF-16 units of the original text).
 * Words are read across the whole text, so a name a hard wrap split over two
 * lines (or over a comment marker such as `//` or `>`) still matches; two words
 * a blank line apart do not. `memo` caches phrase hashes across calls, since a
 * repo repeats the same words constantly.
 */
export function scanText(text, hashes, memo = new Map()) {
  const hit = phrase => {
    let known = memo.get(phrase);
    if (known === undefined) {
      const h = hashPhrase(phrase);
      known = hashes.has(h) ? h : '';
      memo.set(phrase, known);
    }
    return known;
  };
  // Where each line starts, to turn an offset into a line and column.
  const starts = [0];
  for (let i = text.indexOf('\n'); i !== -1; i = text.indexOf('\n', i + 1)) starts.push(i + 1);
  let row = 0;
  const place = offset => {
    while (row + 1 < starts.length && starts[row + 1] <= offset) row++;
    return { line: row + 1, column: offset - starts[row] + 1 };
  };
  const words = [...text.matchAll(WORD)];
  const out = [];
  for (let w = 0; w < words.length; w++) {
    const one = fold(words[w][0]);
    const at = words[w].index;
    if (one.length >= MIN_WORD) {
      const h = hit(one);
      if (h) out.push({ ...place(at), hash: h });
    }
    if (w + 1 < words.length) {
      const gap = text.slice(at + words[w][0].length, words[w + 1].index);
      // At most one line break between the two words (a wrap), never a blank line.
      if ((gap.match(/\n/g) ?? []).length <= 1) {
        const h = hit(`${one} ${fold(words[w + 1][0])}`);
        if (h) out.push({ ...place(at), hash: h });
      }
    }
  }
  return out;
}

/** True when the bytes look like text: no NUL in the first 8 KB. */
export function looksText(buf) {
  return !buf.subarray(0, 8192).includes(0);
}

/**
 * Scans `files` (repo-relative) read through `read(file) -> Buffer | null`.
 * Returns `{ hits: ['path:line:col hash8'], skipped: [path and why], scanned, binary }`:
 * a file that was not scanned (unreadable or over the size cap) is named in
 * `skipped`, never passed in silence; a binary file (a NUL byte in its first
 * 8 KB and no UTF-16 mark) is only counted in `binary`; a UTF-16 text file with
 * its mark is decoded and scanned. A
 * file `read` returns `undefined` for is one that is gone (deleted in the
 * working tree) and is ignored.
 */
export function scanFiles(files, read, hashes) {
  const memo = new Map();
  const hits = [];
  const skipped = [];
  let scanned = 0;
  let binary = 0;
  for (const file of files) {
    const buf = read(file);
    if (buf === undefined) continue;
    if (buf === null) {
      skipped.push(`${file} (could not be read)`);
      continue;
    }
    if (buf.length > MAX_BYTES) {
      skipped.push(`${file} (over ${MAX_BYTES} bytes)`);
      continue;
    }
    let text;
    if (buf.length >= 2 && buf[0] === 0xff && buf[1] === 0xfe) text = buf.subarray(2).toString('utf16le');
    else if (buf.length >= 2 && buf[0] === 0xfe && buf[1] === 0xff) text = Buffer.from(buf.subarray(2)).swap16().toString('utf16le');
    else if (!looksText(buf)) {
      // An image, an archive: a NUL byte in the first 8 KB. Counted, not scanned.
      binary++;
      continue;
    } else text = buf.toString('utf8');
    scanned++;
    for (const h of scanText(text, hashes, memo)) {
      hits.push(`${file}:${h.line}:${h.column} ${h.hash.slice(0, 8)}`);
    }
  }
  return { hits, skipped, scanned, binary };
}

export function trackedFiles(repo = REPO) {
  return execFileSync('git', ['-C', repo, 'ls-files', '-z'], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }).split('\0').filter(Boolean);
}

/**
 * One name per line to a hash per line. A name must be one or two words, a
 * single word at least 4 characters; anything else is refused with the reason.
 */
export function hashNames(text) {
  const out = [];
  const refused = [];
  text.split('\n').forEach((raw, i) => {
    const name = raw.trim();
    if (!name || name.startsWith('#')) return;
    const words = wordsOf(name);
    if (words.length === 0 || words.length > 2) refused.push(`line ${i + 1}: use one or two words (the two most distinctive of a longer name)`);
    else if (words.length === 1 && words[0].length < 4) refused.push(`line ${i + 1}: a one-word name needs 4 or more characters`);
    else out.push(hashPhrase(words.join(' ')));
  });
  return { hashes: out, refused };
}

function main(argv) {
  if (argv.includes('--hash')) {
    const { hashes, refused } = hashNames(readFileSync(0, 'utf8'));
    for (const r of refused) process.stderr.write(`${r}\n`);
    for (const h of hashes) process.stdout.write(`${h}\n`);
    return refused.length ? 1 : 0;
  }
  let hashes;
  try {
    hashes = loadDenylist();
  } catch (e) {
    process.stderr.write(`identifier guard: ${e.message}\n`);
    return 2;
  }
  const read = file => {
    try {
      return readFileSync(join(REPO, file));
    } catch (e) {
      return e.code === 'ENOENT' ? undefined : null;
    }
  };
  const { hits, skipped, scanned, binary } = scanFiles(trackedFiles(), read, hashes);
  for (const s of skipped) process.stderr.write(`identifier guard: not scanned: ${s}\n`);
  if (skipped.length) {
    process.stderr.write('identifier guard: some files were not scanned, so a clean result would not be a pass (fail closed)\n');
    return 2;
  }
  if (hits.length) {
    process.stderr.write(`identifier guard: ${hits.length} listed identifier(s) in tracked files (path:line:column hash prefix; the name is never printed):\n`);
    for (const h of hits) process.stderr.write(`  ${h}\n`);
    return 1;
  }
  process.stdout.write(`identifier guard: ${scanned} text files (${binary} binary skipped), ${hashes.size} listed identifiers, no hits\n`);
  return 0;
}

const isMain = () => {
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
};
if (process.argv[1] && isMain()) process.exit(main(process.argv.slice(2)));
