//! Resolution (CC1 design 2.3): a post-pass that annotates each name-scan
//! edge with the one definition a stated rule ties it to, or the reason
//! none does. It never removes an edge. No type inference: only the
//! index's own definitions, paths, containers and relative imports.
//!
//! Rules, by the `how` they write:
//! - `qualifier`: `a::b::N`, `Type::N`, `Self::N` (the enclosing impl
//!   type): exactly one candidate whose container or module path part is
//!   the last qualifier segment (narrowed by the whole module path).
//! - `self-type`: `self.N()` / `this.N()` with exactly one method N in the
//!   enclosing impl or class.
//! - `same-file` then `unique-name`: a bare call (a bare reference only
//!   `same-file`), free definitions only (never a method: change 2,
//!   `bare-ref-may-be-local`).
//! - `unique-method`: `.N()` on an unknown receiver, exactly one method N.
//! - `import`: TS and Python relative imports, and bare uses of a name the
//!   file imports that way.
//! - `invoke-string`: TS `invoke('n')` to the Rust fn n, preferring the one
//!   under `#[tauri::command]` (change 3).
//!
//! Anything else stays unresolved with its reason (`ambiguous(n)`,
//! `ambiguous-method(n)`, `qualifier-no-match(external?)`, `no-def`).
//! Edges from the Rust macro token scan get `macro-args:` before the rule.

use super::parse::Lang;
use super::FileParse;
use std::collections::HashMap;

/// One edge's annotation: the global node id it resolves to, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Res {
    pub target: Option<i64>,
    pub how: String,
}

/// A definition as the resolver sees it.
struct Cand<'a> {
    id: i64,
    file: usize,
    family: &'static str,
    kind: &'a str,
    container: &'a str,
    modpath: &'a str,
    command: bool,
}

/// The global node id of definition `def` of file `file`: 1-based, files
/// in order. `store::write` numbers nodes the same way.
pub(crate) fn node_ids(files: &[FileParse]) -> Vec<i64> {
    let mut next = 1i64;
    files
        .iter()
        .map(|f| {
            let first = next;
            next += f.parsed.defs.len() as i64;
            first
        })
        .collect()
}

/// Annotate every edge of `files`, per file, in edge order.
pub(crate) fn resolve(files: &[FileParse]) -> Vec<Vec<Res>> {
    resolve_with(files, &node_ids(files))
}

/// [`resolve`] with each file's first node id given (CC3: an overlay
/// resolves base files rebuilt from their rows, whose ids are the base's,
/// beside its own, whose ids continue after them).
pub(crate) fn resolve_with(files: &[FileParse], firsts: &[i64]) -> Vec<Vec<Res>> {
    let mut by_name: HashMap<&str, Vec<Cand>> = HashMap::new();
    for (fi, f) in files.iter().enumerate() {
        for (di, d) in f.parsed.defs.iter().enumerate() {
            by_name.entry(d.name.as_str()).or_default().push(Cand {
                id: firsts[fi] + di as i64,
                file: fi,
                family: f.lang.family(),
                kind: &d.kind,
                container: &d.container,
                modpath: &d.modpath,
                command: d.command,
            });
        }
    }
    let by_path: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.path.as_str(), i))
        .collect();
    files
        .iter()
        .enumerate()
        .map(|(fi, f)| {
            // Names this file imports from another indexed file.
            let mut imported: HashMap<&str, usize> = HashMap::new();
            for e in f.parsed.edges.iter().filter(|e| e.kind == "import") {
                if let Some(to) = import_file(&f.path, f.lang, &e.qual, &by_path) {
                    imported.insert(e.name.as_str(), to);
                }
            }
            f.parsed
                .edges
                .iter()
                .map(|e| {
                    let cands = by_name.get(e.name.as_str()).map_or(&[][..], |v| &v[..]);
                    let enclosing = e
                        .src
                        .and_then(|s| f.parsed.defs.get(s as usize))
                        .map_or("", |d| d.container.as_str());
                    let mut res = one(files, fi, e, cands, &imported, &by_path, enclosing);
                    if e.macro_args {
                        res.how = format!("macro-args:{}", res.how);
                    }
                    res
                })
                .collect()
        })
        .collect()
}

