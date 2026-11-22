// Browser-mock `peer_*` (M2, the peer link; backend `commands/peer_api.rs`,
// `docs/reference/peers.md`). No other Mac is paired until a spec seeds one:
// localStorage `mockPeers` holds a JSON seed (any field of `MockPeers`) or the
// word `demo` (a Mac mini online with three chats: one running, one with an
// open card, one with a canvas). The state is a live object on
// `window.__mockPeers`, so a spec can stand in for the other Mac: change a
// machine's state or a chat, then emit `peer-machines-changed` or
// `peer-chat-event` with `window.__mockEmit`. Sends, stops and answers change
// the chat and emit what the hub would (a `detail` event), as its home would
// answer them. Rejects with a `PeerFailure`, as the commands do.
//
// M5 UI: `chat_move_*` too. `homes` are this Mac's home records (a spec seeds
// a moving or moved chat there); Move to runner moves a chat to `moveTo`
// (it then lists there with `movedFrom`, titled from `titles`), or answers
// `nextMove` once; Keep it here settles at once while that Mac is online and
// waits otherwise; Release here and Forget release as the backend does.
import type { Accepted, ChatMoveResult, HomeRow, MachineRef, OfflineRow, PeerChatDetail, PeerFailure, PeerMachine, PeerMachinesView, PeerSessionsPage, PeerState, RemoteLive, RemoteSession } from './generated/ipc';
import { emitMockEvent } from './nativeEvents';
import { parsePairAddress } from './store/pairAddress';

/** Releases a held `peer_pair` (`pairHold`) when `peer_pair_cancel` arrives. */
let holdRelease: (() => void) | null = null;

export interface MockPeerChat {
  session: RemoteSession;
  permissionMode: string | null;
  /** The wire's turns (`Turn` in docs/PHONE_CHAT_API.md), oldest first. */
  turns: Record<string, unknown>[];
  live: RemoteLive | null;
}

export interface MockPeers {
  thisMac: MachineRef;
  suggestedAlias: string | null;
  machines: PeerMachine[];
  /** Chats per Mac, by machine id. */
  chats: Record<string, MockPeerChat[]>;
  /** The next `peer_pair` fails with this sentence. */
  pairFail?: string | null;
  /** The next `peer_pair` waits until `peer_pair_cancel` (then fails, as the backend's does). */
  pairHold?: boolean;
  /** Macs whose runner COO `peer_start_coo` started, in order. */
  cooStarted?: string[];
  /** Every `peer_pair` address and `peer_pair_cancel` call, in order. */
  pairCalls?: string[];
  /** Chat keys whose next send answers busy. */
  busyKeys?: string[];
  /** A Mac's state after Check now / Retry, by machine id. */
  onCheck?: Record<string, PeerState>;
  /** Every answer sent, in order. */
  answers?: { machineId: string; key: string; questionId: string; answers: unknown }[];
  /** Every message sent, in order. */
  sent?: { machineId: string; key: string; text: string }[];
  /** This Mac's chats moving or moved (`chat_move_homes`). */
  homes?: HomeRow[];
  /** Move to runner's next answer, once (else the chat moves). */
  nextMove?: Pick<ChatMoveResult, 'outcome' | 'message'> | null;
  /** A moved chat's title at its new home, by key. */
  titles?: Record<string, string>;
  /** Every chat_move_* call, in order. */
  moves?: { command: string; key: string }[];
  /** This Mac's offline copies of moved chats (`chat_offline_rows`). */
  offline?: OfflineRow[];
}

declare global {
  interface Window { __mockPeers?: MockPeers }
}

const iso = (minutesAgo: number) => new Date(Date.now() - minutesAgo * 60_000).toISOString();
const THIS_MAC: MachineRef = { machineId: 'm-3f9c1a07e2b45d18', slug: 'dev-mac', name: 'Alex’s MacBook Pro' };
const MINI: MachineRef = { machineId: 'm-8d21c4be90f17a3e', slug: 'build-mac', name: 'Mac mini' };

