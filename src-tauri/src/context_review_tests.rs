//! CX20 tests: the list reads only; Accept writes one file into the personal
//! clone and refuses an existing target, a changed file, a secret and a block
//! flag; Project writes nothing; Org builds CX8's request (no push); Skip
//! hides; there is no batch Accept. Every folder is a temp dir.

use super::*;
use crate::context_scan::{Agent, Flag, ScanBudget, ScanRoot};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

fn canon(dir: &tempfile::TempDir) -> PathBuf {
    std::fs::canonicalize(dir.path()).unwrap()
}

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// Every file below `root`, with its bytes.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in read.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(p.clone(), std::fs::read(&p).unwrap_or_default());
            }
        }
    }
    out
}

fn fake_key() -> String {
    format!("{}{}", "sk-ant-", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4")
}

/// A temp Mac: a `.claude` folder, a project, the two clones, staging and
/// the card's record.
struct Rig {
    _tmp: tempfile::TempDir,
    base: PathBuf,
    staging: Staging,
}

impl Rig {
    fn new() -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let base = canon(&tmp);
        for d in ["claude", "proj", "personal", "org", "app"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        let staging = Staging::new(base.join("app").join("Imports"));
        Rig {
            _tmp: tmp,
            base,
            staging,
        }
    }
    fn p(&self, name: &str) -> PathBuf {
        self.base.join(name)
    }
    fn scan(&self) -> ScanManifest {
        let roots = vec![
            ScanRoot {
                path: self.p("claude"),
                scope: Scope::User,
                label: "Claude Code".into(),
                agent: Some(Agent::ClaudeCode),
            },
            ScanRoot {
                path: self.p("proj"),
                scope: Scope::Project,
                label: "proj".into(),
                agent: None,
            },
        ];
        let id = crate::session_import::staging::mint_id();
        context_scan::scan(
            &self.staging,
            &id,
            &roots,
            &[],
            &ScanBudget::default(),
            &AtomicBool::new(false),
        )
        .unwrap()
    }
    fn decisions(&self) -> PathBuf {
        self.p("app").join(DECISIONS_FILE)
    }
    fn pending(&self) -> PathBuf {
        self.p("app").join("pending.json")
    }
    fn ctx<'a>(&'a self, roots: Roots<'a>, decisions: &'a Path, pending: &'a Path) -> Ctx<'a> {
        Ctx {
            roots,
            decisions,
            pending_file: pending,
            vault: None,
        }
    }
}

fn by_rel<'a>(m: &'a ScanManifest, rel: &str) -> &'a Proposal {
    m.proposals
        .iter()
        .find(|p| p.rel_path == rel)
        .unwrap_or_else(|| panic!("{rel} was not found"))
}

fn no_push(_: &Request) -> Result<Outcome, String> {
    panic!("nothing may be pushed here")
}

#[test]
fn the_list_reads_only_and_proposes_a_layer_per_scope() {
    let rig = Rig::new();
    write(
        &rig.p("claude"),
        "CLAUDE.md",
        "Use tabs. Run the tests before a commit.\n",
    );
    write(
        &rig.p("claude"),
        "commands/ship.md",
        "Ship the branch after the gate passes.\n",
    );
    write(
        &rig.p("proj"),
        "AGENTS.md",
        "This project builds with make.\n",
    );
    write(
        &rig.p("proj"),
        ".cursor/rules/style.mdc",
        "Prefer small functions.\n",
    );
    let m = rig.scan();
    crate::memory_homes::queue_org_fact_at(
        &rig.pending(),
        "Refunds",
        "Refunds go to the original payment.",
        "business-rules",
        Some("chat-1"),
    )
    .unwrap();
    let pending = crate::memory_homes::pending_org_at(&rig.pending()).unwrap();
    let before = snapshot(&rig.base);
    let (personal, org) = (rig.p("personal"), rig.p("org"));
    let roots = Roots {
        personal: Some(&personal),
        org: Some(&org),
        machine: None,
    };
    let decisions = read_decisions(&rig.decisions()).unwrap();
    let out = list(Some(&m), &pending, &decisions, &roots);
    assert_eq!(snapshot(&rig.base), before, "the list wrote something");
    assert!(!rig.decisions().exists());
    assert_eq!(out.scan_id.as_deref(), Some(m.scan_id.as_str()));
    assert_eq!(out.items.len(), 5);

    let item = |rel: &str| out.items.iter().find(|i| i.rel_path == rel).unwrap();
    let claude = item("CLAUDE.md");
    assert_eq!(claude.proposed_layer, ContextReviewLayer::Personal);
    assert_eq!(claude.agent.as_deref(), Some("claudeCode"));
    assert_eq!(claude.scope.as_deref(), Some("user"));
    let personal_choice = &claude.choices[0];
    assert_eq!(
        personal_choice.target.as_deref(),
        Some("memory/imported/claude-code/CLAUDE.md")
    );
    assert_eq!(personal_choice.disabled, None);
    assert_eq!(
        claude.choices[1].disabled.as_deref(),
        Some("not-a-project-file")
    );
    assert!(claude.choices[2].outward);

    let rule = item(".cursor/rules/style.mdc");
    assert_eq!(rule.proposed_layer, ContextReviewLayer::Project);
    assert_eq!(
        rule.choices[0].target.as_deref(),
        Some("memory/imported/cursor/proj/cursor/rules/style.mdc.md")
    );
    assert_eq!(
        rule.choices[1].target.as_deref(),
        Some(rig.p("proj").to_str().unwrap())
    );
    assert_eq!(
        rule.choices[2].target.as_deref(),
        Some("ai-tools/imported/cursor/cursor/rules/style.mdc.md")
    );

    let fact = out.items.iter().find(|i| i.source == "org").unwrap();
    assert_eq!(fact.proposed_layer, ContextReviewLayer::Org);
    assert_eq!(fact.kind, "orgFact");
    assert_eq!(fact.chat.as_deref(), Some("chat-1"));
    assert_eq!(fact.choices[2].disabled, None);
    assert_eq!(fact.choices[0].disabled.as_deref(), Some("org-only"));

    // Without the clones, Accept is off and says why.
    let bare = list(Some(&m), &pending, &decisions, &Roots::default());
    let claude = bare
        .items
        .iter()
        .find(|i| i.rel_path == "CLAUDE.md")
        .unwrap();
    assert_eq!(
        claude.choices[0].disabled.as_deref(),
        Some("no-personal-root")
    );
    assert_eq!(claude.choices[2].disabled.as_deref(), Some("no-org-root"));
}

