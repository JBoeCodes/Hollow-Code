//! Notion to vault: the field map of design section 5.
//!
//! [`import`] reads every configured data source through the read-only
//! client, downloads Working Files once (capped), and plans, on the overlay:
//! - **projects**: portfolio fields patched into `workspaces/notion-<id>/PROJECT.md`;
//! - **tasks**: Notion snapshots patched into `tasks/notion-<id>.md`
//!   (`notion_project_ids` removed); done tasks frozen with `tracker: "none"`
//!   and `archived_from: "notion"`. Open tasks get their tracker identity
//!   from the rekey, never here;
//! - **records**: meetings, people (two sources merged by name), domains,
//!   War Room, captures and Recruiter docs as their own notes, and the AI
//!   Subscription Responses as one table (work-only);
//! - **archive moves** of the Notion sync state and every `NOTION.md`.
//!
//! Existing files are patched through `frontmatter::patch` (other bytes
//! kept); new notes are rendered through it too, so files come out as the
//! app writes them. Property values map by type; a record's keys are its
//! Notion property names in snake case.

use super::files::{Move, Overlay};
use super::notion::{Fetched, NotionClient};
use super::{Item, Report, Settings, SourceInfo};
use crate::frontmatter::{self, KeyEdit};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The Recruiter workspace that holds the Docs (design section 5).
pub(crate) const DOCS_WORKSPACE: &str = "workspaces/notion-00000000000000000000000000000001";
/// AI Account Provisioning, which holds the AI Subscription Responses table.
pub(crate) const AI_SUBSCRIPTIONS_NOTE: &str =
    "workspaces/notion-00000000000000000000000000000002/outputs/ai-subscription-responses.md";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Kind {
    Projects,
    Tasks,
    Meetings,
    PeopleMap,
    PeopleMembers,
    Domains,
    WarRoom,
    Captures,
    Docs,
    AiSubscriptions,
}

impl Kind {
    pub(crate) const ALL: [Kind; 10] = [
        Kind::Projects,
        Kind::Tasks,
        Kind::Meetings,
        Kind::PeopleMap,
        Kind::PeopleMembers,
        Kind::Domains,
        Kind::WarRoom,
        Kind::Captures,
        Kind::Docs,
        Kind::AiSubscriptions,
    ];

    /// The key under `trackerImport.notionSources` in the config.
    pub(crate) fn config_key(self) -> &'static str {
        match self {
            Kind::Projects => "projects",
            Kind::Tasks => "tasks",
            Kind::Meetings => "meetings",
            Kind::PeopleMap => "peopleMap",
            Kind::PeopleMembers => "peopleMembers",
            Kind::Domains => "domains",
            Kind::WarRoom => "warRoom",
            Kind::Captures => "captures",
            Kind::Docs => "docs",
            Kind::AiSubscriptions => "aiSubscriptions",
        }
    }

    /// The ids design section 5 gives in full; the rest come from the config.
    pub(crate) fn default_source(self) -> Option<&'static str> {
        match self {
            Kind::Projects => Some("33333333-3333-4333-8333-333333333331"),
            Kind::Tasks => Some("33333333-3333-4333-8333-333333333332"),
            Kind::Meetings => Some("33333333-3333-4333-8333-333333333333"),
            _ => None,
        }
    }

    fn wants_body(self) -> bool {
        !matches!(self, Kind::Projects | Kind::Tasks | Kind::AiSubscriptions)
    }
}

/// What the importer read from Notion.
#[derive(Debug, Default)]
pub(crate) struct NotionData {
    pub rows: BTreeMap<Kind, Vec<Value>>,
    /// Page content as Markdown, by page id.
    pub bodies: BTreeMap<String, String>,
}

/// Lower case, single spaces: how names and titles are compared.
pub(crate) fn norm(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(crate) fn slug(text: &str) -> String {
    let raw: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let joined = raw
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let cut: String = joined.chars().take(60).collect();
    let cut = cut.trim_end_matches('-').to_string();
    if cut.is_empty() {
        "untitled".into()
    } else {
        cut
    }
}

fn snake(name: &str) -> String {
    slug(name).replace('-', "_")
}

/// A page's id without dashes.
pub(crate) fn pid(page: &Value) -> String {
    page["id"]
        .as_str()
        .unwrap_or("")
        .replace('-', "")
        .to_lowercase()
}

fn plain(rich: &Value) -> String {
    rich.as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p["plain_text"].as_str())
                .collect::<String>()
        })
        .unwrap_or_default()
}

fn props(page: &Value) -> impl Iterator<Item = (&String, &Value)> {
    page["properties"].as_object().into_iter().flatten()
}

/// The property named any of `names`, case-insensitively.
fn find<'a>(page: &'a Value, names: &[&str]) -> Option<&'a Value> {
    props(page)
        .find(|(name, _)| names.iter().any(|n| norm(n) == norm(name)))
        .map(|(_, v)| v)
}

pub(crate) fn title(page: &Value) -> String {
    props(page)
        .find(|(_, v)| v["type"] == "title")
        .map(|(_, v)| plain(&v["title"]).trim().to_string())
        .unwrap_or_default()
}

fn number(n: &Value) -> Option<String> {
    let f = n.as_f64()?;
    Some(if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        f.to_string()
    })
}

/// A converted property value.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Val {
    Text(String),
    List(Vec<String>),
    /// Long or multi-line text: a body section, not a key.
    Body(String),
}

