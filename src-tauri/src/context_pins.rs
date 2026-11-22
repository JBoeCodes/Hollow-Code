//! Pinned memories and the guardrail promotion offer (PX3, 2026-11-16).
//!
//! **Pinned memories.** A memory file of the personal clone (`memory/<name>.md`)
//! whose frontmatter says `pinned: true` (top level or under `metadata:`) is
//! loaded whole in every fresh chat's operator digest, as the section
//! [`PINNED_PATH`], ahead of the generated index. Pins share a budget of
//! [`PINNED_CAP`] bytes (about 1.5 KB): they are taken in file-name order and
//! one that would pass the budget is **not loaded and is named with its size**
//! in the section's closing line, so an over-budget pin is refused in the open,
//! never cut and never silent. The section is a layer section
//! (`context_compiler::is_layer`): fixed size, never cut by the turn budget.
//! Refusing a pin when the memory is *written* (the writer showing the size)
//! is not built: the read side is where the budget is enforced. Only the flat
//! `memory/<name>.md` files are read (Claude's per-project folders under
//! `memory/<project>/` are not). A pin is a frontmatter key on a file, so
//! whoever can write a memory file can pin it, as with the memory itself (the
//! write rules and the scan apply to the file); the budget caps what a pin
//! can put in every prompt, a pinned body can't open a heading of its own, and
//! a file name with a control character is never read.
//!
//! **The promotion offer.** Three of the strongest rules ([`SAFETY_RULES`]) sit
//! in `memory/`, which a chat reads only through recall or the index, not in
//! `me/Guardrails.md`, which every chat reads in full. Once per rule, a minute
//! or two after launch, Hollow files a learning proposal that would add the
//! rule's one-line description to `me/Guardrails.md`: a Waiting on you card
//! (decision D3: Guardrails and Decisions change only by a card, never
//! silently), editable before Save, and Reject keeps the memory as it is. A
//! rule already in Guardrails is not offered, and a rule offered once is never
//! offered again whatever the answer (`context-promotions.json` in app data).
//! Nothing here writes a memory or `me/`.

use crate::context_layers::{fm_value, Section, READ_MAX};
use crate::context_roots as roots;
use crate::vault_learning::LearningProposalPayload;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The pins' shared budget, in bytes (the section's lead and closing line
/// not counted).
pub(crate) const PINNED_CAP: usize = 1_500;
/// The label the digest heading names for the pinned section.
pub(crate) const PINNED_PATH: &str = "personal/pinned.md";
/// Memory files the pin scan reads at most.
const PIN_FILES: usize = 400;
/// Refused pins named in the closing line before "and N more".
const REFUSED_SHOWN: usize = 5;
/// The memories offered for promotion to `me/Guardrails.md`, by file stem.
pub(crate) const SAFETY_RULES: [&str; 3] = [
    "never-stage-outbound-messages-without-approval",
    "never-print-secret-slices",
    "pause-means-stop-at-safe-points",
];
/// How long after launch the offer is made.
const FIRST_OFFER: Duration = Duration::from_secs(120);
/// The offered record in app data.
const MARKER_FILE: &str = "context-promotions.json";

