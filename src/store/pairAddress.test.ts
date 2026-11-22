import { describe, expect, it } from 'vitest';
import { parsePairAddress } from './pairAddress';

const host = (s: string) => { const r = parsePairAddress(s); return r.ok ? r.address : null; };

describe('parsePairAddress', () => {
  it('keeps a name, user@name and an optional port', () => {
    expect(host('runner-host')).toEqual({ host: 'runner-host', port: null });
    expect(host('  runner.tail0000.ts.net ')).toEqual({ host: 'runner.tail0000.ts.net', port: null });
    expect(host('builder@192.168.0.32:48000')).toEqual({ host: 'builder@192.168.0.32', port: 48000 });
    expect(host('build-mac.local.:47680')).toEqual({ host: 'build-mac.local.', port: 47680 });
  });

  it('refuses what ssh would misread, with a reason', () => {
    for (const junk of ['', '   ', 'https://example.com', 'ssh://host', 'host/path', 'host name', 'localhost', 'app.localhost:1234',
      '127.0.0.1', '127.1.2.3:47680', '0.0.0.0', '255.255.255.255', '224.0.0.1', '999.1.1.1', '01.2.3.4', '10.0.0.01', '010.0.0.1', '1.2.3', '2130706433', '::1',
      'fe80::1:47680', 'host:0', 'host:65536', 'host:+80', 'host:', 'host:abc', '@host', 'a@b@host', 'user@', '-oProxyCommand=x', 'a..b', '.', 'host;rm']) {
      const r = parsePairAddress(junk);
      expect(r.ok, JSON.stringify(junk)).toBe(false);
      if (!r.ok) expect(r.message.length).toBeGreaterThan(0);
    }
  });

  // The same table as Rust's `a_typed_address_keeps_a_name_and_port...`: both parsers must agree.
  it('agrees with the Rust parser on IPv4 edge cases', () => {
    const table: [string, boolean][] = [
      ['10.0.0.1', true], ['0.1.2.3', true], ['192.168.0.32:48000', true], ['01.2.3.4', false], ['10.0.0.01', false],
      ['010.0.0.1', false], ['1.2.3', false], ['256.1.1.1', false], ['127.0.0.1', false], ['0.0.0.0', false], ['224.0.0.1', false],
    ];
    for (const [input, ok] of table) expect(parsePairAddress(input).ok, input).toBe(ok);
  });

  it('names the typed text in the refusal', () => {
    const r = parsePairAddress('http://x');
    expect(r.ok ? '' : r.message).toContain('`http://x`');
  });
});
