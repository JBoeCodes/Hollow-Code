//! Sources of truth (H4a): the config block, the resolver, the one field
//! rule, labels and owner switches, on temp vaults with sanitized fixtures.
//! No network: the engine-level switches (a pass after a switch, lost
//! access, held edits) are in `tracker/sync_tests.rs` with the stub server.

use super::migration::{self, Env, Phase, Stop, Target};
use super::*;
use crate::tracker::linear::{PERSONAL_TEAM_ID, TEAM_ID};
use crate::tracker_sync::{self, Conflict, Ledger, Outbox, Record as LedgerRecord};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

const PROJECT: &str = "---\ntitle: \"Tracked project\"\nscope: \"work\"\ntracker: \"linear\"\ntracker_project: \"p1\"\ntracker_url: \"https://linear.app/example/project/p1\"\ntracker_team: \"AI\"\n---\n## Decisions\nKeep this prose exactly.\n";
const TASK: &str = "---\ntitle: \"Mirrored task\"\ntracker: \"linear\"\ntracker_id: \"i1\"\ntracker_url: \"https://linear.app/example/issue/ENG-1\"\ndue_time: \"09:30\"\n---\n## Vault notes\nMine.\n";

fn tracker_block() -> Value {
    json!({"provider":"linear","linear":{"teamId":TEAM_ID,"teamKey":"ENG"}})
}

fn ctx(config: Value) -> Context {
    let map: BTreeMap<String, Value> = serde_json::from_value(config).unwrap();
    Context::from_map(&map, Access::default())
}

struct Bed {
    _dir: tempfile::TempDir,
    env: Env,
}

impl Bed {
    fn new(config: Value) -> Bed {
        let dir = tempfile::Builder::new()
            .prefix("vault-sources-")
            .tempdir()
            .unwrap();
        let vault = dir.path().join("vault");
        let app = dir.path().join("app");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&app).unwrap();
        let config_path = app.join("config.json");
        let mut config = config;
        config["vaultPath"] = json!(vault);
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&config).unwrap() + "\n",
        )
        .unwrap();
        Bed {
            env: Env {
                migrations: Env::migrations_for(&config_path),
                vault,
                config_path,
            },
            _dir: dir,
        }
    }

    fn vault(&self) -> &Path {
        &self.env.vault
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.vault().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.vault().join(rel)).unwrap()
    }

    fn config(&self) -> Vec<u8> {
        fs::read(&self.env.config_path).unwrap()
    }

    fn ctx(&self) -> Context {
        let map = crate::setup::load_config_map(&self.env.config_path).unwrap();
        Context::from_map(&map, Access::default())
    }

    fn ledger(&self) -> Ledger {
        tracker_sync::load(self.vault(), "linear", TEAM_ID).unwrap()
    }

    /// A tracked workspace (`proj-a`, project p1), a personal one, one
    /// mirrored task (i1), one vault task and the ledger that binds them.
    fn tracked() -> Bed {
        let bed = Bed::new(json!({ "tracker": tracker_block() }));
        bed.write("workspaces/proj-a/PROJECT.md", PROJECT);
        bed.write(
            "workspaces/proj-a/WORKSPACE.md",
            "---\ntitle: \"Tracked project\"\n---\nBrief.\n",
        );
        bed.write(
            "workspaces/personal/WORKSPACE.md",
            "---\ntitle: \"Personal\"\n---\nMy own things.\n",
        );
        bed.write("tasks/tracker-i1.md", TASK);
        bed.write("tasks/plain.md", "---\ntitle: \"A vault task\"\n---\n");
        let mut ledger = Ledger::new("linear", TEAM_ID);
        for (id, kind, path, ws) in [
            (
                "p1",
                "project",
                "workspaces/proj-a/PROJECT.md",
                Some("proj-a"),
            ),
            ("i1", "task", "tasks/tracker-i1.md", None),
        ] {
            ledger.records.insert(
                id.into(),
                LedgerRecord {
                    id: id.into(),
                    key: if kind == "project" {
                        "P-ENG-1"
                    } else {
                        "ENG-1"
                    }
                    .into(),
                    kind: kind.into(),
                    path: path.into(),
                    workspace_id: ws.map(str::to_string),
                    base: Default::default(),
                    conflicts: Vec::new(),
                    error: None,
                    remote_missing: false,
                    state: "active".into(),
                    issue: None,
                    project: None,
                },
            );
        }
        ledger.last_sync = Some("2026-11-18T14:40:00Z".into());
        tracker_sync::save(bed.vault(), &ledger).unwrap();
        bed
    }
}

