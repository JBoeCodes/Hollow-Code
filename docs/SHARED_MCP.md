# Shared MCP transport (SE8, in progress)

The transport in `shared_mcp.rs` is wired through `shared_mcp_host.rs`.
The configured core starts one loopback listener. Local Claude/Codex commands
convert only selected supported entries before spawn. Missing host/context,
remote agents and unsupported entries keep their existing stdio path.
Unstacked `fd363faa7` passed fmt, six transport/host tests and Clippy on mini.
Final narrow security review is clear on unchanged production code. SE6
landing, fresh main integration and SE8's full exact-tree gate remain.

The registry mints distinct 256-bit capabilities for each attempt/server,
pending until process adoption. Its owner revokes on Drop. Every HTTP message
checks active state, revocation, expiry, exact route and the host's live-run
predicate, including initialize, list, read and notifications. Tokens never
belong in URLs or logs. A literal loopback listener validates Host/Origin,
bounds headers, bodies, connections and handler time, and accepts single JSON
messages with JSON replies. Notifications use 202. Server shutdown revokes
all capabilities. Blocking adapters must check Permit after waits and before
effects; dropping a future alone does not cancel blocking work.
Each request has its own deadline and cancellation latch. A request guard
invalidates all worker Permit clones when the HTTP future ends or is dropped;
execution capacity remains held until every worker exits. Connections have a
500-second total lifetime, bounding stalled response writes past the longest
480-second tool wait. Clients reconnect with the same live capability.

Computer/team context comes from immutable selected server environments.
Internal app/task/services adapters retain policy, audit and History, with
request validity checked after waits and before effects. Pending grants are
owned by the command itself; Binding activates after adoption and revokes at
the start of cleanup. Isolated OAuth bridge state uses URL-bound refresh.
Mail connector entries retain their existing approval-gated stdio adapter.
Codex HTTP entries retain the same tool timeouts and preapprovals.
Computer requests allow 180 seconds so the 120-second approval card retains
its existing window. App actions carry request scope into nested brokers and
History writes; checks inside History inspect cancellation/expiry only, since
the live-run lock already owns run validity. A second live lookup would deadlock.
Blocking actions explicitly capture/restore that scope through
`shared_mcp::blocking`, including logged vault workers and meeting recap.
Tokio task-local state alone does not cross `spawn_blocking`.
Explicit command metadata retains bound-run ids and private config tags;
HTTP entries contain only HTTP fields, with no stdio environment.
Never impersonate a
Unix peer or temporarily alter the process environment. Never retry an uncertain
HTTP mutation over stdio. Owned child groups must replace the team subprocess's
inherited-group assumption inside the host. Preserve service lifetimes.

Bearer authentication has a different same-user replay boundary from Unix
process ancestry. Live-run checks and revocation limit authority lifetime;
they do not establish the presenting process's ancestry.

Synthetic tests cover pending activation, server/attempt isolation, revocation,
live-run checks for all methods, Host/Origin, body bounds and shutdown. Focused
mini fmt, all four transport tests and Clippy passed on `80dda315a`.
Independent full/repair reviews found and repaired request-authority and
connection-lifetime defects; the final narrow delta is clear of P1/P2.
These earlier checks cover the transport foundation. Production adapters,
attempt lifecycle and local Claude/Codex attachment are now integrated as
described above. Fresh current-main integration and a final full gate remain
required before landing.
