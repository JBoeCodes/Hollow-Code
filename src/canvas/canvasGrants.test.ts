import { beforeEach, describe, expect, it } from 'vitest';
import RUNTIME_JS from '../components/sandbox-doc.js?raw';
import {
  GRANTS_CAP, GRANTS_KEY, allows, blockedLine, canvasGrantKey, escalate, fileMode, grantedMode, setGrant, textKey,
  withInternet, withoutInternet,
} from './canvasGrants';
import { parseChildMessage } from '../canvasBridge';
import type { ArtifactMode } from '../components/htmlSandbox';

// A tiny localStorage for the node test environment.
const mem = new Map<string, string>();
(globalThis as unknown as { localStorage: Storage }).localStorage = {
  getItem: (k: string) => mem.get(k) ?? null,
  setItem: (k: string, v: string) => { mem.set(k, v); },
  removeItem: (k: string) => { mem.delete(k); },
  clear: () => mem.clear(),
  key: () => null,
  length: 0,
};

describe('escalate: the smallest policy that allows a refused load (10-11 B1)', () => {
  const table: Array<[ArtifactMode, 'images' | 'network' | 'scripts', ArtifactMode]> = [
    ['offline', 'images', 'images'],
    ['offline', 'network', 'https'],
    ['offline', 'scripts', 'canvas'],
    ['images', 'network', 'https'],
    ['images', 'scripts', 'interactive'],
    ['https', 'images', 'https'],
    ['https', 'scripts', 'interactive'],
    // A scripted page that may load remote images can send data in an image
    // URL: any network for it is the whole internet, said so.
    ['canvas', 'images', 'interactive'],
    ['canvas', 'network', 'interactive'],
    ['canvas', 'scripts', 'canvas'],
    ['interactive', 'images', 'interactive'],
  ];
  it.each(table)('%s + %s → %s', (mode, want, next) => {
    expect(escalate(mode, want)).toBe(next);
    expect(allows(next, want)).toBe(true);
  });
  it('a refusal the policy already allows is not asked about (the srcdoc fallback’s scripts)', () => {
    expect(allows('canvas', 'scripts')).toBe(true);
    expect(allows('canvas', 'images')).toBe(false);
  });
  it('the internet switch keeps scripts as they are', () => {
    expect(withInternet('canvas')).toBe('interactive');
    expect(withInternet('offline')).toBe('https');
    expect(withoutInternet('interactive')).toBe('canvas');
    expect(withoutInternet('images')).toBe('offline');
  });
});

describe('grants are remembered per canvas, for its exact content', () => {
  beforeEach(() => mem.clear());
  const A = '<p>chart v1</p>';
  it('no grant: the surface default', () => {
    expect(grantedMode('canvas:w:a', 'canvas', A)).toBe('canvas');
    expect(grantedMode('file:/x.html', 'offline', A)).toBe('offline');
  });
  it('a grant reopens with it; null forgets it', () => {
    setGrant('canvas:w:a', 'interactive', A);
    expect(grantedMode('canvas:w:a', 'canvas', A)).toBe('interactive');
    expect(grantedMode('canvas:w:b', 'canvas', A)).toBe('canvas');
    setGrant('canvas:w:a', null, A);
    expect(grantedMode('canvas:w:a', 'canvas', A)).toBe('canvas');
  });
  it('a new revision or an edited file asks again (review B1 medium)', () => {
    setGrant('canvas:w:a', 'interactive', A);
    expect(grantedMode('canvas:w:a', 'canvas', '<p>chart v2, written later</p>')).toBe('canvas');
    setGrant('file:/v/workspaces/w/outputs/r.html', 'canvas', A);
    expect(grantedMode('file:/v/workspaces/w/outputs/r.html', 'offline', A + ' ')).toBe('offline');
  });
  it('a grant never takes scripts away from a scripted default', () => {
    setGrant('k', 'images', A);
    expect(grantedMode('k', 'canvas', A)).toBe('interactive');
  });
  it('corrupt, foreign or pre-content storage is ignored', () => {
    mem.set(GRANTS_KEY, '{not json');
    expect(grantedMode('k', 'canvas', A)).toBe('canvas');
    mem.set(GRANTS_KEY, JSON.stringify({ k: { mode: 'everything', at: 1, text: textKey(A) }, j: 'interactive', i: { mode: 'https', at: 1 } }));
    expect(grantedMode('k', 'offline', A)).toBe('offline');
    expect(grantedMode('j', 'offline', A)).toBe('offline');
    expect(grantedMode('i', 'offline', A)).toBe('offline');
  });
  it(`keeps the newest ${GRANTS_CAP}`, () => {
    for (let i = 0; i <= GRANTS_CAP; i++) setGrant(`k${i}`, 'https', A);
    const kept = Object.keys(JSON.parse(mem.get(GRANTS_KEY)!));
    expect(kept).toHaveLength(GRANTS_CAP);
    expect(kept).not.toContain('k0');
    expect(grantedMode(`k${GRANTS_CAP}`, 'offline', A)).toBe('https');
  });
  it('keys: a saved canvas shares one across reply, Results and curated views; text keys are stable', () => {
    expect(canvasGrantKey({ workspaceId: 'w1', sessionKey: 's' }, 'status')).toBe('canvas:w1:status');
    expect(canvasGrantKey({ workspaceId: null, sessionKey: 's' }, 'status')).toBe('canvas:session:s:status');
    // No store yet: no shared key (new chats must not share grants by id).
    expect(canvasGrantKey({ workspaceId: null, sessionKey: null }, 'status')).toBeUndefined();
    expect(textKey('<p>a</p>')).toBe(textKey('<p>a</p>'));
    expect(textKey('<p>a</p>')).not.toBe(textKey('<p>b</p>'));
  });
});

