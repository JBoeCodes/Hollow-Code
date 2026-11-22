/**
 * SOURCES.md and the workspace's source listing, read for the Sources cards
 * (Canvas C5b, plan §6). Pure, no DOM.
 *
 * `readSourcesManifest` reads SOURCES.md tolerantly: each `## <file>` is an
 * entry, and its `Label: value` lines (a list item or a plain line, the label
 * bold or not) give the recognised fields (Snapshot, Original, Origin,
 * Captured, SHA-256, Contents). Hollow's own attach writes that shape
 * (`workspaces.rs`); agents write it by hand, so other labels, prose, nested
 * lists and code stay as the entry's notes, exactly as written. A bullet
 * before any entry that starts with a file name (`- report.csv · finance
 * export`) is an entry too. As in the other section models every non-blank
 * line belongs to exactly one part, so nothing the file says is dropped.
 *
 * `sourceCardsModel` joins those entries with `WorkspaceDetail.sources`
 * (what Rust lists and measures): one card per listed file with its size,
 * coverage and freshness from the listing and its notes from SOURCES.md.
 * Freshness is only ever what `workspaces.rs` measured (a size and modified
 * time comparison); the checksum is the one SOURCES.md recorded at capture,
 * shown as that, never as a check of the file. Nothing here writes.
 */

import { headingOf, indentOf, isBlank, joinLines, linksOf, listItemOf, plainText, readSections, type SourceLine } from './sectionMarkdown';
import { SOURCE_COVERAGE, SOURCE_FRESHNESS } from './sourceCatalog';
import type { SourceContext } from './workspaceTypes';
import { sourceStateLine, type SourceState } from './sourceAuthority';

export interface SourcesSpan { line: number; endLine: number; markdown: string }

export type SourceFieldKey = 'snapshot' | 'original' | 'origin' | 'captured' | 'sha256' | 'contents';

export interface SourceField extends SourcesSpan {
  key: SourceFieldKey;
  /** The label as written ("SHA-256", "Origin"). */
  label: string;
  /** The text after the label, continuation lines kept on their own lines. */
  value: string;
}

export interface SourceEntry {
  /** The file the entry names: the heading (or the bullet's first token) as plain text. */
  name: string;
  /** `## name` with fields below it, or a `- name · note` bullet. */
  style: 'heading' | 'item';
  line: number;
  endLine: number;
  /** The whole entry as written, heading included. */
  markdown: string;
  /** Recognised fields, the first of each key, in file order. */
  fields: SourceField[];
  /** Everything else under the heading (other labels, prose, lists, code), exact. */
  notes: SourcesSpan[];
  /** The `Snapshot:` link's target, decoded (`sources/a b.md`). */
  snapshot: string | null;
  /** `Original:`, a JSON-quoted path unquoted. */
  original: string | null;
  /** `Origin:` (or `Source:`, `From:`) as written. */
  origin: string | null;
  /** `Captured:` (or `Fetched:`, `Retrieved:`, `Added:`) as written. */
  captured: string | null;
  /** `SHA-256:` when it is 64 hex digits, lowercased. Anything else stays a note. */
  sha256: string | null;
  /** `Contents:` as Markdown. */
  contents: string | null;
  /** A bullet entry's text after the file name, as Markdown. */
  detail: string | null;
}

export interface SourcesManifest {
  frontmatter: SourcesSpan | null;
  /** The first `# Title`. */
  title: string | null;
  titleLine: number | null;
  /** Text before the first entry heading (under the title, or with no heading), exact; bullet entries are taken out. */
  preamble: SourcesSpan[];
  /** Every entry in file order, headings and bullets. */
  entries: SourceEntry[];
  /** A later `# Heading` section, whole. */
  other: SourcesSpan[];
}

const LABELS: Record<string, SourceFieldKey> = {
  snapshot: 'snapshot', file: 'snapshot', copy: 'snapshot', localcopy: 'snapshot',
  original: 'original', originalpath: 'original', originalfile: 'original', originallocation: 'original',
  origin: 'origin', source: 'origin', from: 'origin', fetchedfrom: 'origin', provenance: 'origin', url: 'origin', link: 'origin',
  captured: 'captured', capturedat: 'captured', capturedon: 'captured', fetched: 'captured', fetchedon: 'captured', retrieved: 'captured', downloaded: 'captured', added: 'captured', snapshotdate: 'captured',
  sha256: 'sha256', sha: 'sha256', checksum: 'sha256', hash: 'sha256', sha256checksum: 'sha256', checksumsha256: 'sha256',
  contents: 'contents', content: 'contents', summary: 'contents', description: 'contents', covers: 'contents',
};

