import { describe, expect, it } from 'vitest';
import { NOTHING_STALE, NOTHING_TO_CLEAN, SIZES_UNKNOWN, cleanConfirmBody, cleanConfirmTitle, cleanedText, cleanupErrorText, diskSummary, folderSize, nothingToCleanText, parseThreshold, prunedText, pruneConfirmBody, pruneConfirmTitle, removedIndexesText, staleConfirmBody, staleConfirmTitle, withThreshold } from './diskGuard';
import type { DiskReport, InstallTargetsPlan, StaleIndexesPlan, WorktreeHygieneReport } from './types';

const GB = 1024 ** 3;

describe('disk space words', () => {
  it('says how much is free against the threshold, and when it is low', () => {
    expect(diskSummary({ freeBytes: 74 * GB, thresholdGb: 50, low: false, folders: [], cleanup: [] })).toBe('74 GB free · Hollow warns under 50 GB');
    expect(diskSummary({ freeBytes: 40 * GB, thresholdGb: 50, low: true, folders: [], cleanup: [] })).toBe('40 GB free: under the 50 GB Hollow warns at');
    expect(diskSummary({ freeBytes: null, thresholdGb: 50, low: false, folders: [], cleanup: [] })).toContain("couldn't be read");
  });

  it('names the vault\'s volume when it is another one (morning N6)', () => {
    const base = { thresholdGb: 50, folders: [], cleanup: [] };
    expect(diskSummary({ ...base, freeBytes: 400 * GB, low: true, vault: { path: '/Volumes/Ext', freeBytes: 9 * GB, low: true } }))
      .toBe("400 GB free; the vault's volume has 9 GB free: the vault's volume is under the 50 GB Hollow warns at");
    expect(diskSummary({ ...base, freeBytes: 40 * GB, low: true, vault: { path: '/Volumes/Ext', freeBytes: 9 * GB, low: true } }))
      .toBe("40 GB free; the vault's volume has 9 GB free: both drives are under the 50 GB Hollow warns at");
    expect(diskSummary({ ...base, freeBytes: 40 * GB, low: true, vault: { path: '/Volumes/Ext', freeBytes: 90 * GB, low: false } }))
      .toBe("40 GB free; the vault's volume has 90 GB free: your home drive is under the 50 GB Hollow warns at");
    expect(diskSummary({ ...base, freeBytes: null, low: true, vault: { path: '/Volumes/Ext', freeBytes: 9 * GB, low: true } }))
      .toBe("Free space couldn't be read; the vault's volume has 9 GB free: the vault's volume is under the 50 GB Hollow warns at");
    // A new threshold applies to the last measure without measuring again.
    const r = withThreshold({ ...base, freeBytes: 74 * GB, low: false, vault: { path: '/V', freeBytes: 90 * GB, low: false } }, 80);
    expect([r.thresholdGb, r.low, r.vault?.low]).toEqual([80, true, false]);
    expect(diskSummary({ ...base, freeBytes: 74 * GB, low: false, vault: { path: '/Volumes/Ext', freeBytes: 300 * GB, low: false } }))
      .toBe("74 GB free; the vault's volume has 300 GB free · Hollow warns under 50 GB");
  });

  it('accepts a whole number of GB from 1 to 10,000 as the threshold', () => {
    expect(parseThreshold(' 80 ')).toEqual({ gb: 80 });
    expect(parseThreshold('0')).toEqual({ error: 'Warn under 1 to 10,000 GB.' });
    expect(parseThreshold('10001')).toEqual({ error: 'Warn under 1 to 10,000 GB.' });
    expect(parseThreshold('12.5')).toEqual({ error: 'Enter a whole number of GB.' });
    expect(parseThreshold('')).toEqual({ error: 'Enter a whole number of GB.' });
  });

  it('sizes a folder in GB or MB, and never guesses one it could not size', () => {
    expect(folderSize({ path: '/x', kind: 'cargo-target', bytes: 142 * GB })).toBe('Build cache · 142 GB');
    expect(folderSize({ path: '/x', kind: 'worktree', bytes: 1.5 * GB })).toBe('Worktree · 1.5 GB');
    expect(folderSize({ path: '/x', kind: 'cache', bytes: 300 * 1024 ** 2 })).toBe('Cache · 300 MB');
    expect(folderSize({ path: '/x', kind: 'worktree', bytes: null })).toBe('Worktree · not sized in time');
  });
});