describe('fileMode: agent-written HTML opens in Canvas, outside HTML locked', () => {
  it.each([
    ['/v/workspaces/w1/outputs/report.html', false, 'canvas'],
    ['workspaces/w1/outputs/canvases/x.canvas.html', false, 'canvas'],
    ['/v/workspaces/w1/sources/download.html', false, 'offline'],
    ['/Users/me/Downloads/page.html', false, 'offline'],
    ['/Users/me/project/index.html', true, 'canvas'],
    ['/v/workspaces/w1/outputs-old/x.html', false, 'offline'],
    ['/v/workspaces/w1/outputs/../sources/z.html', false, 'offline'],
    ['/v/workspaces/w1/outputs/./x.html', false, 'offline'],
    ['/Users/me/project/../Downloads/x.html', true, 'offline'],
  ] as const)('%s (agent wrote: %s) → %s', (path, wrote, mode) => {
    expect(fileMode(path, wrote)).toBe(mode);
  });
});

describe('the permission line', () => {
  it('names the host and says what Allow means', () => {
    expect(blockedLine('images', 'cdn.example.com', 'canvas')).toBe('This canvas tried to load images from cdn.example.com. Allowing lets it load from and send to any website.');
    expect(blockedLine('network', 'api.example.com', 'file')).toContain('This file tried to connect to api.example.com.');
    expect(blockedLine('scripts', '', 'file')).toContain('didn’t come from an agent in Hollow');
    expect(blockedLine('scripts', '', 'html')).toBe('Scripts in this HTML are off outside workspaces and Chats.');
  });
});

describe('bridge `blocked` and `source` (10-11 B1)', () => {
  const v1 = (kind: string, fields: Record<string, unknown>) => ({ __hc: 1, kind, ...fields });
  it('accepts a host name, and scripts with none', () => {
    expect(parseChildMessage(v1('blocked', { want: 'images', host: 'upload.wikimedia.org' }))).toEqual({ ok: true, message: { kind: 'blocked', want: 'images', host: 'upload.wikimedia.org' } });
    expect(parseChildMessage(v1('blocked', { want: 'network', host: '[::1]' })).ok).toBe(true);
    expect(parseChildMessage(v1('blocked', { want: 'network', host: 'my_host.example.com' })).ok).toBe(true);
    expect(parseChildMessage(v1('blocked', { want: 'scripts', host: '' })).ok).toBe(true);
    expect(parseChildMessage(v1('source', {}))).toEqual({ ok: true, message: { kind: 'source' } });
  });
  it.each([
    { want: 'images', host: 'https://x.com/a?b' },
    { want: 'images', host: 'X.com' },
    { want: 'images', host: '' },
    { want: 'images', host: 'a b' },
    { want: 'images', host: 'x'.repeat(254) },
    { want: 'scripts', host: 'x.com' },
    { want: 'everything', host: 'x.com' },
    { want: 'images', host: 'x.com', extra: 1 },
  ])('refuses %o', (fields) => {
    expect(parseChildMessage(v1('blocked', fields))).toMatchObject({ ok: false, reason: 'invalid' });
  });
  it('`source` carries nothing', () => {
    expect(parseChildMessage(v1('source', { href: 'x' }))).toMatchObject({ ok: false, reason: 'invalid' });
  });
});

