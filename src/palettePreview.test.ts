import { describe, expect, it, vi } from 'vitest';
import { createPalettePreview, previewText, previewablePath } from './palettePreview';

describe('palette preview ownership', () => {
  it('drops an older read and a response after dismissal', async () => {
    const pending: ((text: string) => void)[] = [];
    const reader = createPalettePreview(() => new Promise<string>(resolve => pending.push(resolve)));
    const receive = vi.fn();
    reader.load('a.md', receive);
    reader.load('b.md', receive);
    pending[0]('old');
    await Promise.resolve();
    expect(receive).not.toHaveBeenCalled();
    pending[1]('current');
    await Promise.resolve();
    expect(receive).toHaveBeenLastCalledWith({ ok: true, value: 'current' });
    reader.load('c.md', receive);
    reader.cancel();
    pending[2]('closed');
    await Promise.resolve();
    expect(receive).toHaveBeenCalledTimes(1);
  });
  it('refuses non-note and retired message paths before reading', () => {
    const read = vi.fn();
    const reader = createPalettePreview(read);
    for (const path of ['emails/a.md', 'teams/a.md', 'slack/a.md', '../notes/a.md', '/tmp/a.md', 'notes/a.png']) {
      expect(previewablePath(path)).toBe(false);
      reader.load(path, vi.fn());
    }
    expect(read).not.toHaveBeenCalled();
    expect(previewablePath('workspaces/a/WORKSPACE.md')).toBe(true);
  });
  it('bounds plain text and removes frontmatter from the reading excerpt', () => {
    expect(previewText('---\ntitle: hidden\n---\n# Outcome\n\nRead this')).toBe('# Outcome\n\nRead this');
    expect(previewText('a'.repeat(3000))).toHaveLength(1801);
  });
});