fn global(to: OwnerKind) -> Target {
    Target {
        workspace: None,
        to,
    }
}

// ── The config block ────────────────────────────────────────────────────

#[test]
fn a_missing_block_reads_as_today() {
    let tracked = ctx(json!({ "tracker": tracker_block() }));
    assert_eq!(tracked.sources.tasks, OwnerKind::Tracker);
    assert!(!tracked.sources.present && !tracked.sources.tasks_set);
    assert!(tracked.sources.problems.is_empty());
    let off = ctx(json!({}));
    assert_eq!(off.sources.tasks, OwnerKind::Vault);
    assert!(off.sources.problems.is_empty());
    // A block without `tasks` still reads tasks from the tracker block.
    let refs = ctx(
        json!({ "tracker": tracker_block(), "sources": {"version": 1, "knowledge": {"references": [{"connector": "notion-official", "label": "Notion archive"}]}} }),
    );
    assert_eq!(refs.sources.tasks, OwnerKind::Tracker);
    assert!(!refs.sources.tasks_set);
    assert_eq!(refs.sources.references[0].connector, "notion-official");
    // The global binding is exactly what the tracker block meant.
    let b = global_tasks(&tracked);
    assert_eq!(b.owner, OwnerKind::Tracker);
    assert_eq!(b.provider_name.as_deref(), Some("Linear"));
    assert_eq!(b.team_id.as_deref(), Some(TEAM_ID));
    assert!(!b.chosen);
    assert_eq!(global_tasks(&off).owner, OwnerKind::Vault);
}

#[test]
fn bad_values_are_ignored_with_a_problem_line_and_never_echo_a_secret() {
    for (sources, words) in [
        (json!("tracker"), "aren't an object"),
        (
            json!({"version": 2, "tasks": {"owner": "vault"}}),
            "version",
        ),
        (
            json!({"tasks": {"owner": "notion"}}),
            "isn't vault or tracker",
        ),
        (json!({"tasks": "vault"}), "isn't an object"),
        (
            json!({"tasks": {"owner": "tracker", "teamId": "team-lp"}}),
            "tracker settings only",
        ),
        (
            json!({"knowledge": {"owner": "notion"}}),
            "always lives in your Hollow folder",
        ),
        (json!({"personal": {"owner": "notion"}}), "Personal context"),
        (
            json!({"knowledge": {"references": "notion"}}),
            "aren't a list",
        ),
        (
            json!({"knowledge": {"references": [{"label": "x"}]}}),
            "doesn't name a connector",
        ),
    ] {
        let c = ctx(json!({ "tracker": tracker_block(), "sources": sources }));
        assert!(
            c.sources.problems.iter().any(|p| p.contains(words)),
            "{words}: {:?}",
            c.sources.problems
        );
        // A bad tasks value never changes what tasks meant.
        assert_eq!(c.sources.tasks, OwnerKind::Tracker, "{words}");
    }
    let secret = "lin_api_fixture_secret_value";
    let c = ctx(
        json!({ "sources": {"knowledge": {"references": [{"connector": "notion-official", "apiKey": secret}, {"connector": "notion-official"}]}} }),
    );
    assert_eq!(c.sources.references.len(), 1);
    assert!(c.sources.problems.iter().all(|p| !p.contains(secret)));
    assert!(c
        .sources
        .problems
        .iter()
        .any(|p| p.contains("secrets stay with the connector")));
    assert!(c.sources.problems.iter().any(|p| p.contains("twice")));
    // Tasks set to the tracker with no tracker chosen: kept, and said.
    let c = ctx(json!({ "sources": {"tasks": {"owner": "tracker"}} }));
    assert_eq!(c.sources.tasks, OwnerKind::Tracker);
    assert!(c
        .sources
        .problems
        .iter()
        .any(|p| p.contains("no tracker is chosen")));
    assert!(!global_tasks(&c).reachable);
}

