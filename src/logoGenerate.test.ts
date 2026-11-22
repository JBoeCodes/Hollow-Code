import { beforeEach, describe, expect, it, vi } from 'vitest';
vi.mock('./nativeEvents', () => ({ emitMockEvent: () => {} }));
import { chooseLogoAgent, generateLogo, hasRoom, logoRequestId, type LogoDeps } from './logoGenerate';
import { mockLogoOptions } from './mockCustomIcons';
import { checkUploadBytes, svgOriginal } from './customIcons';

const full = (agent: string) => ({ agent, room: false });
const free = (agent: string) => ({ agent, room: true });

describe('Generate a logo: which agent draws (overhaul H2)', () => {
  it('Codex first when it has room', () => {
    expect(chooseLogoAgent(['claude', 'codex'], [free('codex'), free('claude')])).toBe('codex');
    expect(chooseLogoAgent(['claude', 'codex'], [])).toBe('codex');
    expect(chooseLogoAgent(['claude', 'codex'], null)).toBe('codex');
  });

  it('else Claude, else any installed agent with room', () => {
    expect(chooseLogoAgent(['claude', 'codex'], [full('codex'), free('claude')])).toBe('claude');
    expect(chooseLogoAgent(['codex', 'grok'], [full('codex')])).toBe('grok');
    expect(chooseLogoAgent(['claude', 'codex', 'antigravity'], [full('codex'), full('claude')])).toBe('antigravity');
    expect(chooseLogoAgent(['claude'], [full('codex')])).toBe('claude');
  });

  it('when nothing has room the first installed in that order still gets it, and none installed is null', () => {
    expect(chooseLogoAgent(['claude', 'codex'], [full('codex'), full('claude')])).toBe('codex');
    expect(chooseLogoAgent([], [free('codex')])).toBeNull();
  });

  it('one account with room is enough; no readings count as room', () => {
    expect(hasRoom('claude', [full('claude'), free('claude')])).toBe(true);
    expect(hasRoom('claude', [full('claude')])).toBe(false);
    expect(hasRoom('codex', [full('claude')])).toBe(true);
  });
});

describe('Generate a logo: the stub agent, generate and pick', () => {
  const store = new Map<string, string>();
  beforeEach(() => {
    store.clear();
    vi.stubGlobal('localStorage', { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => { store.set(k, v); }, removeItem: (k: string) => { store.delete(k); } });
  });
  const deps = (installed: string[], accounts: { agent: string; room: boolean }[] | Error): LogoDeps & { calls: unknown[][] } => {
    const calls: unknown[][] = [];
    return {
      calls,
      agentAvailable: () => Promise.resolve(installed),
      cfoView: () => accounts instanceof Error ? Promise.reject(accounts) : Promise.resolve({ accounts }),
      generateLogoOptions: (agent, target, request, requestId) => { calls.push(requestId === undefined ? [agent, target, request] : [agent, target, request, requestId]); return Promise.resolve(mockLogoOptions(agent)); },
    };
  };

  it('asks the chosen agent once, with the trimmed request, and gets four options to pick from', async () => {
    const d = deps(['claude', 'codex'], [full('codex')]);
    const out = await generateLogo(d, 'mainAgent', '  a lantern  ');
    expect(d.calls).toEqual([['claude', 'mainAgent', 'a lantern']]);
    expect(out.agent).toBe('claude');
    expect(out.options).toHaveLength(4);
    expect(new Set(out.options).size).toBe(4);
    // Picking one turns it into the same original an uploaded SVG is.
    const picked = svgOriginal(out.options[2]);
    expect(picked.format).toBe('svg');
    expect(() => checkUploadBytes('svg', new TextEncoder().encode(out.options[2]))).not.toThrow();
    for (const svg of out.options) expect(svg).not.toMatch(/<script|<style|<image|href=|<pattern|<use/i);
  });

  it('passes the request id through, so closing the dialog can stop the call (lane S 11-13)', async () => {
    const d = deps(['claude'], []);
    const id = logoRequestId();
    expect(id).not.toBe(logoRequestId());
    await generateLogo(d, 'app', 'a lantern', id);
    expect(d.calls).toEqual([['claude', 'app', 'a lantern', id]]);
  });

    it('a failed headroom read still draws, on Codex first', async () => {
    const d = deps(['claude', 'codex'], new Error('no vault'));
    await generateLogo(d, 'app', '');
    expect(d.calls[0][0]).toBe('codex');
  });

  it('no agent installed asks no one and points to Upload', async () => {
    const d = deps([], []);
    await expect(generateLogo(d, 'app', 'x')).rejects.toThrow('Upload an icon');
    expect(d.calls).toEqual([]);
  });

  it('the stub can fail or return fewer options, as a real agent might', () => {
    store.set('mockLogoOptionsCount', '2');
    expect(mockLogoOptions('codex').options).toHaveLength(2);
    store.set('mockLogoOptionsError', 'Codex is out of usage');
    expect(() => mockLogoOptions('codex')).toThrow();
  });
});
