# Shared Codex process owner (SE9)

`shared_codex.rs` supervises an initialized app-server per compatible binary
and account environment. `AgentCapabilities.sharedServer` selects this adapter
seam. It currently applies to local GUI-owned interactive Codex attempts with
only HTTP MCP servers. Remote runs, stdio connectors and unknown CLI options
keep their existing per-turn process before any dispatch.

The app-server process has no per-turn environment or MCP grants. Each thread
gets its own sandbox, approval policy, cwd, model, effort, provider overrides,
MCP config and direct HTTP headers. Shell environment carries that turn's
random run tag. Credentials stay out of shared argv, logs and pool keys that
are printed. The pool key hashes the effective account environment, so changed
account settings cannot reuse an incompatible process.

The existing `AppSession` and chat driver remain per attempt. A lightweight
`codex-relay` child inherits a connected fd from the GUI and forwards protocol
lines. There is no discoverable listener. The relay preserves the existing
History identity, steering input, grant Binding and process-group cleanup.
The actual Codex process is shared; the relay itself is per attempt. This
retains a small process cost. Memory and latency savings are unmeasured.

The owner initializes once, remaps every request to a global id, and maps
responses back to the originating local id. Notifications and server requests
route to exactly one active thread. Unknown global requests retire the
generation. Only the account-wide quota notification fans out to that same
account's clients; other global notifications are ignored.
Thread/turn ids on steering and interrupt are checked against that client.
Only one turn may start per relay.

EOF or SIGTERM requests an interrupt of that turn. Missing completion after
one second retires the shared process; a crash, stuck RPC, invalid route or
bounded output overflow also fails the generation. No partially executed
turn is automatically replayed. The chat driver captures an unfinished shared
attempt before its early-exit handler ends the session and excludes both
resume repair and provider backoff, including earlier network-error wording.
Normal terminal provider failures keep their existing retry rules.
Graceful client closure drains queued replies for at most one second.
Normal turns unsubscribe their thread and
an empty process retires after 60 seconds. Pending RPCs have 30 seconds;
protocol lines retain the driver's existing 16 MiB cap. Queued input events
and output frames share a 64 MiB per-account wire-byte budget, held until
delivery or discard; each output queue also caps at 64 lines. Exhaustion
closes the affected input or retires the generation before unbounded buffering.

The installed CLI's generated schema includes per-thread `config` on start
and resume, and `thread/unsubscribe`. Synthetic child-process tests exercise
the routing and lifetime contract. Real-provider and installed-app checks
remain outside this lane; no release build, restart or install is authorized.
