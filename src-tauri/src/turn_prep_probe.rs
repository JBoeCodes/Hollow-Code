//! Run 15 S0: the turn-preparation probe. Times the retrieval a chat turn
//! runs before its agent starts, over a synthetic vault and Projects folder
//! shaped like the owner's (42,203 listed files, 35,581 of them text, the
//! newest ~3,100 filling the 32 MB per-request read budget; 2,460 vault
//! notes), so a speed PR can show before and after on one fixture. Prints
//! p50 / p90 / max over a fixed set of requests, after one warm-up pass, for
//! the line-by-line scorer (the "before", `vault_search::BY_LINE`) and
//! `LineScorer` (S1) side by side, and fails if their evidence differs:
//!   cd src-tauri && cargo test --lib turn_prep_probe -- --ignored --nocapture
//! Release-like numbers (std's generic string search is unoptimized in a
//! debug build): add `--config profile.test.opt-level=2 --config
//! profile.test.debug-assertions=false --config profile.test.overflow-checks=false`
//! (a separate, slower first build). `#[ignore]`d: the fixture writes ~55 MB
//! and ~44,700 files.
#![cfg(test)]

use std::path::Path;
use std::time::{Duration, Instant};

/// A deterministic xorshift, so every run writes the same fixture.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Words the fixture's text is made of: everyday project words (some of
/// them in the probe's requests), code-ish tokens and filler.
const WORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "this",
    "that",
    "from",
    "into",
    "when",
    "then",
    "chat",
    "turn",
    "agent",
    "workspace",
    "plan",
    "brief",
    "retrieval",
    "index",
    "vault",
    "note",
    "notes",
    "task",
    "tasks",
    "project",
    "projects",
    "file",
    "files",
    "search",
    "query",
    "passage",
    "budget",
    "token",
    "stream",
    "render",
    "canvas",
    "panel",
    "preview",
    "browser",
    "session",
    "receipt",
    "history",
    "snapshot",
    "connector",
    "server",
    "spawn",
    "model",
    "prompt",
    "context",
    "speed",
    "latency",
    "parking",
    "location",
    "revenue",
    "payroll",
    "invoice",
    "report",
    "employee",
    "schedule",
    "meeting",
    "calendar",
    "design",
    "token",
    "color",
    "theme",
    "button",
    "dialog",
    "const",
    "let",
    "fn",
    "pub",
    "struct",
    "impl",
    "return",
    "import",
    "export",
    "async",
    "await",
    "string",
    "number",
    "boolean",
    "array",
    "object",
    "value",
    "result",
    "error",
    "option",
    "some",
    "none",
    "true",
    "false",
    "self",
    "type",
    "interface",
    "props",
    "state",
    "store",
    "signal",
    "effect",
    "memo",
    "event",
    "handler",
    "click",
    "input",
    "output",
    "write",
    "read",
    "open",
    "close",
    "path",
    "folder",
    "config",
    "settings",
    "theme",
    "layout",
    "grid",
    "flex",
    "margin",
    "padding",
    "border",
    "radius",
    "shadow",
    "opacity",
    "transform",
    "transition",
    "alpha",
    "beta",
    "gamma",
    "delta",
    "omega",
    "sigma",
    "lambda",
    "kappa",
    "zeta",
    "theta",
    "garage",
    "lot",
    "stall",
    "permit",
    "ticket",
    "validation",
    "rate",
    "occupancy",
    "shift",
    "manager",
    "regional",
    "monthly",
    "weekly",
    "daily",
    "summary",
    "variance",
    "forecast",
];

/// Requests like the owner's (their words, not their text): what retrieval
/// sees after the composer's injected context is stripped.
const REQUESTS: &[&str] = &[
    "why do chats take so long to start in hollow",
    "plan the retrieval index for workspace notes and connected projects",
    "fix the preview pane so any file I click in chat opens there",
    "summarise the monthly parking revenue variance report for the regional manager",
    "what changed in the canvas kit theme tokens this week",
    "draft the payroll invoice schedule for the new garage location",
    "review the agent session receipt history and snapshot timings",
    "update the design brief with the new button and dialog layout",
    "find the employee occupancy forecast in the projects folder",
    "make the browser panel render the stream without latency",
];