fn hit(c: &Cand, how: &str) -> Res {
    Res {
        target: Some(c.id),
        how: how.into(),
    }
}

fn miss(how: impl Into<String>) -> Res {
    Res {
        target: None,
        how: how.into(),
    }
}

fn free(kind: &str) -> bool {
    kind != "method"
}

fn one(
    files: &[FileParse],
    fi: usize,
    e: &super::parse::Edge,
    cands: &[Cand],
    imported: &HashMap<&str, usize>,
    by_path: &HashMap<&str, usize>,
    enclosing: &str,
) -> Res {
    let f = &files[fi];
    if e.kind == "invoke" {
        let fns: Vec<&Cand> = cands
            .iter()
            .filter(|c| c.family == "rust" && free(c.kind))
            .collect();
        let commands: Vec<&Cand> = fns.iter().copied().filter(|c| c.command).collect();
        return match (commands.len(), fns.len()) {
            (1, _) => hit(commands[0], "invoke-string"),
            (0, 1) => hit(fns[0], "invoke-string"),
            (_, 0) => miss("no-def"),
            (_, n) => miss(format!("ambiguous({n})")),
        };
    }
    let family = f.lang.family();
    let same_lang: Vec<&Cand> = cands.iter().filter(|c| c.family == family).collect();
    if same_lang.is_empty() {
        return miss("no-def");
    }
    if e.kind == "import" {
        return match import_file(&f.path, f.lang, &e.qual, by_path) {
            Some(to) => in_file(&same_lang, to, "import"),
            None if f.lang == Lang::Rust && !e.qual.is_empty() => qualified(e, &same_lang, ""),
            None => miss("qualifier-no-match(external?)"),
        };
    }
    if e.kind == "method" {
        let methods: Vec<&Cand> = same_lang
            .iter()
            .copied()
            .filter(|c| !free(c.kind))
            .collect();
        if (e.qual == ".self" || e.qual == ".this") && !enclosing.is_empty() {
            let own: Vec<&Cand> = methods
                .iter()
                .copied()
                .filter(|c| c.container == enclosing)
                .collect();
            if own.len() == 1 {
                return hit(own[0], "self-type");
            }
        }
        return match methods.len() {
            0 => miss("no-def"),
            1 => hit(methods[0], "unique-method"),
            n => miss(format!("ambiguous-method({n})")),
        };
    }
    if !e.qual.is_empty() {
        return qualified(e, &same_lang, enclosing);
    }
    // A bare call or reference: free definitions only.
    let frees: Vec<&Cand> = same_lang.iter().copied().filter(|c| free(c.kind)).collect();
    if frees.is_empty() {
        // Only methods have this name: a bare use is never one (change 2).
        return miss(if e.kind == "ref" {
            "bare-ref-may-be-local"
        } else {
            "no-def"
        });
    }
    if let Some(&to) = imported.get(e.name.as_str()) {
        let r = in_file(&frees, to, "import");
        if r.target.is_some() {
            return r;
        }
    }
    let local: Vec<&Cand> = frees.iter().copied().filter(|c| c.file == fi).collect();
    if local.len() == 1 {
        return hit(local[0], "same-file");
    }
    if e.kind == "ref" && local.is_empty() {
        // A bare name that is not called, defined in another file and not
        // imported here: as likely a local of the same name (CC1 Q1, Q6).
        return miss("bare-ref-may-be-local");
    }
    if frees.len() == 1 && local.is_empty() {
        return hit(frees[0], "unique-name");
    }
    miss(format!("ambiguous({})", frees.len()))
}

fn in_file(cands: &[&Cand], file: usize, how: &str) -> Res {
    let there: Vec<&&Cand> = cands.iter().filter(|c| c.file == file).collect();
    match there.len() {
        1 => hit(there[0], how),
        0 => miss("qualifier-no-match(external?)"),
        n => miss(format!("ambiguous({n})")),
    }
}

