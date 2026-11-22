# Pressure tests (stakeholder simulator)

> **Changed 11-10 (S6).** The pressure-test screens and the personas setting were cut, and so were the `personas_accept` and `pressure_test_start` commands: nothing in the app accepts a persona or starts a test from a window now. The agent actions (`simulate.pressure_test`, `simulate.pressure_tests`, `personas.personas_list`, `personas.personas_draft`) and the runner in `stakeholder_sim.rs` stay. The runner's `Observation` type and run-reading helper now live in `stakeholder_sim.rs` (the bake-off module they came from is removed).
>
> **Changed 11-18 (C7).** `personas.accept {role}` is the accept path again: a Confirm action on `ALWAYS_ASK` (a card in every mode, Bypass included) whose card shows the whole note, bound to its role and hash. A note holding characters the card can show only as `?` (control, direction-changing, invisible or tag characters) is refused before the card and at apply. S6 removed the old command only because the screen that called it was cut, not for a safety reason (BUILD_LOG 11-10: "until an accept path is rebuilt").


Master plan #8, built overnight 2026-10-10 (B3). Code: `src-tauri/src/stakeholder_sim.rs`,
`src/pressureTests.ts`, `src/store/pressureTests.ts`.

## Personas

A persona is a **role**, never a person: `personas/<role>.md` for `ceo`, `cfo`, `ops-vp` or `it`.

- **Drafted from evidence only.** `personas.personas_draft {role}` (write) collects, at most 12, in this order:
  - `people/` notes whose `role:` (or relationship) names the role: the `role:` line and up to four bullets of the
    person's own text;
  - settled `decisions/` records by those people or on the role's topics (CFO: budget, cost, revenue…; IT: security,
    SSO, integration…);
  - bullets in `me/Decisions.md` on those topics;
  - accepted facts about those people.

  Each becomes `- [E<n>] \`path:line\` — "quote"` inside the Hollow-written block (`records::splice_generated`), so
  the person's own text in the note is kept byte-for-byte on refresh. A role with no evidence is refused; nothing is
  invented. The short word "it" counts only in a person's role, never in running text; "vice president" is not CEO
  evidence, and "security" alone doesn't make someone IT.
