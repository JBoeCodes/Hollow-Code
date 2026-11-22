// Tests for scripts/workbook-recalc.mjs (UN2, 2026-11-18). Every workbook is
// built here as a small zip (stored entries, one deflated), so no fixture
// binaries are committed and each case shows exactly what the book holds.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { crc32, deflateRawSync } from 'node:zlib';
import { checkWorkbook, shiftFormula, MAX_ENTRY_BYTES } from './workbook-recalc.mjs';

const SCRIPT = join(dirname(fileURLToPath(import.meta.url)), 'workbook-recalc.mjs');
const dir = mkdtempSync(join(tmpdir(), 'workbook-recalc-'));
after(() => rmSync(dir, { recursive: true, force: true }));

/** A zip of [name, text] pairs; names in `deflate` are deflated, the rest stored. */
function zip(files, { deflate = [], sizeOverride = {} } = {}) {
  const locals = []; const centrals = []; let offset = 0;
  for (const [name, text] of files) {
    const raw = Buffer.from(text, 'utf8');
    const method = deflate.includes(name) ? 8 : 0;
    const data = method ? deflateRawSync(raw) : raw;
    const nameBuf = Buffer.from(name, 'utf8');
    const size = sizeOverride[name] ?? raw.length;
    const head = Buffer.alloc(30);
    head.writeUInt32LE(0x04034b50, 0); head.writeUInt16LE(20, 4); head.writeUInt16LE(method, 8);
    head.writeUInt16LE(0x21, 12); head.writeUInt32LE(crc32(raw), 14); head.writeUInt32LE(data.length, 18);
    head.writeUInt32LE(size, 22); head.writeUInt16LE(nameBuf.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0); central.writeUInt16LE(20, 4); central.writeUInt16LE(20, 6);
    central.writeUInt16LE(method, 10); central.writeUInt16LE(0x21, 14); central.writeUInt32LE(crc32(raw), 16);
    central.writeUInt32LE(data.length, 20); central.writeUInt32LE(size, 24); central.writeUInt16LE(nameBuf.length, 28);
    central.writeUInt32LE(offset, 42);
    locals.push(head, nameBuf, data); centrals.push(central, nameBuf);
    offset += 30 + nameBuf.length + data.length;
  }
  const cd = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0); end.writeUInt16LE(files.length, 8); end.writeUInt16LE(files.length, 10);
  end.writeUInt32LE(cd.length, 12); end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, cd, end]);
}

const esc = s => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
const order = ref => { const m = /^([A-Z]+)(\d+)$/.exec(ref); return [Number(m[2]), m[1].length, m[1]]; };

/**
 * An .xlsx from `{ 'Sheet name': { A1: spec } }`. A spec is a number, a string
 * (a shared string), or `{ f, v, shared: { si, ref }, e }`: `f` the formula, `v`
 * the cached value (a string makes it t="str"), `e` a cached error.
 */