impl Val {
    fn text(self) -> String {
        match self {
            Val::Text(t) | Val::Body(t) => t,
            Val::List(l) => l.join(", "),
        }
    }
    fn list(self) -> Vec<String> {
        match self {
            Val::Text(t) | Val::Body(t) => vec![t],
            Val::List(l) => l,
        }
    }
}

/// Where a Notion page lives in the vault, and people by name.
#[derive(Debug, Default)]
struct Ctx {
    ids: BTreeMap<String, String>,
    people: BTreeMap<String, String>,
}

impl Ctx {
    fn page(&self, id: &str) -> String {
        let id = id.replace('-', "").to_lowercase();
        self.ids
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("notion:{id}"))
    }
    fn person(&self, name: &str) -> String {
        self.people
            .get(&norm(name))
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }
}

fn extract(prop: &Value, ctx: &Ctx) -> Option<Val> {
    let kind = prop["type"].as_str()?;
    let inner = &prop[kind];
    let text = |t: String| (!t.trim().is_empty()).then(|| t.trim().to_string());
    let val = match kind {
        "title" | "rich_text" => {
            let t = text(plain(inner))?;
            if t.contains('\n') || t.chars().count() > 200 {
                Val::Body(t)
            } else {
                Val::Text(t)
            }
        }
        "number" => Val::Text(number(inner)?),
        "select" | "status" => Val::Text(text(inner["name"].as_str()?.to_string())?),
        "multi_select" => Val::List(
            inner
                .as_array()?
                .iter()
                .filter_map(|o| o["name"].as_str().map(str::to_string))
                .collect(),
        ),
        "date" => Val::Text(text(inner["start"].as_str()?.to_string())?),
        "people" => Val::List(
            inner
                .as_array()?
                .iter()
                .filter_map(|p| p["name"].as_str().map(str::to_string))
                .collect(),
        ),
        "relation" => Val::List(
            inner
                .as_array()?
                .iter()
                .filter_map(|r| r["id"].as_str().map(|id| ctx.page(id)))
                .collect(),
        ),
        "checkbox" => Val::Text(inner.as_bool()?.to_string()),
        "url" | "email" | "phone_number" => Val::Text(text(inner.as_str()?.to_string())?),
        "formula" => {
            let f = inner["type"].as_str()?;
            match f {
                "string" => Val::Text(text(inner["string"].as_str()?.to_string())?),
                "number" => Val::Text(number(&inner["number"])?),
                "boolean" => Val::Text(inner["boolean"].as_bool()?.to_string()),
                "date" => Val::Text(inner["date"]["start"].as_str()?.to_string()),
                _ => return None,
            }
        }
        "rollup" => match inner["type"].as_str()? {
            "number" => Val::Text(number(&inner["number"])?),
            "date" => Val::Text(inner["date"]["start"].as_str()?.to_string()),
            "array" => Val::List(
                inner["array"]
                    .as_array()?
                    .iter()
                    .filter_map(|item| extract(item, ctx))
                    .flat_map(Val::list)
                    .collect(),
            ),
            _ => return None,
        },
        "files" => Val::List(
            inner
                .as_array()?
                .iter()
                .filter_map(|f| f["name"].as_str().map(str::to_string))
                .collect(),
        ),
        "created_time" | "last_edited_time" => Val::Text(inner.as_str()?.to_string()),
        "created_by" | "last_edited_by" => Val::Text(inner["name"].as_str()?.to_string()),
        "unique_id" => {
            let n = number(&inner["number"])?;
            Val::Text(match inner["prefix"].as_str() {
                Some(prefix) if !prefix.is_empty() => format!("{prefix}-{n}"),
                _ => n,
            })
        }
        _ => return None,
    };
    match &val {
        Val::List(l) if l.is_empty() => None,
        _ => Some(val),
    }
}

fn get(page: &Value, names: &[&str], ctx: &Ctx) -> Option<Val> {
    find(page, names).and_then(|p| extract(p, ctx))
}

fn quoted_list(key: &str, items: &[String]) -> KeyEdit {
    let mut lines = vec![format!("{key}:")];
    lines.extend(
        items
            .iter()
            .map(|i| format!("  - {}", serde_json::to_string(i).unwrap())),
    );
    KeyEdit::Block(lines)
}

/// A new note, rendered through `frontmatter::patch` from a one-key start.
fn render_new(pairs: &[(String, Val)], body: &str) -> Result<String, String> {
    let (first, rest) = pairs.split_first().ok_or("A note needs a key.")?;
    let skeleton = format!(
        "---\n{}: {}\n---\n",
        first.0,
        serde_json::to_string(&first.1.clone().text()).unwrap()
    );
    let edits: Vec<(&str, KeyEdit)> = rest
        .iter()
        .map(|(k, v)| {
            let edit = match v {
                Val::List(items) => quoted_list(k, items),
                other => KeyEdit::Quoted(other.clone().text()),
            };
            (k.as_str(), edit)
        })
        .collect();
    frontmatter::patch(&skeleton, &edits, Some(body))
}

/// `text` plus a section, unless a heading starting with `prefix` is there.
pub(crate) fn append_section(text: &str, prefix: &str, section: &str) -> String {
    if frontmatter::parse(text)
        .body
        .lines()
        .any(|l| l.starts_with(prefix))
    {
        return text.to_string();
    }
    let gap = if text.ends_with("\n\n") {
        ""
    } else if text.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    format!("{text}{gap}{}\n", section.trim_end())
}

