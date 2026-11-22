# Chat stream grammar

The rules for one turn's `chat-event` stream (the payloads `chat::emit_event`
sends and `ChatEventPayload` in `src/store/chatRuntime.ts` types). Checker:
`src-tauri/src/chat_grammar.rs` (`check_jsonl`, pure; test-only until a runtime
caller exists). Corpus: `tests/fixtures/chat-streams/` (`manifest.json` lists
each stream's adapter, expected pass or fail, rule and 1-based line). Test:
`cargo test --lib chat_grammar`. Hollow's own rules (AG1); the AG-UI rule list
only inspired the idea, no code or fixtures were copied.

Hollow has no message end event. An **open text segment** runs from a
`turnBoundary` (or the turn's start) to the next `turnBoundary`. An **open
tool** is a `toolUse` with a non-empty id that has no `toolResult` yet. A
toolUse with no id (Grok, Antigravity) cannot be matched and is exempt from
G05 and G06.

A stream is one turn: `runStarted` ... `done`. "Ending events" are `error`,
`cancelled` and `paused`; "content events" are `assistantText`,
`assistantDelta`, `turnBoundary`, `toolUse`, `toolResult`, `toolImages`.

| Id | Rule (MUST) |
|---|---|
| G01 | Every line is a JSON object with a string `type` that `ChatEventPayload` knows. |
| G02 | The first event is `runStarted`, and there is exactly one. The exception is a turn that never started (taken out of the queue, or recovered): it is `cancelled` or `error` first, then only `cancelled`/`error`/`done`. |
| G03 | `done` is terminal: nothing follows it, and a stream ends with one. |
| G04 | No content event follows an ending event (`error`, `cancelled`, `paused`). Only closing events (`usage`, `evidenceReceipt`, `done`, more notices) may. |
| G05 | A `toolUse` id is used once. Every open tool closes (gets its `toolResult`) before `done`, unless the turn was stopped, paused, failed or came back as an error `result`. |
| G06 | A `toolResult` or `toolImages` id matches an earlier `toolUse` id; a `toolResult` comes once per id; `toolImages` follows its `toolResult`. |
| G07 | `assistantText` (a whole block, used when no deltas streamed it) MUST NOT appear in a segment that already streamed `assistantDelta`: the text would draw twice. |
| G08 | Required fields: `session.sessionId`; `assistantText`/`assistantDelta` `text`; `toolUse.name`; `toolResult` `id`, `content`, `isError`; `result.isError`. |
| G09 | One conversation per stream (`conversationKey`, when present, never changes); `activity.runId` is the run's `runStarted.runId`. |
| G10 | `usage` comes at most once and closes the turn: no content event follows it (a `remoteRun` notice, `evidenceReceipt` or an ending event may, before `done`). |

The ordering rules (G04, G05, G07, G10) are read from `chat.rs` and the
adapters, and the fixtures are synthetic streams shaped like each adapter's
events (Claude, Codex, Antigravity, Grok), not recordings of live runs. When a
real recorded stream breaks a rule, decide whether the code or the rule is
wrong, fix one, and add the stream to the corpus.

## The typed schema (AG2)

The payloads themselves are typed by `src-tauri/src/chat_event.rs`: a
`ChatEventEnvelope` (the queued turn's `clientTurnId`, `queueId`,
`conversationKey`) around a `ChatEvent` variant per type. `ts_bindings.rs`
generates `src/generated/chatEvent.ts` from it, and `ChatEventPayload` in
`chatRuntime.ts` is that generated type. G01's known types are the enum's
(`chat_event::TYPES`). In test builds `chat::emit_event` checks every payload
round-trips through the envelope (`chat_event::assert_conforms`; an explicit
`null` reads as absent), so a builder that writes a field the schema lacks
fails the test that emitted it. Each variant has a golden fixture in
`tests/fixtures/chat-events/` (read by `src/chatEventFixtures.test.ts`), and the
corpus's passing streams conform too. Adding a field: add it to the variant's
struct, then from `src-tauri/` run `HOLLOW_REGENERATE_IPC=1 cargo test --lib
chat_event ts_bindings`. `chat.rs` builds its events through `chat_event::build`
(typed field structs, so a field the schema lacks does not compile;
`builders_write_the_old_bytes` pins the bytes); so do the queue, interaction, steer and held-send sites; only `remoteRun` (a `Value`) stays `json!`.

## AG-UI translation (AG9)

`src-tauri/src/ag_ui.rs` turns one turn's chat events into AG-UI events
(`translate(thread_id, events)`, or `Translator::push` per event), written from
AG-UI's published event list; no AG-UI code was copied. Pure and test-only:
there is no HTTP route yet. The row's second step, a `POST` returning SSE
behind the phone API's bearer token and tailnet check, waits until a client
needs it.

| Hollow event | AG-UI |
|---|---|
| `runStarted` | `RUN_STARTED {threadId, runId}`; a turn that never started gets one with `runId: "unstarted-<clientTurnId>"` |
| `assistantDelta` | `TEXT_MESSAGE_START` (role `assistant`, id `<runId>-m<n>`) once, then `TEXT_MESSAGE_CONTENT` |
| `assistantText` | a message of its own: start, content, end |
| `turnBoundary` | `TEXT_MESSAGE_END` of the open message |
| `toolUse` | `TOOL_CALL_START`, `TOOL_CALL_ARGS` (the input as JSON text), `TOOL_CALL_END`; an id-less call gets `<runId>-t<n>`, a repeated id is dropped |
| `toolResult` | `TOOL_CALL_RESULT` (role `tool`) for a started call, once |
| `toolImages` | `CUSTOM hollow.toolImages {toolCallId, count}`: the images stay on the Mac |
| `interaction` | `CUSTOM hollow.interaction {id, mode, status}` |
| `result` | its text becomes `RUN_FINISHED.result`; an error result is `RUN_ERROR` (code `agent_error`, fixed message) |
| `error`, `cancelled`, `paused` | `RUN_ERROR` with code `error`, `cancelled` or `paused` and a fixed message: Hollow's own text (budgets, limits, paths) never crosses |
| `done` | `RUN_FINISHED`; nothing goes out after a terminal event |

Everything else (`session`, `usage`, `evidenceReceipt`, `activity`, budgets,
logins, resets, account switches, the runner, steering, held sends) is
Hollow's own state and never crosses. A stream with no `done` is closed with
`RUN_FINISHED` by `Translator::finish`. Test: `cargo test --lib ag_ui` runs
every corpus stream, failing ones included, through an AG-UI sequence check
(one `RUN_STARTED` first, one terminal last, messages and tool calls opened and
closed in order, a result after its call's end, required fields present).