/// The pinned section for the personal clone at `root`, or `None` when no
/// memory is pinned (or the root is unsafe).
pub(crate) fn pinned_section(root: &Path) -> Option<Section> {
    roots::check_root(root).ok()?;
    let dir = root.join("memory");
    if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
        return None;
    }
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        // A name with a control character could forge a heading in the digest.
        .filter(|n| n.ends_with(".md") && n != "README.md" && !n.chars().any(char::is_control))
        .collect();
    names.sort();
    names.truncate(PIN_FILES);
    let mut loaded: Vec<String> = Vec::new();
    let mut eligible: Vec<String> = Vec::new();
    let mut refused: Vec<(String, usize)> = Vec::new();
    let mut used = 0usize;
    for name in names {
        let Ok(Some(raw)) = roots::read_in_root(root, &format!("memory/{name}"), READ_MAX) else {
            continue;
        };
        if !fm_value(&raw, "pinned").is_some_and(|v| v.eq_ignore_ascii_case("true")) {
            continue;
        }
        let stem = name.trim_end_matches(".md");
        let body = crate::frontmatter::parse(&raw).body;
        // A pinned body can't open a heading of its own: that could pass for
        // another section of the digest (a `## me/Guardrails.md`).
        let body: Vec<String> = body
            .trim()
            .lines()
            .map(|l| {
                if l.trim_start().starts_with('#') {
                    format!("\\{l}")
                } else {
                    l.to_string()
                }
            })
            .collect();
        let entry = format!("### {stem}\n{}", body.join("\n"));
        // Resume deltas compare against every eligible pin, so budget
        // displacement cannot withdraw a still-valid rule already given.
        eligible.push(entry.clone());
        // The entries are joined by a blank line: two bytes each.
        if used + entry.len() + 2 > PINNED_CAP {
            refused.push((stem.to_string(), entry.len()));
        } else {
            used += entry.len() + 2;
            loaded.push(entry);
        }
    }
    if loaded.is_empty() && refused.is_empty() {
        return None;
    }
    let lead = format!(
        "Pinned memories, always loaded ({used} of {PINNED_CAP} bytes). The rest of your memories are in the index; read one in {} when its line fits the task.",
        root.join("memory").display()
    );
    let mut text = lead;
    if !loaded.is_empty() {
        text.push_str("\n\n");
        text.push_str(&loaded.join("\n\n"));
    }
    if !refused.is_empty() {
        let mut list: Vec<String> = refused
            .iter()
            .take(REFUSED_SHOWN)
            .map(|(stem, size)| format!("{stem} ({size} bytes)"))
            .collect();
        if refused.len() > REFUSED_SHOWN {
            list.push(format!("and {} more", refused.len() - REFUSED_SHOWN));
        }
        text.push_str(&format!(
            "\n\nNot loaded, over the {PINNED_CAP}-byte pin budget: {}. Unpin one or shorten it.",
            list.join(", ")
        ));
    }
    Some(Section {
        path: PINNED_PATH.to_string(),
        whole: format!("{text}\n\n{}", eligible.join("\n\n")),
        text,
    })
}

/// A readable subject from a memory's file stem: dashes to spaces, the
/// first letter up.
fn subject_of(stem: &str) -> String {
    let spaced = stem.replace(['-', '_'], " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The proposals that would promote each of [`SAFETY_RULES`] found in the
/// clone at `root` to `me/Guardrails.md`, as `(stem, payload)`: a rule is
/// skipped when it is in `offered`, has no description, or its description
/// is already in `guardrails` (compared lowercase). Nothing is read or
/// written beyond the memory files.
pub(crate) fn guardrail_payloads(
    root: &Path,
    guardrails: &str,
    offered: &BTreeSet<String>,
) -> Vec<(String, LearningProposalPayload)> {
    let have = guardrails.to_lowercase();
    let mut out = Vec::new();
    for stem in SAFETY_RULES {
        if offered.contains(stem) {
            continue;
        }
        let Ok(Some(raw)) = roots::read_in_root(root, &format!("memory/{stem}.md"), READ_MAX)
        else {
            continue;
        };
        let Some(statement) = fm_value(&raw, "description") else {
            continue;
        };
        if have.contains(&statement.to_lowercase()) {
            continue;
        }
        out.push((
            stem.to_string(),
            LearningProposalPayload {
                kind: "preference".into(),
                target_kind: "operator".into(),
                target_path: "me/Guardrails.md".into(),
                subject: subject_of(stem),
                statement,
                rationale: format!(
                    "From your memory {stem}. Guardrails is read in full by every chat, a memory only through the index or recall. Edit the words first if the rule should be shorter. Reject keeps it as a memory."
                ),
                ..Default::default()
            },
        ));
    }
    out
}

fn marker_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join(MARKER_FILE))
}

/// The rules already offered, from the record at `path`. A file that
/// doesn't parse reads as "everything offered": better silent than a card
/// on every launch.
fn offered_at(path: &Path) -> BTreeSet<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Vec<String>>(&text) {
            Ok(list) => list.into_iter().collect(),
            Err(_) => SAFETY_RULES.iter().map(|s| s.to_string()).collect(),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(_) => SAFETY_RULES.iter().map(|s| s.to_string()).collect(),
    }
}

fn save_offered(path: &Path, offered: &BTreeSet<String>) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string(&offered.iter().collect::<Vec<_>>())
        .map_err(|e| format!("encode the offered record: {e}"))?;
    std::fs::write(&tmp, json).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Files a promotion card for each rule not yet offered, through `submit`
