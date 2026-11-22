import { describe, expect, it } from 'vitest';
import { checkFailed, checkState, checkWords, dbLine, dbSize, dbSummary, ownerWords, staleLine } from './hollowDatabases';
import type { HollowDatabases, HollowDbFile } from './types';

const MB = 1024 ** 2;

function file(over: Partial<HollowDbFile> = {}): HollowDbFile {
  return {
    kind: 'searchIndex', stem: 'hollow-vault-1d1d4c39cd10e040', path: '/S/SearchIndex/hollow-vault-1d1d4c39cd10e040.db',
    owner: 'thisVault', ownerName: null, bytes: 86 * MB, walBytes: 0, modified: null, schemaVersion: 2,
    quickCheck: null, tables: null, error: null, ...over,
  };
}

describe("Hollow's databases words", () => {
  it('keeps small files readable and large ones short', () => {
    expect(dbSize(40 * 1024)).toBe('40 KB');
    expect(dbSize(0.3 * MB)).toBe('0.3 MB');
    expect(dbSize(86.5 * MB)).toBe('87 MB');
    expect(dbSize(1.5 * 1024 * MB)).toBe('1.5 GB');
  });

  it("says whose each file is, and that another vault's conversations are kept", () => {
    expect(ownerWords(file())).toBe('this vault');
    expect(ownerWords(file({ kind: 'companion', owner: 'app' }))).toBe('Hollow');
    expect(ownerWords(file({ owner: 'otherVault', ownerName: 'Second-brain' }))).toBe("Second-brain: another vault's");
    expect(ownerWords(file({ kind: 'sessions', owner: 'otherVault', ownerName: 'Second-brain' })))
      .toBe("Second-brain: another vault's conversations · kept");
  });

  it('names the kind, owner, size, WAL and version on one line', () => {
    expect(dbLine(file({ walBytes: 16 * MB }))).toBe('Search index · this vault · 86 MB + 16 MB WAL · version 2');
    expect(dbLine(file({ kind: 'embedIndex', schemaVersion: null }))).toBe('Meaning-search index · this vault · 86 MB');
    // Repo features R14b: a vault's content index of its Projects folder.
    expect(dbLine(file({ kind: 'projectIndex', schemaVersion: 1 }))).toBe('Project contents index · this vault · 86 MB · version 1');
    expect(ownerWords(file({ kind: 'projectIndex', owner: 'otherVault', ownerName: 'Second-brain' }))).toBe("Second-brain: another vault's");
  });

  it('says nothing about integrity before a check, then the result and row counts', () => {
    expect(checkWords(file())).toBe('');
    expect(checkWords(file({ error: 'not an SQLite database' }))).toBe("Couldn't read it: not an SQLite database");
    const ok = file({ quickCheck: 'ok', tables: [{ name: 'notes', rows: 5200 }, { name: 'meta', rows: 1 }] });
    expect(checkWords(ok)).toBe('Integrity ok · notes 5,200 rows · meta 1 row');
    expect(checkFailed(ok)).toBe(false);
    const bad = file({ quickCheck: 'row 3 missing from index x', tables: [] });
    expect(checkWords(bad)).toBe('Integrity: row 3 missing from index x');
    expect(checkFailed(bad)).toBe(true);
    // An FTS file passes with the note the backend adds.
    const fts = file({ quickCheck: 'ok (FTS index not validated read-only)', tables: [] });
    expect(checkWords(fts)).toBe('Integrity ok (FTS index not validated read-only)');
    expect(checkState(fts)).toBe('ok');
  });

  it('reads a check that ran out of time as unknown, not as needing attention', () => {
    const late = file({ quickCheck: 'not finished within 30 s', tables: [] });
    expect(checkState(late)).toBe('unknown');
    expect(checkFailed(late)).toBe(false);
    expect(checkWords(late)).toBe('Integrity unknown: not finished within 30 s');
    const r: HollowDatabases = { folder: '/S', files: [file({ quickCheck: 'ok' }), late], staleCount: 0, staleBytes: 0 };
    expect(dbSummary(r, true)).toBe('2 files, 172 MB, in /S. 1 not checked in time.');
  });

  it('sums the files, and after a check says how many need attention', () => {
    const r: HollowDatabases = { folder: '/S', files: [file(), file({ walBytes: 14 * MB })], staleCount: 0, staleBytes: 0 };
    expect(dbSummary(r, false)).toBe('2 files, 186 MB, in /S');
    expect(dbSummary({ ...r, files: [file({ quickCheck: 'ok' }), file({ quickCheck: 'bad page' })] }, true)).toBe('2 files, 172 MB, in /S. 1 needs attention.');
    expect(dbSummary({ ...r, files: [file({ quickCheck: 'ok' })] }, true)).toBe('1 file, 86 MB, in /S. Every file checked ok.');
  });

  it("counts a file a check couldn't open, and never says every file checked ok then (A4)", () => {
    const unreadable = file({ error: 'unable to open database file' });
    const r: HollowDatabases = { folder: '/S', files: [file({ quickCheck: 'ok' }), unreadable], staleCount: 0, staleBytes: 0 };
    expect(dbSummary(r, true)).toBe("2 files, 172 MB, in /S. 1 couldn't be opened.");
    expect(dbSummary({ ...r, files: [unreadable, unreadable] }, true)).toBe("2 files, 172 MB, in /S. 2 couldn't be opened.");
    expect(dbSummary({ ...r, files: [file({ quickCheck: 'bad page' }), unreadable] }, true))
      .toBe("2 files, 172 MB, in /S. 1 needs attention. 1 couldn't be opened.");
    // Measure reads only headers: an unreadable one is shown on its row, and the summary makes no claim.
    expect(dbSummary({ ...r, files: [file(), unreadable] }, false)).toBe('2 files, 172 MB, in /S');
  });

  it("points to Disk space for other vaults' old indexes, and says nothing when there are none", () => {
    const r: HollowDatabases = { folder: '/S', files: [], staleCount: 0, staleBytes: 0 };
    expect(staleLine(r)).toBe('');
    expect(staleLine({ ...r, staleCount: 6, staleBytes: 405 * MB })).toBe('6 old indexes of other vaults use 405 MB: Disk space → Remove old indexes… removes them.');
    expect(staleLine({ ...r, staleCount: 1, staleBytes: 2 * MB })).toBe('1 old index of other vaults uses 2.0 MB: Disk space → Remove old indexes… removes it.');
  });
});
