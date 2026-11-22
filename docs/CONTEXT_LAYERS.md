# Context layers

The entry doc for how Hollow keeps an agent's context in four layers today.
Each section links the detailed contract; this page only says what is where
and what is built. Plan: the Hollow vault workspace `hollow`,
`outputs/context-layers-2026-11-13/plan.md`. Update this page in the same
change as any layer, cap or guard it names.

## The four-question test

Every fact belongs to exactly one layer. Ask in order; the first yes wins, and
a fact that fits two goes to the narrowest true one
(`vault_learning::LAYER_TEST`, the text the learning extractor carries):

1. Is it false on a teammate's computer (this Mac, its tools, paths, install
   facts)? **Home.**
2. Is it about how one person works, never a teammate? **Personal.**
3. Is it true for everyone on one project? **Project**, never org.
4. Is it true for everyone in your organization whatever the project (people, systems,
   vendors, business rules, brand)? **Organization**, and only through a
   reviewed pull request.

## Where each layer lives

| Layer | Where | Who writes |
|---|---|---|
| Home | `home/<slug>.md` (and `home/<slug>/<topic>.md`) in the personal repo; the slug is this machine's, from `~/.config/hollow/machine.json` | Hollow and agents, this machine's file only |
| Personal | `me/`, `memory/`, `learnings/` in the personal repo (`example-user/personal-context`) | The person; agents write `memory/` and `learnings/`, `me/` waits for Save |
| Organization | `example-org/org-context`, a read-only clone (default `~/Hollow/Context/org`) | Nobody in place: changes go by pull request through `context.propose_org_change` |
| Project | the project's own repository (`CLAUDE.md`, `AGENTS.md`, its docs) | The project, as before; no machine or person facts |

