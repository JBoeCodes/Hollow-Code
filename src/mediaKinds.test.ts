import { afterEach, describe, expect, it } from 'vitest';
import MEDIA_RS from '../src-tauri/src/media.rs?raw';
import {
  MEDIA_BUDGET, VIDEO_EXTS, classifyFailure, isVideoPath, mountedMediaCount, problemCopy, problemHasActions,
  refusalOf, releaseMediaSlot, takeMediaSlot, type MediaSlot,
} from './mediaKinds';
import { chatArtifactKind } from './chatArtifactKind';

describe('video kinds', () => {
  it('matches the backend list in media.rs', () => {
    const m = /pub const VIDEO_EXTS: &\[&str\] = &\[([^\]]*)\]/.exec(MEDIA_RS);
    expect(m, 'media.rs VIDEO_EXTS').not.toBeNull();
    const rust = [...m![1].matchAll(/"([a-z0-9]+)"/g)].map(x => x[1]);
    expect(rust).toEqual([...VIDEO_EXTS]);
  });

  it('plays mp4, m4v and mov, not webm, mkv or avi', () => {
    for (const p of ['/a/b.mp4', 'x.M4V', 'clip.mov']) expect(isVideoPath(p), p).toBe(true);
    for (const p of ['a.webm', 'a.mkv', 'a.avi', 'mp4', 'a.mp4.txt']) expect(isVideoPath(p), p).toBe(false);
  });

  it('classifies artifacts: m4v is video, mkv and webm stay files', () => {
    expect(chatArtifactKind('/x/render.m4v')).toBe('video');
    expect(chatArtifactKind('/x/render.MP4')).toBe('video');
    expect(chatArtifactKind('/x/render.mov')).toBe('video');
    expect(chatArtifactKind('/x/render.mkv')).toBe('binary');
    expect(chatArtifactKind('/x/render.webm')).toBe('binary');
  });
});

describe('grant failures', () => {
  const first = { size: 100, modifiedMs: 5 };
  it('reads the refusal code from a rejected grant', () => {
    expect(refusalOf('MISSING')).toBe('MISSING');
    expect(refusalOf(new Error('OUTSIDE_ROOTS'))).toBe('OUTSIDE_ROOTS');
    expect(refusalOf('something else')).toBeNull();
  });
  it('retries once on a changed file, then says it is still being written', () => {
    expect(classifyFailure(first, { size: 200, modifiedMs: 5 }, false, false)).toBe('retry');
    expect(classifyFailure(first, { size: 100, modifiedMs: 9 }, false, false)).toBe('retry');
    expect(classifyFailure(first, { size: 200, modifiedMs: 9 }, false, true)).toBe('changed');
  });
  it('names a format problem only for an unchanged file WebKit can’t decode', () => {
    expect(classifyFailure(first, { size: 100, modifiedMs: 5 }, true, false)).toBe('format');
    expect(classifyFailure(first, { size: 100, modifiedMs: 5 }, false, false)).toBe('failed');
    expect(classifyFailure(first, 'MISSING', true, false)).toBe('MISSING');
  });
  it('words every problem and keeps actions unless the file is gone', () => {
    expect(problemCopy('MISSING')).toBe('File not found');
    expect(problemCopy('OUTSIDE_ROOTS')).toBe('Outside Hollow’s folders');
    expect(problemCopy('format')).toBe('This video’s format doesn’t play inside Hollow');
    expect(problemCopy('changed')).toMatch(/Still being written/);
    expect(problemHasActions('MISSING')).toBe(false);
    expect(problemHasActions('HIDDEN')).toBe(true);
  });
});

describe('the video budget', () => {
  const taken: MediaSlot[] = [];
  const slot = (explicit: boolean, paused = true, seen = 0) => {
    const s: MediaSlot & { evicted: boolean } = {
      explicit, evicted: false, paused: () => paused, lastVisible: () => seen,
      evict: () => { s.evicted = true; releaseMediaSlot(s); },
    };
    return s;
  };
  afterEach(() => { for (const s of taken.splice(0)) releaseMediaSlot(s); });
  const take = (s: MediaSlot) => { const ok = takeMediaSlot(s); if (ok) taken.push(s); return ok; };

  it('lets automatic inline mounts wait when the budget is full', () => {
    for (let i = 0; i < MEDIA_BUDGET; i++) expect(take(slot(false))).toBe(true);
    expect(take(slot(false))).toBe(false);
    expect(mountedMediaCount()).toBe(MEDIA_BUDGET);
  });

  it('evicts the least recently visible paused inline player for an explicit open, never a playing one', () => {
    const playing = slot(false, false, 0);
    const old = slot(false, true, 1);
    const newer = slot(false, true, 5);
    const pane = slot(true, true, 9);
    for (const s of [playing, old, newer, pane]) take(s);
    const result = slot(true);
    expect(take(result)).toBe(true);
    expect(old.evicted).toBe(true);
    expect(playing.evicted).toBe(false);
    expect(newer.evicted).toBe(false);
    expect(mountedMediaCount()).toBe(MEDIA_BUDGET);
  });
});
