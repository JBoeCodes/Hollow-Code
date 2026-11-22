/**
 * The canvas store in the browser mock (Canvas C3; the real one is
 * `src-tauri/src/workspace_canvases.rs`). Enough of its contract for the chat
 * to save and read canvases in smoke tests: a kept body is `unchanged`
 * whatever the token, a new body needs the current token (else
 * `CANVAS_CHANGED:`), the hash is the body's real SHA-256. Kept in
 * localStorage `mockCanvases`, one record per store (workspace id, else
 * session key). `mockCanvasRetired` (a JSON list of store keys) makes every
 * record into that store refused with `CANVAS_RETIRED:`, like a pruned
 * app-data conversation. Calls are counted by `__mockApiCalls` as usual.

 */
import { parseCanvasFence } from './canvas/fence';
import { sha256Hex } from './canvas/chatCanvas';
import { readCanvasAnswers } from './canvas/askAnswer';
import type { Canvas, CanvasRecorded, CanvasRevision, CanvasSummary, NeedsYouItem } from './generated/ipc';

type Stored = { canvas: Omit<Canvas, 'html'>; bodies: Record<string, string> };
type Stores = Record<string, Record<string, Stored>>;

const COMMANDS = new Set(['list_canvases', 'read_canvas', 'read_canvas_revision', 'record_canvas', 'save_canvas_state', 'canvas_export', 'canvas_measure']);

const load = (): Stores => { try { return JSON.parse(localStorage.getItem('mockCanvases') ?? '{}'); } catch { return {}; } };
const save = (s: Stores) => localStorage.setItem('mockCanvases', JSON.stringify(s));

function storeKey(args: Record<string, unknown> | undefined): { key: string; workspaceId: string | null; sessionKey: string | null } {
  const workspaceId = typeof args?.workspaceId === 'string' && args.workspaceId ? args.workspaceId : null;
  const sessionKey = typeof args?.sessionKey === 'string' && args.sessionKey ? args.sessionKey : null;
  if (!workspaceId && !sessionKey) throw 'CANVAS_INVALID: name a workspace or a conversation';
  return { key: workspaceId ? `ws:${workspaceId}` : `s:${sessionKey}`, workspaceId, sessionKey };
}

const full = (s: Stored): Canvas => ({ ...s.canvas, html: s.bodies[s.canvas.fenceSha256] ?? '' });

export function isMockCanvasCommand(command: string): boolean {
  return COMMANDS.has(command);
}

