# Agents, accounts and the run queue

Part of the agent manual; [AGENTS.md](../../AGENTS.md) is the entry point. Agent execution: [Agents](../reference/agents.md#chat-how-the-headless-agent-integration-works). What context, tools and delegation reach an agent from each entry point: [Agent context](../AGENT_CONTEXT.md).

## Accounts, quota and models

- Claude account rotation is automatic: rank by binding-window headroom, read admission fresh, keep durable quota-rejection overrides and exclude accounts already tried. A missing eligible account or unreadable store never falls back to login. Actual selection evidence goes in the receipt's `accountChoices`. See [quota recovery](../QUOTA_FAILOVER.md).
- Terminal quota recovery tries each eligible added account once per turn, keeping current-attempt native session proof for continuation, organization policy and stop/outage guards.
- Quota rejection is exhaustion evidence; Claude model capacity is an outage. Background-job notices never authorize switching accounts or interrupt work.
- Agent and model selection applies app-wide; conversation history and shared skills are provider-neutral.
- Model catalog requests carry per-agent tickets, and rejection cleanup compares promise identity so an older failure cannot evict a newer Refresh. Codex transport uses the recorded 0.163.0 floor and one stop-aware per-turn snapshot for transport, admission and UI.

## Shared Codex and MCP hosts

- `shared_codex.rs` owns one Codex app-server per account environment, with global RPC routing and immutable thread config. Local HTTP-only chat attempts use an inherited-fd `codex-relay` child so each turn keeps its own process group, History and steering lifecycle. Stdio MCP, unknown options and remote runs keep a private process. The shared process receives no per-turn grant or shell tag; threads unsubscribe after finish and an empty server retires after 60 seconds. Protocol lines keep the 16 MiB driver limit; queued wire bytes have a 64 MiB per-account budget. See [shared Codex](../SHARED_CODEX.md).
- `shared_mcp_host.rs` captures each selected server's immutable context, binds pending grants to the command/binding lifetime, and revokes before group cleanup. Missing host/context and remote agents keep stdio. OAuth state is isolated per attempt and connector. HTTP team groups are GUI-owned; stdio delegates inherit their parent group. An app action's async request fence survives into nested task/services and History writes; check cancellation after taking locks, without reentering History. See [shared MCP](../SHARED_MCP.md).

## Admission, memory headroom and the run queue

- Admission is by physical headroom: free plus inactive memory must cover a 2 GB floor, live-turn reservations and the next adapter estimate (Claude/unknown 0.25 GB, Codex 0.18 GB). The global ceiling is 60; unknown samples use the saved overall limit. Workspace/subagent limits and the toolchain guard are separate. See [automation](../reference/automation.md).
- Under memory pressure the first three queued turns may start, subject to the other rules; others recheck each second. Every held row records `holdKind` and `holdSince` in its phase timings (`run_queue::record_hold`, kinds from `Hold::kind`); `mark_running` writes `holdMs` and logs a wait over 2 s.
- The queue caps idle and scheduled sleeps at five seconds while any row is queued; turn ends and chat-limit saves wake it.
- `work_origin.rs` keeps host request, occurrence, queue and run provenance in the operational store. A person's workflow command creates a distinct request; verified scheduling creates autonomous occurrences. Broker calls and retries inherit only exact retained provenance; manual source, priority, payload fields and standing approval never mint direct origin. Legacy absence stays Unknown. Only an owned or compatibly absorbed Cron occurrence advances its saved schedule, compared exactly inside the workflow mutation; retrying an old occurrence never consumes a newer one, and interruption does not consume one.
- Queue ordering promotes only workflows with validated direct request lineage into the Run now band (FIFO ties). Missing or damaged origin never promotes work or blocks chat. Rank grants no budget or preemption authority.

## Turn preparation and retrieval

- A steerable Claude turn's agent starts once connector selection and the repository snapshot settle, before retrieval ends (`chat::EarlyStart`): its spec is the turn's `template` less the prompt, which goes to stdin. Anything the command is built from must be known before retrieval is awaited. See [AGENT_CONTEXT](../AGENT_CONTEXT.md#starting-the-agent-beside-preparation-run-15-s2).
- Identical in-flight Jev ranking bodies share one request; only exact scores are reused, and sources, permissions and attachments are rebuilt by each caller ([Jev](../JEV.md)). Settings changes invalidate ranking targets, flight generations and late cache writes.
- Pinned rules: budget pressure cannot withdraw a pinned rule already given; only actual unpinning can. Row-specific Stop cancels preparation, and preparation rereads Queued state after reservation so a late Stop is never missed.
- Retrieval scores lines through one `vault_search::LineScorer` per request; its answer must equal `best_lines_by_line`'s (`line_scorer_matches_the_line_by_line_reference`). Time preparation with the ignored `turn_prep_probe`. See [Performance profile](../reference/architecture.md#performance-profile).

## Run records and usage

- Chat, Activity and Automations share the Run record head and disclosure; only automation history uses compact receipts. Copy commands take the exact sanitized record, truncation marker included. See [Run records](../reference/runs.md#shared-run-record-p1-29).
- Codex turn snapshots stay partial until a final usage report arrives; never count a whole-turn snapshot as context, model calls or subagent activity. See [Usage ledger](../reference/runs.md#usage-ledger-paperclip-pr-6) and [Usage instruments](../USAGE_INSTRUMENTS.md).
- Diagnostics mask credential-bearing launch arguments before tallying.

## This Mac note

`this-mac.md` is generated by `this_mac.rs` beside the build-slot tool: detected cores and memory, slot and target folders, the runner (`remote_session::runner_setting`) and the caps from `build_slots::CAPS` and `SLOTS`. A turn gets `HOLLOW_THIS_MAC` only while the note exists; a failed generation removes it. Change the source, never the note.