/// (`stem`, payload), and records each that was accepted by the queue in the
/// record at `marker` so it is never offered again. A rule whose submit
/// failed is tried on the next launch. Returns the stems offered now.
pub(crate) fn offer_with(
    root: &Path,
    guardrails: &str,
    marker: &Path,
    submit: &mut dyn FnMut(&str, LearningProposalPayload) -> Result<(), String>,
) -> Vec<String> {
    let mut offered = offered_at(marker);
    let mut now = Vec::new();
    for (stem, payload) in guardrail_payloads(root, guardrails, &offered) {
        match submit(&stem, payload) {
            Ok(()) => {
                offered.insert(stem.clone());
                now.push(stem);
            }
            Err(e) => eprintln!("[context-pins] the {stem} promotion card could not be filed: {e}"),
        }
    }
    if !now.is_empty() {
        if let Err(e) = save_offered(marker, &offered) {
            eprintln!("[context-pins] {e}");
        }
    }
    now
}

/// The offer for this Mac: the configured personal clone, the vault's
/// `me/Guardrails.md` as it is, a card each through the learning queue.
fn offer(vault: &Path) {
    let Some(root) = roots::root(roots::Layer::Personal) else {
        return;
    };
    let Ok(marker) = marker_path() else {
        return;
    };
    let guardrails = std::fs::read_to_string(vault.join("me/Guardrails.md")).unwrap_or_default();
    let vault = vault.to_path_buf();
    let made = offer_with(&root, &guardrails, &marker, &mut |stem, payload| {
        crate::vault_learning::submit_proposal_from(
            &vault,
            payload,
            "memory",
            Some(format!("memory/{stem}.md")),
        )
        .map(|_| ())
    });
    if !made.is_empty() {
        eprintln!(
            "[context-pins] offered to promote {} to Guardrails (Waiting on you)",
            made.join(", ")
        );
    }
}