const BOLD_LABEL = /^(\*\*|__)([^*_\n]{1,48}?)(:?)\1(:?)[ \t]*(.*)$/;
const PLAIN_LABEL = /^([A-Za-z][\w ./()-]{0,47}?):(?:[ \t]+(.*)|[ \t]*)$/;

/** `Label: value` on a line (after an optional list marker): the label and the rest, or null. */
function labelOf(l: SourceLine): { label: string; rest: string; listed: boolean } | null {
  if (l.fenced || indentOf(l) > 3) return null;
  const item = listItemOf(l);
  if (item && item.indent > 1) return null;
  const text = item ? item.rest : l.text.trim();
  const bold = BOLD_LABEL.exec(text);
  if (bold && (bold[3] || bold[4])) return { label: bold[2].trim(), rest: bold[5], listed: !!item };
  const plain = PLAIN_LABEL.exec(text);
  if (plain) return { label: plain[1].trim(), rest: plain[2] ?? '', listed: !!item };
  return null;
}

const keyOf = (label: string): SourceFieldKey | null => LABELS[label.toLowerCase().replace(/[^a-z0-9]/g, '')] ?? null;
const unquote = (v: string) => v.trim().replace(/^`([^`]*)`$/, '$1').trim();

function snapshotOf(value: string): string | null {
  const href = linksOf(value)[0]?.href ?? unquote(value);
  if (!href) return null;
  let path = href.replace(/^<|>$/g, '').replace(/^\.\//, '');
  try { path = decodeURIComponent(path); } catch { /* not encoded: as written */ }
  return path || null;
}

function originalOf(value: string): string {
  const v = unquote(value);
  if (v.startsWith('"')) { try { const s = JSON.parse(v); if (typeof s === 'string') return s; } catch { /* as written */ } }
  return v;
}

const HEX64 = /^[0-9a-fA-F]{64}$/;

/** A bullet that starts with a file name: `- name.ext · note`, `` - `name` — note ``, `- [name](sources/name)`. */
const ITEM_NAME = /^(?:`([^`\n]+)`|\[([^\]\n]+)\]\(([^)\s]+)\)|([^\s`[\]]+\.[A-Za-z][A-Za-z0-9]{0,7}))(?=$|[\s:,;·—–(→])\s*(?:[:,;·—–→-]\s*)?(.*)$/;
const isFileName = (name: string, href?: string) => !name.includes('://') && (/\.[A-Za-z][A-Za-z0-9]{0,7}$/.test(name) || (!!href && /^(?:\.\/)?sources\//.test(href)));

function entryOf(name: string, style: SourceEntry['style'], line: number, endLine: number, markdown: string): SourceEntry {
  return { name, style, line, endLine, markdown, fields: [], notes: [], snapshot: null, original: null, origin: null, captured: null, sha256: null, contents: null, detail: null };
}

/** Lines `body` split into fields and exact note spans. */
function readEntryBody(entry: SourceEntry, body: SourceLine[]) {
  let note: SourceLine[] = [];
  const flush = () => {
    while (note.length && isBlank(note[note.length - 1])) note.pop();
    while (note.length && isBlank(note[0])) note.shift();
    if (note.length) entry.notes.push({ line: note[0].n, endLine: note[note.length - 1].n, markdown: joinLines(note) });
    note = [];
  };
  for (let i = 0; i < body.length; i++) {
    const l = body[i];
    const found = labelOf(l);
    const key = found && keyOf(found.label);
    if (!found || !key || entry.fields.some(f => f.key === key)) { note.push(l); continue; }
    // The field runs on through indented lines (a nested list, a wrapped
    // value), and through unindented lines that continue a plain line.
    let end = i;
    for (let j = i + 1; j < body.length; j++) {
      const n = body[j];
      if (isBlank(n)) {
        const next = body.slice(j + 1).find(x => !isBlank(x));
        if (next && indentOf(next) >= 2 && !next.fenced) continue;
        break;
      }
      if (indentOf(n) >= 2 || n.fenced) { end = j; continue; }
      if (listItemOf(n) || labelOf(n) || headingOf(n)) break;
      end = j;
    }
    const lines = body.slice(i, end + 1);
    const value = [found.rest, ...lines.slice(1).map(x => x.text.trim())].filter((x, k) => k === 0 || x !== '').join('\n').trim();
    if (key === 'sha256' && !HEX64.test(unquote(value))) { note.push(l); continue; }
    flush();
    entry.fields.push({ key, label: found.label, value, line: l.n, endLine: lines[lines.length - 1].n, markdown: joinLines(lines) });
    if (key === 'snapshot') entry.snapshot = snapshotOf(value);
    else if (key === 'original') entry.original = originalOf(value) || null;
    else if (key === 'origin') entry.origin = value || null;
    else if (key === 'captured') entry.captured = unquote(value) || null;
    else if (key === 'sha256') entry.sha256 = unquote(value).toLowerCase();
    else entry.contents = value || null;
    i = end;
  }
  flush();
}

/** Text before the entries: prose spans, with file-name bullets taken out as entries. */
function readPreamble(lines: SourceLine[], preamble: SourcesSpan[], entries: SourceEntry[]) {
  let prose: SourceLine[] = [];
  const flush = () => {
    while (prose.length && isBlank(prose[prose.length - 1])) prose.pop();
    while (prose.length && isBlank(prose[0])) prose.shift();
    if (prose.length) preamble.push({ line: prose[0].n, endLine: prose[prose.length - 1].n, markdown: joinLines(prose) });
    prose = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i];
    const item = listItemOf(l);
    const m = item && item.indent <= 1 && /^[-*+]$/.test(item.marker) ? ITEM_NAME.exec(item.rest) : null;
    if (!m || !isFileName((m[1] ?? m[2] ?? m[4]).trim(), m[3])) { prose.push(l); continue; }
    let end = i;
    for (let j = i + 1; j < lines.length && !isBlank(lines[j]) && (indentOf(lines[j]) >= 2 || lines[j].fenced || !listItemOf(lines[j])) && !headingOf(lines[j]); j++) end = j;
    flush();
    const span = lines.slice(i, end + 1);
    const name = (m[1] ?? m[2] ?? m[4]).trim();
    const e = entryOf(name, 'item', l.n, span[span.length - 1].n, joinLines(span));
    if (m[3]) e.snapshot = snapshotOf(`[${name}](${m[3]})`);
    e.detail = [m[5], ...span.slice(1).map(x => x.text.trim())].join('\n').trim() || null;
    entries.push(e);
    i = end;
  }
  flush();
}

/** SOURCES.md as entries, preamble and other sections. Nothing is dropped. */
export function readSourcesManifest(text: string): SourcesManifest {
  const doc = readSections(text);
  const out: SourcesManifest = { frontmatter: null, title: null, titleLine: null, preamble: [], entries: [], other: [] };
  if (doc.frontmatter) out.frontmatter = { line: doc.frontmatter.line, endLine: doc.frontmatter.endLine, markdown: joinLines(doc.lines.slice(0, doc.frontmatter.endLine)) };
  let seenEntry = false;
  for (const s of doc.sections) {
    if (s.level === 2) {
      seenEntry = true;
      const heading = s.heading ?? '';
      const e = entryOf(plainText(heading) || heading, 'heading', s.line, s.endLine, joinLines(doc.lines.slice(s.line - 1, s.endLine)));
      readEntryBody(e, s.body);
      out.entries.push(e);
    } else if (s.level === 1 && out.titleLine === null && !seenEntry) {
      out.title = plainText(s.heading ?? '') || null;
      out.titleLine = s.line;
      readPreamble(s.body, out.preamble, out.entries);
    } else if (s.level === 0) {
      readPreamble(s.body, out.preamble, out.entries);
    } else {
      out.other.push({ line: s.line, endLine: s.endLine, markdown: joinLines(doc.lines.slice(s.line - 1, s.endLine)) });
    }
  }
  return out;
}

// ── The join with the listing ──

export type OriginKind = 'local' | 'connector' | 'url' | 'written';

export interface SourceOrigin {
  kind: OriginKind;
  /** Plain text, never a link: a URL is shown, not opened. */
  text: string;
}

export type FreshnessTone = 'ok' | 'warn' | 'unknown';

export interface SourceCard {
  /** As listed, relative to the workspace (`sources/report.csv`). */
  path: string;
  /** The file name, and its folder under `sources/` when nested. */
  name: string;
  folder: string | null;
  sizeBytes: number;
  includedBytes: number;
  coverage: string;
  coverageLabel: string;
  coverageDetail: string;
  freshness: string;
  freshnessLabel: string;
  freshnessDetail: string;
  freshnessTone: FreshnessTone;
  /** SOURCES.md's Origin, else its Original, else the provenance path. */
  origin: SourceOrigin | null;
  /** The original's path (provenance first) when the origin text isn't it. */
  originalPath: string | null;
  captured: { value: string; from: 'provenance' | 'manifest' } | null;
  /** Recorded in SOURCES.md at capture; not a check of the file now. */
  sha256: string | null;
  /** The newest SOURCES.md entry for this file (the attach appends, so the last). */
  entry: SourceEntry | null;
  /** Older entries for the same file, file order. */
  earlier: SourceEntry[];
  /** What it is, in words, summary first (plan L3 X3, `sourceAuthority`). */
  state: SourceState;
  /** The recorded original's path (provenance, else SOURCES.md), for Open original. */
  original: string | null;
  /** A provenance record names its original, so Check for changes can compare them. */
  refreshable: boolean;
}

export interface SourceCounts {
  all: number;
  /** Sent with each turn: whole or in part. */
  inContext: number;
  excerpt: number;
  /** Over the context limit or needing a reader: not read as text. */
  notRead: number;
  /** The original changed or can't be found. */
  changed: number;
  /** No provenance to compare with. */
  untracked: number;
}

export interface SourceCardsModel {
  cards: SourceCard[];
  /** SOURCES.md entries no listed file claims, file order. */
  unmatched: SourceEntry[];
  counts: SourceCounts;
  /** Files in sources/ past the 100 listed (`sourcesOmitted`). */
  omitted: number;
}

const URL_RE = /\bhttps?:\/\/\S+/i;
const CONNECTOR_RE = /\bconnector\b|^(?:notion|google (?:drive|docs|sheets)|gmail|slack|github|linear|jira|confluence|outlook|microsoft 365|sharepoint|onedrive|dropbox|plaud)\b/i;
const PATH_RE = /^(?:~?\/|[A-Za-z]:[\\/])/;

/** What kind of origin a text names. Read from the text alone, never the file name. */
export function originKind(text: string): OriginKind {
  if (URL_RE.test(text)) return 'url';
  if (CONNECTOR_RE.test(text)) return 'connector';
  if (PATH_RE.test(text)) return 'local';
  return 'written';
}

export const ORIGIN_LABEL: Record<OriginKind, string> = { local: 'Local file', connector: 'Connector', url: 'Web address', written: 'Origin' };

export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return 'unknown size';
  if (n < 1024) return `${n.toLocaleString('en-US')} ${n === 1 ? 'byte' : 'bytes'}`;
  const units = ['KB', 'MB', 'GB'];
  let v = n / 1024, u = 0;
  while (v >= 1024 && u < units.length - 1) { v /= 1024; u++; }
  return `${v >= 10 ? Math.round(v) : Math.round(v * 10) / 10} ${units[u]}`;
}

/** workspaces.rs's chunk size (`source_evidence::CHUNK_BYTES`): a file this
 *  small is one passage, sent whole or not at all. */
const SOURCE_CHUNK_BYTES = 3000;

function coverageDetail(s: SourceContext): string {
  switch (s.coverage) {
    case 'complete': return s.totalBytes ? 'All of it is sent with each turn' : 'Empty file';
    // Attached sources are query-centred (agent effectiveness F3): each turn
    // gets the passages that best match its request, 24 KB across sources.
    case 'excerpt': case 'not_selected': return s.totalBytes <= SOURCE_CHUNK_BYTES
      ? 'Sent whole when it matches the request (24 KB of source text per turn)'
      : 'The passages that best match each request are sent (24 KB of source text per turn); agents can read the rest';
    case 'budget_exceeded': return 'Not sent: its matching passages didn’t fit the 24 KB of source text per turn';
    case 'requires_reader': return 'Not read as text: needs a runtime with a file reader, or a text export';
    default: return 'Coverage not reported';
  }
}

const FRESHNESS_DETAIL: Record<string, [string, FreshnessTone]> = {
  unchanged_metadata: ['Compared by size and modified time, not content', 'ok'],
  changed: ['The snapshot is what you attached; Check for changes shows what changed and can replace it', 'warn'],
  unavailable: ['The original is gone or can’t be read; the snapshot is unchanged', 'warn'],
  unknown: ['Nothing records where this file came from, so nothing compares it with an original', 'unknown'],
};

const norm = (s: string) => s.normalize('NFC').replace(/\s+/g, ' ').trim();
const baseName = (p: string) => p.split('/').pop() ?? p;

/**
 * One card per listed source, in the listing's order, with its SOURCES.md
 * entry. An entry is claimed by its `Snapshot:` link first, then by a name
 * equal to the path under `sources/`, then by the file name when only one
 * listed file has it (Hollow writes `[` and `]` in a name as spaces). Several
 * entries for one file: the last is shown, the rest are `earlier`.
 */
export function sourceCardsModel(manifest: SourcesManifest | null, sources: readonly SourceContext[], omitted = 0, now = new Date()): SourceCardsModel {
  const entries = manifest?.entries ?? [];
  const claimed = new Map<SourceEntry, string>();
  const byPath = new Map(sources.map(s => [norm(s.path), s.path]));
  const relOf = (p: string) => norm(p).replace(/^sources\//, '');
  const byRel = new Map(sources.map(s => [relOf(s.path), s.path]));
  const baseCount = new Map<string, string[]>();
  for (const s of sources) {
    for (const b of new Set([norm(baseName(s.path)), norm(baseName(s.path)).replace(/[[\]]/g, ' ')])) baseCount.set(b, [...(baseCount.get(b) ?? []), s.path]);
  }
  for (const e of entries) {
    const snap = e.snapshot ? norm(e.snapshot) : null;
    const hit = (snap && (byPath.get(snap) ?? byPath.get(`sources/${snap}`)))
      ?? byRel.get(norm(e.name))
      ?? (baseCount.get(norm(e.name))?.length === 1 ? baseCount.get(norm(e.name))![0] : undefined);
    if (hit) claimed.set(e, hit);
  }
  const forPath = new Map<string, SourceEntry[]>();
  for (const [e, p] of claimed) forPath.set(p, [...(forPath.get(p) ?? []), e]);
  const cards = sources.map((s): SourceCard => {
    const mine = (forPath.get(s.path) ?? []).sort((a, b) => a.line - b.line);
    const entry = mine.length ? mine[mine.length - 1] : null;
    const rel = s.path.replace(/^sources\//, '');
    const originText = entry?.origin ? plainText(entry.origin) : entry?.original ?? s.original ?? null;
    const originalPath = s.original ?? entry?.original ?? null;
    const fresh = FRESHNESS_DETAIL[s.freshness];
    return {
      path: s.path,
      name: baseName(rel),
      folder: rel.includes('/') ? rel.replace(/\/[^/]+$/, '') : null,
      sizeBytes: s.totalBytes,
      includedBytes: s.includedBytes,
      coverage: s.coverage,
      coverageLabel: SOURCE_COVERAGE[s.coverage] ?? s.coverage,
      coverageDetail: coverageDetail(s),
      freshness: s.freshness,
      freshnessLabel: SOURCE_FRESHNESS[s.freshness] ?? 'Freshness not reported',
      freshnessDetail: fresh?.[0] ?? '',
      freshnessTone: fresh?.[1] ?? 'unknown',
      origin: originText ? { kind: originKind(originText), text: originText } : null,
      originalPath: originalPath && originalPath !== originText ? originalPath : null,
      captured: s.captured ? { value: s.captured, from: 'provenance' } : entry?.captured ? { value: entry.captured, from: 'manifest' } : null,
      sha256: entry?.sha256 ?? null,
      entry,
      earlier: mine.slice(0, -1),
      state: sourceStateLine({ kind: 'snapshot', freshness: s.freshness, captured: s.captured ?? entry?.captured ?? null, original: originalPath }, now),
      original: originalPath,
      refreshable: !!s.original,
    };
  });
  const by = (f: (s: SourceContext) => boolean) => sources.filter(f).length;
  return {
    cards,
    unmatched: entries.filter(e => !claimed.has(e)),
    counts: {
      all: sources.length,
      inContext: by(s => s.coverage === 'complete' || s.coverage === 'excerpt'),
      excerpt: by(s => s.coverage === 'excerpt'),
      notRead: by(s => s.coverage === 'budget_exceeded' || s.coverage === 'requires_reader'),
      changed: by(s => s.freshness === 'changed' || s.freshness === 'unavailable'),
      untracked: by(s => s.freshness === 'unknown'),
    },
    omitted,
  };
}

/** `572af837…4d1e`: enough to compare by eye; the copy control copies all 64. */
export const shortSha = (sha: string) => `${sha.slice(0, 8)}…${sha.slice(-4)}`;