describe('the frame runtime reports refusals (sandbox-doc.js section 6)', () => {
  // Run the runtime against a fake window: `post` goes to `host.postMessage`.
  function boot(baseURI = 'about:srcdoc') {
    const listeners: Record<string, Array<(e: unknown) => void>> = {};
    const posted: unknown[] = [];
    const win: Record<string, unknown> = {};
    win.parent = { postMessage: (m: unknown) => posted.push(m) };
    const doc = { baseURI, readyState: 'complete', documentElement: { offsetHeight: 0, scrollHeight: 0, style: { setProperty() {} }, setAttribute() {} }, body: null };
    const run = new Function('window', 'addEventListener', 'document', 'setTimeout', 'innerHeight', 'ResizeObserver', 'CustomEvent', 'dispatchEvent', RUNTIME_JS);
    run(win, (t: string, f: (e: unknown) => void) => { (listeners[t] ??= []).push(f); }, doc, () => 0, 0, undefined, class {}, () => true);
    const fire = (t: string, e: Record<string, unknown>) => { for (const f of listeners[t] ?? []) f({ preventDefault() {}, ...e }); };
    const blocked = () => posted.filter((m) => (m as { kind: string }).kind === 'blocked' || (m as { kind: string }).kind === 'source');
    return { fire, blocked };
  }
  it('sends the host name only, once per kind', () => {
    const { fire, blocked } = boot();
    fire('securitypolicyviolation', { effectiveDirective: 'img-src', blockedURI: 'https://cdn.example.com/a.png?secret=1' });
    fire('securitypolicyviolation', { effectiveDirective: 'img-src', blockedURI: 'https://other.example.com/b.png' });
    fire('securitypolicyviolation', { effectiveDirective: 'connect-src', blockedURI: 'https://api.example.com/x' });
    fire('securitypolicyviolation', { effectiveDirective: 'script-src-elem', blockedURI: 'inline' });
    expect(blocked()).toEqual([
      { __hc: 1, kind: 'blocked', want: 'images', host: 'cdn.example.com' },
      { __hc: 1, kind: 'blocked', want: 'network', host: 'api.example.com' },
      { __hc: 1, kind: 'blocked', want: 'scripts', host: '' },
    ]);
    for (const m of blocked()) expect(parseChildMessage(m).ok).toBe(true);
  });
  it('a remote script asks for scripts, then the network', () => {
    const { fire, blocked } = boot();
    fire('securitypolicyviolation', { effectiveDirective: 'script-src-elem', blockedURI: 'https://cdn.example.com/lib.js' });
    expect(blocked()).toEqual([
      { __hc: 1, kind: 'blocked', want: 'scripts', host: '' },
      { __hc: 1, kind: 'blocked', want: 'network', host: 'cdn.example.com' },
    ]);
  });
  it('normalises the host (case, a trailing dot) and takes WebKit’s empty inline URI', () => {
    const { fire, blocked } = boot();
    fire('securitypolicyviolation', { effectiveDirective: 'img-src', blockedURI: 'https://CDN.Example.com./a.png' });
    fire('securitypolicyviolation', { effectiveDirective: 'script-src-attr', blockedURI: '' });
    expect(blocked()).toEqual([
      { __hc: 1, kind: 'blocked', want: 'images', host: 'cdn.example.com' },
      { __hc: 1, kind: 'blocked', want: 'scripts', host: '' },
    ]);
    for (const m of blocked()) expect(parseChildMessage(m).ok).toBe(true);
  });
  it('a relative URL on the page’s own origin is not a remote load (the srcdoc fallback)', () => {
    const { fire, blocked } = boot('http://127.0.0.1:1438/?mock=1');
    fire('securitypolicyviolation', { effectiveDirective: 'img-src', blockedURI: 'http://127.0.0.1:1438/chart.png' });
    expect(blocked()).toEqual([]);
    fire('securitypolicyviolation', { effectiveDirective: 'img-src', blockedURI: 'https://cdn.example.com/chart.png' });
    expect(blocked()).toEqual([{ __hc: 1, kind: 'blocked', want: 'images', host: 'cdn.example.com' }]);
  });
  it('ignores local refusals (data:, blob:, eval) and forwards ⌥⌘U', () => {
    const { fire, blocked } = boot();
    fire('securitypolicyviolation', { effectiveDirective: 'img-src', blockedURI: 'data' });
    fire('securitypolicyviolation', { effectiveDirective: 'script-src', blockedURI: 'eval' });
    fire('securitypolicyviolation', { effectiveDirective: 'worker-src', blockedURI: 'blob' });
    expect(blocked()).toEqual([]);
    fire('keydown', { metaKey: true, altKey: true, ctrlKey: false, shiftKey: false, code: 'KeyU', key: 'ˆ' });
    fire('keydown', { metaKey: true, altKey: false, ctrlKey: false, shiftKey: false, code: 'KeyU', key: 'u' });
    expect(blocked()).toEqual([{ __hc: 1, kind: 'source' }]);
  });
});
