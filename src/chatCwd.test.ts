import { describe, expect, it } from 'vitest';
import { UNSAFE_CWD_HINT, UNSAFE_CWD_PREFIX, unsafeCwdNotice, unsafeCwdText } from './chatCwd';
import { CHAT_CONTEXT_RECOVERED_PREFIX } from './chatQueue';

const refusal =
  "UNSAFE_WORKING_DIRECTORY: Hollow won't run an agent in /private/tmp is a system or shared folder. Choose a project folder, or use the vault.";

describe('unsafeCwdText', () => {
  it('parses a string rejection (how Tauri rejects) and an Error, stripping the prefix', () => {
    const expected = "Hollow won't run an agent in /private/tmp is a system or shared folder. Choose a project folder, or use the vault.";
    expect(unsafeCwdText(refusal)).toBe(expected);
    expect(unsafeCwdText(new Error(refusal))).toBe(expected);
    expect(unsafeCwdText(`  ${refusal}  `)).toBe(expected);
  });

  it('does not match the vanished-folder recovery or anything else', () => {
    expect(unsafeCwdText(`${CHAT_CONTEXT_RECOVERED_PREFIX} /Users/me/old-worktree`)).toBeNull();
    expect(unsafeCwdText('Hollow is busy: a chat turn holds the vault')).toBeNull();
    expect(unsafeCwdText(new Error('claude exited 1'))).toBeNull();
    expect(unsafeCwdText(undefined)).toBeNull();
    expect(UNSAFE_CWD_PREFIX.startsWith(CHAT_CONTEXT_RECOVERED_PREFIX)).toBe(false);
  });

  it('builds a notice that tells the user how to pick another folder', () => {
    const notice = unsafeCwdNotice(unsafeCwdText(refusal)!);
    expect(notice.endsWith(UNSAFE_CWD_HINT)).toBe(true);
    expect(notice).toContain('/cwd');
    expect(notice).not.toContain(UNSAFE_CWD_PREFIX);
  });
});