#[test]
fn accept_copies_one_file_into_the_personal_clone_and_never_overwrites() {
    let rig = Rig::new();
    let text = "Use tabs. Run the tests before a commit.\n";
    write(&rig.p("claude"), "CLAUDE.md", text);
    write(
        &rig.p("claude"),
        "commands/ship.md",
        "Ship the branch after the gate passes.\n",
    );
    let m = rig.scan();
    let personal = rig.p("personal");
    let (decisions, pending) = (rig.decisions(), rig.pending());
    let roots = Roots {
        personal: Some(&personal),
        ..Roots::default()
    };
    let ctx = rig.ctx(roots, &decisions, &pending);
    let scanned_before = snapshot(&rig.p("claude"));
    let found = Found::Scan(by_rel(&m, "CLAUDE.md").clone());
    let done = accept(&ctx, &found, ContextReviewLayer::Personal, &no_push).unwrap();
    assert!(done.written);
    assert_eq!(
        done.target.as_deref(),
        Some("memory/imported/claude-code/CLAUDE.md")
    );
    let after = snapshot(&personal);
    assert_eq!(after.len(), 1, "exactly one file: {after:?}");
    assert_eq!(
        std::fs::read_to_string(personal.join("memory/imported/claude-code/CLAUDE.md")).unwrap(),
        text
    );
    assert_eq!(snapshot(&rig.p("claude")), scanned_before);
    // It leaves the list, and a second Accept is refused.
    let d = read_decisions(&decisions).unwrap();
    let out = list(Some(&m), &[], &d, &roots);
    assert_eq!(out.accepted, 1);
    assert!(out.items.iter().all(|i| i.rel_path != "CLAUDE.md"));
    let err = accept(&ctx, &found, ContextReviewLayer::Personal, &no_push).unwrap_err();
    assert!(err.contains("already accepted"), "{err}");

    // A file already at the target is never replaced.
    write(
        &personal,
        "memory/imported/claude-code/commands/ship.md",
        "mine\n",
    );
    let ship = Found::Scan(by_rel(&m, "commands/ship.md").clone());
    let listed = list(Some(&m), &[], &d, &roots);
    let row = listed
        .items
        .iter()
        .find(|i| i.rel_path == "commands/ship.md")
        .unwrap();
    assert_eq!(row.choices[0].disabled.as_deref(), Some("target-exists"));
    let err = accept(&ctx, &ship, ContextReviewLayer::Personal, &no_push).unwrap_err();
    assert!(err.contains("never overwrites"), "{err}");
    assert_eq!(
        std::fs::read_to_string(personal.join("memory/imported/claude-code/commands/ship.md"))
            .unwrap(),
        "mine\n"
    );
}

