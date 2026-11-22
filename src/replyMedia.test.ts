import { describe, expect, it } from 'vitest';
import { REPLY_VIDEO_CAP, receiptVideos, replyVideos, splitReplyVideos } from './replyMedia';
import type { ExecutionReceipt } from './executionReceipt';

const receipt = (over: Partial<ExecutionReceipt>): ExecutionReceipt => ({ files: [], ...over } as unknown as ExecutionReceipt);
const file = (path: string, changeType = 'create') => ({ path, changeType, attribution: 'observed', tool: null, additions: null, deletions: null });

describe('the videos under a reply (inline video)', () => {
  it('keeps created and changed receipt videos, drops deletes and other files, joins vault rows to the vault', () => {
    const r = receipt({ files: [file('workspaces/w/outputs/render.mp4'), file('workspaces/w/outputs/old.mov', 'delete'), file('notes/a.md', 'modify'), file('workspaces/w/outputs/cut.MOV', 'modify')] as never });
    expect(receiptVideos(r, '/vault')).toEqual(['/vault/workspaces/w/outputs/render.mp4', '/vault/workspaces/w/outputs/cut.MOV']);
  });
  it('skips a vault row while the vault path is unknown', () => {
    expect(receiptVideos(receipt({ files: [file('outputs/render.mp4')] as never }), null)).toEqual([]);
  });
  it('joins repository rows to their root and skips clean ones', () => {
    const r = receipt({ repoChanges: [{ root: '/repo', label: 'repo', target: 'cwd', status: 'captured', files: [
      { path: 'out/demo.mp4', changeType: 'create', status: 'untracked', additions: 0, deletions: 0 },
      { path: 'out/same.mp4', changeType: 'modify', status: 'clean', additions: 0, deletions: 0 },
    ], totalFiles: 2 }] as never });
    expect(receiptVideos(r, '/vault')).toEqual(['/repo/out/demo.mp4']);
  });
  it('lists receipt rows before the text’s, each once', () => {
    const r = receipt({ files: [file('w/outputs/render.mp4')] as never });
    expect(replyVideos(r, 'See `outputs/render.mp4` and [the cut](outputs/cut.mp4).', '/vault/w', '/vault'))
      .toEqual(['/vault/w/outputs/render.mp4', '/vault/w/outputs/cut.mp4']);
  });
  it('shows three players and lists the rest', () => {
    const paths = ['/a/1.mp4', '/a/2.mp4', '/a/3.mp4', '/a/4.mp4', '/a/5.mp4'];
    expect(REPLY_VIDEO_CAP).toBe(3);
    expect(splitReplyVideos(paths)).toEqual({ shown: paths.slice(0, 3), more: ['/a/4.mp4', '/a/5.mp4'] });
  });
});
