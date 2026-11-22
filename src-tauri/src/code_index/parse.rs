//! Per-file parsing for the code-context index (CC2): tree-sitter turns one
//! Rust, TypeScript (TSX) or Python file into its definitions and its
//! name-scan edges. Nothing here resolves a name; `resolve.rs` annotates the
//! edges afterwards.
//!
//! An edge is a place a name is used: a call, a method call, a macro, a
//! `new`, a JSX element, a struct literal, an import, a Tauri
//! `invoke('name')` string, or a plain reference. Rust macro arguments are
//! token trees tree-sitter does not parse, so they are scanned token by token
//! (`name(`, `path::name(`, `.name(`, `path::name`, bare names) and those
//! edges carry `macro_args` (CC1 section 5, change 1).
//!
//! Every parse runs under [`PARSE_TIMEOUT`]; a file that runs past it yields
//! no rows and `timed_out`.

use std::time::{Duration, Instant};
use tree_sitter::{Node, Parser};

/// The longest one file may take to parse.
pub(crate) const PARSE_TIMEOUT: Duration = Duration::from_secs(3);

/// The receiver text a method edge keeps (`.self.caps`), at most.
const RECEIVER_CHARS: usize = 32;

/// An indexed language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lang {
    Rust,
    Ts,
    Tsx,
    Python,
}

impl Lang {
    /// The language of `path` by extension (`.rs`, `.ts`, `.tsx`, `.py`).
    /// `.mjs` and `.js` are left out (CC1 open question 5).
    pub(crate) fn of(path: &str) -> Option<Lang> {
        let ext = path.rsplit_once('.').map(|(_, e)| e)?;
        match ext {
            "rs" => Some(Lang::Rust),
            "ts" | "mts" | "cts" => Some(Lang::Ts),
            "tsx" => Some(Lang::Tsx),
            "py" => Some(Lang::Python),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Ts => "ts",
            Lang::Tsx => "tsx",
            Lang::Python => "python",
        }
    }

    pub(crate) fn from_str(s: &str) -> Option<Lang> {
        match s {
            "rust" => Some(Lang::Rust),
            "ts" => Some(Lang::Ts),
            "tsx" => Some(Lang::Tsx),
            "python" => Some(Lang::Python),
            _ => None,
        }
    }

    /// The language family a filter names: `rust`, `ts` (TS and TSX) or
    /// `python`.
    pub(crate) fn family(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Ts | Lang::Tsx => "ts",
            Lang::Python => "python",
        }
    }

    fn grammar(self) -> tree_sitter::Language {
        match self {
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::Ts => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
        }
    }
}

/// The parser versions an index records (`meta.parsers`).
pub(crate) const PARSERS: &str =
    "tree-sitter 0.26.13; rust 0.24.2; typescript 0.23.2; python 0.25.0; scan 1";

/// One definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Def {
    /// fn, method, class, struct, enum, trait, iface, type, const, static,
    /// mod, macro or var.
    pub kind: String,
    pub name: String,
    /// The impl type, trait, class or interface it sits in.
    pub container: String,
    /// The Rust module path (`code_index::parse`); the file's module
    /// otherwise.
    pub modpath: String,
    pub l0: u32,
    pub l1: u32,
    pub test: bool,
    /// Under `#[tauri::command]`.
    pub command: bool,
}

/// One name-scan hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edge {
    /// The innermost definition the use sits in (an index into `defs`).
    pub src: Option<u32>,
    pub line: u32,
    /// call, method, macro, new, jsx, struct, import, invoke or ref.
    pub kind: String,
    pub name: String,
    /// The path or receiver as written (`crate::skills`, `.self`), or the
    /// module an import names.
    pub qual: String,
    /// Found by the Rust token-tree scan.
    pub macro_args: bool,
}

/// One file's definitions and edges.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Parsed {
    pub defs: Vec<Def>,
    pub edges: Vec<Edge>,
    pub timed_out: bool,
}