#[test]
fn accept_refuses_a_changed_file_a_secret_a_block_flag_and_no_clone() {
    let rig = Rig::new();
    write(
        &rig.p("claude"),
        "CLAUDE.md",
        "Use tabs. Run the tests before a commit.\n",
    );
    write(
        &rig.p("claude"),
        "memory/keys.md",
        &format!("# Keys\nkey = {}\n", fake_key()),
    );
    write(
        &rig.p("claude"),
        "commands/ship.md",
        "Ship the branch after the gate passes.\n",
    );
    let m = rig.scan();
    let personal = rig.p("personal");
    let (decisions, pending) = (rig.decisions(), rig.pending());
    let ctx = rig.ctx(
        Roots {
            personal: Some(&personal),
            ..Roots::default()
        },
        &decisions,
        &pending,
    );

    // Changed since the scan.
    write(&rig.p("claude"), "CLAUDE.md", "Use spaces now.\n");
    let changed = Found::Scan(by_rel(&m, "CLAUDE.md").clone());
    let err = accept(&ctx, &changed, ContextReviewLayer::Personal, &no_push).unwrap_err();
    assert!(err.contains("changed since the scan"), "{err}");

    // Secret-like: blocked, whatever the layer.
    let secret = by_rel(&m, "memory/keys.md");
    assert!(secret.secret_like);
    for layer in [ContextReviewLayer::Personal, ContextReviewLayer::Org] {
        let err = accept(&ctx, &Found::Scan(secret.clone()), layer, &no_push).unwrap_err();
        assert!(err.contains("secret"), "{err}");
    }

    // A blocking rule hit.
    let mut flagged = by_rel(&m, "commands/ship.md").clone();
    flagged.flags.push(Flag {
        rule: "test-block".into(),
        group: "injection".into(),
        severity: "block".into(),
        line: 1,
    });
    let item = scan_item(&flagged, &ctx.roots, false);
    assert_eq!(item.blocked.as_deref(), Some("block-flag"));
    assert_eq!(item.blocked_rule.as_deref(), Some("test-block (line 1)"));
    let err = accept(
        &ctx,
        &Found::Scan(flagged),
        ContextReviewLayer::Personal,
        &no_push,
    )
    .unwrap_err();
    assert!(err.contains("blocking rule"), "{err}");

    // No personal clone set up.
    let bare = rig.ctx(Roots::default(), &decisions, &pending);
    let ship = Found::Scan(by_rel(&m, "commands/ship.md").clone());
    let err = accept(&bare, &ship, ContextReviewLayer::Personal, &no_push).unwrap_err();
    assert!(err.contains("Settings, Sources, Context"), "{err}");

    assert!(snapshot(&personal).is_empty(), "something was written");
    assert!(read_decisions(&decisions).unwrap().decisions.is_empty());
}

#[test]
fn project_accept_writes_nothing() {
    let rig = Rig::new();
    write(
        &rig.p("proj"),
        "CLAUDE.md",
        "This project builds with make.\n",
    );
    let m = rig.scan();
    let (personal, org) = (rig.p("personal"), rig.p("org"));
    let (decisions, pending) = (rig.decisions(), rig.pending());
    let ctx = rig.ctx(
        Roots {
            personal: Some(&personal),
            org: Some(&org),
            machine: None,
        },
        &decisions,
        &pending,
    );
    let before = snapshot(&rig.base);
    let p = by_rel(&m, "CLAUDE.md").clone();
    let done = accept(&ctx, &Found::Scan(p), ContextReviewLayer::Project, &no_push).unwrap();
    assert!(!done.written && done.pr_url.is_none());
    assert_eq!(
        done.target.as_deref(),
        Some(rig.p("proj").to_str().unwrap())
    );
    let after = snapshot(&rig.base);
    // Only the card's own record in app data changed.
    let changed: Vec<&PathBuf> = after
        .keys()
        .filter(|k| before.get(*k) != after.get(*k))
        .collect();
    assert_eq!(changed, vec![&decisions]);
}

