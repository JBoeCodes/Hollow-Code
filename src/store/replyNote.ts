// Save as note on a chat reply (One Ask, 2026-11-03): the reply's More
// actions writes it as a Markdown note (where: `turnFooter.ts`
// `replyNotePath`), never over an existing file, and says where in a toast
// whose Open shows it in the Preview pane. It took over Ask your vault's Save
// as note when that view was removed. The Conversation tools' Save
// conversation as note (overnight 11-07, D1) writes the whole conversation
// the same way. Loaded by ChatBody on first use.
import { api } from '../api';
import { joinPath } from '../chatFileLinks';
import { replyNoteBody, replyNotePath, replyNoteTitle } from '../turnFooter';
import { errMsg } from '../utils';
import { vaultRoot } from './chatRuntime';
import { openPreview } from './preview';
import { openVaultNote } from './runtime';
import { flashFooter, flashWithUndo } from './snackbars';

/** Names tried before giving up (`-2` … `-50`). */
const TRIES = 50;
let saving = false;

type Place = { id: string; kind?: string; path?: string } | null | undefined;

async function openSaved(path: string) {
  const root = await vaultRoot();
  if (root) openPreview({ kind: 'file', path: joinPath(root, path) });
  else openVaultNote(path);
}

/** Write one note named from `title` (the first free name), toast where. */
async function saveNote(title: string, body: (now: Date) => string, workspace: Place): Promise<string | null> {
  if (saving) { flashFooter('Still saving the last note…', 2500); return null; }
  saving = true;
  try {
    const now = new Date();
    const text = body(now);
    for (let attempt = 1; attempt <= TRIES; attempt++) {
      const path = replyNotePath(workspace, title, now, attempt);
      try {
        await api.vaultCreateNote(path, text);
      } catch (e) {
        if (/already exists/i.test(errMsg(e))) continue;
        flashFooter(`× Couldn’t save the note: ${errMsg(e)}`);
        return null;
      }
      flashWithUndo({ message: `Saved as a note: ${path}`, kind: 'status', ttlMs: 8000, actionLabel: 'Open', undo: () => openSaved(path) });
      return path;
    }
    flashFooter('× Couldn’t save the note: every name for it is taken.');
    return null;
  } finally {
    saving = false;
  }
}

/** Save `reply` (its Markdown) as a new note; resolves to its vault path, or
 *  null when it wasn't saved (a toast says why, a save still running
 *  included). */
export async function saveReplyAsNote(reply: string, question: string | null, workspace: Place): Promise<string | null> {
  if (!reply.trim()) return null;
  const title = replyNoteTitle(question, reply);
  return saveNote(title, now => replyNoteBody(title, reply, now), workspace);
}

/** Save a whole conversation (`conversationMarkdown`, without a heading of
 *  its own) as a new note under `title`; the same path and toast. */
export async function saveConversationAsNote(conversation: string, title: string, workspace: Place): Promise<string | null> {
  if (!conversation.trim()) { flashFooter('Nothing to save in this conversation yet', 2000); return null; }
  return saveNote(title, now => replyNoteBody(title, conversation, now, 'conversation'), workspace);
}
