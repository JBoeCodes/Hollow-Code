import { describe, expect, it, vi } from 'vitest';

vi.mock('../api', () => ({ api: {} }));
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (id: string) => `artifact://localhost/${id}` }));

import { NO_KIT, THEME_TOKENS, sandboxDocParts, sandboxHead, themePairs, themeVars } from './htmlSandbox';
import { isSafeThemeValue } from '../canvasBridge';

const NONCE = 'AAAAAAAAAAAAAAAAAAAAAAAA';

describe('htmlSandbox theme token sanitization', () => {
  it('filters malicious computed theme tokens through isSafeThemeValue and falls back safely', () => {
    const hostile: Record<string, string> = {
      '--ink': '#000}</style><script>alert(1)</script>',
      '--canvas': 'white;background:url(https://evil.example/x)',
      '--mono': '"JetBrains Mono"}</style><img src=x onerror=alert(1)>',
      '--display': 'system-ui\\61',
      '--text-scale': '1\n}</style>',
      '--primary': '#b5251c',
    };
    vi.stubGlobal('document', { documentElement: { dataset: {} } });
    vi.stubGlobal('getComputedStyle', () => ({ getPropertyValue: (n: string) => hostile[n] ?? '' }));
    try {
      const pairs = themePairs();
      const map = new Map(pairs);
      const fallback = (name: string) => THEME_TOKENS.find(([n]) => n === name)![2];
      expect(map.get('--app-ink')).toBe(fallback('--app-ink'));
      expect(map.get('--app-canvas')).toBe(fallback('--app-canvas'));
      expect(map.get('--app-mono')).toBe(fallback('--app-mono'));
      expect(map.get('--app-display')).toBe(fallback('--app-display'));
      expect(map.get('--app-text-scale')).toBe(fallback('--app-text-scale'));
      expect(map.get('--app-primary')).toBe('#b5251c');
      for (const [, value] of pairs) expect(isSafeThemeValue(value)).toBe(true);

      const vars = themeVars();
      expect(vars).not.toContain('</style>');
      expect(vars).not.toContain('<script>');
      expect(vars).not.toContain('evil.example');

      const head = sandboxHead('', 'offline', NO_KIT, NONCE);
      expect(head.match(/<\/style>/g)).toHaveLength(1);
      const parts = sandboxDocParts('', 'offline', NO_KIT, '<p>ok</p>');
      expect(parts.join(NONCE)).not.toContain('alert(1)');
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('sanitizes live --app-mono/--app-display/--app-text-scale overrides and untrusted look pairs', () => {
    const hostile: Record<string, string> = {
      '--mono': 'monospace}</style><script>evil()</script>',
      '--display': 'Inter, sans-serif',
      '--text-scale': '1.25;color:red',
    };
    vi.stubGlobal('document', { documentElement: { dataset: {} } });
    vi.stubGlobal('getComputedStyle', () => ({ getPropertyValue: (n: string) => hostile[n] ?? '' }));
    try {
      const look = {
        scheme: 'light' as const,
        pairs: [
          ['--app-mono', 'ui-monospace, monospace'],
          ['--app-display', 'system-ui'],
          ['--app-text-scale', 'bad}</style>'],
          ['--app-ink', '#111}</style>'],
          ['--app-canvas', '#fafafa'],
          ['--not-app-token', '#000'],
        ] as Array<[string, string]>,
        adjusted: [],
      };
      const map = new Map(themePairs(look));
      // Hostile live --mono falls back to look's safe value.
      expect(map.get('--app-mono')).toBe('ui-monospace, monospace');
      // Safe live --display overrides look's value.
      expect(map.get('--app-display')).toBe('Inter, sans-serif');
      // Hostile live AND hostile look fallback for --app-text-scale are both dropped.
      expect(map.has('--app-text-scale')).toBe(false);
      // Hostile non-font look value and invalid var name are dropped.
      expect(map.has('--app-ink')).toBe(false);
      expect(map.has('--not-app-token')).toBe(false);
      expect(map.get('--app-canvas')).toBe('#fafafa');

      const vars = themeVars(look);
      expect(vars).not.toContain('</style>');
      expect(vars).not.toContain('evil()');
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('still filters look pairs when getComputedStyle throws', () => {
    vi.stubGlobal('getComputedStyle', () => { throw new Error('no DOM'); });
    try {
      const look = {
        scheme: 'dark' as const,
        pairs: [
          ['--app-ink', '#ffffff'],
          ['--app-canvas', '#000}</style><script>x()</script>'],
          ['--bad-name', '#123456'],
        ] as Array<[string, string]>,
        adjusted: [],
      };
      expect(themePairs(look)).toEqual([['--app-ink', '#ffffff']]);
      expect(themeVars(look)).toBe('--app-ink:#ffffff');
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