#[test]
fn org_accept_builds_the_cx8_request_and_pushes_nothing_itself() {
    let rig = Rig::new();
    write(
        &rig.p("claude"),
        "CLAUDE.md",
        "Use tabs. Run the tests before a commit.\n",
    );
    let m = rig.scan();
    let org = rig.p("org");
    let (decisions, pending) = (rig.decisions(), rig.pending());
    let fact = crate::memory_homes::queue_org_fact_at(
        &pending,
        "Refunds",
        "Refunds go to the original payment.",
        "business-rules",
        Some("chat-7"),
    )
    .unwrap();
    let ctx = rig.ctx(
        Roots {
            org: Some(&org),
            ..Roots::default()
        },
        &decisions,
        &pending,
    );
    let seen: RefCell<Vec<Request>> = RefCell::new(Vec::new());
    let fake = |req: &Request| {
        seen.borrow_mut().push(req.clone());
        Ok::<Outcome, String>(Outcome {
            url: "https://github.com/o/r/pull/9".into(),
            branch: "hollow/refunds-1".into(),
            login: "someone".into(),
            identity: "teammate".into(),
            status: "waiting".into(),
            reason: "needs owner approval".into(),
            warnings: Vec::new(),
        })
    };

    // A queued fact: its own request, attended, and it leaves the queue.
    let done = accept(
        &ctx,
        &Found::Org(fact.clone()),
        ContextReviewLayer::Org,
        &fake,
    )
    .unwrap();
    assert_eq!(
        done.pr_url.as_deref(),
        Some("https://github.com/o/r/pull/9")
    );
    assert_eq!(done.pr_status.as_deref(), Some("waiting"));
    {
        let reqs = seen.borrow();
        assert_eq!(reqs.len(), 1);
        let req = &reqs[0];
        assert!(req.attended);
        assert_eq!(req.chat.as_deref(), Some("chat-7"));
        assert_eq!(req.files.len(), 1);
        assert_eq!(req.files[0].path, "business-rules/refunds.md");
        assert!(req.files[0]
            .content
            .contains("- Refunds go to the original payment."));
        context_propose::validate(req).unwrap();
    }
    assert!(crate::memory_homes::pending_org_at(&pending)
        .unwrap()
        .is_empty());
    assert!(snapshot(&org).is_empty(), "the org clone is read only");

    // A scanned file: one new file under ai-tools/imported/.
    let p = by_rel(&m, "CLAUDE.md").clone();
    accept(&ctx, &Found::Scan(p), ContextReviewLayer::Org, &fake).unwrap();
    {
        let reqs = seen.borrow();
        let req = &reqs[1];
        assert!(req.attended && req.chat.is_none());
        assert_eq!(req.files[0].path, "ai-tools/imported/claude-code/CLAUDE.md");
        assert_eq!(
            req.files[0].content,
            "Use tabs. Run the tests before a commit.\n"
        );
        context_propose::validate(req).unwrap();
    }
    let d = read_decisions(&decisions).unwrap();
    // The submitted fact left the queue and its record went with it; the
    // scanned file's Accept stays recorded.
    assert_eq!(d.decisions.len(), 1);
    assert!(d
        .decisions
        .iter()
        .all(|x| x.url.as_deref() == Some("https://github.com/o/r/pull/9")));

    // A failed pull request records nothing.
    let rig2 = Rig::new();
    write(&rig2.p("claude"), "CLAUDE.md", "Another clean file.\n");
    let m2 = rig2.scan();
    let (d2, p2) = (rig2.decisions(), rig2.pending());
    let org2 = rig2.p("org");
    let ctx2 = rig2.ctx(
        Roots {
            org: Some(&org2),
            ..Roots::default()
        },
        &d2,
        &p2,
    );
    let fail = |_: &Request| Err::<Outcome, String>("push refused".into());
    let err = accept(
        &ctx2,
        &Found::Scan(by_rel(&m2, "CLAUDE.md").clone()),
        ContextReviewLayer::Org,
        &fail,
    )
    .unwrap_err();
    assert_eq!(err, "push refused");
    // The opening marker was cleared: nothing is recorded.
    assert!(read_decisions(&d2).unwrap().decisions.is_empty());
}

#[test]
fn skip_hides_an_item_until_unskip() {
    let rig = Rig::new();
    write(
        &rig.p("claude"),
        "CLAUDE.md",
        "Use tabs. Run the tests before a commit.\n",
    );
    let m = rig.scan();
    let id = by_rel(&m, "CLAUDE.md").id.clone();
    let file = rig.decisions();
    assert!(skip_at(&file, &id).unwrap());
    assert!(
        !skip_at(&file, &id).unwrap(),
        "a second Skip changes nothing"
    );
    let out = list(
        Some(&m),
        &[],
        &read_decisions(&file).unwrap(),
        &Roots::default(),
    );
    assert!(out.items.iter().find(|i| i.id == id).unwrap().skipped);
    assert!(unskip_at(&file, &id).unwrap());
    let out = list(
        Some(&m),
        &[],
        &read_decisions(&file).unwrap(),
        &Roots::default(),
    );
    assert!(!out.items.iter().find(|i| i.id == id).unwrap().skipped);
    assert!(!unskip_at(&file, &id).unwrap());
    // Ids are checked before anything is recorded.
    assert!(skip_at(&file, "../etc").is_err());
    assert!(skip_at(&file, "ctx-ZZ").is_err());
    // A damaged record is an error, never silently emptied.
    std::fs::write(&file, "{not json").unwrap();
    assert!(read_decisions(&file).is_err());
    assert!(skip_at(&file, &id).is_err());
}