export async function mockCanvasCommand(command: string, args: Record<string, unknown> | undefined): Promise<unknown> {
  if (command === 'canvas_export') {
    const r = args?.request as import('./canvas/canvasExport').CanvasExportRequest;
    localStorage.setItem('mockCanvasLastExport', JSON.stringify(r));
    const error = localStorage.getItem('mockCanvasExportError');
    const delay = Number(localStorage.getItem('mockCanvasExportDelay') || 0);
    if (delay) await new Promise(resolve => setTimeout(resolve, delay));
    if (error) throw error;
    const parsed = parseCanvasFence(r.source, true);
    if (parsed.state !== 'ready' || !r.workspaceId) throw 'Invalid canvas export';
    if (r.target === 'attachment') return { rel: null, path: null, pngBase64: 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==', width: 1, height: 1, scale: 1, copied: false, warning: null };
    const { exportName, EXPORT_WIDTHS } = await import('./canvas/canvasExport');
    const rel = `outputs/exports/${exportName(parsed.root.id, r.revision ?? 1, r.preset, r.format, r.scheme === 'dark', r.flowIndex, r.themeName)}`;
    return { rel, path: `/mock/vault/workspaces/${r.workspaceId}/${rel}`, pngBase64: null, width: EXPORT_WIDTHS[r.preset], height: 480, scale: 2, copied: r.copy, warning: null };
  }
  // Repo ideas 10-27 #4: four 1×1 pictures that differ nowhere, so every
  // unresolved text stays unresolved ("too little of it shows").
  if (command === 'canvas_measure') {
    localStorage.setItem('mockCanvasLastMeasure', JSON.stringify(args?.request ?? null));
    const error = localStorage.getItem('mockCanvasMeasureError');
    if (error) throw error;
    const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==';
    return { shown: png, hidden: png, black: png, white: png, width: 1, height: 1, scale: 1 };
  }
  const scope = storeKey(args);
  const stores = load();
  const store = (stores[scope.key] ??= {});
  const id = String(args?.id ?? '');
  switch (command) {
    case 'list_canvases':
      return Object.values(store).map((s): CanvasSummary => ({
        id: s.canvas.id, title: s.canvas.title, revision: s.canvas.revision, updatedAt: s.canvas.updatedAt,
        fenceSha256: s.canvas.fenceSha256, path: s.canvas.path, file: s.canvas.file, error: null,
      })).sort((a, b) => String(b.updatedAt).localeCompare(String(a.updatedAt)));
    case 'read_canvas':
      return store[id] ? full(store[id]) : null;
    case 'read_canvas_revision': {
      const s = store[id];
      const r = s?.canvas.revisions.find(x => x.revision === Number(args?.revision));
      if (!s || !r) throw `no revision ${String(args?.revision)} of canvas ${id}`;
      return { revision: r.revision, fenceSha256: r.fenceSha256, title: r.title, html: s.bodies[r.fenceSha256] ?? '' };
    }
    case 'save_canvas_state': {
      const s = store[id];
      if (!s) throw `CANVAS_CHANGED: canvas ${id} is not saved`;
      if (args?.expectedSha256 !== s.canvas.sha256) throw 'CANVAS_CHANGED: the canvas changed since it was read';
      s.canvas.state = args?.canvasState ?? null;
      s.canvas.stateSavedAt = new Date().toISOString();
      s.canvas.sha256 = await sha256Hex(`${s.canvas.sha256}:state`);
      save(stores);
      return { sha256: s.canvas.sha256, stateSavedAt: s.canvas.stateSavedAt, revision: s.canvas.revision };
    }
    case 'record_canvas': {
      const body = String(args?.body ?? '');
      const p = parseCanvasFence(body, true);
      if (p.state !== 'ready') throw `CANVAS_INVALID: ${p.state === 'invalid' ? p.reason : 'the canvas is unfinished'}`;
      const sha = await sha256Hex(body);
      const cid = p.root.id;
      const prior = store[cid];
      const kept = prior?.canvas.revisions.find(r => r.fenceSha256 === sha);
      if (prior && kept) return { outcome: 'unchanged', matchedRevision: kept.revision, canvas: full(prior) } satisfies CanvasRecorded;
      const retired = JSON.parse(localStorage.getItem('mockCanvasRetired') ?? '[]') as string[];
      if (retired.includes(scope.key)) throw 'CANVAS_RETIRED: this conversation\'s saved canvases were pruned; render the fence itself';
      const expected = (args?.expectedSha256 as string | null | undefined) ?? null;
      if (expected !== (prior?.canvas.sha256 ?? null)) throw 'CANVAS_CHANGED: the canvas changed since it was read';
      const at = new Date().toISOString();
      const revision = (prior?.canvas.revision ?? 0) + 1;
      const rev: CanvasRevision = {
        revision, fenceSha256: sha, title: p.root.title, savedAt: at, ask: p.root.ask, height: p.root.height,
        session: scope.sessionKey, turnId: (args?.turnId as string | null | undefined) ?? null,
      };
      const path = scope.workspaceId ? `workspaces/${scope.workspaceId}/outputs/canvases/${cid}.canvas.html` : null;
      const next: Stored = {
        bodies: { ...(prior?.bodies ?? {}), [sha]: body },
        canvas: {
          scope: scope.workspaceId ? 'workspace' : 'session', workspaceId: scope.workspaceId, sessionKey: scope.sessionKey,
          id: cid, title: p.root.title, kit: (args?.kit as string | null | undefined) ?? prior?.canvas.kit ?? null,
          session: scope.sessionKey, fenceSha256: sha, createdAt: prior?.canvas.createdAt ?? at, updatedAt: at, revision,
          ask: p.root.ask, height: p.root.height, sha256: await sha256Hex(`${cid}:${revision}:${sha}`),
          path, file: path ? `${cid}.canvas.html` : null, htmlChanged: false,
          state: prior?.canvas.state ?? null, stateSavedAt: prior?.canvas.stateSavedAt ?? null,
          revisions: [...(prior?.canvas.revisions ?? []), rev].slice(-30),
        },
      };
      store[cid] = next;
      save(stores);
      return { outcome: prior ? 'revised' : 'created', matchedRevision: revision, canvas: full(next) } satisfies CanvasRecorded;
    }
  }
  return null;
}

/** Canvas C4 A2's ask-canvas conversation (`mockAskCanvasFixture`): an
 *  ask-canvas reply in "Wire the session store", recorded in that
 *  conversation's store as v1, as a live turn would have. A sent answer
 *  (`mockLastChatPrompt`, with the composer's leading context blocks peeled
 *  off as `sessions::strip_injected_context` does) comes back as the next
 *  user turn, so a reload reopens the conversation as the backend would. */
export const ASK_FIXTURE_CANVAS = `<hc-canvas id="release-choices" title="Release choices" ask>
<hc-ask title="Release choices" send="Send answers">
<hc-choice name="ship-day" label="When should the release ship?" required>
* Friday, after the bed passes
Monday, with a second look
</hc-choice>
<hc-multi name="checks" label="Which checks must pass first?">
Bed rows
Bundle budget
</hc-multi>
</hc-ask>
</hc-canvas>`;

/** Canvas C6 P1: a second ask-canvas in the same conversation
 *  (`mockAskCanvasSecond`), so one canvas's draft meets another's. */
export const ASK_FIXTURE_SECOND = `<hc-canvas id="release-owner" title="Release owner" ask>
<hc-ask title="Release owner" send="Send owner">
<hc-choice name="owner" label="Who ships it?" required>
Ada
Grace
</hc-choice>
</hc-ask>
</hc-canvas>`;
const askSecond = () => localStorage.getItem('mockAskCanvasSecond') === '1';

/** P1-33: a later reply with v2 of the fixture canvas (`mockAskCanvasRevised`),
 *  so the first reply's fence is an earlier revision. */
export const ASK_FIXTURE_REVISED = ASK_FIXTURE_CANVAS.replace('Monday, with a second look', 'Monday, with a second look\nTuesday, after the review');
const askRevised = () => localStorage.getItem('mockAskCanvasRevised') === '1';

export async function askCanvasFixtureTurns(createdAt: string, sessionKey: string) {
  await recordAskFixture(sessionKey);
  const turn = (id: string, role: 'user' | 'assistant', text: string, runId: string) => ({
    id, role, text, tools: [], agent: 'codex', runId, startedAt: createdAt, transcriptIncomplete: false,
    ...(role === 'assistant' ? { status: 'success' } : {}),
  });
  const turns = [
    turn('ask-user', 'user', 'Help me choose how to ship the release.', 'run-ask-1'),
    turn('ask-reply', 'assistant', `Two choices decide it; answer them in the canvas.\n\n\`\`\`hollow-canvas\n${ASK_FIXTURE_CANVAS}\n\`\`\``, 'run-ask-1'),
  ];
  if (askSecond()) {
    turns.push(turn('ask-user-2', 'user', 'And who ships it?', 'run-ask-3'));
    turns.push(turn('ask-reply-2', 'assistant', `One more.\n\n\`\`\`hollow-canvas\n${ASK_FIXTURE_SECOND}\n\`\`\``, 'run-ask-3'));
  }
  if (askRevised()) {
    turns.push(turn('ask-user-rev', 'user', 'Add Tuesday as a choice.', 'run-ask-4'));
    turns.push(turn('ask-reply-rev', 'assistant', `Added.\n\n\`\`\`hollow-canvas\n${ASK_FIXTURE_REVISED}\n\`\`\``, 'run-ask-4'));
  }
  // C4 A3: `mockAskCanvasFiller` adds that many exchanges after the ask, so
  // the canvas sits far above the bottom ("waiting on you" must scroll to it).
  const filler = Math.min(200, Number(localStorage.getItem('mockAskCanvasFiller') ?? 0) || 0);
  for (let i = 0; i < filler; i++) {
    turns.push(turn(`ask-filler-u${i}`, 'user', `Side question ${i + 1}`, `run-filler-${i}`));
    turns.push(turn(`ask-filler-a${i}`, 'assistant', `Side answer ${i + 1}.\n\n${'More detail on this side point. '.repeat(6)}`, `run-filler-${i}`));
  }
  let sent = localStorage.getItem('mockLastChatPrompt') ?? '';
  if (sent.includes('hollow-canvas-answer')) {
    for (let m; (m = /^\s*<([a-z-]+)>[\s\S]*?<\/\1>/.exec(sent));) sent = sent.slice(m[0].length);
    turns.push(turn('ask-answer', 'user', sent.replace(/<attached-(?:images|files)[\s\S]*$/, '').trim(), 'run-ask-2'));
  }
  return turns;
}

/** The fixture's ask-canvas, saved in its conversation's store as v1 (a
 *  kept body changes nothing). */
async function recordAskFixture(sessionKey: string) {
  await mockCanvasCommand('record_canvas', { sessionKey, body: ASK_FIXTURE_CANVAS, expectedSha256: null }).catch(() => null);
  if (askSecond()) await mockCanvasCommand('record_canvas', { sessionKey, body: ASK_FIXTURE_SECOND, expectedSha256: null }).catch(() => null);
  if (askRevised()) {
    // A revision is a compare-and-swap on the saved v1, as the backend's is.
    const v1 = await mockCanvasCommand('read_canvas', { sessionKey, id: 'release-choices' }).catch(() => null) as { sha256?: string } | null;
    await mockCanvasCommand('record_canvas', { sessionKey, body: ASK_FIXTURE_REVISED, expectedSha256: v1?.sha256 ?? null }).catch(() => null);
  }
}

/** "Needs you" ask-canvases in the mock (`needs_you::waiting_canvas`): every
 *  saved canvas whose current revision is an ask-canvas, emitted by a
 *  conversation that hasn't sent its answer. The sent message is the last
 *  `chat_send` prompt (`mockLastChatPrompt`), read as the chat's lock reads
 *  it. With `mockAskCanvasFixture`, the fixture conversation's canvas is
 *  saved first, as the backend's store would already hold it. */
export async function mockAskCanvasNeedsYou(fixtureKey: string): Promise<NeedsYouItem[]> {
  if (localStorage.getItem('mockAskCanvasFixture') === '1') await recordAskFixture(fixtureKey);
  const sent = readCanvasAnswers(localStorage.getItem('mockLastChatPrompt') ?? '').map(a => `${a.canvasId}@${a.revision}`);
  const out: NeedsYouItem[] = [];
  for (const store of Object.values(load())) {
    for (const { canvas } of Object.values(store)) {
      const head = canvas.revisions[canvas.revisions.length - 1];
      const key = head?.session;
      const source = `${canvas.id}@${canvas.revision}`;
      if (!head?.ask || !key || sent.includes(source)) continue;
      out.push({
        id: `canvas:${key}/${source}`, kind: 'canvas', sourceId: source, title: head.title, detail: null, state: 'pending',
        at: head.savedAt, workspaceId: canvas.workspaceId, sessionKey: key, path: canvas.path, category: null, link: null,
      });
    }
  }
  return out;
}