function xlsx(sheets, opts = {}) {
  const strings = [];
  const sheetXml = Object.values(sheets).map(cells => {
    const rows = new Map();
    for (const ref of Object.keys(cells).sort((a, b) => { const x = order(a); const y = order(b); return x[0] - y[0] || x[1] - y[1] || (x[2] < y[2] ? -1 : 1); })) {
      const spec = cells[ref]; const row = order(ref)[0];
      let c;
      if (typeof spec === 'number') c = `<c r="${ref}"><v>${spec}</v></c>`;
      else if (typeof spec === 'string') { strings.push(spec); c = `<c r="${ref}" t="s"><v>${strings.length - 1}</v></c>`; }
      else {
        const sh = spec.shared ? ` t="shared" si="${spec.shared.si}"${spec.shared.ref ? ` ref="${spec.shared.ref}"` : ''}` : '';
        const f = spec.f === undefined ? '' : spec.f === '' ? `<f${sh}/>` : `<f${sh}>${esc(spec.f)}</f>`;
        const t = spec.e ? ' t="e"' : typeof spec.v === 'string' ? ' t="str"' : typeof spec.v === 'boolean' ? ' t="b"' : '';
        const v = spec.e ? `<v>${spec.e}</v>` : spec.v === undefined ? '' : `<v>${typeof spec.v === 'boolean' ? Number(spec.v) : esc(spec.v)}</v>`;
        c = `<c r="${ref}"${t}>${f}${v}</c>`;
      }
      rows.set(row, (rows.get(row) ?? '') + c);
    }
    const body = [...rows].map(([r, cs]) => `<row r="${r}">${cs}</row>`).join('');
    return `<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>${body}</sheetData></worksheet>`;
  });
  const names = Object.keys(sheets);
  const R = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships';
  const files = [
    ['[Content_Types].xml', '<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>'],
    ['_rels/.rels', `<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="${R}/officeDocument" Target="xl/workbook.xml"/></Relationships>`],
    ['xl/workbook.xml', `<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="${R}"><sheets>${names.map((n, i) => `<sheet name="${esc(n)}" sheetId="${i + 1}" r:id="rId${i + 1}"/>`).join('')}</sheets></workbook>`],
    ['xl/_rels/workbook.xml.rels', `<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">${names.map((_, i) => `<Relationship Id="rId${i + 1}" Type="${R}/worksheet" Target="worksheets/sheet${i + 1}.xml"/>`).join('')}<Relationship Id="rIdS" Type="${R}/sharedStrings" Target="sharedStrings.xml"/></Relationships>`],
    ['xl/sharedStrings.xml', `<?xml version="1.0"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">${strings.map(s => `<si><t>${esc(s)}</t></si>`).join('')}</sst>`],
    ...sheetXml.map((x, i) => [`xl/worksheets/sheet${i + 1}.xml`, x]),
  ];
  return zip(files, opts);
}

let n = 0;
function cli(buf, args = []) {
  const file = join(dir, `book-${++n}.xlsx`);
  writeFileSync(file, buf);
  const r = spawnSync(process.execPath, [SCRIPT, file, ...args], { encoding: 'utf8' });
  return { code: r.status, out: r.stdout, err: r.stderr, json: args.includes('--json') && r.stdout.startsWith('{') ? JSON.parse(r.stdout) : null };
}

// An invoice-like book whose cached values are all right.
const invoice = () => ({
  Invoice: {
    A1: 'Item', B1: 'Amount',
    A2: 'Parking', B2: 10,
    A3: 'Valet', B3: 20,
    A4: 'Fees', B4: 30.5,
    A5: 'Total', B5: { f: 'SUM(B2:B4)', v: 60.5 },
    B6: { f: 'ROUND(B5*0.0635,2)', v: 3.84 },
    B7: { f: 'B5+B6', v: 64.34 },
    C1: { f: 'IF(B7>60,"over","under")&" budget"', v: 'over budget' },
    C2: { f: 'IFERROR(B2/0,0)', v: 0 },
    C3: { f: 'AND(B2>0,OR(B3<0,NOT(FALSE)))', v: true },
    C4: { f: 'AVERAGE(B2:B4)+MIN(B2:B4)-MAX(B2:B4)+COUNT(B1:B4)*COUNTA(A1:A5)', v: 20.1666666667 - 20.5 + 15 },
    C5: { f: 'SUMPRODUCT(B2:B4,B2:B4)-PRODUCT(B2,B3)+ABS(-2)^2+50%', v: 10 * 10 + 20 * 20 + 30.5 * 30.5 - 200 + 4 + 0.5 },
    C6: { f: 'ROUNDUP(2.121,2)+ROUNDDOWN(-2.129,2)+ROUND(2.675,2)+ROUND(-2.5,0)', v: 2.13 - 2.12 + 2.68 - 3 },
    C7: { f: 'SUM(B:B)', v: 60.5 * 2 + 3.84 + 64.34 },
  },
});