/// Whether `path` is test code by its name (`*_tests.rs`, `*.test.ts`,
/// `tests/`, `test_*.py`).
pub(crate) fn test_path(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    path.split('/')
        .any(|seg| seg == "tests" || seg == "__tests__")
        || file.ends_with("_tests.rs")
        || file.ends_with("_test.rs")
        || file.contains(".test.")
        || file.contains(".spec.")
        || (file.starts_with("test_") && file.ends_with(".py"))
        || file.ends_with("_test.py")
}

/// The module path of a file: the part after its last `src/`, without its
/// extension, `mod.rs`, `lib.rs` or `main.rs` (`code_index/parse.rs` is
/// `code_index::parse`; `eval/runner.py` is `eval::runner`).
pub(crate) fn module_of(path: &str) -> String {
    let rel = match path.rfind("src/") {
        Some(at) => &path[at + 4..],
        None => path,
    };
    let rel = match rel.rsplit_once('.') {
        Some((stem, ext)) if !ext.contains('/') => stem,
        _ => rel,
    };
    let mut segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    if matches!(segs.last(), Some(&"mod") | Some(&"lib") | Some(&"main")) {
        segs.pop();
    }
    segs.join("::")
}

/// Parse one file.
pub(crate) fn parse_file(path: &str, lang: Lang, src: &[u8]) -> Parsed {
    let mut parser = Parser::new();
    if parser.set_language(&lang.grammar()).is_err() {
        return Parsed::default();
    }
    let deadline = Instant::now() + PARSE_TIMEOUT;
    let mut progress = |_: &tree_sitter::ParseState| {
        if Instant::now() > deadline {
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    };
    let options = tree_sitter::ParseOptions::new().progress_callback(&mut progress);
    let mut read = |at: usize, _: tree_sitter::Point| -> &[u8] {
        if at < src.len() {
            &src[at..]
        } else {
            &[]
        }
    };
    let Some(tree) = parser.parse_with_options(&mut read, None, Some(options)) else {
        return Parsed {
            timed_out: true,
            ..Parsed::default()
        };
    };
    let mut walk = Walk {
        src,
        lang,
        path_test: test_path(path),
        out: Parsed::default(),
    };
    walk.run(tree.root_node(), module_of(path));
    walk.out
}

/// What a subtree sits in.
#[derive(Clone)]
struct Ctx {
    container: String,
    modpath: String,
    test: bool,
    src: Option<u32>,
}

struct Walk<'a> {
    src: &'a [u8],
    lang: Lang,
    path_test: bool,
    out: Parsed,
}