#[test]
fn save_refuses_an_owner_change_and_keeps_other_keys() {
    let mut map: BTreeMap<String, Value> =
        serde_json::from_value(json!({ "tracker": tracker_block(), "other": {"kept": true} }))
            .unwrap();
    let change = Update {
        tasks_owner: Some(OwnerKind::Vault),
        references: None,
    };
    assert_eq!(
        apply_update(&mut map, &change).unwrap_err(),
        SWITCH_NOT_SAVE
    );
    assert!(!map.contains_key("sources"));
    let bad = Update {
        tasks_owner: None,
        references: Some(vec![Reference {
            connector: "not a connector id".into(),
            label: None,
        }]),
    };
    assert!(apply_update(&mut map, &bad).is_err());
    assert!(!map.contains_key("sources"));
    let refs = Update {
        tasks_owner: Some(OwnerKind::Tracker),
        references: Some(vec![Reference {
            connector: "notion-official".into(),
            label: Some("Notion archive".into()),
        }]),
    };
    apply_update(&mut map, &refs).unwrap();
    assert_eq!(map["other"], json!({"kept": true}));
    assert_eq!(map["sources"]["tasks"]["owner"], "tracker");
    assert_eq!(map["sources"]["knowledge"]["owner"], "vault");
    let (loaded, _) = SourcesConfig::from_map(&map);
    assert!(loaded.tasks_set && loaded.problems.is_empty());
    assert_eq!(
        loaded.references[0].label.as_deref(),
        Some("Notion archive")
    );
}

// ── Workspace overrides ─────────────────────────────────────────────────

#[test]
fn an_override_cannot_widen_scope() {
    let bed =
        Bed::new(json!({ "tracker": tracker_block(), "sources": {"tasks": {"owner": "vault"}} }));
    let by_id = format!("tasks_source: tracker\ntracker_team: {PERSONAL_TEAM_ID}\n");
    let cases: [(&str, &str, &str); 5] = [
        (
            "lp-team",
            "tasks_source: tracker\ntracker_team: LP\n",
            "Settings doesn't allow",
        ),
        (
            "other-tracker",
            "tasks_source: tracker\ntracker: jira\n",
            "can't widen",
        ),
        // HOL is Linear's pinned personal team, but Settings hasn't added it.
        (
            "hol-not-added",
            "tasks_source: tracker\ntracker_team: HOL\n",
            "Settings doesn't allow",
        ),
        ("by-id", &by_id, "Settings doesn't allow"),
        (
            "bad-value",
            "tasks_source: notion\n",
            "isn't vault or tracker",
        ),
    ];
    for (id, keys, _) in &cases {
        bed.write(
            &format!("workspaces/{id}/WORKSPACE.md"),
            &format!("---\ntitle: \"{id}\"\n{keys}---\n"),
        );
    }
    let c = bed.ctx();
    for (id, _, words) in &cases {
        let b = resolve(&c, bed.vault(), Some(id));
        assert_eq!(
            b.owner,
            OwnerKind::Vault,
            "{id} falls back to Settings' reading"
        );
        assert!(b.inherited, "{id}");
        assert!(b.team_id.is_none(), "{id}");
        assert!(
            b.problems.iter().any(|p| p.contains(words)),
            "{id}: {:?}",
            b.problems
        );
    }
    // With no tracker block at all, a tracker override can't reach one.
    let bare = Bed::new(json!({}));
    bare.write(
        "workspaces/w/WORKSPACE.md",
        "---\ntasks_source: tracker\n---\n",
    );
    let b = resolve(&bare.ctx(), bare.vault(), Some("w"));
    assert_eq!(b.owner, OwnerKind::Vault);
    assert!(b.problems.iter().any(|p| p.contains("no tracker and team")));
    // The allowed team works, and only Settings' teams are ever bound.
    bed.write(
        "workspaces/ok/PROJECT.md",
        "---\ntasks_source: tracker\ntracker_team: ENG\n---\n",
    );
    let b = resolve(&c, bed.vault(), Some("ok"));
    assert_eq!(b.owner, OwnerKind::Tracker);
    assert!(!b.inherited);
    assert_eq!(b.team_id.as_deref(), Some(TEAM_ID));
    // Inheriting the tracker with a disallowed team keeps Settings' team.
    let tracked = Bed::new(json!({ "tracker": tracker_block() }));
    tracked.write(
        "workspaces/lp/PROJECT.md",
        "---\ntracker: linear\ntracker_project: p9\ntracker_team: LP\n---\n",
    );
    let b = resolve(&tracked.ctx(), tracked.vault(), Some("lp"));
    assert_eq!(b.owner, OwnerKind::Tracker);
    assert_eq!(b.team_id.as_deref(), Some(TEAM_ID));
    assert!(b.problems.iter().any(|p| p.contains("can't widen")));
    // With the personal team added, HOL is allowed.
    let two = Bed::new(
        json!({ "tracker": {"provider":"linear","linear":{"teamId":TEAM_ID,"teamKey":"ENG","personalTeamId":PERSONAL_TEAM_ID,"personalTeamKey":"PER"}} }),
    );
    two.write(
        "workspaces/mine/PROJECT.md",
        "---\ntasks_source: tracker\ntracker_team: PER\n---\n",
    );
    let b = resolve(&two.ctx(), two.vault(), Some("mine"));
    assert_eq!(b.team_id.as_deref(), Some(PERSONAL_TEAM_ID));
    assert_eq!(b.team_key.as_deref(), Some("PER"));
}

