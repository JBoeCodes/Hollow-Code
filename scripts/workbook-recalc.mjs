// Headless formula recalculation check for agent-built workbooks (UN2, 2026-11-18).
//
// An agent that writes an .xlsx (the invoice-valet skill, for one) can leave a
// #REF!, a total that skips a row, or a cached value that disagrees with its
// own formula, and nothing in Hollow recalculates formulas: the app reads
// cached values (`xlsx_cells.rs`). This reads the workbook itself, recomputes
// every formula it understands and compares.
//
//   node scripts/workbook-recalc.mjs <book.xlsx> [--expect 'Sheet!B10=1234.56']... [--json]
//
// On the Mac mini, from the repository checkout (the job folder holds only what
// is sent, so the script goes too):
//   hollow-slot --remote --send scripts/workbook-recalc.mjs --send <book.xlsx> node scripts/workbook-recalc.mjs <book.xlsx>
//
// Findings: an error value cached or computed (#REF!, #DIV/0!, #NAME?, #VALUE!,
// #N/A, #NUM!, #NULL!), a formula whose text holds #REF!, a circular
// reference, a recomputed value that differs from the cached `<v>` (numbers
// within max(0.005, 1e-9 x |value|) count as equal), and each `--expect` whose
// cell recomputes to another number (the invoice's stated total; books written
// by openpyxl usually carry no cached values, so only `--expect` catches a
// wrong total there). A formula with no cached value is counted, not flagged.
//
// Supported: numbers, strings, booleans, + - * / ^ & unary minus, percent,
// comparisons, parentheses, A1 and $A$1 references, ranges (also A:A and 1:1),
// sheet-qualified references ('My Sheet'!A1:B2), shared formulas, and SUM,
// AVERAGE, MIN, MAX, COUNT, COUNTA, PRODUCT, ROUND, ROUNDUP, ROUNDDOWN, ABS,
// IF, IFERROR, AND, OR, NOT, SUMPRODUCT. Anything else (another function, a
// defined name, an array formula, an external or 3D reference) makes the cell
// "unchecked", and so does every cell that depends on it: they are listed,
// never guessed.
//
// Exit 0 clean, 1 findings, 2 the file can't be read or the arguments are bad.
// Bounded: a zip entry over 64 MB uncompressed, a file over 256 MB or a book
// over 2 million cells is refused. Node standard library only (no install).
import { readFileSync, statSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { inflateRawSync } from 'node:zlib';

export const MAX_ENTRY_BYTES = 64 * 1024 * 1024;
export const MAX_FILE_BYTES = 256 * 1024 * 1024;
export const MAX_CELLS = 2_000_000;
/** Largest range one reference may expand to. */
const MAX_RANGE_CELLS = 4_000_000;
export const TOLERANCE = { absolute: 0.005, relative: 1e-9 };
const MAX_ROW = 1048576;
const MAX_COL = 16384;
const ERRORS = ['#NULL!', '#DIV/0!', '#VALUE!', '#REF!', '#NAME?', '#NUM!', '#N/A', '#GETTING_DATA', '#SPILL!', '#CALC!'];

/** The file can't be read as a workbook (exit 2). */
export class ReadError extends Error {}
/** The formula (or one it depends on) is outside what this evaluates. */
class Unchecked extends Error {}
/** The formula depends on a circular reference. */
class CycleDep extends Error {}

class XlError {
  constructor(code) { this.code = code; }
  toString() { return this.code; }
}
const err = code => new XlError(code);
const isErr = v => v instanceof XlError;

// ---------------------------------------------------------------- zip

/** Entries of a zip buffer: Map name -> () => Buffer. Stored and deflated only. */
export function readZip(buf) {
  if (buf.length < 22) throw new ReadError('not a zip file (too short)');
  let eocd = -1;
  for (let i = buf.length - 22; i >= Math.max(0, buf.length - 65557); i--) {
    if (buf.readUInt32LE(i) === 0x06054b50) { eocd = i; break; }
  }
  if (eocd < 0) throw new ReadError('not a zip file (no end of central directory)');
  const count = buf.readUInt16LE(eocd + 10);
  const cdOffset = buf.readUInt32LE(eocd + 16);
  if (count === 0xffff || cdOffset === 0xffffffff) throw new ReadError('zip64 archives are not supported');
  const entries = new Map();
  let p = cdOffset;
  for (let n = 0; n < count; n++) {
    if (p + 46 > buf.length || buf.readUInt32LE(p) !== 0x02014b50) throw new ReadError('corrupt zip central directory');
    const flags = buf.readUInt16LE(p + 8);
    const method = buf.readUInt16LE(p + 10);
    const compSize = buf.readUInt32LE(p + 20);
    const size = buf.readUInt32LE(p + 24);
    const nameLen = buf.readUInt16LE(p + 28);
    const extraLen = buf.readUInt16LE(p + 30);
    const commentLen = buf.readUInt16LE(p + 32);
    const local = buf.readUInt32LE(p + 42);
    const name = buf.toString('utf8', p + 46, p + 46 + nameLen);
    p += 46 + nameLen + extraLen + commentLen;
    entries.set(name, () => {
      if (flags & 1) throw new ReadError(`${name}: encrypted zip entries are not supported`);
      if (size > MAX_ENTRY_BYTES) throw new ReadError(`${name}: over ${MAX_ENTRY_BYTES / 1048576} MB uncompressed`);
      if (local + 30 > buf.length || buf.readUInt32LE(local) !== 0x04034b50) throw new ReadError(`${name}: corrupt local header`);
      const start = local + 30 + buf.readUInt16LE(local + 26) + buf.readUInt16LE(local + 28);
      const data = buf.subarray(start, start + compSize);
      if (data.length !== compSize) throw new ReadError(`${name}: truncated`);
      if (method === 0) return data;
      if (method !== 8) throw new ReadError(`${name}: compression method ${method} is not supported`);
      try {
        return inflateRawSync(data, { maxOutputLength: MAX_ENTRY_BYTES });
      } catch (e) {
        throw new ReadError(`${name}: ${e.code === 'ERR_BUFFER_TOO_LARGE' ? `over ${MAX_ENTRY_BYTES / 1048576} MB uncompressed` : 'corrupt deflate data'}`);
      }
    });
  }
  return entries;
}

// ---------------------------------------------------------------- xml

function decodeXml(s) {
  return s
    .replace(/&(#x[0-9a-f]+|#\d+|lt|gt|amp|quot|apos);/gi, (m, e) => {
      if (e[0] === '#') return String.fromCodePoint(e[1] === 'x' || e[1] === 'X' ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10));
      return { lt: '<', gt: '>', amp: '&', quot: '"', apos: "'" }[e.toLowerCase()];
    });
}
/** OOXML's own `_xHHHH_` escapes in string values. */
const decodeOoxml = s => decodeXml(s).replace(/_x([0-9a-fA-F]{4})_/g, (m, h) => String.fromCharCode(parseInt(h, 16)));

