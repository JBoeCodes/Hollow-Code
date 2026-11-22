import { describe, expect, it } from 'vitest';
import TOKENS_CSS from './styles/tokens.css?raw';

// T0 base layer: the names the 11-20 look lanes read, and the one-switch
// reduced-motion rule. Names are a contract: add, never rename.
const NAMES = [
  '--contrast-boost', '--contrast-border-boost', '--ink-boosted', '--muted-boosted', '--hairline-boosted',
  '--alpha-hover', '--alpha-edge', '--alpha-active', '--op-meta', '--op-disabled',
  '--lh-caption', '--lh-meta', '--fw-medium', '--ls-widest',
  '--sp-half', '--sp-1h', '--sp-2h', '--topbar-h', '--chat-measure', '--card-w', '--content-gutter', '--content-gutter-wide', '--hit-coarse',
  '--r-float', '--r-shell', '--r-sheet',
  '--z-card', '--z-modal', '--z-toast', '--z-menu', '--z-tooltip',
  '--elev-composer', '--elev-dialog', '--elev-popover',
  '--ease-standard', '--ease-decel', '--ease-drawer', '--ease-css-out', '--ease-toast',
  '--dur-hover', '--dur-collapse', '--dur-label', '--dur-attach', '--dur-settle', '--dur-stream-in',
  '--panel-duration', '--panel-ease', '--motion-scale',
  '--dur-skeleton', '--dur-pulse', '--dur-ping', '--dur-shine', '--steps-pulse', '--steps-ping', '--steps-shine',
];

describe('base layer tokens', () => {
  it('defines every contract name once', () => {
    for (const name of NAMES) {
      const hits = [...TOKENS_CSS.matchAll(new RegExp(`^\\s*${name}\\s*:`, 'gm'))];
      expect(hits.length, name).toBe(1);
    }
  });

  it('keeps the t3code values the lanes rely on', () => {
    expect(TOKENS_CSS).toContain('--ease-drawer: cubic-bezier(0.32, 0.72, 0, 1)');
    expect(TOKENS_CSS).toContain('--ease-standard: cubic-bezier(0.4, 0, 0.2, 1)');
    expect(TOKENS_CSS).toContain('--panel-duration: var(--panel-duration-user, 0ms)');
    expect(TOKENS_CSS).toContain('--topbar-h: 52px');
  });

  it('zeroes motion with one switch under either reduced-motion preference', () => {
    expect(TOKENS_CSS).toMatch(/:root\[data-motion="reduced"\]\s*\{\s*--motion-scale: 0;\s*--panel-duration: 0ms;/);
    expect(TOKENS_CSS).toMatch(/@media \(prefers-reduced-motion: reduce\)\s*\{\s*:root\s*\{\s*--motion-scale: 0;/);
  });

  it('adds no colour literal to the group (derived mixes only)', () => {
    const start = TOKENS_CSS.indexOf('/* ── Base layer');
    const end = TOKENS_CSS.indexOf('/* ── Liquid Glass');
    const group = TOKENS_CSS.slice(start, end);
    expect(group).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  });

  it('names no dashes or third-party marks', () => {
    const start = TOKENS_CSS.indexOf('/* ── Base layer');
    const group = TOKENS_CSS.slice(start, TOKENS_CSS.indexOf('/* ── Liquid Glass'));
    expect(group).not.toMatch(/[–—]/);
  });
});