/// Offers the promotions once, [`FIRST_OFFER`] after launch, on its own
/// thread (nothing waits for it, and a failure is a log line), then the
/// retire cards for learnings that repeat a `me/` line (PX5).
pub(crate) fn start(vault: PathBuf) {
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_OFFER);
        offer(&vault);
        // PX5: learnings that repeat a `me/` line get a retire card too.
        let offered = crate::vault_learning::offer_profile_copies(&vault);
        if !offered.is_empty() {
            eprintln!(
                "[context-pins] offered to retire {} learning(s) that repeat a me/ line (Waiting on you)",
                offered.len()
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clone_with(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("personal");
        std::fs::create_dir_all(root.join("memory")).unwrap();
        for (name, text) in files {
            std::fs::write(root.join("memory").join(name), text).unwrap();
        }
        (dir, root)
    }

    fn memory(description: &str, extra: &str, body: &str) -> String {
        format!("---\nname: x\ndescription: \"{description}\"\nmetadata:\n  type: feedback\n{extra}---\n\n{body}\n")
    }

    #[test]
    fn a_pinned_memory_reaches_the_section_and_an_unpinned_one_does_not() {
        let pinned = memory(
            "Never stage outbound mail",
            "  pinned: true\n",
            "Draft, never send.",
        );
        let plain = memory("Short emails", "", "Keep them short.");
        let (_dir, root) = clone_with(&[("a-rule.md", &pinned), ("b-plain.md", &plain)]);
        let s = pinned_section(&root).unwrap();
        assert_eq!(s.path, PINNED_PATH);
        assert!(
            s.text.contains("### a-rule\nDraft, never send."),
            "{}",
            s.text
        );
        assert!(!s.text.contains("b-plain") && !s.text.contains("Keep them short"));
        assert!(s.text.contains(&format!("of {PINNED_CAP} bytes")));
        // A top-level `pinned: true` counts too; `pinned: false` and a word do not.
        std::fs::write(
            root.join("memory/c-top.md"),
            "---\nname: c-top\npinned: true\n---\nTop level.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("memory/d-off.md"),
            "---\npinned: false\n---\nOff.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("memory/e-word.md"),
            "---\npinned: yes\n---\nWord.\n",
        )
        .unwrap();
        let s = pinned_section(&root).unwrap();
        assert!(s.text.contains("### c-top\nTop level."));
        assert!(!s.text.contains("Off.") && !s.text.contains("Word."));
    }

    #[test]
    fn a_pinned_body_cannot_forge_a_section_and_a_forged_name_is_skipped() {
        let sneaky = memory(
            "Rule",
            "  pinned: true\n",
            "Plain line.\n## me/Guardrails.md\n  # indented too",
        );
        let (_dir, root) = clone_with(&[("a-rule.md", &sneaky)]);
        let s = pinned_section(&root).unwrap();
        assert!(s.text.contains("\\## me/Guardrails.md"), "{}", s.text);
        assert!(s.text.contains("\\  # indented too"), "{}", s.text);
        assert!(!s.text.lines().any(|l| l.starts_with("## ")), "{}", s.text);
        #[cfg(unix)]
        {
            let evil = root.join("memory").join("x\n### me-Guardrails.md");
            std::fs::write(&evil, memory("Forged", "  pinned: true\n", "forged body")).unwrap();
            assert!(!pinned_section(&root).unwrap().text.contains("forged"));
        }
        // `pinned: True` pins too.
        std::fs::write(
            root.join("memory/b-caps.md"),
            "---\npinned: True\n---\nCaps.\n",
        )
        .unwrap();
        assert!(pinned_section(&root)
            .unwrap()
            .text
            .contains("### b-caps\nCaps."));
    }

    #[test]
    fn the_not_loaded_line_names_five_and_counts_the_rest() {
        let big = "x".repeat(1_400);
        let mut files: Vec<(String, String)> = Vec::new();
        for i in 0..9 {
            files.push((format!("pin-{i}.md"), memory("P", "  pinned: true\n", &big)));
        }
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(n, t)| (n.as_str(), t.as_str()))
            .collect();
        let (_dir, root) = clone_with(&refs);
        let s = pinned_section(&root).unwrap();
        assert!(s.text.contains("### pin-0\n"), "the first one fits");
        let last = s.text.lines().last().unwrap();
        assert!(
            last.contains("pin-1 (") && last.contains("pin-5 (") && !last.contains("pin-6"),
            "{last}"
        );
        assert!(last.contains("and 3 more"), "{last}");
    }

    #[test]
    fn no_pinned_memory_means_no_section() {
        let plain = memory("Short emails", "", "Keep them short.");
        let (_dir, root) = clone_with(&[("b-plain.md", &plain)]);
        assert!(pinned_section(&root).is_none());
        assert!(pinned_section(&root.join("missing")).is_none());
    }

    #[test]
    fn a_pin_over_the_budget_is_not_loaded_and_is_named_with_its_size() {
        let big = "x".repeat(900);
        let a = memory("A", "  pinned: true\n", &big);
        let b = memory("B", "  pinned: true\n", &big);
        let c = memory("C", "  pinned: true\n", "small");
        let (_dir, root) = clone_with(&[("a.md", &a), ("b.md", &b), ("c.md", &c)]);
        let s = pinned_section(&root).unwrap();
        assert!(s.text.contains("### a\n"), "the first pin fits");
        assert!(
            !s.text.contains("### b\n"),
            "the second would pass 1,500 bytes"
        );
        assert!(
            s.text.contains("### c\nsmall"),
            "a later small pin still fits"
        );
        assert!(
            s.text
                .contains("Not loaded, over the 1500-byte pin budget: b (906 bytes)."),
            "{}",
            s.text
        );
        // The pins themselves stay within the budget.
        let used: usize = s
            .text
            .split("\n\n")
            .filter(|p| p.starts_with("### "))
            .map(|p| p.len() + 1)
            .sum();
        assert!(used <= PINNED_CAP, "{used}");
    }

    #[test]
    fn budget_pressure_does_not_withdraw_an_unchanged_pin_on_resume() {
        let body = "Keep this safety rule in force.".repeat(25);
        let pin = memory("B", "  pinned: true\n", &body);
        let (_dir, root) = clone_with(&[("b.md", &pin)]);
        let before = pinned_section(&root).unwrap();
        let earlier = memory("A", "  pinned: true\n", &"x".repeat(900));
        std::fs::write(root.join("memory/a.md"), earlier).unwrap();
        let after = pinned_section(&root).unwrap();
        assert!(!after.text.contains(&body), "the old pin is over budget");
        let delta = crate::context_compiler::profile_delta(
            &root,
            &[(PINNED_PATH.to_string(), before.text.clone())],
            &[(PINNED_PATH.to_string(), after.text.clone())],
            &[(PINNED_PATH.to_string(), after.whole.clone())],
        )
        .unwrap();
        assert!(
            delta.held.iter().any(|(_, text)| text.contains(&body)),
            "a still-pinned rule remains held when only the budget displaced it"
        );
        std::fs::write(root.join("memory/b.md"), memory("B", "", &body)).unwrap();
        let unpinned = pinned_section(&root).unwrap();
        let delta = crate::context_compiler::profile_delta(
            &root,
            &delta.held,
            &[(PINNED_PATH.to_string(), unpinned.text)],
            &[(PINNED_PATH.to_string(), unpinned.whole)],
        )
        .unwrap();
        assert!(delta.removed > 0, "actual unpinning withdraws the rule");
        assert!(!delta.held.iter().any(|(_, text)| text.contains(&body)));
    }

    #[test]
    fn a_symlinked_pin_is_not_followed() {
        let (_dir, root) = clone_with(&[]);
        let outside = root.parent().unwrap().join("outside.md");
        std::fs::write(&outside, "---\npinned: true\n---\nSecret.\n").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("memory/link.md")).unwrap();
            assert!(pinned_section(&root).is_none());
        }
    }

    fn rule_files() -> Vec<(String, String)> {
        SAFETY_RULES
            .iter()
            .map(|s| {
                (
                    format!("{s}.md"),
                    memory(&format!("Rule {s}"), "", "Why it matters."),
                )
            })
            .collect()
    }

    #[test]
    fn the_three_safety_rules_become_guardrail_proposals_for_the_person_to_save() {
        let files = rule_files();
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(n, t)| (n.as_str(), t.as_str()))
            .collect();
        let (_dir, root) = clone_with(&refs);
        let payloads = guardrail_payloads(&root, "", &BTreeSet::new());
        assert_eq!(payloads.len(), 3);
        let (stem, p) = &payloads[0];
        assert_eq!(stem, SAFETY_RULES[0]);
        assert_eq!(
            (p.target_kind.as_str(), p.target_path.as_str()),
            ("operator", "me/Guardrails.md")
        );
        assert_eq!(p.statement, format!("Rule {}", SAFETY_RULES[0]));
        assert_eq!(p.subject, "Never stage outbound messages without approval");
        assert!(p.rationale.contains("Reject keeps it as a memory"));
        assert!(p.retires.is_none() && p.edits_path.is_none());
    }

    #[test]
    fn a_rule_already_in_guardrails_or_already_offered_is_skipped() {
        let files = rule_files();
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(n, t)| (n.as_str(), t.as_str()))
            .collect();
        let (_dir, root) = clone_with(&refs);
        let guardrails = format!("- {}\n", format!("Rule {}", SAFETY_RULES[1]).to_uppercase());
        let offered: BTreeSet<String> = [SAFETY_RULES[2].to_string()].into();
        let payloads = guardrail_payloads(&root, &guardrails, &offered);
        assert_eq!(
            payloads.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>(),
            [SAFETY_RULES[0]]
        );
        // A rule with no memory file, or no description, is not invented.
        let (_dir, bare) = clone_with(&[(&format!("{}.md", SAFETY_RULES[0]), "no frontmatter\n")]);
        assert!(guardrail_payloads(&bare, "", &BTreeSet::new()).is_empty());
    }

    #[test]
    fn each_rule_is_offered_once_and_a_failed_submit_is_tried_again() {
        let files = rule_files();
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(n, t)| (n.as_str(), t.as_str()))
            .collect();
        let (dir, root) = clone_with(&refs);
        let marker = dir.path().join("context-promotions.json");
        let mut seen: Vec<String> = Vec::new();
        let first = offer_with(&root, "", &marker, &mut |stem, _| {
            seen.push(stem.to_string());
            if stem == SAFETY_RULES[1] {
                Err("queue busy".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(first, [SAFETY_RULES[0], SAFETY_RULES[2]]);
        assert_eq!(seen.len(), 3);
        // The next launch offers only the one that failed.
        let mut again: Vec<String> = Vec::new();
        let second = offer_with(&root, "", &marker, &mut |stem, _| {
            again.push(stem.to_string());
            Ok(())
        });
        assert_eq!(second, [SAFETY_RULES[1]]);
        assert_eq!(again, [SAFETY_RULES[1]]);
        // Then never again, whatever the person did with the cards.
        let third = offer_with(&root, "", &marker, &mut |_, _| panic!("offered twice"));
        assert!(third.is_empty());
    }

    #[test]
    fn an_unreadable_record_offers_nothing_rather_than_a_card_every_launch() {
        let files = rule_files();
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(n, t)| (n.as_str(), t.as_str()))
            .collect();
        let (dir, root) = clone_with(&refs);
        let marker = dir.path().join("context-promotions.json");
        std::fs::write(&marker, "{ not json").unwrap();
        let made = offer_with(&root, "", &marker, &mut |_, _| {
            panic!("offered with a broken record")
        });
        assert!(made.is_empty());
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap(),
            "{ not json",
            "the broken file is left alone"
        );
    }
}