const turn = (ord: number, role: 'user' | 'assistant', text: string, more: Record<string, unknown> = {}) => ({
  id: `t-${ord}`, ord, role, text, agent: 'claude', startedAt: iso(30 - ord), status: role === 'assistant' ? 'done' : null,
  error: null, truncated: false, tools: [], questions: null, attachments: [], ...more,
});
const session = (key: string, title: string, minutesAgo: number, more: Partial<RemoteSession> = {}): RemoteSession => ({
  key, title, agent: 'claude', workspace: null, createdAt: iso(minutesAgo + 30), lastActivityAt: iso(minutesAgo),
  turnCount: 2, state: 'idle', needsAnswer: false, preview: null, runsOn: 'this-mac', movedTo: null, movedFrom: null, ...more,
});

/** A Mac mini online, with three chats. */
export function demoPeers(): MockPeers {
  return {
    thisMac: THIS_MAC,
    suggestedAlias: 'runner-host',
    machines: [{
      ...MINI, sshHosts: ['runner-host', 'runner-host-lan'], remotePort: 47680, pairedAt: iso(60 * 48), lastSeenAt: iso(1),
      lastVersion: '0.5.17 (abc1234)', state: 'online', message: null, checkedAt: iso(1), alias: 'runner-host', moves: true,
    }],
    chats: {
      [MINI.machineId]: [
        {
          session: session('s-mini-build', 'Nightly build on the mini', 1, { state: 'running', preview: 'Running the gate' }),
          permissionMode: 'acceptEdits',
          turns: [turn(1, 'user', 'Run the gate on main and tell me what fails.')],
          live: { key: 's-mini-build', ord: 2, gen: 1, startedAt: iso(1), text: 'Running the gate on main. ', truncated: false, tools: [{ id: 'tool-1', name: 'Bash', status: 'running', detail: null }] },
        },
        {
          session: session('s-mini-card', 'Pick a release name', 6, { needsAnswer: true, preview: 'Which name?' }),
          permissionMode: 'default',
          turns: [
            turn(1, 'user', 'Name the next release.'),
            turn(2, 'assistant', 'Two names fit the theme. Pick one and I will update the notes.', {
              questions: { id: 'q-release', state: 'pending', items: [{
                id: 'name', header: 'Name', prompt: 'Which name should the release use?', mode: 'single', required: true, allowsOther: true,
                options: [{ id: 'aurora', label: 'Aurora', description: 'Light, after the night build', recommended: true }, { id: 'basalt', label: 'Basalt', description: null, recommended: false }],
              }] },
            }),
          ],
          live: null,
        },
        {
          session: session('s-mini-disk', 'Clean up the runner disk', 90, { turnCount: 4 }),
          permissionMode: 'plan',
          turns: [
            turn(1, 'user', 'How full is the runner disk?'),
            turn(2, 'assistant', 'The runner workspaces hold **144 GB**. See [the plan](https://example.com/plan) for what can go.'),
            turn(3, 'user', 'Show it as a canvas.'),
            turn(4, 'assistant', 'Here it is:\n\n```hollow-canvas\n<hc-canvas id="disk" title="Runner disk"><p>144 GB</p></hc-canvas>\n```\n\nThe biggest folder is the cargo target.'),
          ],
          live: null,
        },
      ],
    },
  };
}

function seed(): MockPeers {
  const raw = localStorage.getItem('mockPeers');
  const empty: MockPeers = { thisMac: THIS_MAC, suggestedAlias: 'runner-host', machines: [], chats: {} };
  if (!raw) return empty;
  if (raw === 'demo') return demoPeers();
  const saved = JSON.parse(raw) as Partial<MockPeers> & { preset?: 'demo' };
  return { ...(saved.preset === 'demo' ? demoPeers() : empty), ...saved };
}

/** The live state (a spec may change it). */
export function mockPeerState(): MockPeers {
  return (window.__mockPeers ??= seed());
}

const failure = (code: PeerFailure['code'], message: string, state: PeerState): PeerFailure => ({ code, message, state, attemptsLeft: null });

function machineOf(s: MockPeers, id: unknown): PeerMachine {
  const m = s.machines.find(x => x.machineId === id);
  if (!m) throw failure('local', 'That Mac isn’t paired with this one.', 'offline');
  return m;
}

/** A call to a Mac that doesn't answer fails as the hub's would. */
function reach(m: PeerMachine): void {
  if (m.state === 'online') return;
  if (m.state === 'pairing-needed') throw failure('pairing-needed', `${m.name} no longer knows this Mac. Pair again in Settings.`, 'pairing-needed');
  if (m.state === 'update-needed') throw failure('update-needed', `${m.name} runs another version of the link. Update Hollow there.`, 'update-needed');
  throw failure('offline', m.message ?? `Hollow couldn’t reach ${m.name}.`, m.state);
}