#[test]
fn finds_the_newest_scan_and_items_by_id() {
    let rig = Rig::new();
    assert!(latest_manifest(&rig.staging).is_none());
    write(&rig.p("claude"), "CLAUDE.md", "Use tabs.\n");
    let m = rig.scan();
    let latest = latest_manifest(&rig.staging).unwrap();
    assert_eq!(latest.scan_id, m.scan_id);
    let id = &m.proposals[0].id;
    assert!(matches!(find(Some(&m), &[], id), Ok(Found::Scan(_))));
    assert!(find(Some(&m), &[], "ctx-000000000000").is_err());
    assert!(find(None, &[], "nonsense").is_err());
}

/// There is no batch Accept: the command takes one id, the module's Accept
/// one item, and the map has no other review command.
#[test]
fn there_is_no_batch_accept() {
    let api = include_str!("commands/context_review_api.rs");
    let start = api.find("pub async fn context_review_accept(").unwrap();
    let sig = &api[start..start + api[start..].find(") ->").unwrap()];
    assert!(sig.contains("id: String"), "{sig}");
    assert!(!sig.contains("Vec<") && !sig.contains("ids"), "{sig}");
    let map = include_str!("../../docs/HOLLOW_APP_COMMAND_MAP.tsv");
    let rows: Vec<&str> = map
        .lines()
        .filter_map(|l| l.split('\t').next())
        .filter(|c| c.starts_with("context_review"))
        .collect();
    assert_eq!(
        rows,
        vec![
            "context_review_list",
            "context_review_accept",
            "context_review_skip",
            "context_review_unskip"
        ]
    );
    // `accept` is typed to one item.
    let _one: fn(
        &Ctx,
        &Found,
        ContextReviewLayer,
        &dyn Fn(&Request) -> Result<Outcome, String>,
    ) -> Result<ContextReviewAccepted, String> = accept;
}

/// Review fix: a later fact with the same title but other text is a new item,
/// listed and accepted, not hidden as "already accepted".
#[test]
fn a_new_fact_with_the_same_title_lists_and_accepts_after_the_first() {
    let rig = Rig::new();
    let org = rig.p("org");
    let (decisions, pending) = (rig.decisions(), rig.pending());
    let ctx = rig.ctx(
        Roots {
            org: Some(&org),
            ..Roots::default()
        },
        &decisions,
        &pending,
    );
    let ok = |_: &Request| {
        Ok::<Outcome, String>(Outcome {
            url: "https://github.com/o/r/pull/1".into(),
            branch: "b".into(),
            login: "me".into(),
            identity: "teammate".into(),
            status: "waiting".into(),
            reason: String::new(),
            warnings: Vec::new(),
        })
    };
    let first = crate::memory_homes::queue_org_fact_at(
        &pending,
        "Refunds",
        "Refunds go to the original payment.",
        "business-rules",
        None,
    )
    .unwrap();
    accept(
        &ctx,
        &Found::Org(first.clone()),
        ContextReviewLayer::Org,
        &ok,
    )
    .unwrap();
    assert!(crate::memory_homes::pending_org_at(&pending)
        .unwrap()
        .is_empty());

    let second = crate::memory_homes::queue_org_fact_at(
        &pending,
        "Refunds",
        "Refunds take five working days.",
        "business-rules",
        None,
    )
    .unwrap();
    assert_eq!(second.id, first.id, "the queue keys a proposal by its path");
    assert_ne!(org_review_id(&second), org_review_id(&first));
    let queued = crate::memory_homes::pending_org_at(&pending).unwrap();
    let out = list(
        None,
        &queued,
        &read_decisions(&decisions).unwrap(),
        &ctx.roots,
    );
    assert_eq!(out.items.len(), 1, "the new fact is listed");
    assert_eq!(out.items[0].id, org_review_id(&second));
    let found = find(None, &queued, &out.items[0].id).unwrap();
    let done = accept(&ctx, &found, ContextReviewLayer::Org, &ok).unwrap();
    assert_eq!(
        done.pr_url.as_deref(),
        Some("https://github.com/o/r/pull/1")
    );
}

/// Review fix: an Org Accept marks its item before pushing, so a second one
/// while the first is opening (or after its record failed) opens nothing.
#[test]
fn a_second_org_accept_while_one_is_opening_is_refused() {
    let rig = Rig::new();
    let org = rig.p("org");
    let (decisions, pending) = (rig.decisions(), rig.pending());
    let fact = crate::memory_homes::queue_org_fact_at(
        &pending,
        "Refunds",
        "Refunds go to the original payment.",
        "business-rules",
        None,
    )
    .unwrap();
    let ctx = rig.ctx(
        Roots {
            org: Some(&org),
            ..Roots::default()
        },
        &decisions,
        &pending,
    );
    let id = org_review_id(&fact);
    mark_opening(&decisions, &id).unwrap();
    let err = accept(
        &ctx,
        &Found::Org(fact.clone()),
        ContextReviewLayer::Org,
        &no_push,
    )
    .unwrap_err();
    assert!(err.contains("being opened"), "{err}");
    assert!(skip_at(&decisions, &id).is_err());
    assert!(mark_opening(&decisions, &id).is_err());
    // A marker older than OPENING_FOR is from a run that never finished.
    change_decisions(&decisions, |all| {
        for d in &mut all.decisions {
            d.at = "2020-01-01T00:00:00+00:00".into();
        }
        Ok(())
    })
    .unwrap();
    assert!(!read_decisions(&decisions).unwrap().in_flight(&id));
}

