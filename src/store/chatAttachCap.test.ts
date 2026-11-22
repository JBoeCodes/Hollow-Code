import { beforeEach, expect, it, vi } from 'vitest';

// The same stand-ins as chatRuntime.quota.test.ts: the runtime's own
// imports, none of which these checks use.
vi.hoisted(() => {
  const values = new Map<string, string>();
  vi.stubGlobal('localStorage', { getItem: (k: string) => values.get(k) ?? null, setItem: (k: string, v: string) => values.set(k, v), removeItem: (k: string) => values.delete(k) });
});
vi.mock('../nativeEvents', () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock('../api', () => ({ api: { setupStatus: async () => ({ vaultPath: '/vault' }) } }));
vi.mock('./runtime', () => ({ activePage: () => 'today', setActivePage: vi.fn(), closeChatPanel: vi.fn(), refetchShownSessions: vi.fn(), requestChatFocus: vi.fn(), setChatPanelOpen: vi.fn() }));
vi.mock('./overlays', () => ({ showConfirm: vi.fn() }));
vi.mock('./snackbars', () => ({ flashFooter: vi.fn() }));
vi.mock('../windowLayout', () => ({ windowAtMost: () => false }));
vi.mock('../canvas/askHold', () => ({ answeredRevision: () => null, answersBack: vi.fn(), settleHeldAnswers: vi.fn() }));

import { attachChatImage, attachSharedPage, chatAttachments, registerChatImageSink, resetChatState, setChatAttachments, type ChatAttachment } from './chatRuntime';
import { ATTACH_MAX_COUNT } from '../composerFiles';

const png = () => new File(['x'], 'shot.png', { type: 'image/png' });
const page = { url: 'https://vendor.test/a', title: 'A', text: 'words', truncated: false, sharedAt: '2026-11-04T00:00:00Z' };
const full = (): ChatAttachment[] => Array.from({ length: ATTACH_MAX_COUNT }, (_, i) => ({ id: i + 1, name: `f${i}`, mime: 'text/plain', dataUrl: '', saving: false, file: true, size: 1, path: `/w/f${i}` }));

beforeEach(() => { resetChatState(); });

it('says false when the composer refuses an image (a full message), so Annotate and Ask to fix know (C5 review)', () => {
  const refuse = registerChatImageSink(() => false);
  expect(attachChatImage(png())).toBe(false);
  refuse();
  const take = registerChatImageSink(() => true);
  expect(attachChatImage(png())).toBe(true);
  take();
  // A sink that returns nothing took it.
  const quiet = registerChatImageSink(() => {});
  expect(attachChatImage(png())).toBe(true);
  quiet();
});

it('queues an image for a composer not yet mounted, or says false when asked not to', () => {
  expect(attachChatImage(png(), false)).toBe(false);
  expect(attachChatImage(png())).toBe(true);
  const got: File[] = [];
  const release = registerChatImageSink(file => { got.push(file); return true; });
  expect(got).toHaveLength(1);
  release();
});

it('adds a shared page only while the message has room', () => {
  expect(attachSharedPage(page)).toBe(true);
  expect(chatAttachments()).toHaveLength(1);
  setChatAttachments(full());
  expect(attachSharedPage(page)).toBe(false);
  expect(chatAttachments()).toHaveLength(ATTACH_MAX_COUNT);
});