#[test]
fn a_vault_override_keeps_a_personal_workspace_in_hollow() {
    let bed = Bed::tracked();
    bed.write(
        "workspaces/personal/WORKSPACE.md",
        "---\ntitle: \"Personal\"\ntasks_source: vault\n---\n",
    );
    let c = bed.ctx();
    let b = resolve(&c, bed.vault(), Some("personal"));
    assert_eq!(
        (b.owner, b.inherited, b.chosen),
        (OwnerKind::Vault, false, true)
    );
    assert_eq!(label(&b, &ItemFacts::default()).text, "Stored in Hollow");
    let tracked = resolve(&c, bed.vault(), Some("proj-a"));
    assert_eq!(
        (tracked.owner, tracked.inherited),
        (OwnerKind::Tracker, true)
    );
    assert_eq!(tracked.tracker_project.as_deref(), Some("p1"));
}

// ── One owner per field ─────────────────────────────────────────────────

#[test]
fn owner_of_is_the_engines_field_split() {
    let bed = Bed::tracked();
    let tracked = resolve(&bed.ctx(), bed.vault(), Some("proj-a"));
    let mut keys: Vec<&str> = Field::ALL.iter().map(|f| f.key()).collect();
    keys.extend(READ_ONLY_KEYS);
    keys.extend(IDENTITY_KEYS);
    keys.extend(PROJECT_FIELDS);
    keys.extend([
        "due_time",
        "duration_min",
        "recurrence",
        "section",
        "claimed_by",
        "claimed_at",
        "priority",
        "current_state",
        "desired_state",
        "non_goals",
        "success_evidence",
        "goal_urls",
        "my_own_note",
        "tasks_source",
        "status_type",
        "scope",
    ]);
    for (kind, name) in [(ItemKind::Task, "task"), (ItemKind::Project, "project")] {
        for key in &keys {
            let owned = tracker_sync::owned_field(name, key);
            // The engine's field maps admit exactly the keys Hollow sends.
            assert_eq!(sends(&tracked, kind, key), owned, "{name}.{key}");
            if owned {
                assert_eq!(
                    owner_of(&tracked, kind, key),
                    Owner::Tracker("linear".into()),
                    "{name}.{key}"
                );
            }
            if tracker_sync::local_field(name, key) {
                assert_eq!(
                    owner_of(&tracked, kind, key),
                    Owner::Vault,
                    "{name}.{key} is vault-only"
                );
            }
        }
    }
    for key in ["due_time", "claimed_by", "my_own_note", "tasks_source"] {
        assert_eq!(
            owner_of(&tracked, ItemKind::Task, key),
            Owner::Vault,
            "{key}"
        );
    }
    for key in READ_ONLY_KEYS.iter().chain(IDENTITY_KEYS) {
        assert_eq!(
            owner_of(&tracked, ItemKind::Task, key),
            Owner::Tracker("linear".into())
        );
        assert!(
            !sends(&tracked, ItemKind::Task, key),
            "{key} is never pushed"
        );
    }
    // A vault binding owns everything; a reference is read-only throughout.
    let vault = Binding::vault(Role::Tasks);
    assert_eq!(owner_of(&vault, ItemKind::Task, "title"), Owner::Vault);
    assert!(!sends(&vault, ItemKind::Task, "title"));
    let reference = Binding::reference(
        &Reference {
            connector: "notion-official".into(),
            label: None,
        },
        Some("Notion"),
    );
    assert_eq!(
        owner_of(&reference, ItemKind::Task, "title"),
        Owner::ReadOnly("notion-official".into())
    );
    // Lost access keeps the tracker's fields the tracker's.
    let mut c = bed.ctx();
    c.access.failures = 3;
    let lost = resolve(&c, bed.vault(), Some("proj-a"));
    assert!(!lost.reachable);
    assert_eq!(
        owner_of(&lost, ItemKind::Task, "title"),
        Owner::Tracker("linear".into())
    );
}