function attrs(s) {
  const out = {};
  for (const m of s.matchAll(/([\w:.-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')/g)) out[m[1]] = decodeXml(m[2] ?? m[3]);
  return out;
}
/** Elements `<tag ...>body</tag>` or `<tag .../>` (any namespace prefix), not nested in themselves. */
const TAG_RES = new Map();
function* elements(xml, tag) {
  let re = TAG_RES.get(tag);
  if (!re) TAG_RES.set(tag, re = new RegExp(`<(?:[\\w.-]+:)?${tag}(?=[\\s/>])([^>]*?)(?:/>|>([\\s\\S]*?)</(?:[\\w.-]+:)?${tag}>)`, 'g'));
  for (const m of xml.matchAll(re)) yield { attrs: attrs(m[1]), body: m[2] ?? '' };
}
function first(xml, tag) {
  for (const e of elements(xml, tag)) return e;
  return null;
}
const textOf = body => [...elements(body.replace(/<(?:[\w.-]+:)?rPh\b[\s\S]*?<\/(?:[\w.-]+:)?rPh>/g, ''), 't')].map(t => decodeOoxml(t.body)).join('');

// ---------------------------------------------------------------- references

export function colToNum(letters) {
  let n = 0;
  for (const ch of letters.toUpperCase()) n = n * 26 + (ch.charCodeAt(0) - 64);
  return n;
}
export function numToCol(n) {
  let s = '';
  while (n > 0) { const r = (n - 1) % 26; s = String.fromCharCode(65 + r) + s; n = Math.floor((n - 1) / 26); }
  return s;
}
const a1 = (row, col) => `${numToCol(col)}${row}`;
function parseA1(ref) {
  const m = /^\$?([A-Za-z]{1,3})\$?(\d+)$/.exec(ref);
  if (!m) return null;
  const col = colToNum(m[1]); const row = Number(m[2]);
  if (col < 1 || col > MAX_COL || row < 1 || row > MAX_ROW) return null;
  return { row, col };
}
const quoteSheet = name => (/^[A-Za-z_][\w.]*$/.test(name) && !parseA1(name) ? name : `'${name.replace(/'/g, "''")}'`);

// ---------------------------------------------------------------- formula tokens

const CELL = String.raw`(\$?)([A-Za-z]{1,3})(\$?)(\d+)`;
const RE_RANGE = new RegExp(`^${CELL}(?::${CELL})?(?![\\w(.!])`);
const RE_COLS = /^(\$?)([A-Za-z]{1,3}):(\$?)([A-Za-z]{1,3})(?![\w(.!])/;
const RE_ROWS = /^(\$?)(\d+):(\$?)(\d+)(?![\w(.!])/;
const RE_NUMBER = /^(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?/;
const RE_IDENT = /^[A-Za-z_\\][\w.\\]*/;
const RE_SHEET = /^([A-Za-z_\\][\w.\\]*)!/;
const OPS = ['<>', '<=', '>=', '+', '-', '*', '/', '^', '&', '=', '<', '>', '%', '(', ')', ',', ':'];

/** Tokens of a formula (no leading `=`), each with its source span. */
export function tokenize(text) {
  const out = [];
  let i = 0;
  while (i < text.length) {
    const rest = text.slice(i);
    const ws = /^[\s]+/.exec(rest);
    if (ws) { i += ws[0].length; continue; }
    const start = i;
    let sheet = null;
    let r = rest;
    if (r[0] === '[') throw new Unchecked('external workbook reference');
    if (r[0] === "'") {
      let j = 1; let name = '';
      for (;;) {
        if (j >= r.length) throw new Unchecked('unterminated sheet name');
        if (r[j] === "'") { if (r[j + 1] === "'") { name += "'"; j += 2; continue; } break; }
        name += r[j++];
      }
      if (r[j + 1] !== '!') throw new Unchecked('quoted name without a reference');
      if (name.startsWith('[')) throw new Unchecked('external workbook reference');
      sheet = name; r = r.slice(j + 2);
    } else {
      const sm = RE_SHEET.exec(r);
      if (sm) { sheet = sm[1]; r = r.slice(sm[0].length); }
    }
    const used = rest.length - r.length;
    if (sheet !== null && /^[A-Za-z_][\w.]*:[A-Za-z_][\w.]*$/.test(sheet)) throw new Unchecked('3D reference across sheets');
    let m;
    if (r.startsWith('#REF!')) {
      out.push({ type: 'error', value: '#REF!', start, end: i = start + used + 5 }); continue;
    }
    if ((m = RE_RANGE.exec(r))) {
      const one = { colAbs: !!m[1], col: colToNum(m[2]), rowAbs: !!m[3], row: Number(m[4]) };
      const two = m[5] !== undefined ? { colAbs: !!m[5], col: colToNum(m[6]), rowAbs: !!m[7], row: Number(m[8]) } : null;
      out.push({ type: 'ref', sheet, a: one, b: two, start, end: i = start + used + m[0].length }); continue;
    }
    if ((m = RE_COLS.exec(r))) {
      out.push({ type: 'ref', sheet, kind: 'cols', a: { colAbs: !!m[1], col: colToNum(m[2]) }, b: { colAbs: !!m[3], col: colToNum(m[4]) }, start, end: i = start + used + m[0].length }); continue;
    }
    if ((m = RE_ROWS.exec(r))) {
      out.push({ type: 'ref', sheet, kind: 'rows', a: { rowAbs: !!m[1], row: Number(m[2]) }, b: { rowAbs: !!m[3], row: Number(m[4]) }, start, end: i = start + used + m[0].length }); continue;
    }
    if (sheet !== null) throw new Unchecked(`unreadable reference after ${quoteSheet(sheet)}!`);
    if ((m = RE_NUMBER.exec(r))) { out.push({ type: 'num', value: Number(m[0]), start, end: i += m[0].length }); continue; }
    if (r[0] === '"') {
      let j = 1; let s = '';
      for (;;) {
        if (j >= r.length) throw new Unchecked('unterminated string');
        if (r[j] === '"') { if (r[j + 1] === '"') { s += '"'; j += 2; continue; } break; }
        s += r[j++];
      }
      out.push({ type: 'str', value: s, start, end: i += j + 1 }); continue;
    }
    if (r[0] === '#') {
      const e = ERRORS.find(x => r.toUpperCase().startsWith(x));
      if (!e) throw new Unchecked('unknown error literal');
      out.push({ type: 'error', value: e, start, end: i += e.length }); continue;
    }
    if (r[0] === '{') throw new Unchecked('array constant');
    if ((m = RE_IDENT.exec(r))) {
      const after = r.slice(m[0].length);
      const name = m[0].toUpperCase();
      if (/^\s*\(/.test(after)) out.push({ type: 'func', name, start, end: i += m[0].length });
      else if (name === 'TRUE' || name === 'FALSE') out.push({ type: 'bool', value: name === 'TRUE', start, end: i += m[0].length });
      else out.push({ type: 'name', name: m[0], start, end: i += m[0].length });
      continue;
    }
    const op = OPS.find(o => r.startsWith(o));
    if (!op) throw new Unchecked(`unexpected "${r[0]}"`);
    out.push({ type: 'op', value: op, start, end: i += op.length });
  }
  return out;
}

const refPart = (p, dr, dc) => {
  const col = p.col === undefined ? undefined : p.colAbs ? p.col : p.col + dc;
  const row = p.row === undefined ? undefined : p.rowAbs ? p.row : p.row + dr;
  return { ...p, col, row };
};
const partText = p => `${p.col === undefined ? '' : `${p.colAbs ? '$' : ''}${numToCol(p.col)}`}${p.row === undefined ? '' : `${p.rowAbs ? '$' : ''}${p.row}`}`;
const outOfSheet = p => (p.col !== undefined && (p.col < 1 || p.col > MAX_COL)) || (p.row !== undefined && (p.row < 1 || p.row > MAX_ROW));

/** A shared formula's text moved by (dr, dc): relative parts shift, a part off the sheet becomes #REF!. */
export function shiftFormula(text, dr, dc) {
  if (dr === 0 && dc === 0) return text;
  let out = ''; let at = 0;
  for (const t of tokenize(text)) {
    if (t.type !== 'ref') continue;
    const a = refPart(t.a, dr, dc); const b = t.b ? refPart(t.b, dr, dc) : null;
    const sheet = t.sheet !== null ? `${quoteSheet(t.sheet)}!` : '';
    out += text.slice(at, t.start) + (outOfSheet(a) || (b && outOfSheet(b)) ? `${sheet}#REF!` : `${sheet}${partText(a)}${b ? `:${partText(b)}` : ''}`);
    at = t.end;
  }
  return out + text.slice(at);
}

// ---------------------------------------------------------------- parser

/** Parse a formula into an AST; throws Unchecked when it can't. */
export function parseFormula(text) {
  const toks = tokenize(text);
  let k = 0;
  const peek = () => toks[k];
  const isOp = (v) => peek()?.type === 'op' && peek().value === v;
  const eat = (v) => { if (!isOp(v)) throw new Unchecked(`expected "${v}"`); k++; };
  const BIN = [['=', '<>', '<', '>', '<=', '>='], ['&'], ['+', '-'], ['*', '/'], ['^']];
  function binary(level) {
    if (level === BIN.length) return postfix();
    let left = binary(level + 1);
    while (peek()?.type === 'op' && BIN[level].includes(peek().value)) {
      const op = toks[k++].value;
      left = { type: 'bin', op, left, right: binary(level + 1) };
    }
    return left;
  }
  function postfix() {
    let node = prefix();
    while (isOp('%')) { k++; node = { type: 'pct', arg: node }; }
    return node;
  }
  function prefix() {
    if (isOp('-')) { k++; return { type: 'neg', arg: prefix() }; }
    if (isOp('+')) { k++; return prefix(); }
    return primary();
  }
  function primary() {
    const t = toks[k++];
    if (!t) throw new Unchecked('formula ends early');
    if (t.type === 'num' || t.type === 'str' || t.type === 'bool') return { type: 'lit', value: t.value };
    if (t.type === 'error') return { type: 'lit', value: err(t.value) };
    if (t.type === 'ref') {
      if (isOp(':')) throw new Unchecked('range built with ":" from a non-cell');
      return { type: 'ref', sheet: t.sheet, kind: t.kind ?? 'cells', a: t.a, b: t.b };
    }
    if (t.type === 'name') throw new Unchecked(`defined name ${t.name}`);
    if (t.type === 'func') {
      eat('(');
      const args = [];
      if (isOp(')')) { k++; return { type: 'call', name: t.name, args }; }
      for (;;) {
        if (isOp(',') || isOp(')')) args.push({ type: 'missing' });
        else args.push(binary(0));
        if (isOp(',')) { k++; continue; }
        eat(')');
        return { type: 'call', name: t.name, args };
      }
    }
    if (t.type === 'op' && t.value === '(') {
      const inner = binary(0);
      if (isOp(',')) throw new Unchecked('union reference');
      eat(')');
      return inner;
    }
    throw new Unchecked(`unexpected "${t.type === 'op' ? t.value : t.type}"`);
  }
  const ast = binary(0);
  if (k < toks.length) throw new Unchecked(toks[k].type === 'ref' || toks[k].type === 'name' ? 'intersection or unreadable operator' : `unexpected "${toks[k].value ?? toks[k].type}"`);
  return ast;
}

// ---------------------------------------------------------------- workbook

/** Relationships of a part (`xl/workbook.xml`), or of the package with path ''. */
function relsOf(zip, path) {
  const slash = path.lastIndexOf('/');
  const relPath = path === '' ? '_rels/.rels' : `${path.slice(0, slash + 1)}_rels/${path.slice(slash + 1)}.rels`;
  const get = zip.get(relPath);
  const out = [];
  if (!get) return out;
  const xml = get().toString('utf8');
  for (const r of elements(xml, 'Relationship')) {
    if (r.attrs.TargetMode === 'External') continue;
    const target = r.attrs.Target ?? '';
    let full = target.startsWith('/') ? target.slice(1) : path.slice(0, slash + 1) + target;
    const parts = [];
    for (const seg of full.split('/')) { if (seg === '..') parts.pop(); else if (seg !== '.') parts.push(seg); }
    full = parts.join('/');
    out.push({ id: r.attrs.Id, type: r.attrs.Type ?? '', target: full });
  }
  return out;
}

/** Read a workbook buffer into { sheets: [{ name, cells: Map, maxRow, maxCol }] }. */
export function readWorkbook(buf) {
  const zip = readZip(buf);
  const root = relsOf(zip, '').find(r => r.type.endsWith('/officeDocument'));
  const bookPath = root?.target ?? 'xl/workbook.xml';
  if (!zip.has(bookPath)) throw new ReadError(`no workbook part (${bookPath}); is this an .xlsx file?`);
  const bookXml = zip.get(bookPath)().toString('utf8');
  const rels = relsOf(zip, bookPath);
  const ssPath = rels.find(r => r.type.endsWith('/sharedStrings'))?.target;
  const shared = [];
  if (ssPath && zip.has(ssPath)) for (const si of elements(zip.get(ssPath)().toString('utf8'), 'si')) shared.push(textOf(si.body));
  const sheets = [];
  let total = 0;
  for (const s of elements(first(bookXml, 'sheets')?.body ?? '', 'sheet')) {
    const rid = Object.entries(s.attrs).find(([key]) => /^[\w.-]+:id$/.test(key))?.[1];
    const rel = rels.find(r => r.id === rid);
    const name = s.attrs.name ?? `Sheet${sheets.length + 1}`;
    if (!rel || !rel.type.endsWith('/worksheet')) { sheets.push({ name, cells: new Map(), maxRow: 0, maxCol: 0, notWorksheet: true }); continue; }
    if (!zip.has(rel.target)) throw new ReadError(`sheet ${name}: part ${rel.target} is missing`);
    const sheet = { name, cells: new Map(), maxRow: 0, maxCol: 0, sharedMasters: new Map() };
    const xml = zip.get(rel.target)().toString('utf8');
    let rowNo = 0;
    for (const row of elements(first(xml, 'sheetData')?.body ?? '', 'row')) {
      rowNo = row.attrs.r ? Number(row.attrs.r) : rowNo + 1;
      let colNo = 0;
      for (const c of elements(row.body, 'c')) {
        const pos = c.attrs.r ? parseA1(c.attrs.r) : { row: rowNo, col: colNo + 1 };
        if (!pos) throw new ReadError(`sheet ${name}: bad cell reference ${c.attrs.r}`);
        colNo = pos.col;
        if (++total > MAX_CELLS) throw new ReadError(`over ${MAX_CELLS.toLocaleString('en-US')} cells; refusing`);
        const cell = { sheet: name, row: pos.row, col: pos.col, cached: null, formula: null, fkind: null };
        const v = /<(?:[\w.-]+:)?v>([\s\S]*?)<\/(?:[\w.-]+:)?v>/.exec(c.body)?.[1];
        const t = c.attrs.t ?? 'n';
        if (t === 's') cell.cached = v !== undefined && v !== '' ? (shared[Number(v)] ?? null) : null;
        else if (t === 'inlineStr') cell.cached = textOf(first(c.body, 'is')?.body ?? '');
        else if (t === 'str') cell.cached = v === undefined ? null : decodeOoxml(v);
        else if (t === 'b') cell.cached = v === undefined || v === '' ? null : v.trim() === '1';
        else if (t === 'e') cell.cached = v === undefined || v === '' ? null : err(decodeXml(v).trim());
        else if (t === 'd') cell.cached = v === undefined || v === '' ? null : decodeXml(v);
        else if (v === undefined || v.trim() === '') cell.cached = null;
        else { const num = Number(v); cell.cached = Number.isNaN(num) ? decodeXml(v) : num; }
        const f = first(c.body, 'f');
        if (f) {
          const ft = f.attrs.t ?? 'normal';
          const text = decodeXml(f.body).replace(/^=/, '');
          if (ft === 'shared') {
            const si = f.attrs.si;
            if (text !== '') sheet.sharedMasters.set(si, { text, row: pos.row, col: pos.col });
            const master = sheet.sharedMasters.get(si);
            cell.fkind = 'shared';
            if (!master) { cell.formula = ''; cell.uncheckedReason = 'shared formula without its master cell'; }
            else {
              try { cell.formula = shiftFormula(master.text, pos.row - master.row, pos.col - master.col); } catch (e) {
                if (!(e instanceof Unchecked)) throw e;
                cell.formula = master.text; cell.uncheckedReason = e.message;
              }
            }
          } else {
            cell.fkind = ft;
            cell.formula = text;
            if (ft === 'array') {
              cell.uncheckedReason = 'array formula';
              const ref = (f.attrs.ref ?? '').split(':').map(parseA1);
              if (ref[0] && ref[1]) sheet.arrayRanges = [...(sheet.arrayRanges ?? []), { r1: ref[0].row, c1: ref[0].col, r2: ref[1].row, c2: ref[1].col, at: a1(pos.row, pos.col) }];
            } else if (ft !== 'normal') cell.uncheckedReason = `${ft} formula`;
          }
        }
        if (cell.formula === null && cell.cached === null) continue;
        sheet.cells.set(pos.row * 20000 + pos.col, cell);
        if (pos.row > sheet.maxRow) sheet.maxRow = pos.row;
        if (pos.col > sheet.maxCol) sheet.maxCol = pos.col;
      }
    }
    for (const ar of sheet.arrayRanges ?? []) {
      for (const cell of sheet.cells.values()) {
        if (cell.formula === null && cell.row >= ar.r1 && cell.row <= ar.r2 && cell.col >= ar.c1 && cell.col <= ar.c2) cell.arrayPart = ar.at;
      }
    }
    delete sheet.sharedMasters;
    sheets.push(sheet);
  }
  if (sheets.length === 0) throw new ReadError('the workbook lists no sheets');
  return { sheets };
}

// ---------------------------------------------------------------- values

const isBlank = v => v === null || v === undefined;
function toNumber(v) {
  if (isErr(v)) return v;
  if (isBlank(v)) return 0;
  if (typeof v === 'number') return v;
  if (typeof v === 'boolean') return v ? 1 : 0;
  const s = String(v).trim();
  if (s === '' || !/^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?%?$/.test(s)) return err('#VALUE!');
  return s.endsWith('%') ? Number(s.slice(0, -1)) / 100 : Number(s);
}
function toText(v) {
  if (isBlank(v)) return '';
  if (typeof v === 'boolean') return v ? 'TRUE' : 'FALSE';
  if (typeof v === 'number') return String(Number(v.toPrecision(15)));
  return String(v);
}
function toBool(v) {
  if (isErr(v)) return v;
  if (isBlank(v)) return false;
  if (typeof v === 'boolean') return v;
  if (typeof v === 'number') return v !== 0;
  const s = String(v).toUpperCase();
  if (s === 'TRUE') return true;
  if (s === 'FALSE') return false;
  return err('#VALUE!');
}
const finite = n => (Number.isFinite(n) ? n : err('#NUM!'));
function compare(a, b) {
  const rank = v => (typeof v === 'number' ? 0 : typeof v === 'string' ? 1 : 2);
  if (isBlank(a)) a = typeof b === 'string' ? '' : typeof b === 'boolean' ? false : 0;
  if (isBlank(b)) b = typeof a === 'string' ? '' : typeof a === 'boolean' ? false : 0;
  if (rank(a) !== rank(b)) return rank(a) - rank(b);
  if (typeof a === 'string') { const x = a.toLowerCase(); const y = b.toLowerCase(); return x < y ? -1 : x > y ? 1 : 0; }
  return a < b ? -1 : a > b ? 1 : 0;
}
function arith(op, a, b) {
  if (op === '&') { if (isErr(a)) return a; if (isErr(b)) return b; return toText(a) + toText(b); }
  if (['=', '<>', '<', '>', '<=', '>='].includes(op)) {
    if (isErr(a)) return a; if (isErr(b)) return b;
    const c = compare(a, b);
    return { '=': c === 0, '<>': c !== 0, '<': c < 0, '>': c > 0, '<=': c <= 0, '>=': c >= 0 }[op];
  }
  const x = toNumber(a); if (isErr(x)) return x;
  const y = toNumber(b); if (isErr(y)) return y;
  if (op === '+') return finite(x + y);
  if (op === '-') return finite(x - y);
  if (op === '*') return finite(x * y);
  if (op === '/') return y === 0 ? err('#DIV/0!') : finite(x / y);
  if (op === '^') return x === 0 && y === 0 ? err('#NUM!') : finite(x ** y);
  throw new Unchecked(`operator ${op}`);
}
/** Excel's rounding: half away from zero, on the value as shown to 15 digits. */
function roundTo(x, digits, mode) {
  const n = Math.trunc(digits);
  const m = 10 ** n;
  const scaled = Number((Math.abs(x) * m).toPrecision(15));
  const r = mode === 'up' ? Math.ceil(scaled) : mode === 'down' ? Math.floor(scaled) : Math.round(scaled);
  return Math.sign(x) * (r / m) || 0;
}

class Arr {
  constructor(rows, cols, data, fromRange) { this.rows = rows; this.cols = cols; this.data = data; this.fromRange = fromRange; }
  *values() { for (const row of this.data) yield* row; }
}

// ---------------------------------------------------------------- evaluator

export function recalc(book) {
  const byName = new Map(book.sheets.map(s => [s.name.toLowerCase(), s]));
  const memo = new Map();
  const visiting = [];
  const onStack = new Set();
  const cycleCells = new Set();

  const sheetOf = (name, home) => {
    if (name === null) return home;
    const s = byName.get(name.toLowerCase());
    if (!s) throw new Unchecked(`sheet ${name} isn't in the book`);
    if (s.notWorksheet) throw new Unchecked(`${name} isn't a worksheet`);
    return s;
  };

  function cellValue(sheet, row, col) {
    const cell = sheet.cells.get(row * 20000 + col);
    if (!cell) return null;
    if (cell.arrayPart) throw new Unchecked(`depends on array formula at ${quoteSheet(sheet.name)}!${cell.arrayPart}`);
    if (cell.formula === null) return cell.cached;
    const r = evalCell(cell);
    if (r.status === 'ok') return r.value;
    if (r.status === 'cycle') throw new CycleDep();
    throw new Unchecked(`depends on ${quoteSheet(sheet.name)}!${a1(row, col)}`);
  }

  function evalCell(cell) {
    const key = cell;
    if (memo.has(key)) return memo.get(key);
    if (onStack.has(key)) {
      for (let i = visiting.lastIndexOf(key); i < visiting.length; i++) cycleCells.add(visiting[i]);
      throw new CycleDep();
    }
    let result;
    if (cell.uncheckedReason) result = { status: 'unchecked', reason: cell.uncheckedReason };
    else {
      visiting.push(key); onStack.add(key);
      try {
        const home = byName.get(cell.sheet.toLowerCase());
        let v = evalNode(parseFormula(cell.formula), home, false);
        if (v instanceof Arr) v = single(v);
        result = { status: 'ok', value: isBlank(v) ? 0 : v };
      } catch (e) {
        if (e instanceof Unchecked) result = { status: 'unchecked', reason: e.message };
        else if (e instanceof CycleDep) result = { status: 'cycle' };
        else if (e instanceof RangeError && /call stack/i.test(e.message)) result = { status: 'unchecked', reason: 'dependency chain too deep' };
        else throw e;
      } finally {
        visiting.pop(); onStack.delete(key);
      }
    }
    memo.set(key, result);
    return result;
  }

  function single(arr) {
    if (arr.rows === 1 && arr.cols === 1) return arr.data[0][0];
    throw new Unchecked('a range used as one value (implicit intersection)');
  }

  function rangeArr(node, home) {
    const sheet = sheetOf(node.sheet, home);
    let r1, c1, r2, c2;
    if (node.kind === 'cols') { r1 = 1; r2 = sheet.maxRow; c1 = node.a.col; c2 = node.b.col; }
    else if (node.kind === 'rows') { c1 = 1; c2 = sheet.maxCol; r1 = node.a.row; r2 = node.b.row; }
    else { r1 = node.a.row; c1 = node.a.col; r2 = node.b ? node.b.row : r1; c2 = node.b ? node.b.col : c1; }
    if (r1 > r2) [r1, r2] = [r2, r1];
    if (c1 > c2) [c1, c2] = [c2, c1];
    if (r2 < r1 || c2 < c1) return new Arr(0, 0, [], true);
    if ((r2 - r1 + 1) * (c2 - c1 + 1) > MAX_RANGE_CELLS) throw new Unchecked('range too large');
    const data = [];
    for (let r = r1; r <= r2; r++) {
      const row = [];
      for (let c = c1; c <= c2; c++) row.push(cellValue(sheet, r, c));
      data.push(row);
    }
    return new Arr(r2 - r1 + 1, c2 - c1 + 1, data, true);
  }

  function lift(op, a, b) {
    if (!(a instanceof Arr) && !(b instanceof Arr)) return arith(op, a, b);
    const A = a instanceof Arr ? a : null; const B = b instanceof Arr ? b : null;
    const rows = Math.max(A?.rows ?? 1, B?.rows ?? 1); const cols = Math.max(A?.cols ?? 1, B?.cols ?? 1);
    const at = (x, r, c) => {
      if (!(x instanceof Arr)) return x;
      const rr = x.rows === 1 ? 0 : r; const cc = x.cols === 1 ? 0 : c;
      return rr < x.rows && cc < x.cols ? x.data[rr][cc] : err('#N/A');
    };
    const data = [];
    for (let r = 0; r < rows; r++) { const row = []; for (let c = 0; c < cols; c++) row.push(arith(op, at(a, r, c), at(b, r, c))); data.push(row); }
    return new Arr(rows, cols, data, false);
  }

  function evalNode(node, home, arrayCtx) {
    switch (node.type) {
      case 'lit': return node.value;
      case 'missing': return null;
      case 'ref': {
        if (node.kind === 'cells' && !node.b) return cellValue(sheetOf(node.sheet, home), node.a.row, node.a.col);
        return rangeArr(node, home);
      }
      case 'neg': case 'pct': {
        const v = evalNode(node.arg, home, arrayCtx);
        const k = node.type === 'neg' ? -1 : 0.01;
        if (v instanceof Arr) { if (!arrayCtx) return arith('*', single(v), k); return lift('*', v, k); }
        return arith('*', v, k);
      }
      case 'bin': {
        let l = evalNode(node.left, home, arrayCtx);
        let r = evalNode(node.right, home, arrayCtx);
        if (!arrayCtx) { if (l instanceof Arr) l = single(l); if (r instanceof Arr) r = single(r); }
        return lift(node.op, l, r);
      }
      case 'call': return call(node, home, arrayCtx);
      default: throw new Unchecked(`node ${node.type}`);
    }
  }

  /** Numbers of aggregate arguments, Excel's way: a range skips text, booleans and blanks; a typed argument coerces. */
  function numbersOf(args, home, arrayCtx) {
    const out = [];
    for (const a of args) {
      const v = evalNode(a, home, arrayCtx);
      if (v instanceof Arr) {
        for (const x of v.values()) {
          if (isErr(x)) return x;
          if (typeof x === 'number') out.push(x);
          else if (!v.fromRange && typeof x === 'boolean') out.push(x ? 1 : 0);
        }
      } else {
        if (a.type === 'ref' && (isBlank(v) || typeof v === 'string' || typeof v === 'boolean')) continue;
        const n = toNumber(v);
        if (isErr(n)) return n;
        out.push(n);
      }
    }
    return out;
  }

  function scalarArg(node, home, arrayCtx) {
    const v = evalNode(node, home, arrayCtx);
    if (v instanceof Arr) {
      if (arrayCtx && !(v.rows === 1 && v.cols === 1)) throw new Unchecked('an array passed to a single-value function');
      return single(v);
    }
    return v;
  }

  function call(node, home, arrayCtx) {
    const { name, args } = node;
    const need = (lo, hi) => { if (args.length < lo || args.length > hi) throw new Unchecked(`${name} with ${args.length} arguments`); };
    switch (name) {
      case 'SUM': case 'AVERAGE': case 'MIN': case 'MAX': case 'PRODUCT': {
        need(1, 255);
        const ns = numbersOf(args, home, arrayCtx);
        if (isErr(ns)) return ns;
        if (name === 'SUM') return finite(ns.reduce((s, x) => s + x, 0));
        if (name === 'AVERAGE') return ns.length ? finite(ns.reduce((s, x) => s + x, 0) / ns.length) : err('#DIV/0!');
        if (name === 'MIN') return ns.length ? ns.reduce((m, x) => (x < m ? x : m)) : 0;
        if (name === 'MAX') return ns.length ? ns.reduce((m, x) => (x > m ? x : m)) : 0;
        return ns.length ? finite(ns.reduce((s, x) => s * x, 1)) : 0;
      }
      case 'COUNT': case 'COUNTA': {
        need(1, 255);
        let n = 0;
        for (const a of args) {
          const v = evalNode(a, home, arrayCtx);
          for (const x of v instanceof Arr ? v.values() : [v]) {
            if (name === 'COUNTA') { if (!isBlank(x) || (a.type === 'missing')) n++; continue; }
            if (typeof x === 'number') n++;
            else if (!(v instanceof Arr) && a.type !== 'ref' && (typeof x === 'boolean' || (typeof x === 'string' && !isErr(toNumber(x))))) n++;
          }
        }
        return n;
      }
      case 'ROUND': case 'ROUNDUP': case 'ROUNDDOWN': {
        need(2, 2);
        const x = toNumber(scalarArg(args[0], home, arrayCtx)); if (isErr(x)) return x;
        const d = toNumber(scalarArg(args[1], home, arrayCtx)); if (isErr(d)) return d;
        return roundTo(x, d, name === 'ROUNDUP' ? 'up' : name === 'ROUNDDOWN' ? 'down' : 'half');
      }
      case 'ABS': {
        need(1, 1);
        const x = toNumber(scalarArg(args[0], home, arrayCtx));
        return isErr(x) ? x : Math.abs(x);
      }
      case 'IF': {
        need(1, 3);
        const c = toBool(scalarArg(args[0], home, arrayCtx));
        if (isErr(c)) return c;
        if (c) return args.length >= 2 ? (args[1].type === 'missing' ? 0 : scalarArg(args[1], home, arrayCtx)) : true;
        return args.length >= 3 ? (args[2].type === 'missing' ? 0 : scalarArg(args[2], home, arrayCtx)) : false;
      }
      case 'IFERROR': {
        need(2, 2);
        const v = scalarArg(args[0], home, arrayCtx);
        if (!isErr(v)) return isBlank(v) ? 0 : v;
        return args[1].type === 'missing' ? 0 : scalarArg(args[1], home, arrayCtx);
      }
      case 'AND': case 'OR': {
        need(1, 255);
        const bs = [];
        for (const a of args) {
          const v = evalNode(a, home, arrayCtx);
          for (const x of v instanceof Arr ? v.values() : [v]) {
            if (isErr(x)) return x;
            if (isBlank(x)) continue;
            if (typeof x === 'string') { if (v instanceof Arr || a.type === 'ref') continue; const b = toBool(x); if (isErr(b)) return b; bs.push(b); continue; }
            bs.push(typeof x === 'number' ? x !== 0 : x);
          }
        }
        if (!bs.length) return err('#VALUE!');
        return name === 'AND' ? bs.every(Boolean) : bs.some(Boolean);
      }
      case 'NOT': {
        need(1, 1);
        const b = toBool(scalarArg(args[0], home, arrayCtx));
        return isErr(b) ? b : !b;
      }
      case 'SUMPRODUCT': {
        need(1, 255);
        const arrs = args.map(a => { const v = evalNode(a, home, true); return v instanceof Arr ? v : new Arr(1, 1, [[v]], false); });
        const { rows, cols } = arrs[0];
        if (arrs.some(x => x.rows !== rows || x.cols !== cols)) return err('#VALUE!');
        let sum = 0;
        for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++) {
          let p = 1;
          for (const x of arrs) { const v = x.data[r][c]; if (isErr(v)) return v; p *= typeof v === 'number' ? v : 0; }
          sum += p;
        }
        return finite(sum);
      }
      default:
        throw new Unchecked(`function ${name.replace(/^_XL(FN|WS)\./, '')} isn't supported`);
    }
  }

  // Row order first, so a running total down a column recurses one step at a time.
  const formulas = [];
  for (const sheet of book.sheets) {
    const own = [...sheet.cells.values()].filter(cell => cell.formula !== null);
    own.sort((x, y) => x.row - y.row || x.col - y.col);
    for (const cell of own) formulas.push(cell);
  }
  for (const cell of formulas) {
    try { evalCell(cell); } catch (e) { if (!(e instanceof CycleDep)) throw e; }
  }
  for (const cell of formulas) if (!memo.has(cell)) memo.set(cell, { status: 'cycle' });

  return {
    formulas,
    resultOf: cell => memo.get(cell),
    inCycle: cell => cycleCells.has(cell),
    valueAt(sheetName, row, col) {
      const sheet = byName.get(sheetName.toLowerCase());
      const cell = sheet?.cells.get(row * 20000 + col);
      if (!cell) return { status: 'ok', value: null, cell: null };
      if (cell.arrayPart) return { status: 'unchecked', reason: `part of array formula at ${cell.arrayPart}`, cell };
      if (cell.formula === null) return { status: 'ok', value: cell.cached, cell };
      return { ...memo.get(cell), cell };
    },
  };
}

// ---------------------------------------------------------------- check

export const withinTolerance = (a, b) => Math.abs(a - b) <= Math.max(TOLERANCE.absolute, TOLERANCE.relative * Math.max(Math.abs(a), Math.abs(b)));
function same(computed, cached) {
  if (isErr(computed) || isErr(cached)) return isErr(computed) && isErr(cached) && computed.code === cached.code;
  if (typeof computed === 'number' && typeof cached === 'number') return withinTolerance(computed, cached);
  if (typeof cached === 'string' && typeof computed !== 'string') return toText(computed) === cached;
  return computed === cached;
}
const show = v => (isErr(v) ? v.code : v === undefined ? null : v);

/** Parse `Sheet!B10=1234.56` (or `'My Sheet'!B10=1,234.56`, or `B10=...` for the first sheet). */
export function parseExpect(spec, book) {
  const eq = spec.lastIndexOf('=');
  if (eq < 1) throw new ReadError(`--expect ${spec}: write it as Sheet!B10=1234.56`);
  const left = spec.slice(0, eq).trim();
  const right = spec.slice(eq + 1).trim().replace(/[,$\s]/g, '');
  const want = Number(right);
  if (right === '' || !Number.isFinite(want)) throw new ReadError(`--expect ${spec}: ${spec.slice(eq + 1).trim()} isn't a number`);
  let sheetName = book.sheets[0].name; let ref = left;
  const q = /^'((?:[^']|'')+)'!(.+)$/.exec(left) ?? /^([^!]+)!(.+)$/.exec(left);
  if (q) { sheetName = q[1].replace(/''/g, "'"); ref = q[2]; }
  const sheet = book.sheets.find(s => s.name.toLowerCase() === sheetName.toLowerCase());
  if (!sheet) throw new ReadError(`--expect ${spec}: no sheet named ${sheetName} (sheets: ${book.sheets.map(s => s.name).join(', ')})`);
  const pos = parseA1(ref.trim());
  if (!pos) throw new ReadError(`--expect ${spec}: ${ref} isn't a cell like B10`);
  return { spec, sheet: sheet.name, cell: a1(pos.row, pos.col), row: pos.row, col: pos.col, expected: want };
}

/** Check a workbook buffer: the report object `--json` prints. */
export function checkWorkbook(buf, expectSpecs = []) {
  let book;
  try { book = readWorkbook(buf); } catch (e) {
    if (e instanceof ReadError) throw e;
    throw new ReadError(`unreadable workbook (${e.message})`);
  }
  const expects = expectSpecs.map(s => parseExpect(s, book));
  const calc = recalc(book);
  const findings = [];
  const unchecked = [];
  let checked = 0; let noCache = 0; let cells = 0;
  for (const sheet of book.sheets) {
    cells += sheet.cells.size;
    for (const cell of sheet.cells.values()) {
      if (cell.formula === null && isErr(cell.cached)) findings.push({ sheet: sheet.name, cell: a1(cell.row, cell.col), kind: 'error', formula: null, cached: cell.cached.code, computed: null, detail: `the cell holds ${cell.cached.code}` });
    }
  }
  for (const cell of calc.formulas) {
    const r = calc.resultOf(cell);
    const at = { sheet: cell.sheet, cell: a1(cell.row, cell.col), formula: `=${cell.formula}` };
    if (cell.cached === null) noCache++;
    if (/#REF!/i.test(cell.formula)) {
      findings.push({ ...at, kind: 'broken-ref', cached: show(cell.cached), computed: r.status === 'ok' ? show(r.value) : null, detail: 'the formula refers to #REF! (a deleted or moved cell)' });
      if (r.status === 'ok') checked++; else if (r.status === 'unchecked') unchecked.push({ ...at, reason: r.reason });
      continue;
    }
    if (r.status === 'cycle') {
      if (calc.inCycle(cell)) findings.push({ ...at, kind: 'cycle', cached: show(cell.cached), computed: null, detail: 'circular reference' });
      else unchecked.push({ ...at, reason: 'depends on a circular reference' });
      continue;
    }
    if (r.status === 'unchecked') {
      unchecked.push({ ...at, reason: r.reason });
      if (isErr(cell.cached)) findings.push({ ...at, kind: 'error', cached: cell.cached.code, computed: null, detail: `the cached value is ${cell.cached.code} (formula not recomputed: ${r.reason})` });
      continue;
    }
    checked++;
    if (isErr(r.value) || isErr(cell.cached)) {
      findings.push({ ...at, kind: 'error', cached: show(cell.cached), computed: show(r.value), detail: isErr(r.value) ? `computes to ${r.value.code}` : `the cached value is ${cell.cached.code}` });
      continue;
    }
    if (cell.cached !== null && !same(r.value, cell.cached)) {
      findings.push({ ...at, kind: 'mismatch', cached: show(cell.cached), computed: show(r.value), detail: 'the cached value differs from the recomputed one' });
    }
  }
  const expectResults = [];
  for (const e of expects) {
    const r = calc.valueAt(e.sheet, e.row, e.col);
    const formula = r.cell?.formula != null ? `=${r.cell.formula}` : null;
    const base = { sheet: e.sheet, cell: e.cell, kind: 'expect', formula, cached: show(r.cell?.cached ?? null) };
    let ok = false; let detail; let computed = null;
    if (r.status === 'ok') {
      computed = show(r.value);
      if (typeof r.value === 'number') ok = withinTolerance(r.value, e.expected);
      detail = ok ? null : r.cell === null ? `the cell is empty; expected ${e.expected}` : `expected ${e.expected}, recomputed ${computed}`;
    } else {
      detail = r.status === 'cycle' ? `expected ${e.expected}; the cell is in or depends on a circular reference` : `expected ${e.expected}; could not recompute it (${r.reason})`;
    }
    expectResults.push({ target: e.spec, sheet: e.sheet, cell: e.cell, expected: e.expected, computed, ok });
    if (!ok) findings.push({ ...base, computed, detail });
  }
  return {
    sheets: book.sheets.map(s => s.name),
    counts: { cells, formulas: calc.formulas.length, checked, unchecked: unchecked.length, noCachedValue: noCache, findings: findings.length },
    tolerance: { ...TOLERANCE, rule: 'numbers match when |computed - cached| <= max(absolute, relative x the larger magnitude)' },
    findings,
    unchecked,
    expects: expectResults,
  };
}

// ---------------------------------------------------------------- cli

function human(file, rep) {
  const c = rep.counts;
  const lines = [
    `workbook-recalc: ${file}`,
    `  ${rep.sheets.length} sheet${rep.sheets.length === 1 ? '' : 's'}, ${c.cells} cells, ${c.formulas} formulas: ${c.checked} checked, ${c.unchecked} unchecked; ${c.noCachedValue} without a cached value (counted, not flagged)`,
    `  numbers match within max(${TOLERANCE.absolute}, ${TOLERANCE.relative} x |value|)`,
  ];
  const v = x => (x === null ? 'none' : typeof x === 'string' && !x.startsWith('#') ? JSON.stringify(x) : String(x));
  if (rep.findings.length) {
    lines.push(`Findings (${rep.findings.length}):`);
    for (const f of rep.findings) lines.push(`  ${quoteSheet(f.sheet)}!${f.cell}  ${f.kind}  ${f.formula ?? ''}  cached ${v(f.cached)}, computed ${v(f.computed)}: ${f.detail}`);
  }
  if (rep.unchecked.length) {
    lines.push(`Unchecked (${rep.unchecked.length}, not recomputed):`);
    for (const u of rep.unchecked.slice(0, 50)) lines.push(`  ${quoteSheet(u.sheet)}!${u.cell}  ${u.formula}  ${u.reason}`);
    if (rep.unchecked.length > 50) lines.push(`  and ${rep.unchecked.length - 50} more (--json lists all)`);
  }
  if (rep.expects.length) lines.push(`Expectations: ${rep.expects.filter(e => e.ok).length} of ${rep.expects.length} met`);
  lines.push(rep.findings.length ? `Result: ${rep.findings.length} finding${rep.findings.length === 1 ? '' : 's'}` : 'Result: clean');
  return lines.join('\n');
}

const USAGE = "usage: node scripts/workbook-recalc.mjs <book.xlsx> [--expect 'Sheet!B10=1234.56']... [--json]";

export async function main(argv) {
  let file = null; let json = false; const expects = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--json') json = true;
    else if (a === '--expect') { if (i + 1 >= argv.length) { console.error(USAGE); return 2; } expects.push(argv[++i]); }
    else if (a.startsWith('--expect=')) expects.push(a.slice(9));
    else if (a === '-h' || a === '--help') { console.log(USAGE); return 0; }
    else if (a.startsWith('--') || file !== null) { console.error(`workbook-recalc: unexpected argument ${a}\n${USAGE}`); return 2; }
    else file = a;
  }
  if (file === null) { console.error(USAGE); return 2; }
  let rep;
  try {
    const st = statSync(file);
    if (!st.isFile()) throw new ReadError('not a file');
    if (st.size > MAX_FILE_BYTES) throw new ReadError(`over ${MAX_FILE_BYTES / 1048576} MB; refusing`);
    rep = checkWorkbook(readFileSync(file), expects);
  } catch (e) {
    if (e instanceof ReadError || e.code === 'ENOENT' || e.code === 'EACCES' || e.code === 'EISDIR') {
      if (json) console.log(JSON.stringify({ file, error: e.message }));
      console.error(`workbook-recalc: can't check ${file}: ${e.message}`);
      return 2;
    }
    throw e;
  }
  console.log(json ? JSON.stringify({ file, ...rep }, null, 2) : human(file, rep));
  return rep.findings.length ? 1 : 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) process.exit(await main(process.argv.slice(2)));
