import { describe, expect, it } from 'vitest';
import { deliveryRows } from './delivery';
import type { AppBuildStatus, BuildStatus, SelfInstallReceipt } from './workspaceTypes';

const status = (over: Partial<BuildStatus> = {}): BuildStatus => ({ blocked: null, worktree: '/w', branch: 'feat/x', base: 'origin/main', head: 'abcdef1234567', ahead: 2, behind: 0, upstream: 'origin/feat/x', unpushed: 0, files: [], moreFiles: 0, commits: [], pr: { url: 'u', state: 'OPEN', number: 12 }, goal: '', ...over });
const build = (over: Partial<AppBuildStatus> = {}): AppBuildStatus => ({ blocked: null, workspaceId: 'w', isHollow: true, state: 'built', otherJob: null, log: null, logTail: '', startedAt: null, finishedAt: null, error: null, commit: 'abcdef1', branch: 'feat/x', ...over });
const receipt = (over: Partial<SelfInstallReceipt> = {}): SelfInstallReceipt => ({ result: { id: 'r1', ok: true, startedAt: '', finishedAt: '', commit: 'abcdef12', branch: 'feat/x', log: '', message: '' }, acknowledged: true, runningCommit: 'abcdef1234567', runningDirty: false, matchesRunning: true, summary: '', ...over });
const state = (rows: ReturnType<typeof deliveryRows>, step: string) => rows.find(r => r.step === step)?.state;

describe('delivery record (next-level P20)', () => {
  it('an open PR is published but not merged, built or installed', () => {
    const rows = deliveryRows({ status: status(), build: build({ state: 'idle', commit: null }), receipt: null });
    expect([state(rows, 'published'), state(rows, 'merged'), state(rows, 'built'), state(rows, 'installed')]).toEqual(['done', 'pending', 'pending', 'pending']);
  });
  it('a build waiting in the Update button stays pending until a matching receipt and running app', () => {
    const rows = deliveryRows({ status: status({ pr: { url: 'u', state: 'MERGED', number: 12 } }), build: build(), waitingInUpdate: true, receipt: receipt({ result: { ...receipt().result, commit: '9999999' }, runningCommit: '9999999aaaa' }) });
    expect(state(rows, 'scheduled')).toBe('current');
    expect(state(rows, 'installed')).toBe('pending');
    const done = deliveryRows({ status: status({ pr: { url: 'u', state: 'MERGED', number: 12 } }), build: build(), receipt: receipt() });
    expect(state(done, 'installed')).toBe('done');
    expect(state(deliveryRows({ status: status(), build: build(), receipt: receipt({ matchesRunning: false }) }), 'installed')).toBe('pending');
  });
  it('a failed install says the previous app was kept only when the installer confirmed it (slice 6)', () => {
    const failed = (state: 'failed' | 'unknown' | undefined) => deliveryRows({ status: status(), build: build(), receipt: receipt({ result: { ...receipt().result, ok: false }, state }) }).find(r => r.step === 'installed');
    expect(failed('failed')).toEqual({ step: 'installed', state: 'failed', evidence: 'The install of abcdef1 failed; the previous app was kept' });
    expect(failed('unknown')?.evidence).toBe('The install of abcdef1 failed; which app is installed now is unknown');
    expect(failed(undefined)?.evidence).not.toContain('previous app was kept');
  });
  it('a build from an older commit is not this branch built', () => {
    expect(state(deliveryRows({ status: status(), build: build({ commit: '1111111' }), receipt: null }), 'built')).toBe('pending');
  });
  it('an unreadable receipt is unknown, not "no install recorded" (P1-46)', () => {
    const row = deliveryRows({ status: status(), build: build(), receipt: null, receiptError: 'last-result.json is not valid JSON' }).find(r => r.step === 'installed');
    expect(row?.state).toBe('unknown');
    expect(row?.evidence).toContain("couldn't be read");
    expect(row?.evidence).not.toContain('No install recorded');
  });
  it('an install that contains this branch marks Installed (P1-46)', () => {
    const merged = { url: 'u', state: 'merged', number: 12 };
    const later = receipt({ result: { ...receipt().result, commit: '7777777' }, runningCommit: '7777777aaaa' });
    const rows = deliveryRows({ status: status({ pr: merged, installedContainsHead: true }), build: build(), receipt: later });
    expect(rows.find(r => r.step === 'installed')).toEqual({ step: 'installed', state: 'done', evidence: 'Install r1: 7777777 installed, contains this branch, and the running app reports it' });
    expect(state(deliveryRows({ status: status({ pr: merged, installedContainsHead: false }), build: build(), receipt: later }), 'installed')).toBe('pending');
    // Review: a fresh branch (HEAD = the base tip) isn't installed just because a later install contains the base.
    expect(state(deliveryRows({ status: status({ ahead: 0, pr: null, installedContainsHead: true }), build: build(), receipt: later }), 'installed')).toBe('pending');
  });
  it('merged with nothing ahead is committed (P1-46)', () => {
    const rows = deliveryRows({ status: status({ ahead: 0, pr: { url: 'u', state: 'merged', number: 12 } }), build: build(), receipt: null });
    expect(rows.find(r => r.step === 'committed')).toEqual({ step: 'committed', state: 'done', evidence: 'Merged into origin/main with PR #12' });
    expect(state(deliveryRows({ status: status({ ahead: 0, pr: null }), build: build(), receipt: null }), 'committed')).toBe('pending');
  });
  it('non-Hollow repositories stop at merged', () => {
    expect(deliveryRows({ status: status(), build: build({ isHollow: false }), receipt: null }).map(r => r.step)).toEqual(['local', 'committed', 'published', 'merged']);
  });
});