// Overnight H20: the words of the two cleanups' confirm dialogs.
describe('disk space cleanups', () => {
  const measured: DiskReport = { freeBytes: 40 * GB, thresholdGb: 50, low: true, folders: [
    { path: '/C/hollow-install-target-a', kind: 'cargo-target', bytes: 12 * GB },
    { path: '/C/hollow-install-target-b', kind: 'cargo-target', bytes: null },
    { path: '/W/done', kind: 'worktree', bytes: 6 * GB },
  ], cleanup: [] };
  const plan: InstallTargetsPlan = {
    targets: [
      { path: '/C/hollow-install-target-main', name: 'hollow-install-target-main', lastUsed: '2026-10-14T21:10:00Z', keep: 'built the installed app' },
      { path: '/C/hollow-install-target-a', name: 'hollow-install-target-a', lastUsed: '2026-10-03T18:00:00Z', keep: null },
      { path: '/C/hollow-install-target-b', name: 'hollow-install-target-b', lastUsed: null, keep: null },
    ],
    remove: ['/C/hollow-install-target-a', '/C/hollow-install-target-b', '/C/hollow-install-target-x.deleting'],
    keepNewest: 2, hash: 'h', notes: ['No installer journal names the rollback copy.'], leftovers: ['/C/hollow-install-target-x.deleting'],
  };

  it('lists every install build folder that goes, with measured sizes, and every one that stays and why', () => {
    expect(pruneConfirmTitle(plan)).toBe('Delete 3 old install build folders?');
    const body = pruneConfirmBody(plan, measured).split('\n');
    expect(body[0]).toContain('Deletes 3 install build folders in ~/Library/Caches, at least 12 GB as last measured.');
    expect(body[0]).toContain('The apps in /Applications are not touched.');
    expect(body).toContain('• hollow-install-target-a, last used 2026-10-03 · 12 GB');
    expect(body).toContain('• hollow-install-target-b');
    expect(body).toContain('• hollow-install-target-x.deleting (left by an interrupted delete)');
    expect(body).toContain('• hollow-install-target-main (built the installed app, last used 2026-10-14)');
    expect(body).toContain('No installer journal names the rollback copy.');
    expect(body.at(-1)).toBe('If this list changes before you confirm, nothing is deleted.');
    expect(body).not.toContain(SIZES_UNKNOWN);
    // Not measured yet: no sizes, never a guess, and it says why.
    expect(pruneConfirmBody(plan, null)).not.toMatch(/GB|MB/);
    expect(pruneConfirmBody(plan, null).split('\n')[1]).toBe('Sizes unknown: press Check disk space first.');
  });

  it('lists the worktrees and "~" folders that go, what stays, and that branches are kept', () => {
    const r: WorktreeHygieneReport = {
      repos: [{ repo: '/R', defaultBranch: 'origin/main', worktrees: [
        { path: '/W/done', branch: 'lane/done', workspace: 'ws-1', merged: true, clean: true, unpushed: 0, ignored: ['node_modules'], ignoredCount: 3, inUse: false, removable: true, reason: null },
        { path: '/W/open', branch: 'lane/open', workspace: null, merged: false, clean: false, unpushed: 1, ignored: [], ignoredCount: 0, inUse: true, removable: false, reason: 'in use' },
      ], localOnlyBranches: [{ name: 'spike', commits: 2, lastCommit: '2026-10-10' }], tildeFolders: [{ path: '/W/done/~', bytes: 1.5 * GB }], error: null }],
      remove: ['/W/done', '/W/done/~'], hash: 'h', tildeBytes: 1.5 * GB, inUseUnknown: false,
    };
    expect(cleanConfirmTitle(r)).toBe('Remove 1 finished worktree and 1 "~" folder?');
    const body = cleanConfirmBody(r, measured).split('\n');
    expect(body[0]).toMatch(/^Removes 1 worktree that is merged, unchanged, fully pushed and not in use\. Branches are kept/);
    expect(body).toContain('• /W/done [lane/done] (workspace ws-1) · 6.0 GB — also deletes node_modules and 2 more');
    expect(body).toContain('Deletes 1 "~" folder of stray build output (1.5 GB):');
    expect(body).toContain('• /W/done/~ · 1.5 GB');
    expect(body).toContain('Kept: 1 worktree (in use, changed, unpushed, unmerged, locked, just started, built the installed or rollback app, or needed by a build waiting to install).');
    expect(body).toContain('Not touched: 1 branch with commits no remote has.');
    expect(body.at(-1)).toBe('If this list changes before you confirm, nothing is removed.');
    expect(body).not.toContain(SIZES_UNKNOWN);
    expect(cleanConfirmBody(r, null).split('\n')).toContain(SIZES_UNKNOWN);
    expect(cleanConfirmBody({ ...r, inUseUnknown: true }, null)).toMatch(/^Hollow couldn't read which programs are running, so no worktree is removed\./);
    // Running programs unreadable: the list is always empty, so no dialog
    // opens; the status line says why instead of "nothing to remove".
    const unread = { ...r, remove: [], inUseUnknown: true };
    expect(nothingToCleanText(unread)).toBe("Hollow couldn't read which programs are running, so no worktree is removed. Nothing changed.");
    expect(nothingToCleanText({ ...unread, inUseUnknown: false })).toBe(NOTHING_TO_CLEAN);
  });

  it("says a list that changed since the dialog in the person's words, and passes other errors through", () => {
    // The backend's refusals (Rust install_targets::prune_in, worktree_hygiene::clean_for).
    expect(cleanupErrorText('The folders to delete changed since the card was shown. Nothing was deleted; ask again to see the new list.', 'deleted'))
      .toBe('The list changed since you saw it. Nothing was deleted; press the button again to see the new list.');
    expect(cleanupErrorText('The worktrees and folders to remove changed since the card was shown. Nothing was removed; ask again to see the new list.', 'removed'))
      .toBe('The list changed since you saw it. Nothing was removed; press the button again to see the new list.');
    expect(cleanupErrorText('No Caches folder', 'deleted')).toBe('No Caches folder');
  });

  it('says what a cleanup did, naming any folder it could not delete', () => {
    expect(prunedText({ removed: ['/C/a', '/C/b'], failed: [], kept: 2 })).toBe('Deleted 2 install build folders.');
    expect(prunedText({ removed: [], failed: [['/C/a', 'busy']], kept: 2 })).toBe("Nothing was deleted. Couldn't delete a (busy).");
    expect(cleanedText({ removed: ['/W/done', '/W/done/~'], failed: [] })).toBe('Removed 2 worktrees and folders.');
    expect(cleanedText({ removed: ['/W/done'], failed: [] })).toBe('Removed 1 worktree or folder.');
    expect(cleanedText({ removed: ['/W/a'], failed: [], remaining: ['/W/b', '/W/c'], volumes: [{ path: '/V', freeBefore: 10 * 1024 ** 3, freeAfter: 12 * 1024 ** 3, freed: 2 * 1024 ** 3 }], freeBefore: 10 * 1024 ** 3, freeAfter: 12 * 1024 ** 3, freed: 2 * 1024 ** 3 }))
      .toBe('Removed 1 worktree or folder. 2 more are still due: run it again. Free space read from the disk: 10.0 GB before, 12.0 GB after (2.0 GB freed). That is the measured gain, not a size estimate; local snapshots can hold deleted files and keep it near 0 until they go.')
    expect(cleanedText({ removed: [], failed: [], volumes: [], freeBefore: 0, freeAfter: 0, freed: 0 })).toBe('Nothing was removed.');
  });

  // Repo features R7b: Remove old indexes….
  it("lists every other vault's index that goes with its size and the total, and what stays", () => {
    const MB = 1024 ** 2;
    const plan: StaleIndexesPlan = {
      folder: '/S', bytes: 210 * MB, hash: 'h',
      files: [
        { path: '/S/SearchIndex/Second-brain-0123456789abcdef.db', kind: 'searchIndex', ownerName: 'Second-brain', bytes: 120 * MB, modified: '2026-09-30T18:00:00-04:00', why: null },
        { path: '/S/EmbedIndex/Vault-0123456789abcdef.db', kind: 'embedIndex', ownerName: 'Vault', bytes: 90 * MB, modified: null, why: null },
      ],
      kept: [{ path: '/S/Sessions/Vault-0123456789abcdef.db', kind: 'sessions', ownerName: 'Vault', why: "another vault's conversations: never removed" }],
    };
    expect(staleConfirmTitle(plan)).toBe('Remove 2 old indexes?');
    expect(staleConfirmTitle({ ...plan, files: plan.files.slice(1) })).toBe('Remove 1 old index?');
    const body = staleConfirmBody(plan);
    expect(body).toContain('Removes 2 indexes of other vaults from /S, 210 MB in all.');
    expect(body).toContain('• Second-brain-0123456789abcdef.db (Second-brain, search index, last changed 2026-09-30) · 120 MB');
    expect(body).toContain('• Vault-0123456789abcdef.db (Vault, meaning-search index) · 90 MB');
    expect(body).toContain("These stay:\n• Vault-0123456789abcdef.db (Vault, conversations): another vault's conversations: never removed");
    expect(body).toContain("This vault's files and every Sessions database are never removed.");
    expect(body.endsWith('If this list changes before you confirm, nothing is removed.')).toBe(true);
    // The backend's changed-list refusal reads like the other two cleanups'.
    expect(cleanupErrorText('The old indexes to remove changed since the card was shown. Nothing was removed; ask again to see the new list.', 'removed'))
      .toBe('The list changed since you saw it. Nothing was removed; press the button again to see the new list.');
    expect(NOTHING_STALE).toContain('Nothing changed.');
    // A tombstone an interrupted removal left says so instead of a date.
    const tomb = { path: '/S/EmbedIndex/Vault-0123456789abcdef.db.removing', kind: 'embedIndex' as const, ownerName: 'Vault', bytes: 2 * MB, modified: null, why: 'left by an interrupted removal' };
    expect(staleConfirmBody({ ...plan, files: [tomb] })).toContain('• Vault-0123456789abcdef.db.removing (Vault, meaning-search index, left by an interrupted removal) · 2 MB');
  });

  it('says what Remove old indexes… did, naming any file it could not remove', () => {
    expect(removedIndexesText({ removed: ['/S/a.db', '/S/b.db'], failed: [], bytes: 270 * 1024 ** 2 })).toBe('Removed 2 old indexes, 270 MB.');
    expect(removedIndexesText({ removed: [], failed: [['/S/a.db-shm', 'busy']], bytes: 0 })).toBe("Nothing was removed. Couldn't remove a.db-shm (busy).");
  });
});
