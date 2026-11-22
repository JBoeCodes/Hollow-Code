import type { ChatAttachment } from './chatRuntime';

export const DRAFT_STORE_KEY = 'hollow.conversationDrafts.v1';
export const DRAFT_LIMIT = 100;
export const DRAFT_TEXT_BUDGET = 1_000_000;
type StorageLike = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
type Row = { id: string; text: string; legacy?: string; stashOwner?: string };
export const conversationDraftId = (workspace: string, session: string | null) =>
  session ? `session:${session}` : `new:${workspace}`;

/** Text is persisted atomically, merged per key with other writers;
 *  attachments stay in this window. Oldest
 *  inactive drafts expire at 100 entries / one million text characters.
 *  An oversized current draft stays whole in memory and reports unsaved. */
export function createConversationDraftStore(storage: () => StorageLike) {
  let rows: Row[] = [];
  let lastRaw: string | null | undefined;
  // Ids changed here since the last good write (C-D3): another writer's
  // keys are merged in, these stay ours. Oversized text stays listed.
  const dirty = new Set<string>();
  const attachments = new Map<string, ChatAttachment[]>();
  function load() {
    try {
      const raw = storage().getItem(DRAFT_STORE_KEY);
      if (raw === lastRaw) return;
      lastRaw = raw;
      const value: unknown = raw && raw.length <= DRAFT_TEXT_BUDGET * 6 ? JSON.parse(raw) : [];
      const stored = Array.isArray(value) ? value.filter((v): v is Row =>
        !!v && typeof v.id === 'string' && v.id.length <= 1024 && typeof v.text === 'string'
        && (v.legacy === undefined || typeof v.legacy === 'string')
        && (v.stashOwner === undefined || typeof v.stashOwner === 'string' && v.stashOwner.length <= 1024)).slice(-DRAFT_LIMIT) : [];
      rows = [...stored.filter(row => !dirty.has(row.id)), ...rows.filter(row => dirty.has(row.id))];
      prune();
    } catch { /* Keep last-good data if storage is unavailable. */ }
  }
  function prune() {
    const counted = (row: Row) => row.text.length <= DRAFT_TEXT_BUDGET ? row.text.length : 0;
    let chars = rows.reduce((sum, row) => sum + counted(row), 0);
    while (rows.length > DRAFT_LIMIT || chars > DRAFT_TEXT_BUDGET) {
      // Oversized text is memory-only. It neither consumes the persisted
      // budget nor gets removed just because a smaller draft was edited.
      const at = rows.length > DRAFT_LIMIT ? 0 : rows.findIndex(row => row.text.length <= DRAFT_TEXT_BUDGET);
      const [removed] = rows.splice(at, 1); chars -= counted(removed);
    }
    const retained = new Set(rows.map(row => row.id));
    for (const key of attachments.keys()) if (!retained.has(key)) attachments.delete(key);
  }
  function persist() {
    prune();
    try {
      const persisted = rows.filter(row => row.text.length <= DRAFT_TEXT_BUDGET);
      const raw = JSON.stringify(persisted);
      storage().setItem(DRAFT_STORE_KEY, raw);
      lastRaw = raw; dirty.clear();
      for (const row of rows) if (row.text.length > DRAFT_TEXT_BUDGET) dirty.add(row.id);
      return !dirty.size;
    } catch { return false; }
  }
  function put(id: string, text: string, atts: ChatAttachment[]) {
    const legacy = rows.find(row => row.id === id)?.legacy;
    rows = rows.filter(row => row.id !== id); dirty.add(id);
    // A chip still saving is kept: its save finishes into this draft
    // through `settle` (C-D1), so it never comes back stuck at Saving….
    if (!text && !atts.length && !legacy) { attachments.delete(id); return; }
    rows.push({ id, text, ...(legacy ? { legacy } : {}) });
    attachments.set(id, atts);
  }
  return {
    stashes(workspace: string, session: string | null) {
      load();
      const owner = conversationDraftId(workspace, session);
      return rows.filter(row => row.stashOwner === owner).map(row => ({ id: row.id, text: row.text, attachments: attachments.get(row.id) ?? [] }));
    },
    /** Move the composer to a named stash in one storage write. Refuse an
     * oversized draft or a full stash without consuming anything. */
    stash(workspace: string, session: string | null, token: string, text: string, atts: ChatAttachment[]) {
      load();
      const owner = conversationDraftId(workspace, session), id = `stash:${token}`;
      if ((!text && !atts.length) || text.length > DRAFT_TEXT_BUDGET || rows.some(row => row.id === id)
        || rows.filter(row => row.stashOwner === owner).length >= 5) return null;
      put(id, text, atts);
      rows.find(row => row.id === id)!.stashOwner = owner;
      put(owner, '', []);
      return { saved: persist() };
    },
    /** Restore beside newer text and chips, atomically removing the stash.
     * A failed disk write retains the combined draft in memory for retry. */
    restoreStash(workspace: string, session: string | null, id: string, text: string, atts: ChatAttachment[]) {
      load();
      const owner = conversationDraftId(workspace, session);
      const row = rows.find(row => row.id === id && row.stashOwner === owner);
      if (!row) return null;
      const joined = row.text && text ? `${row.text}\n\n${text}` : row.text || text;
      const chips = [...(attachments.get(id) ?? []), ...atts];
      if (joined.length > DRAFT_TEXT_BUDGET || chips.length > 10
        || chips.reduce((sum, chip) => sum + (chip.size ?? 0), 0) > 50 * 1024 * 1024) return null;
      rows = rows.filter(item => item.id !== id); attachments.delete(id); dirty.add(id);
      put(owner, joined, chips);
      return { text: joined, attachments: chips, saved: persist() };
    },
    read(workspace: string, session: string | null) {
      load();
      const id = conversationDraftId(workspace, session);
      return { text: rows.find(row => row.id === id)?.text ?? '', attachments: attachments.get(id) ?? [] };
    },
    save(workspace: string, session: string | null, text: string, atts: ChatAttachment[]) {
      load(); put(conversationDraftId(workspace, session), text, atts);
      return persist();
    },
    /** A chip's save finished (or failed) after its chat was left: update
     *  it in whichever stored draft holds it. Chip ids are unique. */
    settle(chip: number, patch: Partial<ChatAttachment>) {
      for (const [id, list] of attachments) if (list.some(a => a.id === chip)) attachments.set(id, list.map(a => a.id === chip ? { ...a, ...patch } : a));
    },
    /** A handover hid chat `from`'s composer (C-D2): its unsent text and
     *  chips move to `to`, after any draft there, and `from`'s stored draft
     *  goes in the SAME write, so a crash can't leave a copy to move twice.
     *  False means nothing moved: the joined text would be too long to
     *  persist, so both drafts stay where they are. A write that throws
     *  keeps the move in memory and retries on the next save. */
    carry(from: string, to: string, text: string, atts: ChatAttachment[]) {
      load();
      const id = conversationDraftId('', to), source = conversationDraftId('', from);
      if (source === id) return true;
      const prior = rows.find(row => row.id === id)?.text ?? '';
      const joined = prior && text ? `${prior}\n\n${text}` : prior || text;
      if (joined.length > DRAFT_TEXT_BUDGET) return false;
      for (const row of rows) if (row.stashOwner === source) { row.stashOwner = id; dirty.add(row.id); }
      rows = rows.filter(row => row.id !== source); attachments.delete(source); dirty.add(source);
      put(id, joined, [...(attachments.get(id) ?? []), ...atts]);
      persist();
      return true;
    },
    /** Bind old workspace-only text to its previous/default conversation,
     *  before a different conversation (or New chat) can claim it. Delete
     *  the legacy slot only after the new store has been saved successfully. */
    migrate(workspace: string, previous: string | null) {
      load();
      try {
        const legacyKey = `hollow.draft.${workspace}`;
        const text = storage().getItem(legacyKey);
        if (text === null) return true;
        const id = rows.find(row => row.legacy === workspace)?.id ?? conversationDraftId(workspace, previous);
        const prior = rows.find(row => row.id === id);
        rows = rows.filter(row => row.id !== id); dirty.add(id);
        rows.push({ id, text: prior?.text ?? text, legacy: workspace });
        if (!persist()) return false;
        storage().removeItem(legacyKey);
        const migrated = rows.find(row => row.id === id);
        if (migrated) delete migrated.legacy;
        return true;
      } catch { return false; }
    },
    /** The first accepted send acquires the current composer, including
     *  text typed while acceptance was pending. The empty New chat slot is
     *  cleared in the SAME write, so leaving before an effect runs is safe. */
    accept(workspace: string, session: string, text: string, atts: ChatAttachment[]) {
      load();
      const fresh = conversationDraftId(workspace, null);
      for (const row of rows) if (row.stashOwner === fresh) {
        row.stashOwner = conversationDraftId(workspace, session); dirty.add(row.id);
      }
      rows = rows.filter(row => row.id !== fresh); attachments.delete(fresh); dirty.add(fresh);
      put(conversationDraftId(workspace, session), text, atts);
      return persist();
    },
  };
}

export const conversationDrafts = createConversationDraftStore(() => localStorage);
let firstSession: ((cwd: string | null, key: string) => void) | undefined;
let beforeReset: (() => void) | undefined;
export function bindFirstDraftSession(handler: typeof firstSession) { firstSession = handler; }
export function acceptFirstDraftSession(cwd: string | null, key: string) { firstSession?.(cwd, key); }
export function bindBeforeDraftReset(handler: typeof beforeReset) { beforeReset = handler; }
export function rememberBeforeDraftReset() { beforeReset?.(); }