fn pages_sorted(rows: &[Value]) -> Vec<&Value> {
    let mut out: Vec<&Value> = rows.iter().collect();
    out.sort_by_key(|p| (p["created_time"].as_str().unwrap_or("").to_string(), pid(p)));
    out
}

fn allocate(taken: &mut BTreeSet<String>, stem: &str, id: &str) -> String {
    let plain = format!("{stem}.md");
    let path = if taken.contains(&plain.to_lowercase()) {
        format!("{stem}-{}.md", &id[..8.min(id.len())])
    } else {
        plain
    };
    taken.insert(path.to_lowercase());
    path
}

fn date_of(page: &Value) -> String {
    get(page, &["Date", "Meeting date", "When"], &Ctx::default())
        .map(Val::text)
        .or_else(|| {
            props(page)
                .find(|(_, v)| v["type"] == "date")
                .and_then(|(_, v)| v["date"]["start"].as_str().map(str::to_string))
        })
        .or_else(|| page["created_time"].as_str().map(str::to_string))
        .unwrap_or_default()
        .chars()
        .take(10)
        .collect()
}

/// Read every configured source, its long relations and its page content.
async fn fetch(client: &NotionClient, settings: &Settings, report: &mut Report) -> NotionData {
    let mut data = NotionData::default();
    for kind in Kind::ALL {
        let Some(id) = settings.sources.get(&kind) else {
            report.errors.push(format!(
                "No Notion data source id for {0}; set trackerImport.notionSources.{0} in the config.",
                kind.config_key()
            ));
            continue;
        };
        let source_title = match client
            .send(&super::notion::Request::DataSource { id: id.clone() })
            .await
        {
            Ok(v) => plain(&v["title"]),
            Err(e) => {
                report.errors.push(format!("{}: {e}", kind.config_key()));
                continue;
            }
        };
        let mut rows = match client.query_all(id).await {
            Ok(rows) => rows,
            Err(e) => {
                report.errors.push(format!("{}: {e}", kind.config_key()));
                continue;
            }
        };
        for row in &mut rows {
            let page = pid(row);
            let long: Vec<(String, String)> = props(row)
                .filter(|(_, v)| v["type"] == "relation" && v["has_more"] == true)
                .filter_map(|(name, v)| Some((name.clone(), v["id"].as_str()?.to_string())))
                .collect();
            for (name, property) in long {
                match client.property_all(&page, &property).await {
                    Ok(items) => {
                        let ids: Vec<Value> = items
                            .iter()
                            .filter_map(|i| i["relation"]["id"].as_str())
                            .map(|id| json!({ "id": id }))
                            .collect();
                        row["properties"][&name]["relation"] = Value::Array(ids);
                    }
                    Err(e) => report
                        .errors
                        .push(format!("{} {page}: {e}", kind.config_key())),
                }
            }
            if kind.wants_body() {
                match body(client, &page).await {
                    Ok(md) => {
                        data.bodies.insert(page.clone(), md);
                    }
                    Err(e) => report
                        .errors
                        .push(format!("{} {page} content: {e}", kind.config_key())),
                }
            }
        }
        report.sources.insert(
            kind.config_key().into(),
            SourceInfo {
                id: id.clone(),
                title: source_title,
                rows: rows.len(),
            },
        );
        data.rows.insert(kind, rows);
    }
    data
}

/// A page's content as Markdown, three levels of nesting at most.
async fn body(client: &NotionClient, page: &str) -> Result<String, String> {
    let top = client.children_all(page).await?;
    let mut kids: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut queue: Vec<(String, usize)> = nested(&top, 1);
    while let Some((id, depth)) = queue.pop() {
        let children = client.children_all(&id).await?;
        if depth < 3 {
            queue.extend(nested(&children, depth + 1));
        }
        kids.insert(id, children);
    }
    Ok(markdown(&top, &kids, 0).trim().to_string())
}

fn nested(blocks: &[Value], depth: usize) -> Vec<(String, usize)> {
    blocks
        .iter()
        .filter(|b| b["has_children"] == true)
        .filter(|b| !matches!(b["type"].as_str(), Some("child_page" | "child_database")))
        .filter_map(|b| Some((b["id"].as_str()?.to_string(), depth)))
        .collect()
}

fn markdown(blocks: &[Value], kids: &BTreeMap<String, Vec<Value>>, depth: usize) -> String {
    let pad = "  ".repeat(depth);
    let mut out = String::new();
    for block in blocks {
        let kind = block["type"].as_str().unwrap_or("");
        let inner = &block[kind];
        let text = plain(&inner["rich_text"]);
        let line = match kind {
            "paragraph" => text,
            "heading_1" => format!("# {text}"),
            "heading_2" => format!("## {text}"),
            "heading_3" => format!("### {text}"),
            "bulleted_list_item" | "toggle" => format!("- {text}"),
            "numbered_list_item" => format!("1. {text}"),
            "to_do" => format!(
                "- [{}] {text}",
                if inner["checked"] == true { "x" } else { " " }
            ),
            "quote" | "callout" => format!("> {text}"),
            "code" => format!(
                "```{}\n{text}\n```",
                inner["language"].as_str().unwrap_or("")
            ),
            "divider" => "---".into(),
            "child_page" => format!("- (page) {}", inner["title"].as_str().unwrap_or("")),
            "child_database" => format!("- (database) {}", inner["title"].as_str().unwrap_or("")),
            "bookmark" | "embed" | "link_preview" => {
                inner["url"].as_str().unwrap_or("").to_string()
            }
            "image" | "file" | "pdf" | "video" | "audio" => format!(
                "[{kind}: {}]",
                inner["name"]
                    .as_str()
                    .or(inner["caption"][0]["plain_text"].as_str())
                    .unwrap_or("attachment")
            ),
            _ => continue,
        };
        for l in line.lines() {
            out.push_str(&pad);
            out.push_str(l);
            out.push('\n');
        }
        if kind == "paragraph" || kind.starts_with("heading") {
            out.push('\n');
        }
        if let Some(children) = block["id"].as_str().and_then(|id| kids.get(id)) {
            out.push_str(&markdown(children, kids, depth + 1));
        }
    }
    out
}

