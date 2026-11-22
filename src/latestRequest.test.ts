import { describe, expect, it } from 'vitest';
import { latestRequest, runLatest, unchangedSince } from './latestRequest';

const settle = async () => { for (let i = 0; i < 4; i++) await Promise.resolve(); };

describe('latestRequest (plan L3 X21a)', () => {
  it('an older response cannot replace a newer one', () => {
    const order = latestRequest();
    const first = order.start();
    const second = order.start();
    expect(order.isLatest(first)).toBe(false);
    expect(order.isLatest(second)).toBe(true);
    order.cancel();
    expect(order.isLatest(second)).toBe(false);
  });

  it('cancel drops the request in flight and leaves the next one free to answer', () => {
    const order = latestRequest();
    const before = order.start();
    order.cancel();
    expect(order.isLatest(before)).toBe(false);
    const after = order.start();
    expect(order.isLatest(after)).toBe(true);
  });

  it('peek joins the current request without superseding it', () => {
    const order = latestRequest();
    const load = order.start();
    const save = order.peek();
    expect(save).toBe(load);
    expect(order.isLatest(load)).toBe(true);
    expect(order.isLatest(save)).toBe(true);
    // A new load (another file opened) drops both.
    order.start();
    expect(order.isLatest(load)).toBe(false);
    expect(order.isLatest(save)).toBe(false);
  });

  it('peek before any start is current until something starts or cancels', () => {
    const order = latestRequest();
    const t = order.peek();
    expect(order.isLatest(t)).toBe(true);
    order.cancel();
    expect(order.isLatest(t)).toBe(false);
    // After a cancel, peek is the new current ticket (WorkFileEditor's old
    // `generation++` then `token = generation`).
    const u = order.peek();
    expect(order.isLatest(u)).toBe(true);
    expect(u).not.toBe(t);
  });

  it('separate instances do not interfere', () => {
    const list = latestRequest();
    const reads = latestRequest();
    const l = list.start();
    const r = reads.start();
    reads.start();
    expect(list.isLatest(l)).toBe(true);
    expect(reads.isLatest(r)).toBe(false);
  });

  it('a late answer after unmount is dropped, success or failure (the view pattern)', async () => {
    const order = latestRequest();
    const shown: string[] = [];
    let resolve!: (v: string) => void;
    let reject!: (e: unknown) => void;
    const load = async (p: Promise<string>) => {
      const ticket = order.start();
      try { const v = await p; if (order.isLatest(ticket)) shown.push(v); }
      catch (e) { if (order.isLatest(ticket)) shown.push(`error: ${String(e)}`); }
    };
    void load(new Promise<string>(r => { resolve = r; }));
    order.cancel();
    resolve('late');
    void load(new Promise<string>((_, r) => { reject = r; }));
    order.cancel();
    reject('late failure');
    await settle();
    expect(shown).toEqual([]);
  });
});

describe('runLatest (next-level P07)', () => {
  it('a stale query never wins, whatever order the answers arrive in', async () => {
    const order = latestRequest();
    const resolvers: Record<string, (v: string[]) => void> = {};
    const seen: string[][] = [];
    const ask = (q: string) => runLatest(order, () => new Promise<string[]>(r => { resolvers[q] = r; }), o => { if (o.ok) seen.push(o.value); });
    ask('gar');
    const mine = ask('garage 14');
    expect(order.isLatest(mine)).toBe(true);
    resolvers['garage 14'](['garage-14-lease-renewal-terms.md']);
    resolvers['gar'](['garden.md']);
    await settle();
    expect(seen).toEqual([['garage-14-lease-renewal-terms.md']]);
    // A cancel (the query changed, debounce pending) drops the one in flight.
    const errors: unknown[] = [];
    runLatest(order, () => Promise.reject(new Error('index locked')), o => { if (!o.ok) errors.push(o.error); });
    order.cancel();
    await settle();
    expect(errors).toEqual([]);
  });

  it('delivers a failure of the latest request', async () => {
    const order = latestRequest();
    const errors: unknown[] = [];
    runLatest(order, () => Promise.reject(new Error('index locked')), o => { if (!o.ok) errors.push(o.error); });
    await settle();
    expect(errors).toHaveLength(1);
    expect(String(errors[0])).toMatch(/index locked/);
  });
});

describe('unchangedSince (lane S re-review of #1304)', () => {
  it('stays true while every identity is the same, and false once any changes', () => {
    let key: string | null = 'chat-a', version = 3;
    const still = unchangedSince(() => [key, version]);
    expect(still()).toBe(true);
    version = 4;
    expect(still()).toBe(false);
    version = 3;
    expect(still()).toBe(true);
    key = 'chat-b';
    expect(still()).toBe(false);
    key = null;
    expect(still()).toBe(false);
  });

  it('a confirm answered after the chat changed does nothing', async () => {
    let key: string | null = 'chat-a';
    const applied: string[] = [];
    const pick = async (confirm: Promise<boolean>) => {
      const still = unchangedSince(() => [key]);
      if (!(await confirm) || !still()) return;
      applied.push(key!);
    };
    let answer!: (ok: boolean) => void;
    const open = pick(new Promise<boolean>(resolve => { answer = resolve; }));
    key = 'chat-b';
    answer(true);
    await open;
    expect(applied).toEqual([]);
    await pick(Promise.resolve(true));
    expect(applied).toEqual(['chat-b']);
  });

  it('every per-chat mode picker checks it after the confirm', async () => {
    const flat = (text: string) => text.replace(/\s+/g, '');
    const [{ default: chip }, { default: menu }, { default: slash }, { default: runtime }] = await Promise.all([
      import('./components/ComposerAccessChip.tsx?raw'), import('./components/ChatModelMenu.tsx?raw'),
      import('./components/chat/slashCommands.ts?raw'), import('./store/chatRuntime.ts?raw'),
    ]);
    expect(flat(runtime)).toContain('returnunchangedSince(()=>[chatAgentSessionKey(),chatConversationVersion()]);');
    expect(flat(chip)).toContain('conststill=sameChatAsNow();if(!(awaitconfirmPermissionModeChange(next,chatAgent()))||locked()||!still())return;');
    expect(flat(menu)).toContain('conststill=sameChatAsNow();constok=awaitconfirmPermissionModeChange(next,chatAgent());if(!ok||locked()||!still())return;');
    expect(flat(slash)).toContain('conststill=sameChatAsNow();confirmPermissionModeChange(norm,agent).then(ok=>{if(!still())return;');
  });
});
