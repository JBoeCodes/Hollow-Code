// The synthetic vault the candidate test bed opens (merged plan step 0).
// Fixed content, so every candidate is checked against the same files, and
// disposable: writeSyntheticVault() replaces the folder each time, but only a
// folder it created itself (it carries MARKER).
//
// It holds what the plan's checks need: outcome workspaces with brief, plan,
// sources and outputs; the scripted journey's workspace; a Notion-mirrored
// project; a file over 2 MiB (History does not capture those); local tasks; a
// manual workflow that never runs by itself; a recording with "Action items:" and "To do:" sections; the me/
// profile, including me/Guardrails.md for the protected-path check; and, for
// the agent effectiveness root check (scripts/ae-w1.mjs), a workspace and a
// learnings/ note outside it whose phrase appears nowhere else; and, for the
// continuity check (scripts/ae-continuity.mjs), a workspace for a child
// conversation.
//
// Agent effectiveness E2 adds the Eval outcome workspace the paid pilot runs
// in (scripts/agent-eval/pilot-tasks.json): four attached sources whose key fact
// is in the fourth by name, an older decision note behind newer notes about
// the same thing (EVAL_MTIMES dates them), a superseded learning with its
// approved correction, and a settled constraint in me/Decisions.md. E1 adds
// the synthetic skill its selected-skill task names (EVAL_SKILL).
import { existsSync, mkdirSync, readdirSync, rmSync, utimesSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

export const MARKER = '.hollow-test-bed';
const CREATED = '2026-10-02T09:00:00Z';

const workspace = (title, outcome, extra = '') => `---
hollow_workspace: 1
title: "${title}"
status: active
created: ${CREATED}
${extra}---

# ${title}

## Outcome

${outcome}

## Current context

Synthetic test-bed content. Nothing here is real.

## Next step

Run the candidate's smoke checks.

## Decisions

- This vault is disposable; \`node scripts/test-bed.mjs vault\` re-creates it.
`;

const meNote = (title, body) => `---
title: ${title}
last_updated: 2026-10-02
last_reviewed: 2026-10-02
review_cadence: monthly
---

# ${title}

${body}
`;

/** Deterministic text just over 2.5 MiB. */
function largeText() {
  const line = (n) => `${String(n).padStart(6, '0')},synthetic,row,${(n * 7919) % 100003},${'x'.repeat(48)}\n`;
  const lines = [];
  let size = 0;
  for (let n = 0; size < 2.5 * 1024 * 1024; n++) {
    const l = line(n);
    lines.push(l);
    size += l.length;
  }
  return lines.join('');
}

// ── The Eval outcome workspace (agent effectiveness E2) ────────────────

const EVAL = 'workspaces/eval-outcome';
const KESTREL = 'projects/kestrel-pilot.md';
const OLD_CADENCE = 'learnings/kestrel-report-cadence-old00001.md';
const NEW_CADENCE = 'learnings/kestrel-report-cadence-new00002.md';
const VENDOR_DECISION = 'notes/kestrel/2026-06-02-sensor-vendor-decision.md';
/** The synthetic skill the selected-skill pilot task names (scripts/agent-eval/pilot-tasks.json). */
export const EVAL_SKILL = '.agents/skills/kestrel-status-report/SKILL.md';
const SYNC_DAYS = ['01', '02', '03', '04', '07', '08', '09', '10', '11'];

/** Deterministic filler lines until the text is at least `bytes` long. */
function padTo(head, bytes, line) {
  const out = [head];
  let size = Buffer.byteLength(head);
  for (let n = 1; size < bytes; n++) {
    const l = line(n);
    out.push(l);
    size += Buffer.byteLength(l);
  }
  return out.join('');
}

const learning = ({ subject, statement, learnedAt, id, link }) => `---
type: learned-context
title: ${statement}
target_kind: project
target_path: ${KESTREL}
subject: ${subject}
trust: approved
proposal_id: review-${id}
learned_at: ${learnedAt}
${link}
---

${statement}
`;

function evalFiles() {
  const sites = ['North depot', 'Canal yard', 'Hill reservoir'];
  return {
    [`${EVAL}/WORKSPACE.md`]: `---
hollow_workspace: 1
title: "Eval outcome"
status: active
created: ${CREATED}
---

# Eval outcome

## Outcome

Run the Kestrel field pilot at three synthetic sites and report what it shows.

## Current context

Synthetic evaluation content for the test bed's pilot. Nothing here is real.
The attached sources are in sources/; project notes live in the vault.

## Next step

Answer the steering group's questions about the pilot.
`,
    [`${EVAL}/PLAN.md`]: '# Plan\n\n- [ ] Confirm the budget and the sensor vendor\n- [ ] Send the status report\n- [x] Set up the workspace\n',
    [`${EVAL}/AGENTS.md`]: '# Working in this workspace\n\nRead WORKSPACE.md and PLAN.md. Put deliverables in outputs/.\n',
    [`${EVAL}/SOURCES.md`]: `# Sources\n\n${['a-site-survey.md', 'b-field-log.md', 'c-supplier-catalogue.md', 'd-budget-ceiling.md'].map((name) => `## ${name}\n\n- Origin: synthetic fixture.\n`).join('\n')}`,
    [`${EVAL}/sources/a-site-survey.md`]: padTo('# Site survey (synthetic)\n\nThree sites were surveyed for the Kestrel field pilot.\n\n', 9 * 1024,
      (n) => `- Survey point ${String(n).padStart(3, '0')}: ${sites[n % 3]}, mast height ${(n * 37) % 12 + 3} m, signal ${(n * 13) % 40 + 50} dB, ground firm.\n`),
    [`${EVAL}/sources/b-field-log.md`]: padTo('# Field log (synthetic)\n\n## Open risks\n\n- Gateway firmware: the gateways need firmware 2.4 before go-live; the vendor has not confirmed a date.\n- Site access: Canal yard allows crews only on weekday mornings.\n\n## Entries\n\n', 9 * 1024,
      (n) => `- Day ${String(n).padStart(3, '0')}: crew visited ${sites[n % 3]}; checked mounts, logged readings, no incidents.\n`),
    [`${EVAL}/sources/c-supplier-catalogue.md`]: padTo('# Supplier catalogue (synthetic)\n\nParts the pilot may order. Prices are list prices before any discount.\n\n', 9 * 1024,
      (n) => `- Part K-${String(n).padStart(4, '0')}: mounting bracket type ${n % 5}, ${(n * 7) % 90 + 10} units in stock, lead time ${(n * 3) % 20 + 2} days.\n`),
    [`${EVAL}/sources/d-budget-ceiling.md`]: '# Budget ceiling (synthetic)\n\nThe steering group approved a budget ceiling of 48,500 EUR for the Kestrel field pilot on 2026-08-28.\nCommitted so far: 31,200 EUR (sensors and gateways). Anything above the ceiling needs a new approval.\n',
    [`${EVAL}/outputs/.keep`]: '',
    [KESTREL]: '---\ntitle: Kestrel pilot\nstatus: active\n---\n\n# Kestrel pilot\n\nA synthetic field pilot of wireless level sensors at three sites. The Eval outcome workspace holds its work.\n',
    [OLD_CADENCE]: learning({ subject: 'Kestrel status reports', statement: 'Kestrel status reports go out every Friday.', learnedAt: '2026-07-01T09:00:00Z', id: 'old00001', link: `superseded_by: ${NEW_CADENCE}` }),
    [NEW_CADENCE]: learning({ subject: 'Kestrel status reports', statement: 'Kestrel status reports go out every second Tuesday, since the steering group changed the cadence on 2026-09-15.', learnedAt: '2026-09-15T09:00:00Z', id: 'new00002', link: `supersedes: ${OLD_CADENCE}` }),
    [VENDOR_DECISION]: '---\ntitle: Kestrel sensor vendor decision\ndate: 2026-06-02\n---\n\n# Kestrel sensor vendor decision\n\nDecision: the Kestrel pilot uses Halvard sensors. Brightline was ruled out because its battery life fell short of the 18-month requirement.\n',
    // A skill the pilot's selected-skill task points at, as Skills → Use in
    // workspace does (E1). In a dot folder, like ~/.agents/skills, so vault
    // search and retrieval never surface it for the other tasks. The vault's
    // .agents/skills is one of the skill index's two roots (F9, V2): a turn
    // that selects it gets its text inline, and hollow_vault read_skill reads
    // it (row ae-skill-index).
    [EVAL_SKILL]: `---
name: kestrel-status-report
description: Format a Kestrel pilot status report the way the steering group reads it.
---

# Kestrel status report

When you write a Kestrel status report:

1. The first line is exactly \`KESTREL STATUS · <date>\`, with today's date as YYYY-MM-DD.
2. Then three sections, in this order: \`## Done\`, \`## Next\`, \`## Risks\`.
3. The last line is exactly: Prepared for the Kestrel steering group.
`,
    ...Object.fromEntries(SYNC_DAYS.map((day, i) => [`notes/kestrel/2026-09-${day}-weekly-sync.md`,
      `---\ntitle: Kestrel weekly sync 2026-09-${day}\ndate: 2026-09-${day}\n---\n\n# Kestrel weekly sync 2026-09-${day}\n\n- The sensor vendor came up again: crews asked about mounting brackets for ${sites[i % 3]}.\n- Install schedule holds; readings look normal.\n- Next sync in a week.\n`])),
  };
}

/** Modification times the vault gets after writing: the vendor decision is
 *  older than the weekly notes in front of it. */
export const EVAL_MTIMES = {
  [VENDOR_DECISION]: '2026-06-02T10:00:00Z',
  ...Object.fromEntries(SYNC_DAYS.map((day) => [`notes/kestrel/2026-09-${day}-weekly-sync.md`, `2026-09-${day}T10:00:00Z`])),
};

export function files() {
  return {
    'README.md': '# Synthetic Hollow vault\n\nCreated by `scripts/test-bed.mjs vault` for the candidate test bed. Disposable; nothing here is real.\n',
    [MARKER]: 'This folder is the candidate test bed\'s synthetic vault. test-bed.mjs may delete and re-create it.\n',

    'workspaces/synthetic-outcome/WORKSPACE.md': workspace('Synthetic outcome', 'Ship a one-page synthetic report from the attached brief.'),
    'workspaces/synthetic-outcome/PLAN.md': '# Plan\n\n- [ ] Read the brief in sources/\n- [ ] Draft the report in outputs/\n- [x] Set up the workspace\n',
    'workspaces/synthetic-outcome/AGENTS.md': '# Working in this workspace\n\nRead WORKSPACE.md and PLAN.md. Put deliverables in outputs/.\n',
    'workspaces/synthetic-outcome/SOURCES.md': '# Sources\n\n## brief.md\n\n- Origin: synthetic fixture.\n',
    'workspaces/synthetic-outcome/sources/brief.md': '# Brief\n\nThe report should list three synthetic findings and one recommendation.\n',
    'workspaces/synthetic-outcome/sources/recording-weekly-sync-0000abcd.md': `---
source: plaud
recording_id: 0000abcd-synthetic
retrieved_at: ${CREATED}
title: Weekly Sync (synthetic)
---

## Summary

The team reviewed the synthetic report.

Action items:
- Dana: send the draft by 2026-10-09
- Review the findings (owner: Lee)

3. To do
1. Book the follow-up
2. Archive the old draft

## Transcript

[00:01 - 00:05] Dana: Let's review the synthetic report.
[00:06 - 00:09] Lee: I'll check the findings.
`,
    'workspaces/synthetic-outcome/outputs/report.md': '# Synthetic report\n\n1. Finding one.\n2. Finding two.\n3. Finding three.\n\nRecommendation: keep testing on the bed.\n',
    'workspaces/synthetic-outcome/sessions/.keep': '',

    'workspaces/large-file-outcome/WORKSPACE.md': workspace('Large file outcome', 'Hold a file over 2 MiB for the History coverage checks.'),
    'workspaces/large-file-outcome/PLAN.md': '# Plan\n\n- [ ] Change sources/large-fixture.txt and restore it from History\n',
    'workspaces/large-file-outcome/SOURCES.md': '# Sources\n\n## large-fixture.txt\n\n- Origin: generated, deterministic, about 2.5 MiB.\n',
    'workspaces/large-file-outcome/sources/large-fixture.txt': largeText(),
    'workspaces/large-file-outcome/outputs/.keep': '',

    // The scripted journey's workspace (test-bed.mjs journey). Nothing here
    // may contain the option label the fake agent keys its answer turn on.
    'workspaces/journey-outcome/WORKSPACE.md': workspace('Journey outcome', 'Hold the scripted journey: a draft, a run, a decision and a Result.'),
    'workspaces/journey-outcome/PLAN.md': '# Plan\n\n- [ ] Ask the person one question\n- [ ] Put the result in outputs/\n',
    'workspaces/journey-outcome/outputs/.keep': '',

    // Agent effectiveness F1 (scripts/ae-w1.mjs): a chat here must find
    // learnings/bed-root-check.md, which is outside this folder.
    'workspaces/ae-root-outcome/WORKSPACE.md': workspace('AE root check', 'Hold the agent effectiveness root check: explicit vault search from here covers the whole vault.'),
    'workspaces/ae-root-outcome/PLAN.md': '# Plan\n\n- [ ] Search the whole vault from this workspace\n',
    'learnings/bed-root-check.md': `---
type: learning
title: Bed root check
created: ${CREATED}
---

# Bed root check

The periwinkle lodestar note lives outside every workspace, so only a search rooted at the vault finds it.
`,

    // Agent effectiveness F4 (scripts/ae-continuity.mjs): a conversation an
    // agent starts here must get the me/ profile once. The row counts the
    // profile's "Synthetic Tester", which only me/Identity.md carries.
    'workspaces/ae-child-outcome/WORKSPACE.md': workspace('AE child check', 'Hold the agent effectiveness continuity check: a conversation an agent starts here gets the operator profile once.'),
    'workspaces/ae-child-outcome/PLAN.md': '# Plan\n\n- [ ] Start a child conversation and message it once\n',

    'workspaces/notion-abc123/WORKSPACE.md': workspace(
      'Mirror Project',
      'A Notion-mirrored project. Notion sync is off on the bed; no token is configured.',
      'notion_id: abc123\nnotion_project_path: "workspaces/notion-abc123/PROJECT.md"\n',
    ),
    'workspaces/notion-abc123/PROJECT.md': `---
title: Mirror Project
status: In progress
description: Synthetic Notion mirror (never synced).
priority: P2
target_date: 2026-11-01
notion_id: abc123
notion_url: https://www.notion.so/abc123
custom_unmodelled_key: keep me
---

# Mirror Project

Editing this project must not move the file or drop \`custom_unmodelled_key\`.
`,

    'tasks/draft-synthetic-report.md': `---
title: Draft synthetic report
status: Not started
priority: P2
due: 2026-10-09
project: Mirror Project
project_url: vault:workspaces/notion-abc123/PROJECT.md
---
Write the synthetic report.

- [ ] Outline
- [ ] Draft
`,
    'tasks/finished-synthetic-task.md': '---\ntitle: Finished synthetic task\nstatus: Done\npriority: P3\n---\nAlready done.\n',

    'workflows/synthetic-manual-check.md': `---
title: Synthetic manual check
description: Never runs on its own
schedule: manual
permission_mode: plan
notify: never
---
List the workspaces in this vault and say how many there are. Do not change any file.

## Run log
`,

    'recordings/weekly-sync.md': `---
type: recording
title: Weekly Sync (synthetic)
---
## Summary

Action items:
- Dana: send the draft by 2026-10-09

## Transcript
[00:01] Dana: Synthetic transcript.
`,

    'me/Identity.md': meNote('Identity', '- **Name:** Synthetic Tester\n- **Role:** Test bed operator'),
    'me/Communication.md': meNote('Communication', '- Short, direct answers.'),
    'me/Anti-patterns.md': meNote('Anti-patterns', '- (none; synthetic)'),
    'me/Guardrails.md': meNote('Guardrails', '- Agents must never edit this file. The protected-path check targets it, including as `me//Guardrails.md`.'),
    'me/Decisions.md': meNote('Decisions', '## 2026-10-02 — Synthetic vault\n\n- This vault is disposable.\n\n## 2026-09-20 — Kestrel reporting\n\n- Settled: every Kestrel report states amounts in EUR, never USD.'),

    ...evalFiles(),
  };
}

/** Required paths, for callers that check the vault is complete. */
export const REQUIRED = [
  'workspaces/synthetic-outcome/WORKSPACE.md',
  'workspaces/large-file-outcome/sources/large-fixture.txt',
  'workspaces/journey-outcome/WORKSPACE.md',
  'workspaces/notion-abc123/PROJECT.md',
  'tasks/draft-synthetic-report.md',
  'workflows/synthetic-manual-check.md',
  'workspaces/synthetic-outcome/sources/recording-weekly-sync-0000abcd.md',
  'me/Guardrails.md',
  'workspaces/ae-root-outcome/WORKSPACE.md',
  'learnings/bed-root-check.md',
  'workspaces/ae-child-outcome/WORKSPACE.md',
  `${EVAL}/WORKSPACE.md`,
  `${EVAL}/sources/d-budget-ceiling.md`,
];

/** (Re)create the synthetic vault at `dir`. Refuses a folder it didn't make. */
export function writeSyntheticVault(dir) {
  if (existsSync(dir)) {
    const entries = readdirSync(dir);
    if (entries.length > 0 && !entries.includes(MARKER)) {
      throw new Error(`${dir} exists and is not a test-bed vault (no ${MARKER}); not replacing it`);
    }
    rmSync(dir, { recursive: true, force: true });
  }
  const all = files();
  for (const [rel, body] of Object.entries(all)) {
    const path = join(dir, rel);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, body);
  }
  for (const [rel, at] of Object.entries(EVAL_MTIMES)) {
    const when = new Date(at);
    utimesSync(join(dir, rel), when, when);
  }
  return { files: Object.keys(all).length, required: REQUIRED };
}