// ── Labels ──────────────────────────────────────────────────────────────

#[test]
fn labels_say_where_an_item_lives_and_lost_access_never_says_stored() {
    let bed = Bed::tracked();
    let mirrored = ItemFacts::from_text(TASK);
    assert!(mirrored.mirrored());
    let c = bed.ctx();
    let tracked = label(&resolve(&c, bed.vault(), None), &mirrored);
    assert_eq!(tracked.kind, LabelKind::Tracked);
    assert_eq!(tracked.text, "Tracked in Linear");
    assert_eq!(tracked.open_label.as_deref(), Some("Open in Linear"));
    assert_eq!(
        tracked.url.as_deref(),
        Some("https://linear.app/example/issue/ENG-1")
    );
    let plain = label(
        &Binding::vault(Role::Tasks),
        &ItemFacts::from_text("---\ntitle: x\n---\n"),
    );
    assert_eq!(
        (plain.kind, plain.text.as_str()),
        (LabelKind::Stored, "Stored in Hollow")
    );
    let reference = Binding::reference(
        &Reference {
            connector: "notion-official".into(),
            label: Some("Notion".into()),
        },
        None,
    );
    assert_eq!(
        label(&reference, &ItemFacts::default()).text,
        "Read-only from Notion"
    );
    // Key revoked, or the last passes failed: unreachable since the last read.
    for access in [
        Access {
            key: KeyState::Missing,
            last_read: Some("2026-11-18T14:40:00Z".into()),
            failures: 0,
        },
        Access {
            key: KeyState::Saved,
            last_read: Some("2026-11-18T14:40:00Z".into()),
            failures: 2,
        },
    ] {
        let mut c = bed.ctx();
        c.access = access;
        let l = label(&resolve(&c, bed.vault(), Some("proj-a")), &mirrored);
        assert_eq!(l.kind, LabelKind::Unreachable);
        assert!(
            l.text.starts_with("Linear unreachable since "),
            "{}",
            l.text
        );
        assert_eq!(l.since.as_deref(), Some("2026-11-18T14:40:00Z"));
        assert_ne!(l.text, "Stored in Hollow");
    }
    // The tracker block removed under a mirror: still unreachable, not stored.
    let gone = Bed::new(json!({}));
    let l = label(&resolve(&gone.ctx(), gone.vault(), None), &mirrored);
    assert_eq!(
        (l.kind, l.text.as_str()),
        (LabelKind::Unreachable, "Linear unreachable")
    );
    // Chosen: stored in Hollow, with where it was tracked.
    let chosen =
        Bed::new(json!({ "tracker": tracker_block(), "sources": {"tasks": {"owner": "vault"}} }));
    let l = label(&resolve(&chosen.ctx(), chosen.vault(), None), &mirrored);
    assert_eq!(l.kind, LabelKind::Stored);
    assert_eq!(l.was_tracked.as_deref(), Some("Was tracked in Linear"));
    assert_eq!(
        l.was_tracked_url.as_deref(),
        Some("https://linear.app/example/issue/ENG-1")
    );
}

