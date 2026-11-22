import { describe, expect, it } from 'vitest';
import { createConversationDraftStore, DRAFT_LIMIT, DRAFT_STORE_KEY, DRAFT_TEXT_BUDGET } from './conversationDrafts';
import type { ChatAttachment } from './chatRuntime';

function fixture() {
  const values = new Map<string, string>();
  let fail = false;
  const storage = {
    getItem: (k: string) => values.get(k) ?? null,
    setItem: (k: string, v: string) => { if (fail) throw new Error('quota'); values.set(k, v); },
    removeItem: (k: string) => { if (fail) throw new Error('quota'); values.delete(k); },
  };
  return { values, storage, store: createConversationDraftStore(() => storage), fail: (v: boolean) => { fail = v; } };
}
const image = { id: 1, name: 'diagram', mime: 'image/png', dataUrl: 'data:image/png;base64,abc', saving: false } as ChatAttachment;

describe('conversation draft persistence', () => {
  it('stashes and restores atomically beside newer text, scoped to one conversation', () => {
    const { store, storage } = fixture();
    store.save('w', 'one', 'Old', [image]);
    expect(store.stash('w', 'one', 'a', 'Old', [image])).toEqual({ saved: true });
    expect(store.read('w', 'one')).toEqual({ text: '', attachments: [] });
    expect(store.stashes('w', 'two')).toEqual([]);
    expect(store.restoreStash('w', 'two', 'stash:a', 'Other', [])).toBe(null);
    const restored = store.restoreStash('w', 'one', 'stash:a', 'New', []);
    expect(restored).toEqual({ text: 'Old\n\nNew', attachments: [image], saved: true });
    const reloaded = createConversationDraftStore(() => storage);
    expect(reloaded.read('w', 'one').text).toBe('Old\n\nNew');
    expect(reloaded.stashes('w', 'one')).toEqual([]);
  });
  it('carries a new-chat stash into its first accepted session and settles its saving chip', () => {
    const { store, storage } = fixture();
    store.stash('w', null, 'a', 'Later', [{ ...image, saving: true }]);
    store.accept('w', 'first', 'Next', []);
    store.settle(image.id, { path: '/saved/image.png', saving: false });
    expect(store.stashes('w', null)).toEqual([]);
    expect(store.stashes('w', 'first')[0].attachments[0].saving).toBe(false);
    expect(createConversationDraftStore(() => storage).stashes('w', 'first')[0].text).toBe('Later');
  });
  it('keeps stashes intact when restoring would exceed attachment limits', () => {
    const { store } = fixture();
    store.stash('w', 'one', 'a', 'Later', [image]);
    expect(store.restoreStash('w', 'one', 'stash:a', 'Now', Array.from({ length: 10 }, (_, i) => ({ ...image, id: i + 2 })))).toBe(null);
    expect(store.stashes('w', 'one')).toHaveLength(1);
  });
  it('moves stashes across a handoff even when the active composer is empty', () => {
    const { store, storage } = fixture();
    store.stash('w', 'one', 'a', 'Later', [image]);
    expect(store.carry('one', 'two', '', [])).toBe(true);
    expect(store.stashes('w', 'one')).toEqual([]);
    expect(store.stashes('w', 'two')[0].attachments).toEqual([image]);
    expect(createConversationDraftStore(() => storage).stashes('w', 'two')[0].text).toBe('Later');
  });
  it('retains the moved draft in memory through storage failure and retries on save', () => {
    const { store, fail, storage } = fixture();
    fail(true);
    expect(store.stash('w', 'one', 'a', 'Later', [image])).toEqual({ saved: false });
    expect(store.stashes('w', 'one')[0].text).toBe('Later');
    expect(store.restoreStash('w', 'one', 'stash:a', 'Now', [])?.saved).toBe(false);
    fail(false); store.save('w', 'one', 'Later\n\nNow', [image]);
    expect(createConversationDraftStore(() => storage).read('w', 'one').text).toBe('Later\n\nNow');
  });
  it('keeps a chip still saving and lets its save finish into the left draft (C-D1)', () => {
    const { store } = fixture();
    const saving = { ...image, id: 2, name: 'report.pdf', saving: true, file: true } as ChatAttachment;
    store.save('a', 'one', 'With a file', [image, saving]);
    store.save('a', 'two', '', [{ ...saving, id: 3 }]);
    store.settle(2, { path: '/saved/report.pdf', saving: false });
    store.settle(3, { saving: false, error: true });
    expect(store.read('a', 'one').attachments).toEqual([image, { ...saving, path: '/saved/report.pdf', saving: false }]);
    expect(store.read('a', 'two')).toEqual({ text: '', attachments: [{ ...saving, id: 3, saving: false, error: true }] });
  });
  it('carries a hidden composer into the chat it continued in, after any draft there (C-D2)', () => {
    const { store, storage, values } = fixture();
    store.save('w', 'prev', 'Follow-up', [image]);
    expect(store.carry('prev', 'next', 'Follow-up', [image])).toBe(true);
    expect(store.read('w', 'next')).toEqual({ text: 'Follow-up', attachments: [image] });
    // One write: the successor has the text and the predecessor's entry is gone.
    expect(JSON.parse(values.get(DRAFT_STORE_KEY)!)).toEqual([{ id: 'session:next', text: 'Follow-up' }]);
    const reloaded = createConversationDraftStore(() => storage);
    expect(reloaded.read('w', 'prev').text).toBe('');
    // Idempotent: the predecessor's draft is empty after the move.
    reloaded.carry('prev', 'next', reloaded.read('w', 'prev').text, []);
    expect(reloaded.read('w', 'next').text).toBe('Follow-up');
    store.carry('prev2', 'next', 'More', []);
    expect(store.read('w', 'next').text).toBe('Follow-up\n\nMore');
  });
  it('moves nothing when the joined draft would be too long to persist (C-D2 review)', () => {
    const { store, storage } = fixture();
    const half = 'x'.repeat(DRAFT_TEXT_BUDGET / 2);
    store.save('w', 'prev', half, []); store.save('w', 'next', half, []);
    expect(store.carry('prev', 'next', half, [])).toBe(false);
    const reloaded = createConversationDraftStore(() => storage);
    expect(reloaded.read('w', 'prev').text).toBe(half);
    expect(reloaded.read('w', 'next').text).toBe(half);
  });
  it('merges another writer per key after a failed save instead of overwriting it (C-D3)', () => {
    const f = fixture(); const other = createConversationDraftStore(() => f.storage);
    f.store.save('a', 'mine', 'Before', []);
    f.fail(true); expect(f.store.save('a', 'mine', 'Unsaved edit', [])).toBe(false); f.fail(false);
    other.save('a', 'theirs', 'Other window', []);
    expect(f.store.save('a', 'third', 'Recovered', [])).toBe(true);
    const fresh = createConversationDraftStore(() => f.storage);
    expect(fresh.read('a', 'theirs').text).toBe('Other window');
    expect(fresh.read('a', 'mine').text).toBe('Unsaved edit');
    expect(fresh.read('a', 'third').text).toBe('Recovered');
  });
  it('keeps other drafts and attachments when one oversized draft cannot persist', () => {
    const { store, storage } = fixture();
    store.save('a', 'one', 'Keep one', [image]); store.save('a', 'two', 'Keep two', []);
    const huge = 'x'.repeat(DRAFT_TEXT_BUDGET + 1);
    expect(store.save('a', 'huge', huge, [])).toBe(false);
    expect(store.read('a', 'one')).toEqual({ text: 'Keep one', attachments: [image] });
    store.save('a', 'three', 'Another small draft', []);
    expect(store.read('a', 'huge').text).toBe(huge);
    store.save('a', 'huge', '', []);
    expect(createConversationDraftStore(() => storage).read('a', 'one').text).toBe('Keep one');
    expect(store.read('a', 'two').text).toBe('Keep two');
  });
  it('does not let visiting empty composers evict real unsent drafts', () => {
    const { store } = fixture(); store.save('a', 'important', 'Keep my work', []);
    for (let i = 0; i < DRAFT_LIMIT + 1; i++) store.save('a', `empty${i}`, '', []);
    expect(store.read('a', 'important').text).toBe('Keep my work');
  });
  it('keeps sessions separate from each other and the new-chat slot, including after moving projects', () => {
    const { store, storage } = fixture();
    store.save('a', 'one', 'One', [image]); store.save('a', 'two', 'Two', []); store.save('a', null, 'New', []);
    expect(store.read('destination', 'one')).toEqual({ text: 'One', attachments: [image] });
    expect(store.read('a', 'two').text).toBe('Two'); expect(store.read('a', null).text).toBe('New');
    const reloaded = createConversationDraftStore(() => storage);
    expect(reloaded.read('a', 'one')).toEqual({ text: 'One', attachments: [] });
  });
  it('migrates a legacy slot once without overwriting a newer conversation draft', () => {
    const { store, values } = fixture();
    values.set('hollow.draft.a', 'Legacy');
    store.save('a', 'one', 'Newer', []);
    expect(store.migrate('a', 'one')).toBe(true);
    expect(store.read('a', 'one').text).toBe('Newer'); expect(store.read('a', 'two').text).toBe('');
    expect(values.has('hollow.draft.a')).toBe(false);
  });
  it('retains the legacy source and in-memory draft when storage fails, then retries', () => {
    const f = fixture(); f.values.set('hollow.draft.a', 'Legacy'); f.fail(true);
    expect(f.store.migrate('a', 'one')).toBe(false);
    expect(f.values.get('hollow.draft.a')).toBe('Legacy');
    f.store.save('a', 'one', 'Newest unsaved', [image]);
    expect(f.store.read('a', 'one').text).toBe('Newest unsaved');
    expect(f.store.read('a', 'two').text).toBe('');
    f.fail(false); expect(f.store.migrate('a', 'two')).toBe(true);
    expect(f.store.read('a', 'one').text).toBe('Newest unsaved'); expect(f.store.read('a', 'two').text).toBe('');
    expect(f.values.has('hollow.draft.a')).toBe(false);
  });
  it('moves only the new-chat slot on acceptance and keeps an in-flight follow-up', () => {
    const { store, storage } = fixture();
    store.save('a', null, 'Sent prompt', [image]); store.save('a', 'foreign', 'Foreign draft', [image]);
    store.accept('a', 'first', 'Typed during send', []);
    expect(store.read('a', null)).toEqual({ text: '', attachments: [] });
    expect(store.read('a', 'first').text).toBe('Typed during send');
    expect(store.read('a', 'foreign')).toEqual({ text: 'Foreign draft', attachments: [image] });
    expect(createConversationDraftStore(() => storage).read('a', null).text).toBe('');
  });
  it('bounds persisted entries and attachment retention without trimming the active text', () => {
    const { store, values } = fixture();
    for (let i = 0; i <= DRAFT_LIMIT; i++) store.save('a', `s${i}`, `text${i}`, [image]);
    expect(JSON.parse(values.get(DRAFT_STORE_KEY)!)).toHaveLength(DRAFT_LIMIT);
    expect(store.read('a', 's0')).toEqual({ text: '', attachments: [] });
    expect(store.read('a', `s${DRAFT_LIMIT}`).attachments).toEqual([image]);
    const huge = 'x'.repeat(DRAFT_TEXT_BUDGET + 1);
    expect(store.save('a', 'huge', huge, [])).toBe(false);
    expect(store.read('a', 'huge').text).toBe(huge);
    expect(values.get(DRAFT_STORE_KEY)!.length).toBeLessThan(DRAFT_TEXT_BUDGET);
  });
  it('recovers from malformed storage and expires oldest text when the character budget fills', () => {
    const { store, values } = fixture(); values.set(DRAFT_STORE_KEY, '{');
    expect(store.read('a', 'one').text).toBe('');
    store.save('a', 'one', 'x'.repeat(600_000), []); store.save('a', 'two', 'y'.repeat(600_000), []);
    expect(store.read('a', 'one').text).toBe(''); expect(store.read('a', 'two').text.length).toBe(600_000);
    expect(values.get(DRAFT_STORE_KEY)!.length).toBeLessThan(DRAFT_TEXT_BUDGET + 1000);
  });
});