test('a clean book passes, deflated entries included', () => {
  const r = cli(xlsx(invoice(), { deflate: ['xl/worksheets/sheet1.xml', 'xl/sharedStrings.xml'] }), ['--json']);
  assert.equal(r.code, 0, r.out + r.err);
  assert.deepEqual(r.json.findings, []);
  assert.equal(r.json.counts.formulas, 10);
  assert.equal(r.json.counts.checked, 10);
  assert.equal(r.json.counts.unchecked, 0);
  assert.equal(r.json.tolerance.absolute, 0.005);
  assert.match(cli(xlsx(invoice())).out, /Result: clean/);
});

test('a planted wrong total is caught: a cached value that disagrees with SUM', () => {
  const book = invoice();
  book.Invoice.B5 = { f: 'SUM(B2:B4)', v: 999 };
  const r = cli(xlsx(book), ['--json']);
  assert.equal(r.code, 1);
  const f = r.json.findings.find(x => x.cell === 'B5');
  assert.deepEqual([f.kind, f.formula, f.cached, f.computed], ['mismatch', '=SUM(B2:B4)', 999, 60.5]);
});

test('a SUM that skips a row is caught by --expect against the stated total (no cached values)', () => {
  const book = { Invoice: { B2: 10, B3: 20, B4: 30.5, B5: { f: 'SUM(B2:B3)' } } };
  const r = cli(xlsx(book), ['--expect', 'Invoice!B5=60.50', '--json']);
  assert.equal(r.code, 1);
  assert.equal(r.json.counts.noCachedValue, 1);
  assert.deepEqual(r.json.expects.map(e => [e.cell, e.expected, e.computed, e.ok]), [['B5', 60.5, 30, false]]);
  assert.equal(r.json.findings.length, 1);
  assert.equal(r.json.findings[0].kind, 'expect');
  // The same book with the right formula meets its stated total, written with a thousands comma.
  book.Invoice.B5 = { f: 'SUM(B2:B4)' };
  book.Invoice.B6 = { f: 'B5*100' };
  const ok = cli(xlsx(book), ['--expect', 'invoice!B5=60.5', '--expect', 'B6=6,050', '--json']);
  assert.equal(ok.code, 0, ok.out);
  assert.ok(ok.json.expects.every(e => e.ok));
});

test('a #REF! formula and computed or cached errors are caught', () => {
  const r = checkWorkbook(xlsx({
    S: {
      A1: 5,
      A2: { f: 'SUM(#REF!)+A1', e: '#REF!' },
      A3: { f: 'IFERROR(S!#REF!,0)', v: 0 },
      A4: { f: 'A1/0', e: '#DIV/0!' },
      A5: { f: 'A1*2', v: 10 },
      A6: { e: '#N/A' },
    },
  }));
  const kinds = Object.fromEntries(r.findings.map(f => [f.cell, f.kind]));
  assert.deepEqual(kinds, { A2: 'broken-ref', A3: 'broken-ref', A4: 'error', A6: 'error' });
  assert.equal(r.findings.find(f => f.cell === 'A4').computed, '#DIV/0!');
});

test('a shared formula expands by relative offset', () => {
  assert.equal(shiftFormula("B2*$C$1+'My Sheet'!A$1+SUM(B$2:B2)", 2, 1), "C4*$C$1+'My Sheet'!B$1+SUM(C$2:C4)");
  assert.equal(shiftFormula('A1+1', -1, 0), '#REF!+1');
  const sheet = {
    B2: 10, B3: 20, B4: 30.5, C1: 2,
    D2: { f: 'B2*$C$1', shared: { si: 0, ref: 'D2:D4' }, v: 20 },
    D3: { f: '', shared: { si: 0 }, v: 40 },
    D4: { f: '', shared: { si: 0 }, v: 60 },
  };
  const r = checkWorkbook(xlsx({ S: sheet }));
  assert.equal(r.counts.checked, 3);
  assert.deepEqual(r.findings.map(f => [f.cell, f.formula, f.computed]), [['D4', '=B4*$C$1', 61]]);
});

