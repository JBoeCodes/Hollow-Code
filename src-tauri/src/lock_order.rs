//! THE lock order for the whole crate, in one place (merged plan slice 4,
//! `lifecycle::PENDING_QUIT` is also a leaf. Exit queue/registry reads run
//! off the event-loop thread; release the request lock before window calls.
//! review finding A·D7). Code lives elsewhere; this module only documents.
//! Every PR that adds a lock, or takes two locks at once that were never
//! nested before, updates this list and the "Lock order" section of
//! docs/reference/architecture.md in the same change.
//!
//! **The rule.** While holding a lock you may take a lock only from a LATER
//! tier (or a later sub-tier: 4a before 4b, 5a before 5b before 5c), never
//! one from the same or an earlier one; two locks in the same tier or
//! sub-tier are never held together. Every std `Mutex` here is
//! non-reentrant: a function that takes a lock must never call a helper that
//! takes it again (that is why `vault` has `_locked` writer variants that
//! take a [`MutateGuard`]).
//!
//! **Tier 1 is the one exception.** Its locks are never waited on without a
//! bound: the lease is a try-lock that refuses when busy, the run queue
//! owner flock is tried once and a process that can't get it just doesn't
//! own the queue, and a run permit polls its slots within a bounded wait and
//! then refuses. So the ordering rule does not apply to them. They may be
//! held together (a run holds its permit and then tries the lease; the run
//! queue owner flock is held for the life of the process, so it is held
//! together with every other lock), and a tier 2 sync holder may take the
//! background write lease. Never turn one of them into an unbounded wait:
//! that is what would make this exception a deadlock.
//!
//! 1. **Admission (cross-process, refused rather than waited on).** The
//!    full-vault run lease (`automated_history`: the O_EXCL
//!    `.hollow-history/active-run.nosync` file plus the `Leases/{vault}.lock` flock
//!    from `lease_lock`), the run queue owner flock (`run_pump`, held for the
//!    life of the process) and the run permits (`run_permit`, flock slots
//!    polled within a bounded wait; a permit holder never waits on the lease
//!    or the run queue). Taking the lease takes `HISTORY_LOCK` briefly
//!    inside it; it is never taken while holding a lock from tiers 3-7.
//! 2. **Sync.** `tracker_sync::root_lock(root)` then `.tracker-sync.lock`.
//!    Tracker writes then take the History lease before the vault CAS writer. Never
//!    hold a vault writer while requesting the sync lock. A sync may take
//!    the background write lease (tier 1's lease file, through
//!    `automated_history::background_write_lease`; a try-lock, so allowed
//!    by the tier 1 exception), then the vault writers (tier 5). Notion
//!    two-way sync and its locks were removed on 2026-11-10.
//!    Two domain locks also sit here, before `HISTORY_LOCK` and never under
//!    it (COO parity L13 review): `directory_connections::UPDATE` (a Projects
//!    rescan can hold it for minutes; the COO's
//!    `workspaces.open_project_workspace` takes it, then writes a first
//!    open's workspace files through `with_live_run`), and
//!    `companion_inbox::PASS` (an inbox pass holds it while an Apply takes
//!    History; `companion.revoke_device` takes it first, then its History
//!    write, with `companion_inbox::DEVICES` a leaf inside).
//! 3. **`automated_history::HISTORY_LOCK`.** `with_live_run` runs its
//!    closure while holding it; the closure may take `CREATE_LOCK` and
//!    tiers 5-7, but never `HISTORY_LOCK` or the permission write guard.
//!    Since SE5 (2026-11-17) it is a writer-preferring shared/exclusive
//!    lock: a scoped run's admission (`begin_scoped`: register, walk the
//!    folder, write the record) holds it shared, so chats starting together
//!    walk at once; every other holder takes it exclusively and still sees
//!    each admission whole. A shared holder never asks for it exclusively.
//!    The stat cache's load and save take `STAT_CACHE_IO`, with its save
//!    counter `STAT_CACHE_SAVES` nested inside as a leaf.
//!    Right under it, History's object store flock
//!    (`Leases/{vault}.objects.lock`, Q-R9 2026-11-13): a write that stores
//!    objects before its record names them holds it shared
//!    (`automated_history::objects_in_flight`, waiting at most 30 s, or
//!    250 ms at a run's admission, then refusing: a write never goes ahead
//!    without it), across a live write's closure too; object cleanup holds
//!    it exclusively (a try-lock for at most 250 ms, else it skips the
//!    round; retention deletes only in the first 20 s of its hold) and is
//!    never run on a thread that holds it shared.
//! 4. **Domain gates.**
//!    - 4a. `workflow_permissions::write_guard`.
//!    - 4b. `vault_goals::GOALS_LOCK` or `vault_tasks::CREATE_LOCK`, never
//!      both. `CREATE_LOCK` is never taken while holding the vault mutate
//!      lock or from inside a `mutate_note` closure.
//! 5. **The vault mutate lock and its siblings.**
//!    - 5a. The vault mutate lock ([`crate::vault::lock_mutate`]: the
//!      in-process `vault::MUTATE_LOCK`, then a cross-process flock on
//!      `Leases/vault-mutate.lock`, released in the reverse order). It is
//!      held by the `vault` writers (`write_note`, the blind overwrite,
//!      `write_note_cas`, `write_note_expecting`, `mutate_note`,
//!      `move_note`) and by the task, commitment, workflow-rename and
//!      workspace-attach mutators and the Plan row toggle (`plan_rows`,
//!      which calls `write_note_expecting_locked` under it). Not every
//!      vault writer takes it:
//!      canvases and reviews, the agenda, recordings and
//!      the History restore write with `vault::atomic_write` under their own
//!      lock or CAS, so a blind `write_note` does not serialize with them.
//!      `create_note` and `delete_note` do not take it (`create_note`
//!      publishes with an exclusive hard link that never replaces anything),
//!      so a caller already holding it may call them.
//!    - 5b. `budgets::BUDGETS_LOCK`, or the stamp store's `STORE_LOCK`.
//!    - 5c. The review queue's `QUEUE_LOCK`.
//!
//!    The mutate lock is never nested with 5b or 5c in practice (release it
//!    before taking them); the sub-tiers order the one nesting that does
//!    happen, budgets' `BUDGETS_LOCK` → `QUEUE_LOCK`, which runs after the
//!    mutate lock is released.
//! 6. **Derived-index leaves.** The vault link `INDEX`, the walk cache,
//!    `vault_search_index` and `vault_embed_index`, taken inside the vault
//!    writers. They never take another lock.
//! 7. **`run_guard::TRACKED`** (and its `OWNED_QUIET` map): always
//!    innermost among the vault locks.
//!
//! **The session store chain is separate.** `chat_rows::CHAT_ROW_ACTIVE` →
//! `session_store::WRITE_LOCK`, and `run_queue::WAITERS` → `WRITE_LOCK`;
//! `WRITE_LOCK` is a leaf, never held while taking the lease,
//! `HISTORY_LOCK`, the permission write guard or the review queue lock. The
//! usage ledger and interactions take only `WRITE_LOCK`. Across processes
//! the store relies on SQLite IMMEDIATE transactions and a 2 s busy timeout.
//! Every other module-level `Mutex` (caches, notice queues, settings files)
//! is a leaf taken briefly and never held while calling out.
//!
//! **What reaches other processes.** Only the flocks (lease lock, History's
//! object store lock, run queue owner, run permits, `.tracker-sync.lock`,
//! the vault mutate lock) and the
//! O_EXCL lease file; SQLite has its own locking. Everything else above is
//! in-process.
//! Writers that ignore Hollow's locks (Obsidian, an agent's own file tools)
//! are only caught by compare-and-swap and the no-replace renames, never by
//! a lock.
//!
//! **The mutate flock waits a bounded time.** If another process holds it
//! for longer than `vault::MUTATE_PROCESS_WAIT` (a hung writer), Hollow logs
//! it and carries on under the in-process lock alone rather than freeze the
//! UI; a stuck flock can therefore never deadlock two processes.
//!
//! [`MutateGuard`]: crate::vault::MutateGuard
