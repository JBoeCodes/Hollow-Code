/**
 * Unsafe chat working folders (Paperclip PR 19).
 *
 * The backend refuses to run an agent in a system or shared root (`/`,
 * `/tmp`, `/usr`, `/System`, `/Library`, the bare home folder; never the
 * vault) with an error that starts `UNSAFE_WORKING_DIRECTORY:`. It is NOT the
 * `CHAT_CONTEXT_RECOVERED:` error of a folder that vanished: the folder
 * exists, so the conversation is kept and the user picks another folder.
 *
 * Pure and dependency-light: the node vitest suite loads it directly.
 */
import { errMsg } from './utils';

/** The backend's prefix (`agent::UNSAFE_CWD_ERROR` + `: `). */
export const UNSAFE_CWD_PREFIX = 'UNSAFE_WORKING_DIRECTORY:';

/** What the composer adds after the reason. */
export const UNSAFE_CWD_HINT = 'Use /cwd <folder> or /cwd reset (the vault).';

/** The reason without its prefix when `err` is an unsafe-folder refusal,
 *  else null. Tauri rejects with a plain string, so this reads through
 *  `errMsg`, never `err.message`. */
export function unsafeCwdText(err: unknown): string | null {
  const message = errMsg(err).trim();
  if (!message.startsWith(UNSAFE_CWD_PREFIX)) return null;
  return message.slice(UNSAFE_CWD_PREFIX.length).trim();
}

/** The system notice the composer shows for an unsafe-folder refusal. */
export function unsafeCwdNotice(text: string): string {
  return `${text} ${UNSAFE_CWD_HINT}`;
}
