import { render } from 'solid-js/web';
import { createSignal } from 'solid-js';
import { ChatMessage } from '../../../src/components/ChatMessage';
import { ToolCall } from '../../../src/components/ToolCall';
import { MediaPreview } from '../../../src/components/MediaPreview';
import { api } from '../../../src/api';
import { chatPreviewPath } from '../../../src/store/chat';
import { setChatCwd, setChatConversationVersion } from '../../../src/store/chatRuntime';

export function mount(kind: 'reply' | 'bash' | 'media', initial = '') {
  const host = document.createElement('section');
  host.id = 'reply-study';
  host.style.cssText = 'position:fixed;inset:0;z-index:99999;overflow:auto;background:var(--canvas);color:var(--ink);padding:24px';
  document.body.append(host);
  const [text, setText] = createSignal(initial);
  const [session, setSession] = createSignal('first');
  const [expanded, setExpanded] = createSignal(true);
  setChatCwd('/vault/workspaces/reply');
  let failed = true, opens = 0, grants = 0;
  let releaseImage = () => {};
  let releaseHeldImages!: () => void;
  const heldImages = new Promise<void>(resolve => { releaseHeldImages = resolve; });
  let holdGrant = false, releaseGrant = () => {};
  const released: string[] = [], reads: string[] = [], remote: string[] = [];
  const originals = { grant: api.mediaGrant, release: api.mediaRelease, preview: api.chatPreviewFile, fetch: api.fetchRemoteImage };
  api.mediaGrant = async () => {
    const number = ++grants;
    if (failed) throw new Error('temporary read failure');
    if (holdGrant) { holdGrant = false; await new Promise<void>(resolve => { releaseGrant = resolve; }); }
    return { token: String(number).padStart(32, '0'), size: 42, mime: 'video/mp4', modifiedMs: 1 };
  };
  api.mediaRelease = async token => { released.push(token); };
  api.chatPreviewFile = async path => {
    reads.push(path);
    if (path.includes('held-')) await heldImages;
    if (path === '/vault/workspaces/reply/slow.png') await new Promise<void>(resolve => { releaseImage = resolve; });
    if (path.endsWith('missing.png') || path.startsWith('/vault/workspaces/gone/')) throw new Error('No such file');
    return { kind: 'image', text: null, dataUrl: PNG, size: path.endsWith('large.png') ? 11 * 1024 * 1024 : 68, modifiedMs: 1, truncated: false };
  };
  api.fetchRemoteImage = async url => {
    remote.push(url);
    if (url.includes('fail')) throw new Error('offline');
    return PNG;
  };
  const dispose = render(() => <>
    <h1>Reply content</h1>
    {kind === 'reply' ? <ChatMessage text={text()} role="assistant" streaming />
      : kind === 'bash' ? <ToolCall tool={{ name: 'Bash', input: { command: 'printf "hello\\n"' }, result: 'hello\n', status: 'done', id: 'bash-test' }} expanded={expanded()} onToggle={() => setExpanded(v => !v)} />
      : <MediaPreview path="/tmp/retry.mp4" sessionKey={session()} variant="inline" onOpenPane={() => opens++} />}
  </>, host);
  return {
    text: setText, session: setSession, allowMedia: () => { failed = false; },
    move: (cwd: string) => { setChatCwd(cwd); setChatConversationVersion(v => v + 1); },
    // A new native session in the same folder with the transcript still on
    // screen (an agent switch, a mode change that starts fresh).
    restart: () => setChatConversationVersion(v => v + 1),
    // The folder alone changes (the composer's /cwd keeps the conversation).
    cd: (cwd: string) => setChatCwd(cwd),
    releaseImage: () => releaseImage(),
    releaseHeldImages: () => releaseHeldImages(),
    holdNextGrant: () => { holdGrant = true; }, releaseGrant: () => releaseGrant(),
    state: () => ({ opens, grants, released, reads, remote, preview: chatPreviewPath() }),
    dispose: () => { dispose(); host.remove(); Object.assign(api, { mediaGrant: originals.grant, mediaRelease: originals.release, chatPreviewFile: originals.preview, fetchRemoteImage: originals.fetch }); },
  };
}
const PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6LJ8AAAAASUVORK5CYII=';