function chatOf(s: MockPeers, m: PeerMachine, key: unknown): MockPeerChat {
  const c = (s.chats[m.machineId] ?? []).find(x => x.session.key === key);
  if (!c) throw failure('not-found', 'That chat isn’t there any more.', 'online');
  return c;
}

const ref = (m: PeerMachine): MachineRef => ({ machineId: m.machineId, slug: m.slug, name: m.name });
/** The backend's pickers (`move_targets`): the Mac the runner alias
 *  reaches, else (Move to runner only) the only one paired; each only when
 *  it takes moves. */
function targets(s: MockPeers): { moveTo: string | null; sleepTo: string | null } {
  const runner = s.machines.find(m => s.suggestedAlias && m.sshHosts.includes(s.suggestedAlias)) ?? null;
  const pick = runner ?? (s.machines.length === 1 ? s.machines[0] : null);
  return { moveTo: pick?.moves ? pick.machineId : null, sleepTo: runner?.moves ? runner.machineId : null };
}
const view = (s: MockPeers): PeerMachinesView => ({ thisMac: s.thisMac, machines: s.machines.map(m => ({ ...m })), suggestedAlias: s.suggestedAlias, ...targets(s) });

function detailOf(m: PeerMachine, c: MockPeerChat, before: number | null, limit: number): PeerChatDetail {
  const all = [...c.turns].sort((a, b) => Number(a.ord) - Number(b.ord));
  const upTo = before == null ? all : all.filter(t => Number(t.ord) < before);
  const turns = upTo.slice(Math.max(0, upTo.length - limit));
  return {
    home: ref(m),
    detail: { session: { ...c.session }, permissionMode: c.permissionMode, turns: structuredClone(turns), hasMore: upTo.length > turns.length, live: c.live ? { ...c.live } : null },
  };
}

/** The hub's `peer-chat-event`s after a change at the chat's home. */
function changed(m: PeerMachine, c: MockPeerChat): void {
  setTimeout(() => {
    emitMockEvent('peer-chat-event', { machineId: m.machineId, key: c.session.key, kind: 'detail', data: detailOf(m, c, null, 20) });
    emitMockEvent('peer-chat-event', { machineId: m.machineId, key: null, kind: 'list', data: page(m) });
  }, 0);
}

function page(m: PeerMachine): PeerSessionsPage {
  const s = mockPeerState();
  return { home: ref(m), sessions: (s.chats[m.machineId] ?? []).map(c => ({ home: ref(m), ...c.session })), truncated: false };
}

const nextOrd = (c: MockPeerChat) => Math.max(0, ...c.turns.map(t => Number(t.ord)), c.live?.ord ?? 0) + 1;
const accepted = (state: string): Accepted => ({ accepted: true, state, queueId: 1, waitingFor: null, forwardedTo: null });