The two clone folders and the machine slug are set in Settings → Sources →
Context ([Settings, Context](DIRECTORIES.md#settings-context-cx17-2026-11-15));
while they are unset Hollow behaves as it did before the layers. Path rules
and the slug: [Context roots](DIRECTORIES.md#context-roots-cx3-2026-11-13)
(`context_roots.rs`).

The vault is not a layer: it holds workspaces, outputs and the library, and
stays its own repository ([vault reference](reference/vault.md#context-layers-and-the-vault)).

## What each chat gets

A fresh native session's `<operator-context>` block (layer 12 in
[AGENT_CONTEXT.md](AGENT_CONTEXT.md#ui-chats-what-the-agent-reads-in-order))
carries five parts, built by `context_compiler::Profile::read_within` and
`context_layers.rs`:

- the `me/` digest (personal), as before;
- `## home/<slug>.md`: a lead line naming the machine and its topic folder,
  then the home file, cut to whole lines within 4,000 bytes (`HOME_CAP`),
  while the personal root and the slug are set;
- `## personal/pinned.md` (PX3): memories with `pinned: true`, whole, in file-name
  order within 1,500 bytes (`PINNED_CAP`); one that would pass the budget is not
  loaded and is named with its size in the closing line. Shown ahead of the index.
  Resume deltas retain the complete eligible pin source separately: budget
  displacement does not revoke an unchanged rule already given to a session.
  Actual unpinning still removes that rule from the resumed context.
  Also once per launch window, a minute or two in, Hollow files a Waiting on you
  card for each of three safety memories (`context_pins::SAFETY_RULES`: no
  outbound message without approval, no secret slices, pause at safe points) that
  would add its one-line description to `me/Guardrails.md` (decision D3: a card,
  never silent; editable before Save; Reject keeps the memory). A rule already in
  Guardrails, or offered once before (`context-promotions.json` in app data), is
  not offered. Refusing a pin when the memory is written is not built; the
  budget is enforced on read. Only the flat `memory/<name>.md` files are read
  (Claude's per-project folders are not). A pin is a frontmatter key, so whoever
  can write a memory file can pin it, as with the memory itself; the budget caps
  what a pin puts in every prompt, a pinned body can't open a heading of its
  own (a line starting `#` is escaped), and a file name with a control
  character is skipped;
- `## personal/INDEX.md` (PX2): a lead line, then one line per Markdown file of
  the personal clone's `memory/` and `learnings/` (top level, no `README.md`),
  `- <name>: <description>` (a learning's `title`; a file with none is listed by
  name), grouped by the memory's `type` (Feedback, Project, Reference, About
  you, Other, then Learnings), each line cut to 110 characters, within 2,000
  bytes (`INDEX_CAP`), while the personal root is set and holds such a file.
  Generated on each read and never written, so a new memory file appears after
  the next sync and the index can't drift from the files. Only the flat files
  are read (Claude's own per-project folders under `memory/<project>/` are not),
  a learning another replaced (`superseded_by`, `status: superseded`) is left
  out, and a file whose name holds a control character is skipped. Nothing is cut silently:
  past the cap the last line says how many files are not listed and that
  merging or retiring memories makes them fit. **Not built:** a Waiting on
  you card proposing that consolidation. The learning-proposal card only
  writes an accepted line into a `me/` file, so an overflow card needs its
  own kind;
- `## org/INDEX.md`: a lead line naming the read-only clone (read with Read,
  Grep or Glob, never write), then the clone's `INDEX.md`, or a generated
  list of its top-level entries, within 2,000 bytes (`ORG_CAP`), while the
  org root is set.

All five ride the profile: sent once per native session while About you is
on, a changed line reaches a resumed session as `<operator-update>`, and the
turn budget never cuts the two layer sections. The rest of the org clone is
on demand: its Markdown and text files are retrieval kind `org`
(`context_layers::org_passages`), cited by absolute path. Claude turns also get
`autoMemoryDirectory` in the personal clone (CX4b). Details:
[Context layers, read side](DIRECTORIES.md#context-layers-read-side-cx4-cx5-2026-11-14).

## Write guards

- **Per layer** (`memory_guard::check_layer`, CX7): Home writes only this
  machine's files; Personal writes `memory/` and `learnings/` and holds `me/`
  for Save (Guardrails, Decisions and standing orders always ask); Org refuses
  every direct write and points to `context.propose_org_change`; Project
  refuses a line naming a computer or a person
  ([Per-layer write rules](reference/vault.md#alignment-trusted-learning-evidence-and-recovery)).
- **The org write block** (CX5b): Claude's `--disallowed-tools` refuses Edit
  and Bash text naming the clone; Codex auto names it read only. Its limits
  (Codex full access, Grok and Antigravity get no rule; Bash rules match
  command text only) are in `context_layers::ORG_BLOCK_LIMITS` and the
  per-agent table under
  [Context layers, read side](DIRECTORIES.md#context-layers-read-side-cx4-cx5-2026-11-14).
- **Org changes** (`context_propose.rs`, CX8): `context.propose_org_change`
  scans the files, writes them in a throwaway clone, pushes a branch and opens
  a pull request with `gh`. The owner's PR from an attended chat merges with
  `context.merge_org_change` once checks pass; a teammate's waits for review.
  Owner PRs that merged are listed in the nightly digest
  ([Org changes digest](DIRECTORIES.md#org-changes-digest-cx8b-2026-11-15),
  `context_org_digest.rs`).
- **Chat facts** (CX9, `memory_homes.rs`): the extractor sends a machine fact
  the person stated to `home/<slug>.md` and a Acme fact to a pending org
  proposal, never written to the clone.
- **Leak checks** (`leak_checks.rs`, CX10): flag-only for a week. A Project
  write naming a computer or person is written and logged to
  `context-leak-flags.jsonl`; a secret is refused in either mode
  ([Leak checks](DIRECTORIES.md#leak-checks-cx10-2026-11-15)). Every check
  uses the shared rules (`scan_rules.rs`, `vendor/ctx-scan/scan-rules.json`).

## Sync

- **Personal** (`sync_engine.rs`, CX6a): the vault's `me/`, `memory/`,
  `learnings/` and `home/` sync to the personal remote, git directory in app
  data, a merge never a rebase, clashes kept beside the file
  (`sync_clash.rs`), mass removals held
  ([Context sync engine](DIRECTORIES.md#context-sync-engine-cx6a-2026-11-14)).
- **Organization:** `git pull --ff-only` in the clone 30 s after launch and
  every 30 minutes; it never pushes.
- **Vault** (`sync_vault.rs`, CX6b): a second root of the same engine, off
  unless the person turns it on, local commits only, the personal folders kept
  out ([Vault root](DIRECTORIES.md#vault-root-cx6b-2026-11-15)).

## Audit and the context check hook

`context.audit` (read) scans the personal clone, the org clone and every
connected project repo at its layer; the built-in **Monthly context audit**
template runs it once saved. **Install context check** (Settings → Sources →
Context, `context.install_check`) writes a local `pre-push` hook that runs the
app data copy of `ctx-scan` and fails a push only on a blocking finding. Both:
[Leak checks](DIRECTORIES.md#leak-checks-cx10-2026-11-15).

## First-sync dry run (PX1)

Before the personal sync has ever run, `context_layers_status` carries `seed`
(`context_settings::seed_report`): for each synced folder (`me`, `memory`,
`learnings`, `home`) the files only the vault has, only the personal clone has,
or both have with different text, plus how many match, and one sentence ("The
first sync would send 2 files from the vault to the repo, write 3 into the
vault, keep both versions of 1"). Settings → Context → Personal shows it above
the sync controls with a fold listing the files. It reads only (hidden files
and symlinks skipped, at most 500 files per side, 6 folders deep) and compares
the vault with the clone, which stands for the remote as last pulled, so it is
a guide, not the cycle's own plan. It disappears after the first sync. Not
built: a one-click copy of the vault's learnings into the repo, because the
first cycle merges both ways itself and a copy into the clone's work tree would
collide with the clone's next pull; that waits on a decision on which side seeds.

## Doctor line (PX7)

Settings → System → Capability doctor lists **Personal context** once a personal
clone is set (`Diagnostics.contextHealth`, from `context_layers::health`; read
only, computed on each diagnostics read). It is `stale`, never `broken`, when:
a profile file (`Identity`, `Communication`, `Anti-patterns`, `Guardrails`,
`Decisions`; `me/README.md` is a note and left out) differs between the vault's
`me/` and the clone's (SHA-256 of the text, trailing whitespace ignored; a file
in only one copy counts), until CX13 retires the vault copy; the home file's
last `Last checked: YYYY-MM-DD` line is over 30 days old; a memory in the
clone's `memory/` is past its `valid_until`; or its `verified_at` is over 180
days old (`memory_guard::memory_dates` reads both, top level or under
`metadata:`). A memory with neither field is not flagged, only counted. The fix
button opens Settings → Context → Personal.

## Code

| Module | Owns |
|---|---|
| `context_roots.rs` | clone roots, machine slug, no-symlink reads |
| `context_layers.rs` | home and org sections, the generated personal index (`personal_index`, PX2), org retrieval, the org write rules, the pull timer, the doctor's context health (`health`, PX7) |
| `context_compiler.rs` | where the sections join the profile block |
| `memory_guard.rs`, `leak_checks.rs`, `scan_rules.rs` | per-layer write rules, flag or block, the shared scanner |
| `context_propose.rs`, `context_org_digest.rs` | org pull requests and their nightly digest |
| `memory_homes.rs`, `vault_learning.rs` | routing chat facts to Home and Org |
| `sync_engine.rs`, `sync_vault.rs`, `sync_clash.rs` | the personal and vault roots, clash cards |
| `context_settings.rs`, `settings/ContextLayersPanel.tsx`, `contextLayers.ts` | Settings → Sources → Context |
| `src/contextSetupBrief.ts` | the portable setup brief copied from Settings |
| `mac_context.rs`, `settings/MacContextCard.tsx` | "Connect this Mac's context" when another Mac appears (F3): clones, this Mac's name, its home file |

## Setup without Hollow (CX22)

People who do not use Hollow can set up the same four layers from an organization's `GUIDE.md` or by copying the portable setup brief (`src/contextSetupBrief.ts`) from Settings → Sources → Context → Organization ("Set up without Hollow": Open the guide and Copy setup brief).

## Planned, not built

- **CX10b:** leak checks flip `leak_checks::DEFAULT_MODE` to `block`.
- **CX11:** `context.stub_instruction_files`, shrinking `~/.claude/CLAUDE.md`,
  `~/CLAUDE.md` and `~/.codex/AGENTS.md` to stubs that point at the layers.
- **CX12:** the Mac mini's own clones with deploy keys and its home file; until
  then a remote turn gets this Mac's home file.
- **CX13:** the migration of existing files into the layers, after the
  person's OK on the dry run.
- **CX18 and V6:** creating the personal repo through `gh`, and a remote for
  the vault root.