/// A Working File to bring into the vault.
#[derive(Debug, Clone)]
struct Wanted {
    rel: String,
    url: String,
    name: String,
    project: String,
}

#[derive(Debug, Clone)]
enum Outcome {
    Have,
    TooLarge(u64),
    Failed(String),
}

fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ' | '(' | ')') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    let cleaned: String = cleaned.chars().take(120).collect();
    if cleaned.is_empty() {
        "file".into()
    } else {
        cleaned
    }
}

fn wanted_files(data: &NotionData) -> Vec<Wanted> {
    let mut out = Vec::new();
    for page in data.rows.get(&Kind::Projects).into_iter().flatten() {
        let id = pid(page);
        let Some(files) = find(page, &["Working Files"]).and_then(|p| p["files"].as_array()) else {
            continue;
        };
        let mut taken = BTreeSet::new();
        for (i, file) in files.iter().enumerate() {
            if file["type"] != "file" {
                continue;
            }
            let Some(url) = file["file"]["url"].as_str() else {
                continue;
            };
            let name = file["name"].as_str().unwrap_or("file");
            let mut safe = safe_name(name);
            if !taken.insert(safe.to_lowercase()) {
                safe = format!("{i}-{safe}");
            }
            out.push(Wanted {
                rel: format!("workspaces/notion-{id}/sources/notion-files/{safe}"),
                url: url.to_string(),
                name: name.to_string(),
                project: id.clone(),
            });
        }
    }
    out
}

/// Read Notion and plan every import file on the overlay.
pub(crate) async fn import(
    client: &NotionClient,
    settings: &Settings,
    overlay: &mut Overlay,
    cap: u64,
    report: &mut Report,
) -> Result<(), String> {
    let data = fetch(client, settings, report).await;
    if !data.rows.contains_key(&Kind::Projects) || !data.rows.contains_key(&Kind::Tasks) {
        return Err(format!(
            "Notion projects and tasks couldn't be read: {}",
            report.errors.join(" ")
        ));
    }
    let mut outcomes = BTreeMap::new();
    for want in wanted_files(&data) {
        let outcome = if let Some(bytes) = overlay.read(&want.rel) {
            report.working_files_bytes += bytes.len() as u64;
            Outcome::Have
        } else {
            match client.file(&want.url, cap).await {
                Ok(Fetched::Bytes(bytes)) => {
                    report.working_files_bytes += bytes.len() as u64;
                    overlay.set(&want.rel, bytes)?;
                    Outcome::Have
                }
                Ok(Fetched::TooLarge(size)) => Outcome::TooLarge(size),
                Err(e) => Outcome::Failed(e),
            }
        };
        match &outcome {
            Outcome::Have => report.working_files += 1,
            Outcome::TooLarge(size) => report.working_files_skipped.push(Item::new(
                &want.rel,
                &want.name,
                format!("{size} bytes or more, over the {cap} byte cap; not downloaded"),
            )),
            Outcome::Failed(e) => {
                report
                    .working_files_skipped
                    .push(Item::new(&want.rel, &want.name, e.clone()))
            }
        }
        outcomes
            .entry(want.project.clone())
            .or_insert_with(Vec::new)
            .push((want, outcome));
    }
    plan(&data, &outcomes, overlay, report)?;
    archive_moves(
        overlay,
        &chrono::Local::now().format("%Y-%m-%d").to_string(),
        report,
    );
    Ok(())
}