/// Review fix: no name below a clone is `.`, `..` or empty.
#[test]
fn plain_names_never_become_dot_parts() {
    for bad in [" ..", "..", ".", " . ", "", "...", " ", ". ."] {
        assert_eq!(plain_name(bad), "file", "{bad:?}");
    }
    assert_eq!(plain_name(" .env "), "env");
    assert_eq!(plain_name(".cursor"), "cursor");
    assert_eq!(plain_name("a b?.md"), "a b-.md");
    assert_eq!(plain_rel(".x/ ../y.md"), "x/file/y.md");
}

// ── PX6: Claude Code's auto-memory entries, sorted ──────────────────────

const MEM: &str = "projects/-tmp-app/memory";

/// A temp Claude folder with one auto-memory entry per layer, an index and
/// a plain instructions file (no sort).
fn memory_rig() -> (Rig, ScanManifest) {
    let rig = Rig::new();
    let c = rig.p("claude");
    write(
        &c,
        "CLAUDE.md",
        "Use tabs. Run the tests before a commit.\n",
    );
    write(
        &c,
        &format!("{MEM}/MEMORY.md"),
        "- [Runner](runner.md): where the runner lives\n",
    );
    write(
        &c,
        &format!("{MEM}/runner.md"),
        "---\nname: runner\ndescription: Where the runner lives\ntype: reference\n---\n\nThe mac mini runner keeps its data on the external SSD; check free space first.\n",
    );
    write(
        &c,
        &format!("{MEM}/tone.md"),
        "---\nname: tone\ntype: feedback\n---\n\nHe prefers short answers and dislikes long tables in replies.\n",
    );
    write(
        &c,
        &format!("{MEM}/bindings.md"),
        "---\nname: bindings\ntype: project\n---\n\nShared bindings are generated by ts_bindings.rs into ipc.ts; rebase before the merge.\n",
    );
    write(
        &c,
        &format!("{MEM}/old.md"),
        "---\nname: old\nstatus: superseded\n---\n\nThe old rule about worktrees and their target folders, kept for the record.\n",
    );
    let m = rig.scan();
    (rig, m)
}

fn item_at<'a>(out: &'a ContextReviewList, rel: &str) -> &'a ContextReviewItem {
    out.items
        .iter()
        .find(|i| i.rel_path == rel)
        .unwrap_or_else(|| panic!("{rel} is not listed"))
}

fn choice_of(item: &ContextReviewItem, layer: ContextReviewLayer) -> &ContextReviewChoice {
    item.choices
        .iter()
        .find(|c| c.layer == layer)
        .unwrap_or_else(|| panic!("{} has no {layer:?} choice", item.rel_path))
}