- **Accepted by the person only.** `personas.accept {role}` (C7) asks on a card in every mode, Bypass included, and
  never runs from an automation or a COO check-in. The card shows the whole note (as a setup hook's script: lines
  wrap at 160 characters, control and invisible characters show as `?`, at most 400 card lines) and is bound to the
  role and the note's SHA-256; at apply a note changed since the card is refused. Refused before asking: no note, a
  note over 16 KB (it can't be shown whole), a note with hidden characters (a tab and line breaks are fine), a linked or
  non-UTF-8 note (each with its own reason), no evidence that still holds, or this exact version already accepted. The hash is kept in app data
  (`<support>/Personas/<vault>.json`), outside the vault, so writing "accepted" into the note proves nothing. Each
  entry is signed (HMAC-SHA256 of role, hash and time) with a random key beside it (`<vault>.key`, owner-only, made on
  the first acceptance), and an entry that doesn't verify counts as not accepted (morning P2). A shell writing JSON is
  no longer enough; code running as the same user that reads the key still could. Any change to the note (a refresh,
  an agent's edit, the person's own) needs acceptance again.
- **Evidence must still hold.** At accept and at start each anchor is re-read: a visible vault file outside
  `personas/` (checked as written and as resolved through symlinks, without case), whose line still contains the quote.
  Stale anchors keep their numbers but are neither shown to the persona nor citable, and are listed in the report; a
  persona with none left is refused. A persona note (or `personas/` folder) that is a symlink is never read or
  written.

## A pressure test

`simulate.pressure_test {doc, personas, workspace?, model?}` (write; only from a chat the person started: never
automations, delegated or swarm chats, or unattended Main agent turns) or **Pressure-test a draft…** (user-only
`pressure_test_start`): the palette asks for the draft; **Pressure-test this draft…** in a workspace Markdown/text
file's or Result's ⋯ menu passes the one on screen (`isPressureTestDraft`, consolidation D).

- **The draft:** a `.md`/`.txt`/`.markdown` file in the vault, at most 200 KB. The report goes to its workspace
  (or `workspace`); built-in and archived workspaces are refused.
- **Refused up front** when any picked persona is missing, unaccepted, changed since accepted or without holding
  evidence, when more personas are picked than the swarm limit, or when the Main agent's daily cap is reached. Nothing
  starts.
- **One chat per persona**, as the bake-off runs variants: an ordinary chat through `send_chat`, in `plan` mode (no
  writes, no connectors), in an empty scratch folder `<support>/PressureTests/<vault>/<id>/<opaque>/` holding only a
  numbered `DRAFT.md`. The prompt carries the persona's own text (without its evidence block), only the anchors that
  still hold, and the numbered draft (60 KB inline) between marker lines as material, not instructions. It asks the
  persona to use nothing else. The scratch folder runs **bare** (`mcp_attach::bare_scratch`, overnight S7):
  - none of Hollow's tools (no vault search, app or task reads, questions or teammates) and no shared connectors;
  - none of the vault's context: no retrieval, recall, operator profile or skill index, and no connector selection.
    The appended prompt is only a short brief plus "answer in plain Markdown": no vault path, capability index,
    provider contract or question guidance;
  - no question card, and no learning extraction afterwards.

  Plan mode still forbids writes. The agent CLI's own read tools remain. Parts run **one at a time**: a scratch folder
  outside the vault makes an unscoped chat, which takes the vault lane, so the person's own vault work can run between
  them. Model: the Settings swarm model (Claude Haiku 5.5 by default). A part not yet started is skipped once the cap
  is reached. `stakeholder_sim::spent_since` adds their known cost to `budgets::main_agent_spent` (also when the Main
  agent's workspace doesn't exist yet).
- **Every point cites evidence.** A persona replies with one JSON block (objections, missing numbers, one question,
  fix-first). `parse_critique` keeps a point only if it cites at least one of that persona's `E` ids; draft lines `L12`
  may be added but never stand alone. A persona with no cited point is "uncited" and adds nothing to the summary.
- **The report** is written by Hollow, not a model, once every part has ended:
  `workspaces/<id>/outputs/pressure-tests/<date>-<slug>.md` (never over an existing file; `-2`, `-3`…). Frontmatter
  `label: simulated critique`; a banner saying the roles were played locally and nobody was contacted; **Fix before it
  goes up** (each persona's fix-first and missing numbers); the draft path is quoted as a JSON string in the
  frontmatter, and a path with control characters is refused; each persona's objections, numbers and question with every
  citation spelled out (`E2 \`people/x.md:8\` "quote"`, `draft L3`); what wasn't counted and why; a runs table with
  model, cost (unknown, never $0) and session.
- `simulate.pressure_tests {id?}` (read) lists recent tests or one with its report.

State: `<support>/PressureTests/<vault>/<id>.json` (schema 1); the scratch folders and the draft copy go once their part
or the test ends. Moved along by `advance` after each chat ends and each minute (beside swarms and bake-offs).

Launches are idempotent (morning M4):
- **Per part.** A part's chat is found by its own scratch folder (`part_chat`: its live row, or the conversation once
  it ran) before a launch and before the daily cap is weighed (`Launcher::adopt` in `step`), so a part launched but
  never saved as launched (a failed save, a crash) is adopted, not queued again or skipped from under its chat. A
  turn that had started before a crash is adopted with its interrupted or failed outcome. If the check can't read
  the store, the part waits for the next step.
- **Per start.** The palette sends a start key (`requestId`, kept across its Retry); a start whose key and starter
  match a saved test returns that test instead of starting another.

## Known limits

- An unpriced model's cost is unknown and doesn't count toward the cap (as for swarms).
- A failed save after the report is written can write a second one (`-2`), as in the bake-off. (A crash between
  queuing a part's chat and saving the test no longer queues that part again; see above.)
- Agents' `simulate.pressure_test` sends no start key, so an agent retrying after an error can start a second test
  (its parts are still never launched twice).
- The agent CLI's own sources are untouched:
  - its read tools (Claude's Read/Grep in plan mode, Codex's read-only sandbox) can read local files by absolute path
    if a persona ignores its brief;
  - its memory files load (Claude: `~/.claude/CLAUDE.md` and `~/CLAUDE.md`, since the scratch folder sits under the
    home folder; Codex: `~/.codex/AGENTS.md`);
  - Grok's and Antigravity's own MCP servers attach;
  - the working folder's name (`<support>/PressureTests/<vault-folder>-<hash>/…`) appears in the CLI's environment
    block.

  Hollow adds no vault path or route of its own. Turning off the CLIs' memory needs flags verified per CLI; that is a
  follow-up.

## Not built (v1)

Critique cards in the conversation (the report is a Result); a model-written synthesis; named-person personas
(the plan's recommendation: roles only); a Settings page for personas.