fn plan(
    data: &NotionData,
    files: &BTreeMap<String, Vec<(Wanted, Outcome)>>,
    overlay: &mut Overlay,
    report: &mut Report,
) -> Result<(), String> {
    let rows = |kind: Kind| data.rows.get(&kind).map(Vec::as_slice).unwrap_or(&[]);
    let mut ctx = Ctx::default();
    for page in rows(Kind::Projects) {
        ctx.ids
            .insert(pid(page), format!("workspaces/notion-{}", pid(page)));
    }
    for page in rows(Kind::Tasks) {
        ctx.ids
            .insert(pid(page), format!("tasks/notion-{}.md", pid(page)));
    }

    // Record paths first, so every relation can name them.
    let mut taken = BTreeSet::new();
    let mut paths: BTreeMap<(Kind, String), String> = BTreeMap::new();
    let place = |kind: Kind,
                 page: &Value,
                 stem: String,
                 ctx: &mut Ctx,
                 taken: &mut BTreeSet<String>,
                 paths: &mut BTreeMap<(Kind, String), String>| {
        let path = allocate(taken, &stem, &pid(page));
        ctx.ids.insert(pid(page), path.clone());
        paths.insert((kind, pid(page)), path);
    };
    for page in pages_sorted(rows(Kind::Meetings)) {
        let date = date_of(page);
        let year = date.get(..4).unwrap_or("undated").to_string();
        place(
            Kind::Meetings,
            page,
            format!(
                "library/records/meetings/{year}/{date}-{}",
                slug(&title(page))
            ),
            &mut ctx,
            &mut taken,
            &mut paths,
        );
    }
    // People: both sources, merged by name.
    let mut people: BTreeMap<String, (Option<&Value>, Option<&Value>)> = BTreeMap::new();
    for page in pages_sorted(rows(Kind::PeopleMap)) {
        people
            .entry(norm(&title(page)))
            .or_default()
            .0
            .get_or_insert(page);
    }
    for page in pages_sorted(rows(Kind::PeopleMembers)) {
        people
            .entry(norm(&title(page)))
            .or_default()
            .1
            .get_or_insert(page);
    }
    let mut person_paths = BTreeMap::new();
    for (name, (map, member)) in &people {
        let page = map.or(*member).unwrap();
        let path = allocate(
            &mut taken,
            &format!("library/records/people/{}", slug(&title(page))),
            &pid(page),
        );
        for p in [map, member].into_iter().flatten() {
            ctx.ids.insert(pid(p), path.clone());
        }
        ctx.people.insert(name.clone(), path.clone());
        person_paths.insert(name.clone(), path);
    }
    for (kind, folder) in [
        (Kind::Domains, "library/records/domains"),
        (Kind::WarRoom, "library/records/war-room"),
    ] {
        for page in pages_sorted(rows(kind)) {
            place(
                kind,
                page,
                format!("{folder}/{}", slug(&title(page))),
                &mut ctx,
                &mut taken,
                &mut paths,
            );
        }
    }
    for page in pages_sorted(rows(Kind::Captures)) {
        place(
            Kind::Captures,
            page,
            format!(
                "library/records/captures/{}-{}",
                date_of(page),
                slug(&title(page))
            ),
            &mut ctx,
            &mut taken,
            &mut paths,
        );
    }
    if overlay.root().join(DOCS_WORKSPACE).is_dir() {
        for page in pages_sorted(rows(Kind::Docs)) {
            let stem = format!("{DOCS_WORKSPACE}/outputs/docs/{}", slug(&title(page)));
            place(Kind::Docs, page, stem, &mut ctx, &mut taken, &mut paths);
        }
    } else if !rows(Kind::Docs).is_empty() {
        report.errors.push(format!(
            "The Recruiter workspace {DOCS_WORKSPACE} is missing; its docs were not imported."
        ));
    }

    // Projects.
    report.projects_notion = rows(Kind::Projects).len();
    for page in rows(Kind::Projects) {
        project(
            page,
            &ctx,
            files.get(&pid(page)).map(Vec::as_slice).unwrap_or(&[]),
            overlay,
            report,
        )?;
    }
    // Tasks.
    let project_titles: BTreeMap<String, String> = rows(Kind::Projects)
        .iter()
        .map(|p| (pid(p), title(p)))
        .collect();
    for page in rows(Kind::Tasks) {
        task(page, &ctx, &project_titles, overlay, report)?;
    }
    // Records.
    for ((kind, id), path) in &paths {
        let page = rows(*kind).iter().find(|p| &pid(p) == id).unwrap();
        let note = record(
            page,
            &ctx,
            data.bodies.get(id).map(String::as_str).unwrap_or(""),
            &[],
        )?;
        overlay.set(path, note.into_bytes())?;
        match kind {
            Kind::Meetings => report.meetings += 1,
            Kind::Domains => report.domains += 1,
            Kind::WarRoom => report.war_room += 1,
            Kind::Captures => report.captures += 1,
            Kind::Docs => report.docs += 1,
            _ => {}
        }
    }
    for (name, (map, member)) in &people {
        let path = &person_paths[name];
        let note = person(*map, *member, &ctx, data)?;
        overlay.set(path, note.into_bytes())?;
        report.people += 1;
        if map.is_some() && member.is_some() {
            report.people_in_both += 1;
        }
    }
    if !rows(Kind::AiSubscriptions).is_empty() {
        let folder = AI_SUBSCRIPTIONS_NOTE.rsplit_once("/outputs/").unwrap().0;
        if overlay.root().join(folder).is_dir() {
            let note = subscriptions(rows(Kind::AiSubscriptions), &ctx)?;
            overlay.set(AI_SUBSCRIPTIONS_NOTE, note.into_bytes())?;
            report.ai_subscription_rows = rows(Kind::AiSubscriptions).len();
        } else {
            report.errors.push(format!("The AI Account Provisioning workspace {folder} is missing; its table was not imported."));
        }
    }
    if !paths.is_empty() || !people.is_empty() {
        let readme = "library/records/README.md";
        if !overlay.exists(readme) {
            overlay.set(readme, RECORDS_README.as_bytes().to_vec())?;
        }
    }
    Ok(())
}

const RECORDS_README: &str = "---\ntitle: \"Records\"\nscope: \"work\"\n---\n\n# Records\n\nImported once from the Notion archive by `hollow tracker-import`. Work data: never mirrored to the phone.\nThe task tracker holds tasks and project tracking; these notes hold what it doesn't.\n\n- `meetings/<YYYY>/<YYYY-MM-DD>-<slug>.md`: date, category, attendees (people notes), projects (workspaces), action items (task notes). Plaud meeting notes go here from now on.\n- `people/<slug>.md`: the relationship map and the workspace members, merged by name. Agent-only.\n- `domains/<slug>.md`, `war-room/<slug>.md` (the AI use-case inventory), `captures/<YYYY-MM-DD>-<slug>.md`.\n\nKeys are the Notion property names in snake case; relations name vault paths, or `notion:<id>` for a page that wasn't imported.\n";

