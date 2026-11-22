import { describe, expect, it } from 'vitest';
import { previewFailure } from './filePreviewFailure';

describe('previewFailure', () => {
  it('words chat::preview_file’s refusals', () => {
    expect(previewFailure('read /v/src/App.tsx: No such file or directory (os error 2)')).toMatchObject({ kind: 'missing', detail: expect.stringContaining('os error 2') });
    expect(previewFailure('not a file: /v/outputs/canvases')).toMatchObject({ kind: 'folder', text: expect.stringContaining('folder') });
    expect(previewFailure('image too large to preview (14 MB)')).toMatchObject({ kind: 'too-large', text: expect.stringContaining('14 MB'), detail: null });
    // The read that stopped at the cap says only the limit (2026-11-04).
    expect(previewFailure('image too large to preview (limit 10 MB)')).toMatchObject({ kind: 'too-large', text: 'This picture is over 10 MB, too large to preview here. Open it in its own app instead.', detail: null });
    expect(previewFailure('preview path must be absolute').kind).toBe('missing');
  });
  it('falls back to the plain sentence, with the raw error under Details', () => {
    const f = previewFailure(new Error('read /v/a.md: Permission denied (os error 13)'));
    expect(f.kind).toBe('other');
    expect(f.text).toContain('isn’t allowed');
    expect(f.detail).toContain('os error 13');
    expect(previewFailure('something new')).toEqual({ kind: 'other', text: 'something new', detail: null });
  });
});