/// Lines of space-separated words, at least `bytes` long.
fn text_of(rng: &mut Rng, bytes: usize) -> String {
    let mut out = String::with_capacity(bytes + 128);
    while out.len() < bytes {
        let words = 6 + rng.below(9);
        for i in 0..words {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(WORDS[rng.below(WORDS.len() as u64) as usize]);
        }
        out.push('\n');
    }
    out
}

/// A file size like the owner's newest text files (~10 KB on average:
/// production reads ~3,300 of them for its 32 MB), capped well below the
/// 1 MB text limit.
fn owner_size(rng: &mut Rng) -> usize {
    match rng.below(100) {
        0..=9 => 200 + rng.below(400) as usize,
        10..=49 => 600 + rng.below(3_500) as usize,
        50..=89 => 4_000 + rng.below(16_000) as usize,
        90..=98 => 20_000 + rng.below(40_000) as usize,
        _ => 60_000 + rng.below(140_000) as usize,
    }
}

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// The Projects folder: 69 projects, 38,000 small old files, then 4,200
/// files sized like the owner's newest, written last so they are the newest
/// (the ones the per-request budget reads first), ~55 MB in all.
fn projects(root: &Path, rng: &mut Rng) -> (usize, u64) {
    let exts = [
        "ts", "md", "tsx", "json", "py", "rs", "css", "html", "txt", "sql",
    ];
    let (mut files, mut bytes) = (0usize, 0u64);
    for i in 0..38_000usize {
        let project = i % 69;
        let ext = exts[i % exts.len()];
        let size = 120 + rng.below(160) as usize;
        let text = text_of(rng, size);
        put(
            root,
            &format!("project-{project:02}/src/old/{}/file-{i:05}.{ext}", i % 40),
            &text,
        );
        files += 1;
        bytes += text.len() as u64;
    }
    for i in 0..4_200usize {
        let project = i % 23;
        let ext = exts[(i * 7) % exts.len()];
        let size = owner_size(rng);
        let text = text_of(rng, size);
        put(
            root,
            &format!(
                "project-{project:02}/src/recent/{}/recent-{i:05}.{ext}",
                i % 30
            ),
            &text,
        );
        files += 1;
        bytes += text.len() as u64;
    }
    (files, bytes)
}