/** `peer_*` for the browser mock. */
export function mockPeers(command: string, args: Record<string, unknown> = {}): unknown {
  const s = mockPeerState();
  switch (command) {
    case 'peer_machines': {
      if (args.check) {
        for (const m of s.machines) {
          m.state = s.onCheck?.[m.machineId] ?? m.state;
          m.checkedAt = new Date().toISOString();
          if (m.state === 'online') m.lastSeenAt = m.checkedAt;
        }
      }
      return view(s);
    }
    case 'peer_pair_cancel': {
      (s.pairCalls ??= []).push('cancel');
      const release = holdRelease;
      holdRelease = null;
      release?.();
      return release !== null;
    }
    case 'peer_pair': {
      const typed = parsePairAddress(String(args.alias ?? ''));
      if (!typed.ok) throw failure('bad-request', typed.message, 'offline');
      const alias = typed.address.host;
      (s.pairCalls ??= []).push(alias);
      if (s.pairHold) {
        s.pairHold = false;
        return new Promise((_, reject) => { holdRelease = () => reject(failure('local', 'Pairing was cancelled. Nothing was paired.', 'offline')); });
      }
      if (s.pairFail) {
        const message = s.pairFail;
        s.pairFail = null;
        throw failure('offline', message, 'offline');
      }
      const now = new Date().toISOString();
      let m = s.machines.find(x => x.sshHosts.includes(alias));
      if (m) Object.assign(m, { state: 'online', message: null, checkedAt: now, lastSeenAt: now, alias, pairedAt: now });
      else {
        m = { ...MINI, sshHosts: [alias], remotePort: 47680, pairedAt: now, lastSeenAt: now, lastVersion: '0.5.17 (abc1234)', state: 'online', message: null, checkedAt: now, alias, moves: true };
        s.machines.push(m);
        s.chats[m.machineId] ??= demoPeers().chats[MINI.machineId];
      }
      setTimeout(() => emitMockEvent('peer-machines-changed', view(s)), 0);
      return { ...m };
    }
    case 'peer_forget': {
      const before = s.machines.length;
      s.machines = s.machines.filter(m => m.machineId !== args.machineId);
      // Chats still moving there come home; the ones that moved stay there.
      s.homes = (s.homes ?? []).filter(h => !(h.peer === args.machineId && h.state === 'leaving'));
      setTimeout(() => emitMockEvent('peer-machines-changed', view(s)), 0);
      return s.machines.length < before;
    }
    case 'peer_reorder_hosts': {
      const m = machineOf(s, args.machineId);
      m.sshHosts = (args.hosts as string[]) ?? m.sshHosts;
      return { ...m };
    }
    case 'peer_start_coo': {
      // The runner COO's first conversation: the chat's key; the status stops
      // saying "not started" once it has one.
      machineOf(s, args.machineId);
      s.cooStarted = [...(s.cooStarted ?? []), String(args.machineId)];
      return 's-mini-coo';
    }
    case 'peer_cancel_connect': {
      // Cancel while it connects: offline with Cancel's sentence until Retry.
      const m = machineOf(s, args.machineId);
      if (m.state === 'connecting') Object.assign(m, { state: 'offline', message: 'Connecting was cancelled. Retry connects again.', checkedAt: new Date().toISOString() });
      setTimeout(() => emitMockEvent('peer-machines-changed', view(s)), 0);
      return { ...m };
    }
    case 'peer_sessions': {
      const m = machineOf(s, args.machineId);
      reach(m);
      return page(m);
    }
    case 'peer_detail': {
      const m = machineOf(s, args.machineId);
      reach(m);
      return detailOf(m, chatOf(s, m, args.key), typeof args.before === 'number' ? args.before : null, typeof args.limit === 'number' ? args.limit : 20);
    }
    case 'peer_watch': {
      const m = machineOf(s, args.machineId);
      // The chat's stream opens, as the hub says once it is up.
      if (args.key && m.state === 'online') setTimeout(() => emitMockEvent('peer-chat-event', { machineId: m.machineId, key: args.key, kind: 'connected', data: { reconnected: false } }), 0);
      return undefined;
    }
    case 'peer_unwatch':
      machineOf(s, args.machineId);
      return undefined;
    case 'peer_send': {
      const m = machineOf(s, args.machineId);
      reach(m);
      const c = chatOf(s, m, args.key);
      if (s.busyKeys?.includes(c.session.key)) {
        s.busyKeys = s.busyKeys.filter(k => k !== c.session.key);
        throw failure('busy', `${m.name} is busy with this chat.`, 'online');
      }
      const text = String(args.text ?? '');
      (s.sent ??= []).push({ machineId: m.machineId, key: c.session.key, text });
      const ord = nextOrd(c);
      c.turns.push(turn(ord, 'user', text, { startedAt: new Date().toISOString() }));
      c.live = { key: c.session.key, ord: ord + 1, gen: 1, startedAt: new Date().toISOString(), text: '', truncated: false, tools: [] };
      c.session = { ...c.session, state: 'running', turnCount: c.session.turnCount + 1, lastActivityAt: new Date().toISOString() };
      changed(m, c);
      return accepted('queued');
    }
    case 'peer_stop': {
      const m = machineOf(s, args.machineId);
      reach(m);
      const c = chatOf(s, m, args.key);
      const was = c.session.state === 'running' || !!c.live;
      if (c.live) c.turns.push(turn(c.live.ord, 'assistant', c.live.text, { status: 'cancelled' }));
      c.live = null;
      c.session = { ...c.session, state: 'idle' };
      changed(m, c);
      return was;
    }
    case 'peer_answer': {
      const m = machineOf(s, args.machineId);
      reach(m);
      const c = chatOf(s, m, args.key);
      const asked = c.turns.find(t => (t.questions as { id?: string } | null)?.id === args.questionId);
      const questions = asked?.questions as { state: string } | undefined;
      if (!questions || questions.state !== 'pending') throw failure('question-closed', 'That card was answered or replaced there.', 'online');
      questions.state = 'closed';
      (s.answers ??= []).push({ machineId: m.machineId, key: c.session.key, questionId: String(args.questionId), answers: args.answers });
      c.turns.push(turn(nextOrd(c), 'user', `Answered: ${Object.values(args.answers as Record<string, unknown>).flat().filter(Boolean).join(', ')}`));
      c.session = { ...c.session, needsAnswer: false, lastActivityAt: new Date().toISOString() };
      changed(m, c);
      return accepted('queued');
    }
    case 'chat_move_homes':
      return (s.homes ?? []).map(h => ({ ...h }));
    case 'chat_offline_rows':
      return (s.offline ?? []).map(r => ({ ...r }));
    case 'chat_continue_offline': {
      // As `chat_offline::continue_offline`: only a chat that moved away; the
      // open copy again while it hasn't synced.
      const key = String(args.key ?? '');
      const home = (s.homes ?? []).find(h => h.key === key);
      if (home?.state !== 'away') throw 'Only a chat that moved to another Mac continues offline; this one runs here.';
      const rows = (s.offline ??= []);
      const open = rows.find(r => r.key === key && r.state === 'open');
      if (open) return open.offlineKey;
      const offlineKey = `${key}-offline-${rows.length + 1}`;
      rows.push({ offlineKey, key, peer: home.peer, state: 'open', note: null, partly: false });
      return offlineKey;
    }
    case 'chat_move_to_runner':
    case 'chat_move_keep_here':
    case 'chat_move_release_here':
      return moveCommand(s, command, String(args.key ?? ''));
    default:
      throw new Error(`mockPeers: no fixture for '${command}'.`);
  }
}

