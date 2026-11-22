import { addToDraft } from '../chatQueue';
import { chatCwd, chatInput, setChatInput, requestChatFocus, flashFooter } from './runtime';
import { closePreview } from './preview';
import { windowAtMost } from '../windowLayout';
import { workspaceDetail, workspaceId, focusWorkspaceConversation } from './workspaces';

const pending = new Map<string, Promise<boolean>>();
const failure = () => {
  flashFooter("× Couldn't open this workspace's conversation, so nothing was added.", 7000);
  return false;
};

/** Open this workspace's conversation before touching the global composer. Never sends. */
export function draftInWorkspace(text: string): Promise<boolean> {
  const id = workspaceId();
  const cwd = workspaceDetail()?.workspace.cwd;
  if (!id || !cwd) return Promise.resolve(failure());
  const key = JSON.stringify([id, text]);
  const existing = pending.get(key);
  if (existing) return existing;
  // Defer the body until the map entry exists, including synchronous failures.
  const request = Promise.resolve().then(async () => {
    if (workspaceId() !== id || workspaceDetail()?.workspace.cwd !== cwd) return failure();
    try {
      if (!await focusWorkspaceConversation()
        || workspaceId() !== id || workspaceDetail()?.workspace.cwd !== cwd
        || chatCwd() !== cwd) return failure();
      setChatInput(addToDraft(chatInput(), text));
      // In a narrow window the Preview pane covers the page: close it so the
      // draft is on screen (the request below brings the Chat view back).
      if (windowAtMost('medium')) closePreview();
      requestChatFocus();
      return true;
    } catch { return failure(); }
  }).finally(() => { pending.delete(key); });
  pending.set(key, request);
  return request;
}

/** Library passages retain their existing current-conversation behavior. */
export function draftInCurrentConversation(text: string): void {
  setChatInput(`${chatInput()}${chatInput() ? '\n\n' : ''}${text}`);
  requestChatFocus();
}
