import { describe, expect, it } from 'vitest';
import { addressFile, isWebUrl, normalizeAddress, tabLabel } from './browserUrl';

describe('normalizeAddress', () => {
  it('keeps web addresses as typed', () => {
    expect(normalizeAddress(' https://example.com/a?b=1 ')).toBe('https://example.com/a?b=1');
    expect(normalizeAddress('http://localhost:5173')).toBe('http://localhost:5173');
  });
  it('adds http for this Mac and the LAN, https for the web', () => {
    expect(normalizeAddress('localhost:3000/app')).toBe('http://localhost:3000/app');
    expect(normalizeAddress('127.0.0.1:8080')).toBe('http://127.0.0.1:8080');
    expect(normalizeAddress('192.168.0.20')).toBe('http://192.168.0.20');
    expect(normalizeAddress('workstation.local:8123')).toBe('http://workstation.local:8123');
    expect(normalizeAddress('github.com/example-user')).toBe('https://github.com/example-user');
    expect(normalizeAddress('docs.rs')).toBe('https://docs.rs');
  });
  it('reads a bare port as localhost', () => {
    expect(normalizeAddress('5173')).toBe('http://localhost:5173');
    expect(normalizeAddress(':8080')).toBe('http://localhost:8080');
  });
  it('searches for anything else', () => {
    expect(normalizeAddress('tauri multiwebview')).toBe('https://www.google.com/search?q=tauri%20multiwebview');
    expect(normalizeAddress('hollow')).toBe('https://www.google.com/search?q=hollow');
  });
  it('refuses empty input and other schemes', () => {
    expect(normalizeAddress('   ')).toBeNull();
    for (const s of ['file:///etc/passwd', 'tauri://localhost/', 'artifact://localhost/x', 'javascript:alert(1)', 'data:text/html,hi']) {
      expect(normalizeAddress(s), s).toBeNull();
    }
  });
});

describe('tab labels and web links', () => {
  it('prefers the title, then the host', () => {
    expect(tabLabel('https://example.com/a', ' Example ')).toBe('Example');
    expect(tabLabel('http://localhost:5173/', '')).toBe('localhost:5173');
    expect(tabLabel('', '')).toBe('New tab');
  });
  it('treats only http(s) as web links', () => {
    expect(isWebUrl('https://x.test')).toBe(true);
    expect(isWebUrl('mailto:a@b.c')).toBe(false);
  });
});

describe('addressFile (run 15 P2)', () => {
  it('reads a file address or a path as a file to preview, never a search', () => {
    expect(addressFile('file:///Users/me/Report%20Q3.pdf')).toBe('/Users/me/Report Q3.pdf');
    expect(addressFile(' file://localhost/tmp/a.html ')).toBe('/tmp/a.html');
    expect(addressFile('/Users/me/notes/a.md')).toBe('/Users/me/notes/a.md');
    expect(addressFile('FILE:///a/b.txt')).toBe('/a/b.txt');
  });
  it('leaves web addresses, searches and network paths alone', () => {
    for (const s of ['https://example.com', 'localhost:5173', 'two words', '//server/share', 'file://server/share/a.txt', '', 'file://']) {
      expect(addressFile(s), s).toBeNull();
    }
  });
});
