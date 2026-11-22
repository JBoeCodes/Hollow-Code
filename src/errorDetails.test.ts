import { describe, expect, it } from 'vitest';
import { errorDetails } from './errorDetails';

describe('errorDetails', () => {
  it('copies the message even when the stack holds frames only (WKWebView)', () => {
    // Solid's castError: a Tauri rejection string wrapped, with it as the cause.
    const e = new Error('read /v/src/App.tsx: No such file or directory (os error 2)', { cause: 'read /v/src/App.tsx: No such file or directory (os error 2)' });
    e.stack = 'se@tauri://localhost/assets/index-C07uc7sA.js:2:9269\n@tauri://localhost/assets/index-C07uc7sA.js:2:4387';
    const text = errorDetails(e);
    expect(text.split('\n')[0]).toBe('Error: read /v/src/App.tsx: No such file or directory (os error 2)');
    expect(text).toContain('se@tauri://localhost/assets/index-C07uc7sA.js:2:9269');
    expect(text).not.toContain('Cause:'); // the same text twice says nothing more
  });
  it('adds a cause that says more, stringified safely', () => {
    expect(errorDetails(new Error('Unknown error', { cause: { code: 'MISSING', path: '/a' } }))).toContain('Cause: {"code":"MISSING","path":"/a"}');
    const loop: Record<string, unknown> = {}; loop.self = loop;
    expect(errorDetails(new Error('Unknown error', { cause: loop }))).toContain('Cause: [object Object]');
    expect(errorDetails(new Error('outer', { cause: new TypeError('inner') }))).toContain('Cause: TypeError: inner');
  });
  it('keeps a Chromium stack once, after the message', () => {
    const e = new Error('boom'); e.stack = 'Error: boom\n    at f (a.js:1:2)';
    expect(errorDetails(e)).toBe('Error: boom\n    at f (a.js:1:2)');
  });
  it('describes a thrown non-Error', () => {
    expect(errorDetails('plain string')).toBe('plain string');
    expect(errorDetails({ message: 'x' })).toBe('{"message":"x"}');
    expect(errorDetails(undefined)).toBe('undefined');
  });
});