#[test]
fn status_has_three_role_rows_and_flags_a_reference_missing_from_the_library() {
    let bed = Bed::tracked();
    bed.write(
        "workspaces/personal/WORKSPACE.md",
        "---\ntitle: \"Personal\"\ntasks_source: vault\n---\n",
    );
    let mut c = bed.ctx();
    c.sources.references = vec![
        Reference {
            connector: "notion-official".into(),
            label: None,
        },
        Reference {
            connector: "gone".into(),
            label: None,
        },
    ];
    let facts = LedgerFacts::read(bed.vault()).unwrap();
    let library = vec![("notion-official".to_string(), "Notion".to_string())];
    let s = status(&c, bed.vault(), Some(&facts), Some(library.as_slice()));
    let roles: Vec<Role> = s.rows.iter().map(|r| r.role).collect();
    assert_eq!(roles, vec![Role::Tasks, Role::Knowledge, Role::Personal]);
    assert_eq!(s.rows[0].label.text, "Tracked in Linear");
    assert_eq!(s.rows[0].last_read.as_deref(), Some("2026-11-18T14:40:00Z"));
    assert_eq!(s.rows[1].binding.owner, OwnerKind::Vault);
    assert_eq!(s.rows[2].label.text, "Stored in Hollow");
    assert_eq!(s.references[0].label.text, "Read-only from Notion");
    assert!(!s.references[1].in_library);
    assert!(s
        .problems
        .iter()
        .any(|p| p.starts_with("gone isn't in the connectors library")));
    let ids: Vec<&str> = s.workspaces.iter().map(|w| w.id.as_str()).collect();
    assert_eq!(ids, vec!["personal", "proj-a"]);
    assert_eq!(s.workspaces[0].label.text, "Stored in Hollow");
    assert_eq!(s.workspaces[1].label.text, "Tracked in Linear");
    // The keys editors hold come from the same rule.
    assert!(s.tracker_task_keys.iter().any(|k| k == "title"));
    assert!(s.tracker_task_keys.iter().any(|k| k == "creator"));
    assert!(!s.sent_task_keys.iter().any(|k| k == "creator"));
    assert!(!s.tracker_task_keys.iter().any(|k| k == "due_time"));
    assert_eq!(s.sent_project_keys.len(), PROJECT_FIELDS.len());
}

// ── Owner switches ──────────────────────────────────────────────────────

#[tokio::test]
async fn a_switch_is_refused_while_a_queued_edit_or_conflict_is_open() {
    for open in ["outbox", "conflict"] {
        let bed = Bed::tracked();
        let mut ledger = bed.ledger();
        if open == "outbox" {
            ledger.outbox.insert(
                "i1".into(),
                Outbox {
                    values: [("title".to_string(), json!("Queued"))].into(),
                    ..Outbox::default()
                },
            );
        } else {
            ledger
                .records
                .get_mut("i1")
                .unwrap()
                .conflicts
                .push(Conflict {
                    field: "title".into(),
                    local: json!("Mine"),
                    remote: json!("Theirs"),
                    at: "2026-11-18T14:40:00Z".into(),
                });
        }
        tracker_sync::save(bed.vault(), &ledger).unwrap();
        let before = migration::tree(bed.vault());
        let config = bed.config();
        let plan = migration::plan(&bed.env, &global(OwnerKind::Vault)).unwrap();
        assert!(
            plan.refused
                .as_deref()
                .is_some_and(|r| r.contains("are open")),
            "{open}"
        );
        assert_eq!(plan.queued_edits + plan.conflicts, 1, "{open}");
        let err = migration::apply(&bed.env, &global(OwnerKind::Vault))
            .await
            .unwrap_err();
        assert!(err.contains("Nothing was changed"), "{err}");
        assert_eq!(migration::tree(bed.vault()), before, "{open}");
        assert_eq!(bed.config(), config, "{open}");
        assert!(migration::list(&bed.env).is_empty());
    }
}