#[test]
fn auto_memory_entries_list_with_a_proposed_layer_and_reason_and_nothing_is_written() {
    let (rig, m) = memory_rig();
    let (personal, org) = (rig.p("personal"), rig.p("org"));
    let roots = Roots {
        personal: Some(&personal),
        org: Some(&org),
        machine: Some("macbook"),
    };
    let before = snapshot(&rig.base);
    let decisions = read_decisions(&rig.decisions()).unwrap();
    let out = list(Some(&m), &[], &decisions, &roots);
    assert_eq!(snapshot(&rig.base), before, "the list wrote something");
    assert!(!rig.decisions().exists());
    assert_eq!(out.items.len(), 6);

    // Each entry has a proposed layer and a reason.
    for item in out.items.iter().filter(|i| i.rel_path.starts_with(MEM)) {
        let sort = item.sort.as_ref().expect("an auto-memory entry is sorted");
        assert_eq!(item.proposed_layer, sort.layer, "{}", item.rel_path);
        assert!(!sort.reason.is_empty() && sort.reason.contains(&sort.rule));
    }

    let runner = item_at(&out, &format!("{MEM}/runner.md"));
    assert_eq!(runner.proposed_layer, ContextReviewLayer::Home);
    assert!(
        runner.sort.as_ref().unwrap().reason.starts_with("Home by "),
        "{:?}",
        runner.sort
    );
    let home = choice_of(runner, ContextReviewLayer::Home);
    assert_eq!(home.target.as_deref(), Some("home/macbook/runner.md"));
    assert_eq!(home.disabled, None);
    assert!(!home.outward);
    // Project: a suggestion for that project's lead, never a write.
    let project = choice_of(runner, ContextReviewLayer::Project);
    assert_eq!(project.target.as_deref(), Some("-tmp-app"));
    assert_eq!(project.disabled, None);
    assert!(runner
        .choices
        .iter()
        .all(|c| c.layer != ContextReviewLayer::Stale));

    let tone = item_at(&out, &format!("{MEM}/tone.md"));
    assert_eq!(tone.proposed_layer, ContextReviewLayer::Personal);
    let bindings = item_at(&out, &format!("{MEM}/bindings.md"));
    assert_eq!(bindings.proposed_layer, ContextReviewLayer::Project);

    for (rel, rule) in [("MEMORY.md", "index"), ("old.md", "superseded")] {
        let stale = item_at(&out, &format!("{MEM}/{rel}"));
        let sort = stale.sort.as_ref().unwrap();
        assert_eq!(stale.proposed_layer, ContextReviewLayer::Stale, "{rel}");
        assert_eq!(sort.rule, rule);
        let skip = choice_of(stale, ContextReviewLayer::Stale);
        assert_eq!(skip.disabled.as_deref(), Some("stale"));
        assert_eq!(skip.detail.as_deref(), Some(sort.reason.as_str()));
        assert!(disabled_sentence(skip).contains("proposes Skip"));
    }

    // Any other file: no sort, no Home or Stale choice, as before.
    let claude = item_at(&out, "CLAUDE.md");
    assert!(claude.sort.is_none());
    assert_eq!(claude.choices.len(), 3);
    assert_eq!(
        choice_of(claude, ContextReviewLayer::Project)
            .disabled
            .as_deref(),
        Some("not-a-project-file")
    );
}

#[test]
fn a_stale_entry_is_never_accepted_and_skip_works() {
    let (rig, m) = memory_rig();
    let personal = rig.p("personal");
    let (decisions, pending) = (rig.decisions(), rig.pending());
    let roots = Roots {
        personal: Some(&personal),
        machine: Some("macbook"),
        ..Roots::default()
    };
    let ctx = rig.ctx(roots, &decisions, &pending);
    let old = by_rel(&m, &format!("{MEM}/old.md")).clone();
    let err = accept(
        &ctx,
        &Found::Scan(old.clone()),
        ContextReviewLayer::Stale,
        &no_push,
    )
    .unwrap_err();
    assert!(err.contains("proposes Skip"), "{err}");
    assert!(snapshot(&personal).is_empty(), "nothing is written");
    assert!(!decisions.exists(), "nothing is recorded");

    assert!(skip_at(&decisions, &old.id).unwrap());
    let d = read_decisions(&decisions).unwrap();
    let out = list(Some(&m), &[], &d, &roots);
    assert!(item_at(&out, &format!("{MEM}/old.md")).skipped);
    // The person may still pick another layer for it.
    let done = accept(
        &ctx,
        &Found::Scan(old),
        ContextReviewLayer::Personal,
        &no_push,
    )
    .unwrap();
    assert!(done.written);
}