fn line(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

fn same(a: Option<Node>, b: Node) -> bool {
    a.is_some_and(|a| a.id() == b.id())
}

impl<'a> Walk<'a> {
    fn text(&self, node: Node) -> &'a str {
        node.utf8_text(self.src).unwrap_or("")
    }

    fn field_text(&self, node: Node, field: &str) -> Option<&'a str> {
        node.child_by_field_name(field).map(|n| self.text(n))
    }

    fn run(&mut self, root: Node, modpath: String) {
        let mut ctxs = vec![Ctx {
            container: String::new(),
            modpath,
            test: self.path_test,
            src: None,
        }];
        let mut stack: Vec<(Node, usize)> = vec![(root, 0)];
        while let Some((node, at)) = stack.pop() {
            let ctx = ctxs[at].clone();
            let next = match self.lang {
                Lang::Rust => self.rust(node, &ctx),
                Lang::Ts | Lang::Tsx => self.ts(node, &ctx),
                Lang::Python => self.python(node, &ctx),
            };
            let child_at = match next {
                Step::Skip => continue,
                Step::Same => at,
                Step::Into(new) => {
                    ctxs.push(new);
                    ctxs.len() - 1
                }
            };
            let mut cursor = node.walk();
            let children: Vec<Node> = node.children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                stack.push((child, child_at));
            }
        }
    }

    fn def(&mut self, node: Node, kind: &str, name: &str, ctx: &Ctx, test: bool) -> Ctx {
        let index = self.out.defs.len() as u32;
        let test = ctx.test || test;
        self.out.defs.push(Def {
            kind: kind.to_string(),
            name: name.to_string(),
            container: ctx.container.clone(),
            modpath: ctx.modpath.clone(),
            l0: line(node),
            l1: node.end_position().row as u32 + 1,
            test,
            command: false,
        });
        Ctx {
            container: String::new(),
            modpath: ctx.modpath.clone(),
            test,
            src: Some(index),
        }
    }

    fn edge(
        &mut self,
        node: Node,
        kind: &str,
        name: &str,
        qual: &str,
        ctx: &Ctx,
        macro_args: bool,
    ) {
        if name.is_empty() {
            return;
        }
        self.out.edges.push(Edge {
            src: ctx.src,
            line: line(node),
            kind: kind.to_string(),
            name: name.to_string(),
            qual: qual.to_string(),
            macro_args,
        });
    }

    fn receiver(&self, node: Option<Node>) -> String {
        let text = node.map(|n| self.text(n)).unwrap_or("");
        let one_line: String = text.split_whitespace().collect::<Vec<_>>().join("");
        let short: String = one_line.chars().take(RECEIVER_CHARS).collect();
        format!(".{short}")
    }

    // ── Rust ──────────────────────────────────────────────────────────────

    /// The attributes right before `node`, as text.
    fn rust_attrs(&self, node: Node) -> String {
        let mut out = String::new();
        let mut prev = node.prev_sibling();
        while let Some(p) = prev {
            match p.kind() {
                "attribute_item" => out.push_str(self.text(p)),
                "line_comment" | "block_comment" => {}
                _ => break,
            }
            prev = p.prev_sibling();
        }
        out
    }

    fn rust(&mut self, node: Node, ctx: &Ctx) -> Step {
        let kind = node.kind();
        match kind {
            "attribute_item"
            | "inner_attribute_item"
            | "line_comment"
            | "block_comment"
            | "string_literal"
            | "raw_string_literal"
            | "char_literal" => Step::Skip,
            "function_item" | "function_signature_item" => {
                let name = self.field_text(node, "name").unwrap_or("");
                let attrs = self.rust_attrs(node);
                let test = attrs.contains("#[test]")
                    || attrs.contains("::test")
                    || attrs.contains("cfg(test)");
                let what = if ctx.container.is_empty() {
                    "fn"
                } else {
                    "method"
                };
                let inner = self.def(node, what, name, ctx, test);
                if attrs.contains("tauri::command") {
                    if let Some(def) = self.out.defs.last_mut() {
                        def.command = true;
                    }
                }
                Step::Into(inner)
            }
            "impl_item" => {
                let ty = node
                    .child_by_field_name("type")
                    .map(|t| base_type(self.text(t)))
                    .unwrap_or_default();
                Step::Into(Ctx {
                    container: ty,
                    ..ctx.clone()
                })
            }
            "trait_item" => {
                let name = self.field_text(node, "name").unwrap_or("").to_string();
                let mut inner = self.def(node, "trait", &name, ctx, false);
                inner.container = name;
                Step::Into(inner)
            }
            "struct_item" | "union_item" | "enum_item" | "type_item" | "const_item"
            | "static_item" | "macro_definition" => {
                let what = match kind {
                    "struct_item" | "union_item" => "struct",
                    "enum_item" => "enum",
                    "type_item" => "type",
                    "const_item" => "const",
                    "static_item" => "static",
                    _ => "macro",
                };
                let name = self.field_text(node, "name").unwrap_or("");
                Step::Into(self.def(node, what, name, ctx, false))
            }
            "mod_item" => {
                let name = self.field_text(node, "name").unwrap_or("").to_string();
                let test = self.rust_attrs(node).contains("cfg(test)");
                let mut inner = self.def(node, "mod", &name, ctx, test);
                if node.child_by_field_name("body").is_some() {
                    inner.modpath = if ctx.modpath.is_empty() {
                        name
                    } else {
                        format!("{}::{name}", ctx.modpath)
                    };
                    inner.src = ctx.src;
                    Step::Into(inner)
                } else {
                    Step::Skip
                }
            }
            "token_tree" => {
                if node
                    .parent()
                    .is_some_and(|p| p.kind() == "macro_invocation")
                {
                    self.scan_tokens(node, ctx);
                }
                Step::Skip
            }
            "identifier" => {
                self.rust_identifier(node, ctx);
                Step::Skip
            }
            "field_identifier" => {
                // `.name(...)`, `.name::<T>(...)`: a method call.
                if let Some(field) = node.parent().filter(|p| p.kind() == "field_expression") {
                    if same(field.child_by_field_name("field"), node) && called(field) {
                        let qual = self.receiver(field.child_by_field_name("value"));
                        self.edge(node, "method", self.text(node), &qual, ctx, false);
                    }
                }
                Step::Skip
            }
            "type_identifier" => {
                let parent = node.parent();
                let is_def_name = parent.is_some_and(|p| {
                    matches!(
                        p.kind(),
                        "struct_item"
                            | "union_item"
                            | "enum_item"
                            | "type_item"
                            | "trait_item"
                            | "type_parameter"
                            | "associated_type"
                    ) && same(p.child_by_field_name("name"), node)
                });
                if !is_def_name {
                    let p = parent.map(|p| p.kind()).unwrap_or("");
                    let qual = match parent {
                        Some(s) if p == "scoped_type_identifier" => {
                            self.field_text(s, "path").unwrap_or("").to_string()
                        }
                        _ => String::new(),
                    };
                    let what = if parent.is_some_and(|p| {
                        p.kind() == "struct_expression" && same(p.child_by_field_name("name"), node)
                    }) {
                        "struct"
                    } else {
                        "ref"
                    };
                    self.edge(node, what, self.text(node), &qual, ctx, false);
                }
                Step::Skip
            }
            _ => Step::Same,
        }
    }

    fn rust_identifier(&mut self, node: Node, ctx: &Ctx) {
        let Some(parent) = node.parent() else { return };
        let name = self.text(node);
        let pk = parent.kind();
        // A definition's own name.
        if same(parent.child_by_field_name("name"), node)
            && matches!(
                pk,
                "function_item"
                    | "function_signature_item"
                    | "struct_item"
                    | "enum_item"
                    | "union_item"
                    | "type_item"
                    | "const_item"
                    | "static_item"
                    | "mod_item"
                    | "trait_item"
                    | "macro_definition"
                    | "enum_variant"
                    | "field_declaration"
            )
        {
            return;
        }
        if binding(parent, node) {
            return;
        }
        if pk == "scoped_identifier" {
            if same(parent.child_by_field_name("name"), node) {
                self.rust_path(parent, name, ctx);
            }
            return;
        }
        if pk == "use_as_clause" && same(parent.child_by_field_name("alias"), node) {
            return;
        }
        if pk == "use_list" {
            let qual = parent
                .parent()
                .and_then(|p| p.child_by_field_name("path"))
                .map(|p| self.text(p))
                .unwrap_or("");
            self.edge(node, "import", name, qual, ctx, false);
            return;
        }
        let kind = if in_use(node) {
            "import"
        } else if pk == "macro_invocation" {
            "macro"
        } else if call_target(parent, node) {
            "call"
        } else {
            "ref"
        };
        self.edge(node, kind, name, "", ctx, false);
    }

    /// `path::name` (the scoped identifier `scoped`): a call, a macro, an
    /// import, a struct literal or a reference.
    fn rust_path(&mut self, scoped: Node, name: &str, ctx: &Ctx) {
        let qual = self.field_text(scoped, "path").unwrap_or("");
        let Some(parent) = scoped.parent() else {
            return;
        };
        // The inner part of a longer path (`a::b` in `a::b::c`).
        if parent.kind() == "scoped_identifier" && same(parent.child_by_field_name("path"), scoped)
        {
            return;
        }
        let kind = if in_use(scoped) {
            "import"
        } else if parent.kind() == "macro_invocation" {
            "macro"
        } else if parent.kind() == "struct_expression" {
            "struct"
        } else if call_target(parent, scoped) {
            "call"
        } else {
            "ref"
        };
        self.edge(scoped, kind, name, qual, ctx, false);
    }

    /// Scan a macro's token tree for calls and paths (CC1 section 5).
    fn scan_tokens(&mut self, tree: Node, ctx: &Ctx) {
        let mut tokens: Vec<Node> = Vec::new();
        let mut stack = vec![tree];
        while let Some(node) = stack.pop() {
            let kind = node.kind();
            if kind.ends_with("_literal") || kind.ends_with("comment") {
                continue;
            }
            if node.child_count() == 0 {
                tokens.push(node);
                continue;
            }
            let mut cursor = node.walk();
            let children: Vec<Node> = node.children(&mut cursor).collect();
            stack.extend(children.into_iter().rev());
        }
        let src = self.src;
        let text = |n: &Node| n.utf8_text(src).unwrap_or("");
        let seg = |n: &Node| {
            n.kind() == "identifier" || matches!(text(n), "self" | "Self" | "crate" | "super")
        };
        let mut i = 0;
        while i < tokens.len() {
            let tok = tokens[i];
            if tok.kind() != "identifier" {
                i += 1;
                continue;
            }
            let prev = if i > 0 { text(&tokens[i - 1]) } else { "" };
            if prev == "." {
                if tokens.get(i + 1).map(text) == Some("(") {
                    self.edge(tok, "method", text(&tok), ".", ctx, true);
                }
                i += 1;
                continue;
            }
            // Gather `a::b::name`, starting at a path keyword before it.
            let mut start = i;
            while start >= 2 && text(&tokens[start - 1]) == "::" && seg(&tokens[start - 2]) {
                start -= 2;
            }
            let mut segs: Vec<&str> = (start..=i).step_by(2).map(|k| text(&tokens[k])).collect();
            let mut j = i;
            while j + 2 < tokens.len() && text(&tokens[j + 1]) == "::" && seg(&tokens[j + 2]) {
                segs.push(text(&tokens[j + 2]));
                j += 2;
            }
            let name = segs.pop().unwrap_or("");
            let qual = segs.join("::");
            let at = tokens[j];
            let next = tokens.get(j + 1).map(text).unwrap_or("");
            let kind = match next {
                "!" => "macro",
                "(" => "call",
                _ => "ref",
            };
            if seg(&at) && at.kind() == "identifier" {
                self.edge(at, kind, name, &qual, ctx, true);
            }
            i = j + 1;
        }
    }

    // ── TypeScript and TSX ────────────────────────────────────────────────

    fn ts(&mut self, node: Node, ctx: &Ctx) -> Step {
        let kind = node.kind();
        match kind {
            "comment" | "string" | "template_string" | "regex" => Step::Skip,
            "function_declaration" | "generator_function_declaration" | "function_signature" => {
                let name = self.field_text(node, "name").unwrap_or("");
                Step::Into(self.def(node, "fn", name, ctx, false))
            }
            "class_declaration" | "abstract_class_declaration" | "interface_declaration" => {
                let name = self.field_text(node, "name").unwrap_or("").to_string();
                let what = if kind == "interface_declaration" {
                    "iface"
                } else {
                    "class"
                };
                let mut inner = self.def(node, what, &name, ctx, false);
                inner.container = name;
                Step::Into(inner)
            }
            "method_definition" | "method_signature" | "abstract_method_signature" => {
                let name = self.field_text(node, "name").unwrap_or("");
                Step::Into(self.def(node, "method", name, ctx, false))
            }
            "public_field_definition" => {
                let value = node.child_by_field_name("value");
                if value
                    .is_some_and(|v| matches!(v.kind(), "arrow_function" | "function_expression"))
                {
                    let name = self.field_text(node, "name").unwrap_or("");
                    Step::Into(self.def(node, "method", name, ctx, false))
                } else {
                    Step::Same
                }
            }
            "type_alias_declaration" | "enum_declaration" => {
                let name = self.field_text(node, "name").unwrap_or("");
                let what = if kind == "enum_declaration" {
                    "enum"
                } else {
                    "type"
                };
                Step::Into(self.def(node, what, name, ctx, false))
            }
            "variable_declarator" => {
                let Some(name) = node
                    .child_by_field_name("name")
                    .filter(|n| n.kind() == "identifier")
                else {
                    return Step::Same;
                };
                let name = self.text(name);
                let function = node.child_by_field_name("value").is_some_and(|v| {
                    matches!(
                        v.kind(),
                        "arrow_function" | "function_expression" | "function"
                    )
                });
                if function {
                    Step::Into(self.def(node, "fn", name, ctx, false))
                } else if top_level(node) {
                    Step::Into(self.def(node, "const", name, ctx, false))
                } else {
                    Step::Same
                }
            }
            "import_statement" => {
                let spec = node
                    .child_by_field_name("source")
                    .map(|s| {
                        self.text(s)
                            .trim_matches(|c| c == '\'' || c == '"' || c == '`')
                    })
                    .unwrap_or("");
                self.ts_imports(node, spec, ctx);
                Step::Skip
            }
            "call_expression" => {
                self.ts_invoke(node, ctx);
                Step::Same
            }
            "identifier" => {
                self.ts_identifier(node, ctx);
                Step::Skip
            }
            "property_identifier" => {
                if let Some(member) = node.parent().filter(|p| p.kind() == "member_expression") {
                    if same(member.child_by_field_name("property"), node) && called(member) {
                        let qual = self.receiver(member.child_by_field_name("object"));
                        self.edge(node, "method", self.text(node), &qual, ctx, false);
                    }
                }
                Step::Skip
            }
            "type_identifier" => {
                let def_name = node.parent().is_some_and(|p| {
                    matches!(
                        p.kind(),
                        "class_declaration"
                            | "abstract_class_declaration"
                            | "interface_declaration"
                            | "type_alias_declaration"
                            | "type_parameter"
                    ) && same(p.child_by_field_name("name"), node)
                });
                if !def_name {
                    self.edge(node, "ref", self.text(node), "", ctx, false);
                }
                Step::Skip
            }
            "shorthand_property_identifier" => {
                self.edge(node, "ref", self.text(node), "", ctx, false);
                Step::Skip
            }
            _ => Step::Same,
        }
    }

    fn ts_imports(&mut self, statement: Node, spec: &str, ctx: &Ctx) {
        let mut stack = vec![statement];
        while let Some(node) = stack.pop() {
            match node.kind() {
                "import_specifier" => {
                    if let Some(name) = node.child_by_field_name("name") {
                        self.edge(name, "import", self.text(name), spec, ctx, false);
                    }
                    continue;
                }
                "identifier" => {
                    // A default import (`import X from`) or `* as X`.
                    self.edge(node, "import", self.text(node), spec, ctx, false);
                    continue;
                }
                "string" => continue,
                _ => {}
            }
            let mut cursor = node.walk();
            let children: Vec<Node> = node.children(&mut cursor).collect();
            stack.extend(children.into_iter().rev());
        }
    }

    /// `invoke('name', ...)` (Tauri): an edge to the Rust command `name`.
    fn ts_invoke(&mut self, call: Node, ctx: &Ctx) {
        let Some(callee) = call.child_by_field_name("function") else {
            return;
        };
        let name = match callee.kind() {
            "identifier" => self.text(callee),
            "member_expression" => self.field_text(callee, "property").unwrap_or(""),
            _ => return,
        };
        if name != "invoke" {
            return;
        }
        let Some(args) = call.child_by_field_name("arguments") else {
            return;
        };
        let Some(first) = args.named_child(0).filter(|a| a.kind() == "string") else {
            return;
        };
        let command = self.text(first).trim_matches(|c| c == '\'' || c == '"');
        if !command.is_empty() && command.chars().all(|c| c.is_alphanumeric() || c == '_') {
            self.edge(first, "invoke", command, "", ctx, false);
        }
    }

    fn ts_identifier(&mut self, node: Node, ctx: &Ctx) {
        let Some(parent) = node.parent() else { return };
        let pk = parent.kind();
        if same(parent.child_by_field_name("name"), node)
            && matches!(
                pk,
                "function_declaration"
                    | "generator_function_declaration"
                    | "function_signature"
                    | "variable_declarator"
                    | "class_declaration"
                    | "function_expression"
                    | "enum_declaration"
            )
        {
            return;
        }
        if binding(parent, node) {
            return;
        }
        let name = self.text(node);
        let kind = match pk {
            "call_expression" if same(parent.child_by_field_name("function"), node) => "call",
            "new_expression" if same(parent.child_by_field_name("constructor"), node) => "new",
            "jsx_opening_element" | "jsx_self_closing_element" | "jsx_closing_element" => {
                if pk == "jsx_closing_element"
                    || !name.chars().next().is_some_and(char::is_uppercase)
                {
                    return;
                }
                "jsx"
            }
            _ => "ref",
        };
        self.edge(node, kind, name, "", ctx, false);
    }

    // ── Python ────────────────────────────────────────────────────────────

    fn python(&mut self, node: Node, ctx: &Ctx) -> Step {
        let kind = node.kind();
        match kind {
            "comment" | "string" => Step::Skip,
            "function_definition" => {
                let name = self.field_text(node, "name").unwrap_or("");
                let what = if ctx.container.is_empty() {
                    "fn"
                } else {
                    "method"
                };
                let test = name.starts_with("test_");
                Step::Into(self.def(node, what, name, ctx, test))
            }
            "class_definition" => {
                let name = self.field_text(node, "name").unwrap_or("").to_string();
                let mut inner = self.def(node, "class", &name, ctx, false);
                inner.container = name;
                Step::Into(inner)
            }
            "assignment" => {
                let left = node
                    .child_by_field_name("left")
                    .filter(|l| l.kind() == "identifier");
                let module_level = node
                    .parent()
                    .and_then(|p| p.parent())
                    .is_some_and(|g| g.kind() == "module");
                match left {
                    Some(left) if module_level => {
                        let name = self.text(left);
                        Step::Into(self.def(node, "var", name, ctx, false))
                    }
                    _ => Step::Same,
                }
            }
            "import_from_statement" => {
                let module = self
                    .field_text(node, "module_name")
                    .unwrap_or("")
                    .to_string();
                let mut cursor = node.walk();
                let names: Vec<Node> = node.children_by_field_name("name", &mut cursor).collect();
                for name in names {
                    let target = match name.kind() {
                        "aliased_import" => name.child_by_field_name("name"),
                        _ => Some(name),
                    };
                    if let Some(target) = target {
                        let text = self.text(target);
                        let last = text.rsplit('.').next().unwrap_or(text);
                        self.edge(target, "import", last, &module, ctx, false);
                    }
                }
                Step::Skip
            }
            "import_statement" => Step::Skip,
            "identifier" => {
                self.python_identifier(node, ctx);
                Step::Skip
            }
            _ => Step::Same,
        }
    }

    fn python_identifier(&mut self, node: Node, ctx: &Ctx) {
        let Some(parent) = node.parent() else { return };
        let pk = parent.kind();
        if same(parent.child_by_field_name("name"), node)
            && matches!(
                pk,
                "function_definition" | "class_definition" | "keyword_argument"
            )
        {
            return;
        }
        if matches!(
            pk,
            "parameters" | "default_parameter" | "typed_parameter" | "typed_default_parameter"
        ) || (pk == "assignment" && same(parent.child_by_field_name("left"), node))
            || (pk == "for_statement" && same(parent.child_by_field_name("left"), node))
            || pk.ends_with("pattern")
        {
            return;
        }
        let name = self.text(node);
        if pk == "attribute" {
            if same(parent.child_by_field_name("attribute"), node) {
                if called(parent) {
                    let qual = self.receiver(parent.child_by_field_name("object"));
                    self.edge(node, "method", name, &qual, ctx, false);
                }
                return;
            }
            self.edge(node, "ref", name, "", ctx, false);
            return;
        }
        let kind = if pk == "call" && same(parent.child_by_field_name("function"), node) {
            "call"
        } else {
            "ref"
        };
        self.edge(node, kind, name, "", ctx, false);
    }
}