#[tokio::test]
async fn a_switch_to_hollow_keeps_mirrors_marks_the_workspace_and_rolls_back_byte_exact() {
    let bed = Bed::tracked();
    let before = migration::tree(bed.vault());
    let config = bed.config();
    let plan = migration::plan(&bed.env, &global(OwnerKind::Vault)).unwrap();
    assert_eq!(plan.refused, None);
    assert_eq!(plan.from, OwnerKind::Tracker);
    assert_eq!((plan.generation_from, plan.generation_to), (0, 1));
    assert_eq!(
        (
            plan.mirrored_tasks,
            plan.mirrored_projects,
            plan.tracker_writes
        ),
        (1, 1, 0)
    );
    assert_eq!(plan.files.len(), 1);
    assert_eq!(plan.files[0].path, "workspaces/proj-a/PROJECT.md");
    assert_eq!(
        plan.config_change.as_deref(),
        Some("sources.tasks.owner: tracker to vault")
    );
    // The dry run wrote nothing.
    assert_eq!(migration::tree(bed.vault()), before);
    assert_eq!(bed.config(), config);

    let out = migration::apply(&bed.env, &global(OwnerKind::Vault))
        .await
        .unwrap();
    let record = out.record.unwrap();
    assert_eq!(record.phase, Phase::Applied);
    // Mirrors kept byte for byte; the workspace says where it was tracked.
    assert_eq!(bed.read("tasks/tracker-i1.md"), TASK);
    let project = bed.read("workspaces/proj-a/PROJECT.md");
    assert!(project.starts_with(PROJECT));
    assert!(project.contains("Was tracked in Linear: [P-ENG-1](https://linear.app/example/project/p1) <!-- hollow:was-tracked -->"));
    let ledger = bed.ledger();
    assert_eq!((ledger.generation, ledger.stored_in_hollow), (1, true));
    assert_eq!(ledger.records.len(), 2, "no record dropped");
    let c = bed.ctx();
    assert!(c.sources.tasks_set);
    assert_eq!(global_tasks(&c).owner, OwnerKind::Vault);
    let b = resolve(&c, bed.vault(), Some("proj-a"));
    assert_eq!(
        b.was_tracked.as_deref(),
        Some("https://linear.app/example/project/p1")
    );
    let l = label(&b, &ItemFacts::from_text(&project));
    assert_eq!(
        (l.kind, l.was_tracked.as_deref()),
        (LabelKind::Stored, Some("Was tracked in Linear"))
    );
    // Applying it again changes nothing.
    let again = migration::apply(&bed.env, &global(OwnerKind::Vault))
        .await
        .unwrap();
    assert!(again.record.is_none() && again.plan.nothing_to_do);

    let rolled = migration::rollback(&bed.env, &record.id).await.unwrap();
    assert_eq!(rolled.phase, Phase::RolledBack);
    assert_eq!(
        migration::tree(bed.vault()),
        before,
        "every vault byte is back, the ledger included"
    );
    assert_eq!(bed.config(), config, "config.json is back byte for byte");
    assert!(migration::rollback(&bed.env, &record.id)
        .await
        .unwrap_err()
        .contains("already"));
}

#[tokio::test]
async fn an_interrupted_apply_resumes_and_still_rolls_back_byte_exact() {
    for stop in [Stop::AfterFiles, Stop::AfterLedger] {
        let bed = Bed::tracked();
        let before = migration::tree(bed.vault());
        let config = bed.config();
        assert!(
            migration::apply_with(&bed.env, &global(OwnerKind::Vault), Some(stop))
                .await
                .is_err()
        );
        let status = migration::status(&bed.env);
        let id = status
            .unfinished
            .clone()
            .expect("the switch stopped part way");
        assert_eq!(status.switches[0].phase, Phase::Applying);
        // Another switch waits for this one.
        let other = migration::apply(&bed.env, &global(OwnerKind::Tracker))
            .await
            .unwrap_err();
        assert!(other.contains("stopped part way"), "{other}");
        let out = migration::apply(&bed.env, &global(OwnerKind::Vault))
            .await
            .unwrap();
        assert!(out.resumed, "{stop:?}");
        let record = out.record.unwrap();
        assert_eq!(
            (record.id.as_str(), record.phase),
            (id.as_str(), Phase::Applied)
        );
        let ledger = bed.ledger();
        assert_eq!(
            (ledger.generation, ledger.stored_in_hollow),
            (1, true),
            "{stop:?}"
        );
        assert_eq!(
            bed.read("workspaces/proj-a/PROJECT.md")
                .matches(WAS_TRACKED_MARK)
                .count(),
            1
        );
        assert_eq!(global_tasks(&bed.ctx()).owner, OwnerKind::Vault);
        assert!(migration::unfinished(&bed.env).is_none());
        migration::rollback(&bed.env, &id).await.unwrap();
        assert_eq!(migration::tree(bed.vault()), before, "{stop:?}");
        assert_eq!(bed.config(), config, "{stop:?}");
    }
    // An interrupted switch can be rolled back without finishing it.
    let bed = Bed::tracked();
    let before = migration::tree(bed.vault());
    assert!(
        migration::apply_with(&bed.env, &global(OwnerKind::Vault), Some(Stop::AfterLedger))
            .await
            .is_err()
    );
    let id = migration::unfinished(&bed.env).unwrap().id;
    migration::rollback(&bed.env, &id).await.unwrap();
    assert_eq!(migration::tree(bed.vault()), before);
}