#[test]
fn home_accept_writes_one_new_file_in_this_macs_folder_and_never_overwrites() {
    let (rig, m) = memory_rig();
    let personal = rig.p("personal");
    let (decisions, pending) = (rig.decisions(), rig.pending());
    let roots = Roots {
        personal: Some(&personal),
        machine: Some("macbook"),
        ..Roots::default()
    };
    let ctx = rig.ctx(roots, &decisions, &pending);
    let runner = Found::Scan(by_rel(&m, &format!("{MEM}/runner.md")).clone());
    let done = accept(&ctx, &runner, ContextReviewLayer::Home, &no_push).unwrap();
    assert!(done.written);
    assert_eq!(done.layer, ContextReviewLayer::Home);
    assert_eq!(done.target.as_deref(), Some("home/macbook/runner.md"));
    let after = snapshot(&personal);
    assert_eq!(after.len(), 1, "exactly one file: {after:?}");
    assert_eq!(
        std::fs::read_to_string(personal.join("home/macbook/runner.md")).unwrap(),
        std::fs::read_to_string(rig.p("claude").join(format!("{MEM}/runner.md"))).unwrap()
    );
    let err = accept(&ctx, &runner, ContextReviewLayer::Home, &no_push).unwrap_err();
    assert!(err.contains("already accepted"), "{err}");

    // An existing file at the target stays as it is.
    write(&personal, "home/macbook/tone.md", "mine\n");
    let d = read_decisions(&decisions).unwrap();
    let out = list(Some(&m), &[], &d, &roots);
    let tone = item_at(&out, &format!("{MEM}/tone.md"));
    assert_eq!(
        choice_of(tone, ContextReviewLayer::Home)
            .disabled
            .as_deref(),
        Some("target-exists")
    );
    let tone = Found::Scan(by_rel(&m, &format!("{MEM}/tone.md")).clone());
    let err = accept(&ctx, &tone, ContextReviewLayer::Home, &no_push).unwrap_err();
    assert!(err.contains("never overwrites"), "{err}");
    assert_eq!(
        std::fs::read_to_string(personal.join("home/macbook/tone.md")).unwrap(),
        "mine\n"
    );

    // Without this Mac's name, or without the clone, Home is off.
    let bindings = by_rel(&m, &format!("{MEM}/bindings.md")).clone();
    let unnamed = Roots {
        personal: Some(&personal),
        ..Roots::default()
    };
    let out = list(Some(&m), &[], &d, &unnamed);
    let row = item_at(&out, &bindings.rel_path);
    assert_eq!(
        choice_of(row, ContextReviewLayer::Home).disabled.as_deref(),
        Some("no-machine")
    );
    let ctx2 = rig.ctx(unnamed, &decisions, &pending);
    let before = snapshot(&personal);
    let err = accept(
        &ctx2,
        &Found::Scan(bindings.clone()),
        ContextReviewLayer::Home,
        &no_push,
    )
    .unwrap_err();
    assert!(err.contains("machine name"), "{err}");
    assert_eq!(snapshot(&personal), before);
    let out = list(Some(&m), &[], &d, &Roots::default());
    assert_eq!(
        choice_of(item_at(&out, &bindings.rel_path), ContextReviewLayer::Home)
            .disabled
            .as_deref(),
        Some("no-personal-root")
    );
}

#[test]
fn a_project_entry_needs_its_lead_and_writes_nothing() {
    let (rig, m) = memory_rig();
    let (personal, org) = (rig.p("personal"), rig.p("org"));
    let (decisions, pending) = (rig.decisions(), rig.pending());
    let ctx = rig.ctx(
        Roots {
            personal: Some(&personal),
            org: Some(&org),
            machine: Some("macbook"),
        },
        &decisions,
        &pending,
    );
    let before = [
        snapshot(&personal),
        snapshot(&org),
        snapshot(&rig.p("claude")),
    ];
    let bindings = Found::Scan(by_rel(&m, &format!("{MEM}/bindings.md")).clone());
    let done = accept(&ctx, &bindings, ContextReviewLayer::Project, &no_push).unwrap();
    assert!(!done.written);
    assert_eq!(done.target.as_deref(), Some("-tmp-app"));
    assert_eq!(
        [
            snapshot(&personal),
            snapshot(&org),
            snapshot(&rig.p("claude"))
        ],
        before
    );
}

#[test]
fn each_sorted_layer_maps_to_the_cards_layer() {
    use crate::memory_sort::{MemorySort, SortLayer};
    let (rig, m) = memory_rig();
    let personal = rig.p("personal");
    let roots = Roots {
        personal: Some(&personal),
        machine: Some("macbook"),
        ..Roots::default()
    };
    let base = by_rel(&m, &format!("{MEM}/tone.md")).clone();
    for (sorted, want) in [
        (SortLayer::Home, ContextReviewLayer::Home),
        (SortLayer::Personal, ContextReviewLayer::Personal),
        (SortLayer::Project, ContextReviewLayer::Project),
        (SortLayer::Stale, ContextReviewLayer::Stale),
    ] {
        let mut p = base.clone();
        p.memory_sort = Some(MemorySort {
            layer: sorted,
            rule: "r".into(),
            reason: "Because r.".into(),
        });
        let item = scan_item(&p, &roots, false);
        assert_eq!(item.proposed_layer, want);
        assert_eq!(item.sort.as_ref().unwrap().layer, want);
        assert_eq!(item.sort.as_ref().unwrap().reason, "Because r.");
        assert_eq!(
            item.choices
                .iter()
                .any(|c| c.layer == ContextReviewLayer::Stale),
            want == ContextReviewLayer::Stale
        );
    }
    // A manifest staged before PX6 has no sort: the entry lists as before.
    let mut old = base.clone();
    old.memory_sort = None;
    let item = scan_item(&old, &roots, false);
    assert!(item.sort.is_none());
    assert_eq!(item.proposed_layer, ContextReviewLayer::Personal);
    assert_eq!(item.choices.len(), 3);
    // The field round-trips through the staged manifest, and an old one
    // without it still reads.
    let raw = serde_json::to_string(&base).unwrap();
    assert!(raw.contains("memorySort"));
    let mut v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    v.as_object_mut().unwrap().remove("memorySort");
    let back: Proposal = serde_json::from_value(v).unwrap();
    assert!(back.memory_sort.is_none());
}