const result = (key: string, outcome: ChatMoveResult['outcome'], message: string, to: string | null = null): ChatMoveResult =>
  ({ key, outcome, to, message, continued: false });

/** `chat_move_*` as `commands/chat_move_api.rs` answers them. */
function moveCommand(s: MockPeers, command: string, key: string): ChatMoveResult {
  (s.moves ??= []).push({ command, key });
  const homes = (s.homes ??= []);
  const home = homes.find(h => h.key === key);
  const now = new Date().toISOString();
  if (command === 'chat_move_to_runner') {
    const id = targets(s).moveTo;
    const m = s.machines.find(x => x.machineId === id);
    if (!m) throw 'No Mac is paired with this one. Pair one in Settings first.';
    const next = s.nextMove;
    s.nextMove = null;
    if (next) {
      if (next.outcome === 'waiting') homes.push({ key, state: 'leaving', peer: m.machineId, at: now, waiting: false });
      return result(key, next.outcome, next.message, m.name);
    }
    if (home) return result(key, 'refused', 'This chat is already moving or moved.', m.name);
    homes.push({ key, state: 'away', peer: m.machineId, at: now, waiting: false });
    (s.chats[m.machineId] ??= []).unshift({
      session: session(key, s.titles?.[key] ?? 'Moved chat', 0, { workspace: { id: 'quick-chats', title: 'Quick chats', kind: 'quick' }, movedFrom: s.thisMac.machineId }),
      permissionMode: 'default', turns: [turn(1, 'user', 'Moved here with its history.')], live: null,
    });
    setTimeout(() => emitMockEvent('peer-chat-event', { machineId: m.machineId, key: null, kind: 'list', data: page(m) }), 0);
    return result(key, 'moved', `Moved to ${m.name}; it keeps going there.`, m.name);
  }
  if (!home || home.state !== 'leaving') throw 'This chat isn’t moving; it is this Mac’s.';
  if (command === 'chat_move_release_here') {
    s.homes = homes.filter(h => h !== home);
    return result(key, 'kept', 'Released: the chat runs on this Mac again.');
  }
  const m = s.machines.find(x => x.machineId === home.peer);
  if (m?.state === 'online') {
    s.homes = homes.filter(h => h !== home);
    return result(key, 'kept', 'Kept on this Mac; it runs here again.');
  }
  home.waiting = true;
  return result(key, 'waiting', 'The other Mac hasn’t answered. The chat runs here once it does, or Release here if it never comes back.');
}