#[tokio::test]
async fn only_the_newest_switch_can_be_rolled_back() {
    let bed = Bed::tracked();
    let a = migration::apply(&bed.env, &global(OwnerKind::Vault))
        .await
        .unwrap()
        .record
        .unwrap();
    let b = migration::apply(&bed.env, &global(OwnerKind::Tracker))
        .await
        .unwrap()
        .record
        .unwrap();
    let err = migration::rollback(&bed.env, &a.id).await.unwrap_err();
    assert!(err.contains("newer switch"), "{err}");
    migration::rollback(&bed.env, &b.id).await.unwrap();
    migration::rollback(&bed.env, &a.id).await.unwrap();
}

#[tokio::test]
async fn one_workspace_switches_and_a_tracked_project_alone_is_refused() {
    let bed = Bed::tracked();
    let to_vault = Target {
        workspace: Some("personal".into()),
        to: OwnerKind::Vault,
    };
    let plan = migration::plan(&bed.env, &to_vault).unwrap();
    assert_eq!(plan.files[0].path, "workspaces/personal/WORKSPACE.md");
    assert!(plan.config_change.is_none());
    let out = migration::apply(&bed.env, &to_vault).await.unwrap();
    assert!(bed
        .read("workspaces/personal/WORKSPACE.md")
        .contains("tasks_source: vault"));
    assert_eq!(
        bed.ledger().generation,
        1,
        "every switch is a new generation"
    );
    assert!(!bed.ledger().stored_in_hollow, "the rest stay tracked");
    let b = resolve(&bed.ctx(), bed.vault(), Some("personal"));
    assert_eq!((b.owner, b.inherited), (OwnerKind::Vault, false));
    // And back: the override goes and it inherits Linear again.
    let back = Target {
        workspace: Some("personal".into()),
        to: OwnerKind::Tracker,
    };
    migration::apply(&bed.env, &back).await.unwrap();
    assert!(!bed
        .read("workspaces/personal/WORKSPACE.md")
        .contains("tasks_source"));
    assert_eq!(bed.ledger().generation, 2);
    let b = resolve(&bed.ctx(), bed.vault(), Some("personal"));
    assert_eq!((b.owner, b.inherited), (OwnerKind::Tracker, true));
    assert!(out.record.is_some());
    // A workspace mirroring a tracker project can't leave on its own yet.
    let alone = Target {
        workspace: Some("proj-a".into()),
        to: OwnerKind::Vault,
    };
    let plan = migration::plan(&bed.env, &alone).unwrap();
    assert!(plan
        .refused
        .as_deref()
        .is_some_and(|r| r.contains("mirrors a Linear project")));
    assert!(migration::apply(&bed.env, &alone).await.is_err());
    // Nor can one workspace be tracked while Settings keeps tasks in Hollow.
    let vault_bed =
        Bed::new(json!({ "tracker": tracker_block(), "sources": {"tasks": {"owner": "vault"}} }));
    vault_bed.write("workspaces/w/WORKSPACE.md", "---\ntitle: \"W\"\n---\n");
    let plan = migration::plan(
        &vault_bed.env,
        &Target {
            workspace: Some("w".into()),
            to: OwnerKind::Tracker,
        },
    )
    .unwrap();
    assert!(plan.refused.is_some());
}

#[tokio::test]
async fn a_switch_to_the_tracker_never_widens_settings() {
    let bed = Bed::new(json!({}));
    bed.write("workspaces/w/WORKSPACE.md", "---\ntitle: \"W\"\n---\n");
    let plan = migration::plan(&bed.env, &global(OwnerKind::Tracker)).unwrap();
    assert!(plan
        .refused
        .as_deref()
        .is_some_and(|r| r.contains("Choose a task tracker")));
    assert!(migration::apply(&bed.env, &global(OwnerKind::Tracker))
        .await
        .is_err());
    // Nothing to do when tasks already live where asked.
    let plan = migration::plan(&bed.env, &global(OwnerKind::Vault)).unwrap();
    assert!(plan.nothing_to_do && plan.refused.is_none());
    let _: PathBuf = bed.env.migrations.clone();
}