/// The vault: 2,460 notes of ~1.5 KB outside workspaces (what retrieval
/// may read), beside a workspace brief.
fn vault(root: &Path, rng: &mut Rng) {
    for i in 0..2_460usize {
        let folder = ["learnings", "library", "docs", "drafts"][i % 4];
        let size = 900 + rng.below(1_200) as usize;
        let body = text_of(rng, size);
        put(
            root,
            &format!("{folder}/note-{i:04}.md"),
            &format!("---\ntitle: Note {i}\n---\n\n# Note {i}\n\n{body}"),
        );
    }
    put(
        root,
        "workspaces/local-probe/WORKSPACE.md",
        "---\nhollow_workspace: 1\ntitle: \"Probe\"\nstatus: active\n---\n\n# Probe\n\n## Outcome\n\nMake chats fast.\n",
    );
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn quantiles(mut v: Vec<f64>) -> (f64, f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |p: f64| v[((p * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1];
    (at(0.5), at(0.9), *v.last().unwrap())
}

/// One retrieval the way a chat turn's preparation runs it, minus the
/// semantic half and Jev (no model, no network): the vault prefilter and
/// the project half as production wires them. Also a digest of what it
/// found (paths relative to `root`, lines, excerpts, scores), so a run
/// before a change and one after can show the evidence is the same.
fn retrieve(
    root: &Path,
    vault: &Path,
    query: &str,
    kinds: Option<&[String]>,
) -> (Duration, usize, u64, u64) {
    let scope = crate::retrieval_broker::Scope::all();
    let req = crate::retrieval_broker::Request {
        vault,
        query,
        scope: &scope,
        kinds,
    };
    let started = Instant::now();
    let ranked = crate::retrieval_broker::retrieve_with(
        &req,
        &|texts: &[String], notes: &[crate::types::Note]| {
            crate::vault_search_index::ranked_candidates(vault, texts, notes)
        },
        crate::vault_search_index::prefilter_kind(vault),
        &|_| Err("off in the probe".into()),
    );
    let took = started.elapsed();
    use std::hash::{Hash, Hasher};
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    let prefix = root.to_string_lossy().to_string();
    for p in &ranked.passages {
        p.path.trim_start_matches(&prefix).hash(&mut digest);
        (p.line, &p.excerpt, p.score, &p.source).hash(&mut digest);
    }
    (
        took,
        ranked.passages.len(),
        ranked.stats.project_files_read,
        digest.finish(),
    )
}

#[test]
#[ignore = "prints prep timings over an owner-scale synthetic vault; writes ~45 MB"]
fn turn_prep_probe() {
    let t = tempfile::tempdir().expect("temp");
    let p = t.path().join("Projects");
    let v = t.path().join("Vault");
    std::fs::create_dir_all(&p).unwrap();
    std::fs::create_dir_all(&v).unwrap();
    let (p, v) = (p.canonicalize().unwrap(), v.canonicalize().unwrap());
    let mut rng = Rng(0x5eed_1234_abcd_0001);
    let started = Instant::now();
    let (files, bytes) = projects(&p, &mut rng);
    vault(&v, &mut rng);
    eprintln!(
        "turn_prep_probe fixture: {files} project files, {:.1} MB, written in {:.1} s",
        bytes as f64 / 1_048_576.0,
        started.elapsed().as_secs_f64()
    );
    let started = Instant::now();
    crate::directory_connections::connect(
        p.to_str().unwrap(),
        v.to_str().unwrap(),
        &t.path().join("config.json"),
    )
    .expect("connect");
    eprintln!(
        "turn_prep_probe connect (first inventory): {:.0} ms",
        ms(started.elapsed())
    );
    // The vault half's BM25 prefilter, engaged as the app engages it.
    let started = Instant::now();
    crate::vault_search_index::engage(&v);
    eprintln!(
        "turn_prep_probe engage vault index: {:.0} ms",
        ms(started.elapsed())
    );
    let project_only = vec!["connected_project".to_string()];
    let cases: [(&str, Option<&[String]>); 2] = [
        ("retrieval (both halves)", None),
        ("retrieval project half", Some(&project_only)),
    ];
    // The scorer alone, old against new, over the files the budget reads:
    // the newest ~32 MB of text, every request; outputs must be equal.
    {
        let mut newest: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
        let mut stack = vec![p.clone()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    newest.push((e.metadata().unwrap().modified().unwrap(), path));
                }
            }
        }
        newest.sort_by_key(|f| std::cmp::Reverse(f.0));
        let mut texts = Vec::new();
        let mut total = 0usize;
        for (_, path) in newest {
            let text = std::fs::read_to_string(&path).unwrap();
            if total + text.len() > 32 * 1024 * 1024 {
                continue;
            }
            total += text.len();
            texts.push((path, text));
            if texts.len() >= 4_000 {
                break;
            }
        }
        // One request's scoring by either side: (ms, every file's lines).
        let score = |by_line: bool, q: &str| {
            let terms = crate::directory_connections::retrieval_terms(q);
            let masks: Vec<u32> = texts
                .iter()
                .map(|(path, _)| {
                    crate::vault_search::name_mask(
                        &terms,
                        &[path.to_string_lossy().to_lowercase().as_str()],
                    )
                })
                .collect();
            let started = Instant::now();
            let lines: Vec<_> = if by_line {
                texts
                    .iter()
                    .zip(&masks)
                    .map(|((_, text), &mask)| {
                        crate::vault_search::best_lines_by_line(&terms, text, mask, 2)
                    })
                    .collect()
            } else {
                let scorer = crate::vault_search::LineScorer::new(&terms);
                texts
                    .iter()
                    .zip(&masks)
                    .map(|((_, text), &mask)| scorer.best_lines(text, mask, 2))
                    .collect()
            };
            (ms(started.elapsed()), lines)
        };
        // Both sides warmed once; then each request scored by both, the
        // side that goes first alternating (review S1: no order bias).
        score(true, REQUESTS[0]);
        score(false, REQUESTS[0]);
        let (mut old_t, mut new_t) = (Vec::new(), Vec::new());
        for (k, q) in REQUESTS.iter().copied().enumerate() {
            let (old, new) = if k % 2 == 0 {
                let old = score(true, q);
                (old, score(false, q))
            } else {
                let new = score(false, q);
                (score(true, q), new)
            };
            old_t.push(old.0);
            new_t.push(new.0);
            assert_eq!(
                old.1, new.1,
                "the scorer differs from the line-by-line reference for {q}"
            );
        }
        let (o50, o90, _) = quantiles(old_t);
        let (n50, n90, _) = quantiles(new_t);
        eprintln!(
            "turn_prep_probe scorer over {} files / {:.1} MB: line by line p50 {o50:.1} ms p90 {o90:.1} ms; LineScorer p50 {n50:.1} ms p90 {n90:.1} ms (equal output)",
            texts.len(),
            total as f64 / 1_048_576.0
        );
    }
    let root = t.path().canonicalize().unwrap();
    for (name, kinds) in cases {
        // One warm-up pass of each side: the page cache and the in-memory
        // inventory.
        for by_line in [true, false] {
            for q in REQUESTS {
                crate::vault_search::BY_LINE.with(|c| c.set(by_line));
                retrieve(&root, &v, q, kinds);
                crate::vault_search::BY_LINE.with(|c| c.set(false));
            }
        }
        // Before (the line-by-line scorer) and after for every request,
        // the side that goes first alternating, so neither gets the other's
        // warm caches or the machine's quieter moments.
        use std::hash::{Hash, Hasher};
        let mut times = [Vec::new(), Vec::new()];
        let (mut passages, mut read) = ([0usize; 2], [0u64; 2]);
        let mut digests = [
            std::collections::hash_map::DefaultHasher::new(),
            std::collections::hash_map::DefaultHasher::new(),
        ];
        for (k, q) in REQUESTS.iter().copied().enumerate() {
            let order = if k % 2 == 0 {
                [(0usize, true), (1, false)]
            } else {
                [(1, false), (0usize, true)]
            };
            for (side, by_line) in order {
                crate::vault_search::BY_LINE.with(|c| c.set(by_line));
                let (took, n, files_read, d) = retrieve(&root, &v, q, kinds);
                crate::vault_search::BY_LINE.with(|c| c.set(false));
                times[side].push(ms(took));
                passages[side] += n;
                read[side] += files_read;
                d.hash(&mut digests[side]);
            }
        }
        let digests = digests.map(|d| d.finish());
        for (side, label) in [(0usize, "line by line"), (1, "LineScorer")] {
            let (p50, p90, max) = quantiles(times[side].clone());
            eprintln!(
                "turn_prep_probe {name:<26} {label:<13} p50 {p50:>7.1} ms  p90 {p90:>7.1} ms  max {max:>7.1} ms  ({} requests, {} passages, {} project files read per request, evidence digest {:016x})",
                REQUESTS.len(),
                passages[side],
                read[side] / REQUESTS.len() as u64,
                digests[side]
            );
        }
        assert_eq!(digests[0], digests[1], "{name}: the evidence changed");
    }
}