/// What the walk does below a node.
enum Step {
    /// Don't look inside.
    Skip,
    /// Children share this node's context.
    Same,
    /// Children get a new context.
    Into(Ctx),
}

/// `impl<T> a::Foo<T>` is `Foo`; `&mut Foo` is `Foo`.
fn base_type(text: &str) -> String {
    let no_generics = text.split('<').next().unwrap_or(text);
    let last = no_generics.rsplit("::").next().unwrap_or(no_generics);
    last.trim_start_matches(['&', ' '])
        .trim_start_matches("mut ")
        .trim_start_matches("dyn ")
        .trim()
        .to_string()
}

/// `node` is the callee of a call (Rust `call_expression`, TS
/// `call_expression`, Python `call`), directly or through a turbofish.
fn called(node: Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        "call_expression" | "call" => same(parent.child_by_field_name("function"), node),
        "generic_function" => called(parent),
        _ => false,
    }
}

/// `node` is the callee of `parent`.
fn call_target(parent: Node, node: Node) -> bool {
    match parent.kind() {
        "call_expression" => same(parent.child_by_field_name("function"), node),
        "generic_function" => same(parent.child_by_field_name("function"), node) && called(parent),
        _ => false,
    }
}

/// `node` binds a new local name here (a pattern, a parameter, a `let`,
/// a `for`), so it is not a use.
fn binding(parent: Node, node: Node) -> bool {
    let pk = parent.kind();
    if pk.ends_with("pattern") && pk != "shorthand_property_identifier_pattern" {
        return true;
    }
    match pk {
        "shorthand_property_identifier_pattern" | "closure_parameters" | "catch_clause" => true,
        "let_declaration" | "for_expression" | "parameter" | "required_parameter"
        | "optional_parameter" | "for_in_statement" => {
            same(parent.child_by_field_name("pattern"), node)
                || same(parent.child_by_field_name("left"), node)
        }
        "arrow_function" => same(parent.child_by_field_name("parameter"), node),
        _ => false,
    }
}

/// Inside a Rust `use` declaration.
fn in_use(node: Node) -> bool {
    let mut at = node.parent();
    for _ in 0..8 {
        match at {
            Some(n) if n.kind() == "use_declaration" => return true,
            Some(n) => at = n.parent(),
            None => return false,
        }
    }
    false
}

/// A TS declaration at module level (or exported there).
fn top_level(declarator: Node) -> bool {
    let Some(decl) = declarator.parent() else {
        return false;
    };
    match decl.parent() {
        Some(p) if p.kind() == "program" => true,
        Some(p) if p.kind() == "export_statement" => {
            p.parent().is_some_and(|g| g.kind() == "program")
        }
        _ => false,
    }
}
