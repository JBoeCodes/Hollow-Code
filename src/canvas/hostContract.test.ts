import { describe, expect, it } from 'vitest';
import {
  CANVAS_HOSTS, HOST_CONTRACT, HOST_CONTROLS, KEPT_VERSIONS,
  isOlder, offers, olderLabel, permissionTag, pinnedVersionLabel, savedWords,
} from './hostContract';

// Plan L3 X7/X8 (2026-10-14; review U canvases): one table says what every
// canvas host shows, and the head's words come from one place.
describe('the canvas host contract', () => {
  it('discloses transient changes, retry, source navigation and explicit pin replacement', () => {
    expect(HOST_CONTRACT.older.controls.saved).toMatchObject({ where: expect.stringContaining('Changes here aren’t kept. Open vN to keep them') });
    expect(HOST_CONTRACT.inline.controls.saved).toMatchObject({ where: expect.stringContaining('Try again') });
    expect(HOST_CONTRACT.pinned.controls.pin).toMatchObject({ where: expect.stringContaining('Update to vN') });
    for (const host of ['result', 'gallery'] as const) {
      expect(HOST_CONTRACT[host].controls.expand).toMatchObject({ why: expect.stringContaining('Open source chat') });
    }
    expect(HOST_CONTRACT.inline.controls.export).toMatchObject({ where: expect.stringContaining('Export canvas') });
  });
  it('the compact panel host retains metadata and permission in named places', () => {
    for (const key of ['version', 'saved', 'title'] as const) {
      const offer = HOST_CONTRACT.expanded.controls[key];
      expect(offer.offered && offer.where).toContain('ActionMenu first line');
    }
    expect(HOST_CONTRACT.expanded.controls.pin).toMatchObject({ where: expect.stringContaining('Versions and pin') });
    expect(HOST_CONTRACT.inline.controls.pin).toMatchObject({ where: expect.stringContaining('head') });
  });
  it('names all six hosts and accounts for every control in each', () => {
    expect([...CANVAS_HOSTS].sort()).toEqual(['expanded', 'gallery', 'inline', 'older', 'pinned', 'result']);
    for (const host of CANVAS_HOSTS) {
      const terms = HOST_CONTRACT[host];
      expect(Object.keys(terms.controls).sort(), host).toEqual([...HOST_CONTROLS].sort());
      for (const control of HOST_CONTROLS) {
        const o = terms.controls[control];
        // Offered says where; withheld says why. Never a bare yes or no.
        const said = o.offered ? o.where : o.why;
        expect(said.length, `${host}.${control}`).toBeGreaterThan(10);
      }
    }
  });

  it('every host shows a title, a version, a saved state, a permission boundary, copy and a text fallback', () => {
    for (const host of CANVAS_HOSTS) {
      for (const control of ['title', 'version', 'saved', 'permission', 'copy', 'readAsText'] as const) {
        expect(offers(host, control), `${host} offers ${control}`).toBe(true);
      }
    }
  });

  it('expand, pin and compare are offered or withheld with a reason', () => {
    expect(offers('inline', 'expand')).toBe(true);
    expect(offers('expanded', 'expand')).toBe(false);
    for (const host of ['inline', 'expanded', 'result', 'gallery'] as const) {
      expect(offers(host, 'pin'), host).toBe(true);
      expect(offers(host, 'compare'), host).toBe(true);
    }
    expect(offers('pinned', 'compare')).toBe(false);
  });

  it('an older version never offers accept, answer or pin (X8)', () => {
    for (const control of ['accept', 'answer', 'pin'] as const) expect(offers('older', control), control).toBe(false);
    expect(offers('older', 'version')).toBe(true);
    // Only Results accepts, and only the current version.
    for (const host of CANVAS_HOSTS) if (host !== 'result') expect(offers(host, 'accept'), host).toBe(false);
    const accept = HOST_CONTRACT.result.controls.accept;
    expect(accept.offered ? accept.where : '').toMatch(/current version/);
  });

  it('only a reply (inline or expanded) offers to answer an ask-canvas, and only as a draft', () => {
    const answering = CANVAS_HOSTS.filter(h => offers(h, 'answer'));
    expect(answering.sort()).toEqual(['expanded', 'inline']);
    for (const host of answering) {
      const o = HOST_CONTRACT[host].controls.answer;
      expect(o.offered ? o.where : '').toMatch(/never sends/);
    }
  });
});

describe('the words a canvas head shows', () => {
  it('the permission tag says the boundary before anything is refused', () => {
    expect(permissionTag('canvas', 'canvas', false)).toMatchObject({ kind: 'scripts', text: 'Scripts on · no internet' });
    expect(permissionTag('offline', 'offline', false)).toMatchObject({ kind: 'none', text: 'No scripts · no internet' });
    expect(permissionTag('interactive', 'canvas', false)).toMatchObject({ kind: 'internet', text: 'Internet on (you allowed it)' });
    expect(permissionTag('https', 'offline', false, 'HTML').detail).toBe('This HTML may load from and send to any website. ⋯ → Turn internet off.');
    // A pin: local values only, whatever the policy.
    expect(permissionTag('canvas', 'canvas', true)).toMatchObject({ kind: 'local', text: 'Read-only local data' });
    expect(permissionTag('interactive', 'canvas', true).kind).toBe('local');
  });

  it('the saved state is said in words, and says what is true', () => {
    expect(savedWords({ store: 'workspace', saved: true, revision: 2, latest: 2 }).text).toBe('Saved in this workspace');
    expect(savedWords({ store: 'quick', saved: true, revision: 1, latest: 1 }).text).toBe('Saved with the COO');
    expect(savedWords({ store: 'conversation', saved: true, revision: 1, latest: 3 }).text).toBe('Saved with this conversation');
    // Aged out: the store knew this version and no longer keeps it.
    const aged = savedWords({ store: 'workspace', saved: false, revision: 3, latest: 40 });
    expect(aged.text).toBe('Not kept');
    expect(aged.detail).toContain(`last ${KEPT_VERSIONS} versions`);
    expect(savedWords({ store: 'workspace', saved: false, revision: null, latest: null }).text).toBe('Not saved');
    expect(savedWords({ store: null, saved: false, revision: null, latest: null }).text).toBe('Not saved');
  });

  it('older versions and pins say which version is on screen', () => {
    expect(olderLabel(1, 3)).toBe('Older version (v1 of 3) · read only');
    expect(isOlder(1, 3)).toBe(true);
    expect(isOlder(3, 3)).toBe(false);
    expect(isOlder(null, 3)).toBe(false);
    expect(isOlder(2, null)).toBe(false);
    expect(pinnedVersionLabel(2, false)).toBe('Pinned v2 · current');
    expect(pinnedVersionLabel(2, true)).toBe('Pinned v2 · a newer version exists');
    expect(pinnedVersionLabel(2, null)).toBe('Pinned v2');
  });
});
