import { describe, expect, it } from 'vitest';
import paneScaleSource from './paneScale.ts?raw';

// R03: resizing a pane reflows content without changing its type size. The
// width-driven --pane-scale this module used to write is gone; keep it gone.
const styles = import.meta.glob('./styles/*.css', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

describe('pane type scale (R03)', () => {
  it('no stylesheet scales text by pane width', () => {
    for (const [file, css] of Object.entries(styles)) expect(css.includes('--pane-scale'), file).toBe(false);
  });
  it('split panes only report their width', () => {
    expect(paneScaleSource).not.toMatch(/setProperty\(\s*'--pane-scale'/);
    expect(paneScaleSource).toMatch(/export function observePaneWidth/);
  });
});