fn base_pairs(page: &Value) -> Vec<(String, Val)> {
    let mut out = vec![("notion_id".into(), Val::Text(pid(page)))];
    for (key, field) in [
        ("notion_url", "url"),
        ("notion_created", "created_time"),
        ("notion_last_edited", "last_edited_time"),
    ] {
        if let Some(v) = page[field].as_str().filter(|v| !v.is_empty()) {
            out.push((key.into(), Val::Text(v.into())));
        }
    }
    out
}

/// One record note: title, scope, every property by type, then the body.
fn record(
    page: &Value,
    ctx: &Ctx,
    content: &str,
    extra: &[(String, Val)],
) -> Result<String, String> {
    let name = title(page);
    let mut pairs: Vec<(String, Val)> = vec![
        ("title".into(), Val::Text(name.clone())),
        ("scope".into(), Val::Text("work".into())),
    ];
    let mut sections = Vec::new();
    for (prop_name, prop) in props(page) {
        if prop["type"] == "title" {
            continue;
        }
        let key = snake(prop_name);
        if pairs.iter().any(|(k, _)| *k == key) {
            continue;
        }
        match extract(prop, ctx) {
            Some(Val::Body(text)) => sections.push(format!("## {prop_name}\n\n{text}\n")),
            Some(val) => {
                if prop["type"] == "date" {
                    if let Some(end) = prop["date"]["end"].as_str() {
                        pairs.push((format!("{key}_end"), Val::Text(end.into())));
                    }
                }
                pairs.push((key, val));
            }
            None => {}
        }
    }
    pairs.extend(extra.iter().cloned());
    pairs.extend(base_pairs(page));
    let mut body = format!("# {name}\n\n");
    for s in sections {
        body.push_str(&s);
        body.push('\n');
    }
    if !content.trim().is_empty() {
        body.push_str(content.trim());
        body.push('\n');
    }
    render_new(&pairs, &body)
}

fn person(
    map: Option<&Value>,
    member: Option<&Value>,
    ctx: &Ctx,
    data: &NotionData,
) -> Result<String, String> {
    let first = map.or(member).ok_or("A person needs a page.")?;
    let mut extra: Vec<(String, Val)> = Vec::new();
    let mut content = String::new();
    if let (Some(map), Some(member)) = (map, member) {
        // The member page's own properties, where the map has no such key.
        let map_keys: BTreeSet<String> = props(map).map(|(n, _)| snake(n)).collect();
        for (name, prop) in props(member) {
            let key = snake(name);
            if prop["type"] == "title" || map_keys.contains(&key) {
                continue;
            }
            if let Some(val) = extract(prop, ctx) {
                extra.push((key, Val::Text(val.text())));
            }
        }
        extra.push(("notion_member_id".into(), Val::Text(pid(member))));
    }
    if member.is_some() {
        extra.push(("membership".into(), Val::Text("workspace member".into())));
    }
    for page in [map, member].into_iter().flatten() {
        if let Some(text) = data.bodies.get(&pid(page)).filter(|t| !t.trim().is_empty()) {
            if !content.contains(text.as_str()) {
                content.push_str(text);
                content.push_str("\n\n");
            }
        }
    }
    record(first, ctx, &content, &extra)
}

fn subscriptions(rows: &[Value], ctx: &Ctx) -> Result<String, String> {
    let mut columns: BTreeSet<String> = BTreeSet::new();
    for row in rows {
        for (name, prop) in props(row) {
            if prop["type"] != "title" {
                columns.insert(name.clone());
            }
        }
    }
    let title_name = rows
        .first()
        .and_then(|r| {
            props(r)
                .find(|(_, v)| v["type"] == "title")
                .map(|(n, _)| n.clone())
        })
        .unwrap_or_else(|| "Name".into());
    let cell = |s: String| s.replace('|', "\\|").replace('\n', " ");
    let mut table = format!(
        "| {} | {} |\n|{}\n",
        cell(title_name),
        columns
            .iter()
            .cloned()
            .map(cell)
            .collect::<Vec<_>>()
            .join(" | "),
        "---|".repeat(columns.len() + 1)
    );
    let mut sorted: Vec<&Value> = rows.iter().collect();
    sorted.sort_by_key(|r| (norm(&title(r)), pid(r)));
    for row in sorted {
        let values: Vec<String> = columns
            .iter()
            .map(|c| {
                cell(
                    find(row, &[c])
                        .and_then(|p| extract(p, ctx))
                        .map(Val::text)
                        .unwrap_or_default(),
                )
            })
            .collect();
        table.push_str(&format!(
            "| {} | {} |\n",
            cell(title(row)),
            values.join(" | ")
        ));
    }
    let body = format!("# AI Subscription Responses\n\nImported once from Notion. Holds work emails: work-only, never mirrored to the phone.\n\n{table}");
    render_new(
        &[
            (
                "title".into(),
                Val::Text("AI Subscription Responses".into()),
            ),
            ("scope".into(), Val::Text("work".into())),
            ("rows".into(), Val::Text(rows.len().to_string())),
        ],
        &body,
    )
}