/// A path-qualified use (`a::b::N`, `Type::N`, `Self::N`).
fn qualified(e: &super::parse::Edge, cands: &[&Cand], enclosing: &str) -> Res {
    let segs: Vec<&str> = e
        .qual
        .split("::")
        .filter(|s| !matches!(*s, "" | "crate" | "self" | "super"))
        .collect();
    let last = match segs.last() {
        Some(&"Self") => enclosing,
        Some(s) => s,
        None => "",
    };
    if last.is_empty() {
        // `crate::N`, `self::N`, `super::N`: a free definition of N.
        let frees: Vec<&&Cand> = cands.iter().filter(|c| free(c.kind)).collect();
        return match frees.len() {
            1 => hit(frees[0], "qualifier"),
            0 => miss("qualifier-no-match(external?)"),
            n => miss(format!("ambiguous({n})")),
        };
    }
    let matched: Vec<&&Cand> = cands
        .iter()
        .filter(|c| c.container == last || c.modpath.split("::").any(|p| p == last))
        .collect();
    match matched.len() {
        1 => hit(matched[0], "qualifier"),
        0 => miss("qualifier-no-match(external?)"),
        n => {
            let path = segs.join("::");
            let exact: Vec<&&&Cand> = matched
                .iter()
                .filter(|c| c.modpath == path || c.modpath.ends_with(&format!("::{path}")))
                .collect();
            if exact.len() == 1 {
                hit(exact[0], "qualifier")
            } else {
                miss(format!("ambiguous({n})"))
            }
        }
    }
}

/// The indexed file a TS or Python import names, when it is relative (TS
/// `./x`, `../x`; Python `.x`) or, for Python, a dotted module path.
pub(crate) fn import_file(
    path: &str,
    lang: Lang,
    spec: &str,
    by_path: &HashMap<&str, usize>,
) -> Option<usize> {
    let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
    match lang {
        Lang::Ts | Lang::Tsx => {
            if !spec.starts_with("./") && !spec.starts_with("../") {
                return None;
            }
            let base = normalize(&format!("{dir}/{spec}"))?;
            let base = base
                .strip_suffix(".js")
                .or_else(|| base.strip_suffix(".jsx"))
                .unwrap_or(&base)
                .to_string();
            [
                base.clone(),
                format!("{base}.ts"),
                format!("{base}.tsx"),
                format!("{base}/index.ts"),
                format!("{base}/index.tsx"),
            ]
            .iter()
            .find_map(|p| by_path.get(p.as_str()).copied())
        }
        Lang::Python => {
            let dots = spec.chars().take_while(|c| *c == '.').count();
            let rest = spec[dots..].replace('.', "/");
            let candidates = |stem: &str| [format!("{stem}.py"), format!("{stem}/__init__.py")];
            if dots > 0 {
                let mut base = dir.to_string();
                for _ in 1..dots {
                    base = base
                        .rsplit_once('/')
                        .map_or(String::new(), |(d, _)| d.into());
                }
                let stem = [base.as_str(), rest.as_str()]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join("/");
                return candidates(&stem)
                    .iter()
                    .find_map(|p| by_path.get(p.as_str()).copied());
            }
            if rest.is_empty() {
                return None;
            }
            let ends: Vec<usize> = by_path
                .iter()
                .filter(|(p, _)| {
                    candidates(&rest)
                        .iter()
                        .any(|c| *p == c || p.ends_with(&format!("/{c}")))
                })
                .map(|(_, i)| *i)
                .collect();
            (ends.len() == 1).then(|| ends[0])
        }
        Lang::Rust => None,
    }
}

/// `a/b/../c/./d` is `a/c/d`; `None` when it climbs out of the root.
pub(crate) fn normalize(path: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            p => out.push(p),
        }
    }
    Some(out.join("/"))
}
