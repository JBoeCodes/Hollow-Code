import { describe, expect, it } from 'vitest';
import type { ChatEventEnvelope } from './generated/chatEvent';

/**
 * AG2: `chat-event` payloads are typed by `src/generated/chatEvent.ts`,
 * generated from `src-tauri/src/chat_event.rs`. Rust writes one golden
 * fixture per variant (`tests/fixtures/chat-events/`, checked by
 * `cargo test --lib chat_event`); here each one is read through the
 * generated type. `TYPES` must name every variant exactly: a variant added
 * or removed in Rust is a type error until this list (and the UI) follow.
 */
const FIXTURES = import.meta.glob('../tests/fixtures/chat-events/*.json', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

type EventType = ChatEventEnvelope['type'];
const TYPES = {
  session: true, runStarted: true, assistantText: true, assistantDelta: true, turnBoundary: true,
  toolUse: true, toolResult: true, toolImages: true, evidenceReceipt: true, result: true, usage: true,
  activity: true, budgetWarning: true, loginLow: true, panel: true, interaction: true,
  cancelled: true, paused: true, sessionReset: true, accountSwitched: true, remoteRun: true, error: true,
  done: true, steer: true, heldSent: true, agentContinued: true, heldFailed: true,
} as const satisfies Record<EventType, true>;
type Listed = keyof typeof TYPES;
/** Fails to compile when the generated union has a type `TYPES` lacks. */
const everyTypeListed: [Exclude<EventType, Listed>] extends [never] ? true : never = true;

function fixture(type: EventType): ChatEventEnvelope {
  const text = FIXTURES[`../tests/fixtures/chat-events/${type}.json`];
  if (text === undefined) throw new Error(`missing tests/fixtures/chat-events/${type}.json`);
  return JSON.parse(text) as ChatEventEnvelope;
}

describe('chat-event fixtures (AG2)', () => {
  it('has exactly one fixture per generated variant', () => {
    expect(everyTypeListed).toBe(true);
    const files = Object.keys(FIXTURES).map(path => path.split('/').pop()!.replace(/\.json$/, '')).sort();
    expect(files).toEqual(Object.keys(TYPES).sort());
  });

  it('reads each fixture as the variant its file names', () => {
    for (const type of Object.keys(TYPES) as EventType[]) {
      const event = fixture(type);
      expect(event.type).toBe(type);
      expect(event.clientTurnId).toBe('ct-1');
      expect(event.queueId).toBe(42);
    }
  });

  it('narrows on type to the variant fields', () => {
    const result = fixture('toolResult');
    if (result.type !== 'toolResult') throw new Error('narrowing');
    expect([result.id, result.content, result.isError]).toEqual(['toolu_1', 'ok', false]);
    const started = fixture('runStarted');
    if (started.type !== 'runStarted') throw new Error('narrowing');
    expect(started.runId).toBe('run-1');
    expect(started.recall[0]?.path).toBe('notes/a.md');
    const remote = fixture('remoteRun');
    if (remote.type !== 'remoteRun') throw new Error('narrowing');
    expect(remote.phase).toBe('start');
  });
});