fn project(
    page: &Value,
    ctx: &Ctx,
    files: &[(Wanted, Outcome)],
    overlay: &mut Overlay,
    report: &mut Report,
) -> Result<(), String> {
    let id = pid(page);
    let rel = format!("workspaces/notion-{id}/PROJECT.md");
    let Some(old) = overlay.text(&rel) else {
        report.errors.push(format!(
            "Project \"{}\" has no {rel}; skipped.",
            title(page)
        ));
        return Ok(());
    };
    let mut edits: Vec<(String, KeyEdit)> = Vec::new();
    let scalar = |key: &str, value: Option<String>, edits: &mut Vec<(String, KeyEdit)>| {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            edits.push((key.into(), KeyEdit::Quoted(v)));
        }
    };
    let people_paths = |val: Val| -> Vec<String> {
        val.list()
            .iter()
            .map(|n| {
                if n.contains('/') {
                    n.clone()
                } else {
                    ctx.person(n)
                }
            })
            .collect()
    };
    // A tracked project's title, status and target date belong to the
    // tracker; only vault fields change once the rekey has bound it.
    let tracked = tracked(&old);
    let status = get(page, &["Status"], ctx).map(Val::text);
    if !tracked {
        scalar("title", Some(title(page)), &mut edits);
        scalar("status", status.clone(), &mut edits);
        scalar(
            "target_date",
            get(page, &["Target date"], ctx).map(Val::text),
            &mut edits,
        );
    }
    scalar("notion_status", status, &mut edits);
    for (key, names) in [
        ("created", &["Created"][..]),
        ("department", &["Department"][..]),
        ("type", &["Type"][..]),
        (
            "hours_saved_per_year",
            &["Hours Saved / yr", "Hours saved per year"][..],
        ),
        ("value_per_year", &["Value / yr", "Value per year"][..]),
        ("est_yearly_impact", &["Est. yearly impact"][..]),
        ("notion_health", &["Health"][..]),
        ("github_url", &["GitHub"][..]),
        ("live_url", &["Live URL"][..]),
    ] {
        scalar(key, get(page, names, ctx).map(Val::text), &mut edits);
    }
    scalar(
        "owners_note",
        get(page, &["Assignees", "Assignee"], ctx).map(Val::text),
        &mut edits,
    );
    let mut lists: Vec<(&str, Vec<String>)> = Vec::new();
    for (key, names, people) in [
        ("requested_by", &["Requested by"][..], true),
        ("stakeholders", &["Stakeholders"][..], true),
        ("domains", &["Domain", "Domains"][..], false),
        ("meetings", &["Meetings"][..], false),
        ("captures", &["Captures"][..], false),
        ("war_room", &["War Room"][..], false),
        ("supersedes", &["Supersedes"][..], false),
        ("superseded_by", &["Superseded by"][..], false),
        ("docs", &["Docs"][..], false),
    ] {
        if let Some(val) = get(page, names, ctx) {
            lists.push((
                key,
                if people {
                    people_paths(val)
                } else {
                    val.list()
                },
            ));
        }
    }
    if let Some(signals) = find(page, &["Signals"]).and_then(|p| p["relation"].as_array()) {
        let ids: Vec<String> = signals
            .iter()
            .filter_map(|r| r["id"].as_str().map(|i| i.replace('-', "")))
            .collect();
        if !ids.is_empty() {
            lists.push(("notion_signal_ids", ids));
        }
    }
    let mut kept = Vec::new();
    let mut skipped = Vec::new();
    for (want, outcome) in files {
        match outcome {
            Outcome::Have => kept.push(want.rel.clone()),
            Outcome::TooLarge(_) => skipped.push(format!("{} (over the 25 MB cap)", want.name)),
            Outcome::Failed(_) => skipped.push(format!("{} (download failed)", want.name)),
        }
    }
    if let Some(all) = find(page, &["Working Files"]).and_then(|p| p["files"].as_array()) {
        kept.extend(
            all.iter()
                .filter(|f| f["type"] == "external")
                .filter_map(|f| f["external"]["url"].as_str().map(str::to_string)),
        );
    }
    if !kept.is_empty() {
        lists.push(("working_files", kept));
    }
    if !skipped.is_empty() {
        lists.push(("working_files_skipped", skipped));
    }
    scalar(
        "notion_last_edited",
        page["last_edited_time"].as_str().map(str::to_string),
        &mut edits,
    );
    scalar("notion_id", Some(id.clone()), &mut edits);
    scalar(
        "notion_url",
        page["url"].as_str().map(str::to_string),
        &mut edits,
    );
    for (key, items) in lists {
        edits.push((key.into(), quoted_list(key, &items)));
    }
    let pairs: Vec<(&str, KeyEdit)> = edits.iter().map(|(k, e)| (k.as_str(), e.clone())).collect();
    let mut text = frontmatter::patch(&old, &pairs, None).map_err(|e| format!("{rel}: {e}"))?;
    if let Some(description) = get(page, &["Description"], ctx).map(Val::text) {
        let date = page["last_edited_time"]
            .as_str()
            .unwrap_or("")
            .get(..10)
            .unwrap_or("")
            .to_string();
        text = append_section(
            &text,
            "## Notion description",
            &format!("## Notion description (final, {date})\n\n{description}"),
        );
    }
    overlay.set(&rel, text.into_bytes())
}

/// Bound to a tracker issue or project (not frozen with `tracker: "none"`).
fn tracked(text: &str) -> bool {
    frontmatter::parse(text)
        .get("tracker")
        .is_some_and(|t| !t.trim().is_empty() && t != "none")
}

