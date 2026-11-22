# Learning, memory and skills

Part of the agent manual; [AGENTS.md](../../AGENTS.md) is the entry point. Capture triggers, privacy scope (`turns.capture_scope`), grounding and redaction: [vault reference](../reference/vault.md#alignment-trusted-learning-evidence-and-recovery).

## Capture

- Automatic capture is host-owned (`learning_capture.rs`). It may save exact, grounded, positive Know-how with a host-capture log actor and durable Undo (per-card Undo only for `learning-batch-auto-<id>`). The proof is the person's own messages: tool-result quotes, negatives and email addresses stay pending. You, Project and private-memory promotions always stay pending.
- A profile rule is proposed only from an explicitly general user decision. Task procedures become skill lessons in Library Learnings, waiting for Save.

## Skill lessons

- Existing shared skills are read fresh, pinned and appended without recomposing their frontmatter. A new skill name never overwrites an existing one, and a new skill refuses an occupied directory even without `SKILL.md`, so unreviewed scripts stay inactive.
- A skill lesson needs a proven shared home: shared inline text, an explicitly scoped shared read, or a read of the exact indexed path. Name-only legacy reads and vault/provider namesakes never authorize editing a shared skill.
- The one-line lesson is scanned again before claim; Save scans the whole resulting `SKILL.md` and writes only through `skills::write_skill_file`. Safety refusals log classes and positions, never the rejected text. A skill changed since the lesson closes it as stale (`Superseded:`). Undo (`undo_skill_lesson`) works while the skill is exactly what Save wrote. Same-run checks need a run id on both sides (`same_run`).
- Shared skill descriptions serialize as YAML-compatible quoted strings and decode escapes once when listed. See [Skills](../reference/agents.md).

## Private provider memory

- Library Learnings reads private provider notes for the selected project only; it never rewrites them or adds them to prompt context. Provider roots are exact folder matches (vault workspace, active worktree, the build workspace's repository). A Codex memory group shows only when every cwd it names belongs to the project. Promotion queues a pending learning only.

## Memory maintenance

- Library → Learnings is the one review home; the bell and Needs you only link to it. Installation never accepts or migrates pending items.
- Usage metadata is in app data's `memory-usage/<vault-stem>.json`; corrupt state fails closed. Know-how is stale after 14 quiet days and archived after 30, never deleted. Pin prevents quiet aging; explicit expiry still applies. Profile rules never age automatically.
- Accepted You and Project lines are keyed `line:<proposal id>`; a due line gets a `retires` proposal and changes only when the person saves it. A line edited by hand is left alone. Profile source files over 16,000 bytes get a consolidation proposal; nothing is trimmed automatically.
- Overwrite guards for shared skills, regenerated meeting notes and custom icons: [Skills](../reference/agents.md), [Meeting notes](../PREP_PACKS.md), [Custom icons](../CUSTOM_ICONS.md). A confirmation never authorizes a later changed note or icon.