test('a cross-sheet quoted reference works', () => {
  const r = checkWorkbook(xlsx({
    "Bob's Lines": { B1: 2.5, B2: 4 },
    Summary: {
      A1: { f: "SUM('Bob''s Lines'!B1:B2)", v: 6.5 },
      A2: { f: "'Bob''s Lines'!B2*Summary!A1", v: 26 },
    },
  }), ["'Bob''s Lines'!B2=4", 'Summary!A2=26']);
  assert.deepEqual(r.findings, []);
  assert.equal(r.counts.checked, 2);
  assert.ok(r.expects.every(e => e.ok));
});

test('an unsupported function is reported unchecked, not as a finding, and so are its dependents', () => {
  const r = cli(xlsx({ S: { A1: { f: 'VLOOKUP(3,B1:C5,2,FALSE)', v: 5 }, A2: { f: 'A1+1', v: 999 }, A3: { f: 'TaxRate*2', v: 1 } } }), ['--json']);
  assert.equal(r.code, 0, r.out);
  assert.deepEqual(r.json.findings, []);
  assert.deepEqual(r.json.unchecked.map(u => [u.cell, u.reason]), [
    ['A1', "function VLOOKUP isn't supported"],
    ['A2', 'depends on S!A1'],
    ['A3', 'defined name TaxRate'],
  ]);
  // An expectation on an unchecked cell can't be met: it is a finding, never a guess.
  const e = cli(xlsx({ S: { A1: { f: 'VLOOKUP(3,B1:C5,2,FALSE)', v: 5 } } }), ['--expect', 'S!A1=5', '--json']);
  assert.equal(e.code, 1);
  assert.match(e.json.findings[0].detail, /could not recompute/);
});

test('a cycle is reported, not a hang', () => {
  const r = checkWorkbook(xlsx({ S: { A1: { f: 'B1+1', v: 1 }, B1: { f: 'A1+1', v: 2 }, C1: { f: 'A1*2', v: 2 }, D1: { f: 'D1', v: 0 } } }));
  assert.deepEqual(r.findings.map(f => [f.cell, f.kind]).sort(), [['A1', 'cycle'], ['B1', 'cycle'], ['D1', 'cycle']]);
  assert.deepEqual(r.unchecked.map(u => [u.cell, u.reason]), [['C1', 'depends on a circular reference']]);
});

test('a long running-total column is checked without running out of stack', () => {
  const sheet = { A1: 1 };
  for (let i = 2; i <= 20000; i++) sheet[`A${i}`] = { f: `A${i - 1}+1`, v: i };
  const r = checkWorkbook(xlsx({ S: sheet }));
  assert.equal(r.counts.checked, 19999);
  assert.deepEqual(r.findings, []);
});

test('a corrupt file, a missing file, a too-large entry and bad arguments exit 2', () => {
  assert.equal(cli(Buffer.from('this is not a zip file at all, just text')).code, 2);
  const good = xlsx(invoice());
  assert.equal(cli(good.subarray(0, good.length - 40)).code, 2);
  const corrupt = cli(Buffer.concat([good.subarray(0, 200), Buffer.alloc(300, 7), good.subarray(500)]), ['--json']);
  assert.equal(corrupt.code, 2);
  const big = cli(xlsx(invoice(), { sizeOverride: { 'xl/worksheets/sheet1.xml': MAX_ENTRY_BYTES + 1 } }));
  assert.equal(big.code, 2);
  assert.match(big.err, /over 64 MB/);
  const missing = spawnSync(process.execPath, [SCRIPT, join(dir, 'nope.xlsx')], { encoding: 'utf8' });
  assert.equal(missing.status, 2);
  assert.equal(cli(good, ['--expect', 'Invoice!B5=lots']).code, 2);
  assert.equal(cli(good, ['--expect', 'Nowhere!B5=1']).code, 2);
  assert.equal(cli(good, ['--bogus']).code, 2);
});