const CLOSED: &[&str] = &[
    "done",
    "delivered",
    "complete",
    "completed",
    "canceled",
    "cancelled",
    "dropped",
    "superseded",
    "archived",
];

/// A Notion status word that means the task is finished.
pub(crate) fn closed_word(word: &str) -> bool {
    CLOSED.contains(&norm(word).as_str())
}

fn task(
    page: &Value,
    ctx: &Ctx,
    project_titles: &BTreeMap<String, String>,
    overlay: &mut Overlay,
    report: &mut Report,
) -> Result<(), String> {
    let id = pid(page);
    let rel = format!("tasks/notion-{id}.md");
    let status = get(page, &["Status"], ctx)
        .map(Val::text)
        .unwrap_or_default();
    let done =
        closed_word(&status) || get(page, &["Done"], ctx).is_some_and(|v| v.text() == "true");
    let mut edits: Vec<(&str, KeyEdit)> = Vec::new();
    let task_id = get(page, &["Task ID", "ID"], ctx)
        .map(Val::text)
        .or_else(|| {
            props(page)
                .find(|(_, v)| v["type"] == "unique_id")
                .and_then(|(_, v)| extract(v, ctx))
                .map(Val::text)
        });
    if let Some(task_id) = task_id {
        edits.push(("notion_task_id", KeyEdit::Quoted(task_id)));
    }
    if !status.is_empty() {
        edits.push(("notion_status", KeyEdit::Quoted(status.clone())));
    }
    let due = get(page, &["Due date", "Due"], ctx)
        .map(Val::text)
        .unwrap_or_default();
    if let Some(time) = due
        .split_once('T')
        .map(|(_, t)| t.chars().take(5).collect::<String>())
    {
        edits.push(("due_time", KeyEdit::Quoted(time)));
    }
    if let Some(meeting) =
        get(page, &["Meeting", "Meetings"], ctx).and_then(|v| v.list().into_iter().next())
    {
        edits.push(("meeting", KeyEdit::Quoted(meeting)));
    }
    let old_text = overlay.text(&rel);
    if old_text.as_deref().is_some_and(tracked) {
        // Bound by the rekey: the tracker owns its state now.
        report.open_tasks += usize::from(!done);
        report.done_tasks += usize::from(done);
    } else if done {
        report.done_tasks += 1;
        edits.push(("tracker", KeyEdit::Quoted("none".into())));
        edits.push(("archived_from", KeyEdit::Quoted("notion".into())));
        if let Some(completed) = get(page, &["Completed"], ctx).map(Val::text) {
            edits.push(("completed", KeyEdit::Quoted(completed)));
        }
    } else {
        report.open_tasks += 1;
        if let Some(names) = get(page, &["Assignee", "Assignees"], ctx).map(Val::text) {
            edits.push(("assignee_note", KeyEdit::Quoted(names)));
        }
    }
    edits.push(("notion_project_ids", KeyEdit::Remove));
    let old = match old_text {
        Some(old) => old,
        None => {
            report.task_files_created += 1;
            let project = get(page, &["Project"], ctx)
                .and_then(|v| v.list().into_iter().next())
                .filter(|p| p.starts_with("workspaces/"));
            let mut pairs = vec![
                ("notion_id".to_string(), Val::Text(id.clone())),
                ("title".into(), Val::Text(title(page))),
                ("status".into(), Val::Text(status.clone())),
            ];
            if let Some(p) = get(page, &["Priority"], ctx) {
                pairs.push(("priority".into(), Val::Text(p.text())));
            }
            if !due.is_empty() {
                pairs.push(("due".into(), Val::Text(due.chars().take(10).collect())));
            }
            if let Some(folder) = project {
                let pid = folder.trim_start_matches("workspaces/notion-").to_string();
                if let Some(name) = project_titles.get(&pid) {
                    pairs.push(("project".into(), Val::Text(name.clone())));
                }
                pairs.push((
                    "project_url".into(),
                    Val::Text(format!("vault:{folder}/PROJECT.md")),
                ));
            }
            if let Some(url) = page["url"].as_str() {
                pairs.push(("notion_url".into(), Val::Text(url.into())));
            }
            render_new(&pairs, "")?
        }
    };
    let text = frontmatter::patch(&old, &edits, None).map_err(|e| format!("{rel}: {e}"))?;
    overlay.set(&rel, text.into_bytes())
}

/// The Notion sync state and every NOTION.md, into `.archive/<date>/`.
fn archive_moves(overlay: &mut Overlay, date: &str, report: &mut Report) {
    let root = overlay.root().to_path_buf();
    let mut from: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if (name.starts_with(".notion-sync") && name.ends_with(".json") && is_file)
            || (name == ".notion-sync-backups" && is_dir)
            || (name.starts_with(".notion-imports.") && (is_file || is_dir))
            || (name == "NOTION.md" && is_file)
        {
            from.push(name);
        }
    }
    for dir in overlay.dirs_in("workspaces") {
        let rel = format!("workspaces/{dir}/NOTION.md");
        if std::fs::symlink_metadata(root.join(&rel)).is_ok_and(|m| m.is_file()) {
            from.push(rel);
        }
    }
    from.sort();
    for rel in from {
        let to = format!(".archive/{date}/{rel}");
        report.archive_moves.push(format!("{rel} -> {to}"));
        overlay.moves.push(Move { from: rel, to });
    }
}
