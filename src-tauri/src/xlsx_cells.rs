//! A bounded, typed reader for Excel `.xlsx` workbooks: sheets, in workbook
//! order, then rows, then cells with their types.
//!
//! The old reader returned `xl/sharedStrings.xml` as if it were the sheet:
//! every distinct string in the workbook, in pool order, with no numbers,
//! dates, booleans or formulas, and with strings no cell uses (review B,
//! slice 5 item 5). This one walks each worksheet's `<c>` elements and
//! resolves them: a shared-string index to its string, inline strings,
//! numbers as stored, booleans, error values, numbers whose cell style is a
//! date or time format converted (with the serial kept), ISO dates, and a
//! formula's cached value next to the formula.
//!
//! Everything is capped ([`LIMITS`]): sheets, rows per sheet, cells, total
//! cell text and total *uncompressed* XML (a zip bomb inflates no further
//! than the cap). Anything a cap cut is listed in [`Workbook::truncated`],
//! and [`render`] puts that marker first so a text clip can't hide it. No
//! entities beyond the five XML ones and character references are expanded.
use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

/// Caps on what one read may take.
pub(crate) struct Limits {
    pub sheets: usize,
    /// Rows with at least one cell, per sheet.
    pub rows: usize,
    /// Cells with a value or formula, across the workbook.
    pub cells: usize,
    /// Uncompressed XML read from the archive, across every part.
    pub xml_bytes: u64,
    /// Cell text kept, across the workbook (characters).
    pub text_chars: usize,
    /// Longest single cell value kept (characters); longer ones end in "…".
    pub cell_chars: usize,
}

pub(crate) const LIMITS: Limits = Limits {
    sheets: 32,
    rows: 10_000,
    cells: 20_000,
    xml_bytes: 50 * 1024 * 1024,
    text_chars: 60_000,
    cell_chars: 2_000,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CellValue {
    Text(String),
    /// A number as the file stores it (`1200`, `0.1`, `1.5E+20`).
    Number(String),
    Bool(bool),
    /// An error value such as `#DIV/0!` or `#N/A`.
    Error(String),
    /// A number whose cell style is a date or time format: the serial as
    /// stored and, when it converts, its date/time.
    Date {
        serial: String,
        reading: Option<String>,
    },
    /// `t="d"`: an ISO 8601 date stored as text.
    IsoDate(String),
    /// A value Hollow couldn't resolve, with the reason (e.g. a shared-string
    /// index the workbook doesn't have).
    Unresolved(String),
    /// A formula with no cached value.
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Cell {
    /// The cell reference, e.g. `B2`.
    pub at: String,
    pub value: CellValue,
    /// The formula without its `=`; empty for a shared formula's dependents.
    pub formula: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Row {
    /// The sheet row number (1-based); [`render`] doesn't need it, as each
    /// cell's reference carries it.
    pub number: u32,
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sheet {
    pub name: String,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Workbook {
    pub sheets: Vec<Sheet>,
    /// What a cap cut, one line each; empty when the read was complete.
    pub truncated: Vec<String>,
}

/// Read a workbook's cells within `lim`.
pub(crate) fn read(bytes: &[u8], lim: &Limits) -> Result<Workbook, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| "not a readable Office file".to_string())?;
    let mut budget = Budget {
        left: lim.xml_bytes,
        cut: false,
    };
    let workbook = part(&mut zip, "xl/workbook.xml", &mut budget)?.unwrap_or_default();
    let rels = part(&mut zip, "xl/_rels/workbook.xml.rels", &mut budget)?.unwrap_or_default();
    let styles = part(&mut zip, "xl/styles.xml", &mut budget)?.unwrap_or_default();
    let date1904 = tokens(&workbook).any(|t| match t {
        Tok::Open {
            name: "workbookPr",
            attrs,
            ..
        } => matches!(attr(attrs, "date1904").as_deref(), Some("1" | "true")),
        _ => false,
    });
    let (mut list, shared_path) = sheet_list(&workbook, &rels);
    if list.is_empty() {
        list = fallback_sheets(zip.file_names());
    }
    let shared_xml = part(&mut zip, &shared_path, &mut budget)?.unwrap_or_default();
    let ctx = Ctx {
        shared: shared_strings(&shared_xml, lim.cell_chars),
        shared_cut: budget.cut,
        dates: date_styles(&styles),
        date1904,
    };
    drop(shared_xml);

    let mut wb = Workbook::default();
    if list.len() > lim.sheets {
        wb.truncated
            .push(format!("first {} of {} sheets", lim.sheets, list.len()));
        list.truncate(lim.sheets);
    }
    let mut used = Used {
        cells: 0,
        chars: 0,
        stop: false,
    };
    for (name, path) in list {
        if used.stop {
            break;
        }
        if budget.left == 0 {
            wb.truncated.push(format!(
                "stopped before sheet \"{name}\": the {} uncompressed-XML limit was reached",
                size(lim.xml_bytes)
            ));
            break;
        }
        let before = budget.cut;
        let xml = part(&mut zip, &path, &mut budget)?.unwrap_or_default();
        if budget.cut && !before {
            wb.truncated.push(format!(
                "sheet \"{name}\" read only up to the {} uncompressed-XML limit",
                size(lim.xml_bytes)
            ));
        }
        let sheet = read_sheet(&xml, name, &ctx, lim, &mut used, &mut wb.truncated);
        wb.sheets.push(sheet);
    }
    if ctx.shared_cut && !wb.truncated.iter().any(|t| t.contains("uncompressed-XML")) {
        wb.truncated.push(format!(
            "the shared strings were read only up to the {} uncompressed-XML limit",
            size(lim.xml_bytes)
        ));
    }
    Ok(wb)
}

/// `50 MB`, `64 KB`.
fn size(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{} MB", bytes >> 20)
    } else {
        format!("{} KB", bytes >> 10)
    }
}

/// The workbook as text, one line per row: `A1 "Item" | B1 1200`. Text is
/// JSON-quoted so `"1200"` (text) and `1200` (a number) stay apart. Any
/// truncation is stated on the first line.
pub(crate) fn render(wb: &Workbook) -> String {
    let mut out = String::new();
    if !wb.truncated.is_empty() {
        out.push_str(&format!("[truncated: {}]\n\n", wb.truncated.join("; ")));
    }
    out.push_str(&format!(
        "Excel workbook, {} sheet{}. Cells as `ref value`: text quoted, numbers bare, dates converted with their serial, formulas with their cached value.\n",
        wb.sheets.len(),
        if wb.sheets.len() == 1 { "" } else { "s" }
    ));
    for (i, sheet) in wb.sheets.iter().enumerate() {
        out.push_str(&format!("\n## Sheet {}: {}\n", i + 1, sheet.name));
        if sheet.rows.is_empty() {
            out.push_str("(no cells)\n");
        }
        for row in &sheet.rows {
            let line: Vec<String> = row.cells.iter().map(show_cell).collect();
            out.push_str(&line.join(" | "));
            out.push('\n');
        }
    }
    out
}

fn show_cell(c: &Cell) -> String {
    let v = match &c.value {
        CellValue::Text(s) => serde_json::to_string(s).unwrap_or_default(),
        CellValue::Number(n) => n.clone(),
        CellValue::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).to_string(),
        CellValue::Error(e) => format!("{e} (error)"),
        CellValue::Date {
            serial,
            reading: Some(r),
        } => format!("{r} (date, serial {serial})"),
        CellValue::Date {
            serial,
            reading: None,
        } => format!("{serial} (date-formatted serial)"),
        CellValue::IsoDate(d) => format!("{d} (date)"),
        CellValue::Unresolved(why) => format!("[{why}]"),
        CellValue::Empty => "(no cached value)".to_string(),
    };
    match c.formula.as_deref() {
        Some("") => format!("{} {v} (shared formula)", c.at),
        Some(f) => format!("{} {v} (formula ={f})", c.at),
        None => format!("{} {v}", c.at),
    }
}

// ── archive parts ──────────────────────────────────────────────────────────

struct Budget {
    left: u64,
    /// A part was cut short by the cap.
    cut: bool,
}

/// One part's XML, at most `budget.left` bytes of it (`None` if absent).
fn part(
    zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    name: &str,
    budget: &mut Budget,
) -> Result<Option<String>, String> {
    let Ok(entry) = zip.by_name(name) else {
        return Ok(None);
    };
    let mut buf = Vec::new();
    // One byte past the cap tells a cut part from one that fits exactly.
    entry
        .take(budget.left.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|_| "unreadable Office XML".to_string())?;
    if buf.len() as u64 > budget.left {
        buf.truncate(budget.left as usize);
        budget.cut = true;
    }
    budget.left -= buf.len() as u64;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

/// `(sheet name, part path)` in workbook order, and the shared-strings path.
fn sheet_list(workbook: &str, rels: &str) -> (Vec<(String, String)>, String) {
    let mut targets = HashMap::new();
    let mut shared = "xl/sharedStrings.xml".to_string();
    for t in tokens(rels) {
        if let Tok::Open {
            name: "Relationship",
            attrs,
            ..
        } = t
        {
            let (Some(id), Some(target)) = (attr(attrs, "Id"), attr(attrs, "Target")) else {
                continue;
            };
            let path = match target.strip_prefix('/') {
                Some(abs) => abs.to_string(),
                None => format!("xl/{target}"),
            };
            if attr(attrs, "Type").is_some_and(|ty| ty.ends_with("/sharedStrings")) {
                shared = path.clone();
            }
            targets.insert(id, path);
        }
    }
    let mut out = Vec::new();
    for t in tokens(workbook) {
        if let Tok::Open {
            name: "sheet",
            attrs,
            ..
        } = t
        {
            let Some(path) = attr(attrs, "id").and_then(|id| targets.get(&id).cloned()) else {
                continue;
            };
            let mut name = attr(attrs, "name").unwrap_or_else(|| format!("Sheet{}", out.len() + 1));
            if attr(attrs, "state").is_some_and(|s| s != "visible") {
                name.push_str(" (hidden)");
            }
            out.push((name, path));
        }
    }
    (out, shared)
}

/// Without a usable workbook.xml: `xl/worksheets/sheetN.xml` in number order.
fn fallback_sheets<'a>(names: impl Iterator<Item = &'a str>) -> Vec<(String, String)> {
    let mut paths: Vec<String> = names
        .filter(|n| n.starts_with("xl/worksheets/sheet") && n.ends_with(".xml"))
        .map(str::to_string)
        .collect();
    paths.sort_by_key(|n| (n.len(), n.clone()));
    paths
        .into_iter()
        .enumerate()
        .map(|(i, p)| (format!("Sheet{}", i + 1), p))
        .collect()
}

fn shared_strings(xml: &str, cell_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    let (mut capture, mut phonetic) = (false, false);
    for t in tokens(xml) {
        match t {
            Tok::Open {
                name: "si",
                empty: true,
                ..
            } => out.push(String::new()),
            Tok::Open { name: "si", .. } => cur = Some(String::new()),
            Tok::Close("si") => {
                if let Some(s) = cur.take() {
                    out.push(clip(&s, cell_chars));
                }
            }
            // Phonetic (furigana) runs carry `<t>` too; they aren't the text.
            Tok::Open {
                name: "rPh",
                empty: false,
                ..
            } => phonetic = true,
            Tok::Close("rPh") => phonetic = false,
            Tok::Open {
                name: "t",
                empty: false,
                ..
            } => capture = cur.is_some() && !phonetic,
            Tok::Close("t") => capture = false,
            Tok::Text(text, cdata) if capture => {
                if let Some(s) = cur.as_mut() {
                    if s.len() <= cell_chars * 4 {
                        s.push_str(&if cdata {
                            text.to_string()
                        } else {
                            unescape(text)
                        });
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// For each cell style (`cellXfs` index), whether its number format is a date
/// or time format.
fn date_styles(styles: &str) -> Vec<bool> {
    let mut custom: HashMap<u32, bool> = HashMap::new();
    let mut out = Vec::new();
    let mut in_xfs = false;
    for t in tokens(styles) {
        match t {
            Tok::Open {
                name: "numFmt",
                attrs,
                ..
            } => {
                if let (Some(id), Some(code)) = (
                    attr(attrs, "numFmtId").and_then(|s| s.parse().ok()),
                    attr(attrs, "formatCode"),
                ) {
                    custom.insert(id, is_date_format(&code));
                }
            }
            Tok::Open {
                name: "cellXfs",
                empty,
                ..
            } => in_xfs = !empty,
            Tok::Close("cellXfs") => in_xfs = false,
            Tok::Open {
                name: "xf", attrs, ..
            } if in_xfs => {
                let id: u32 = attr(attrs, "numFmtId")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                out.push(custom.get(&id).copied().unwrap_or_else(|| builtin_date(id)));
            }
            _ => {}
        }
    }
    out
}

/// Excel's built-in number formats that are dates or times (ECMA-376 §18.8.30,
/// plus the East Asian date formats).
fn builtin_date(id: u32) -> bool {
    matches!(id, 14..=22 | 27..=36 | 45..=47 | 50..=58)
}

/// Whether a custom format code shows a date or time: a `y m d h s` token
/// outside quoted text, `[...]` sections, escapes and fill characters.
fn is_date_format(code: &str) -> bool {
    let lower = code.to_ascii_lowercase().replace("general", "");
    let mut chars = lower.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                for q in chars.by_ref() {
                    if q == '"' {
                        break;
                    }
                }
            }
            '[' => {
                for q in chars.by_ref() {
                    if q == ']' {
                        break;
                    }
                }
            }
            '\\' | '_' | '*' => {
                chars.next();
            }
            'y' | 'm' | 'd' | 'h' | 's' => return true,
            _ => {}
        }
    }
    false
}

/// A date serial as `YYYY-MM-DD`, `YYYY-MM-DD HH:MM:SS` or (a time with no
/// day) `HH:MM:SS`. `None` when it isn't a date Excel could show.
pub(crate) fn serial_date(serial: &str, date1904: bool) -> Option<String> {
    let n: f64 = serial.trim().parse().ok()?;
    if !n.is_finite() || !(0.0..2_958_466.0).contains(&n) {
        return None;
    }
    let total = (n * 86_400.0).round() as i64;
    let (days, secs) = (total.div_euclid(86_400), total.rem_euclid(86_400));
    let time = chrono::NaiveTime::from_num_seconds_from_midnight_opt(secs as u32, 0)?;
    let date = if date1904 {
        chrono::NaiveDate::from_ymd_opt(1904, 1, 1)?
            .checked_add_days(chrono::Days::new(days as u64))?
    } else if days == 0 {
        return (secs > 0).then(|| time.format("%H:%M:%S").to_string());
    } else if days == 60 {
        // Excel's 1900-02-29, a day that never was (the Lotus 1-2-3 leap bug).
        return None;
    } else {
        // Day 1 is 1900-01-01; past the phantom 29 February every day is one later.
        let base = if days < 60 {
            chrono::NaiveDate::from_ymd_opt(1899, 12, 31)?
        } else {
            chrono::NaiveDate::from_ymd_opt(1899, 12, 30)?
        };
        base.checked_add_days(chrono::Days::new(days as u64))?
    };
    Some(if secs == 0 {
        date.format("%Y-%m-%d").to_string()
    } else {
        format!("{} {}", date.format("%Y-%m-%d"), time.format("%H:%M:%S"))
    })
}

// ── worksheets ─────────────────────────────────────────────────────────────

struct Ctx {
    shared: Vec<String>,
    /// The shared strings were cut by the byte cap, so a missing index may
    /// just be past it.
    shared_cut: bool,
    dates: Vec<bool>,
    date1904: bool,
}

struct Used {
    cells: usize,
    chars: usize,
    stop: bool,
}

#[derive(Clone, Copy)]
enum Capture {
    V,
    F,
    T,
}

#[derive(Default)]
struct Pending {
    at: String,
    kind: String,
    style: usize,
    v: Option<String>,
    f: Option<String>,
    inline: Option<String>,
}

fn read_sheet(
    xml: &str,
    name: String,
    ctx: &Ctx,
    lim: &Limits,
    used: &mut Used,
    notes: &mut Vec<String>,
) -> Sheet {
    let mut rows: Vec<Row> = Vec::new();
    let mut row: Option<Row> = None;
    let (mut row_no, mut next_col) = (0u32, 0u32);
    let mut cell: Option<Pending> = None;
    let mut capture: Option<Capture> = None;
    let (mut in_is, mut phonetic) = (false, false);
    for t in tokens(xml) {
        match t {
            Tok::Open {
                name: "row",
                attrs,
                empty,
            } => {
                if let Some(r) = row.take() {
                    if !r.cells.is_empty() {
                        rows.push(r);
                    }
                }
                if rows.len() >= lim.rows {
                    notes.push(format!(
                        "sheet \"{name}\": first {} rows with cells only",
                        lim.rows
                    ));
                    break;
                }
                row_no = attr(attrs, "r")
                    .and_then(|r| r.parse().ok())
                    .unwrap_or(row_no.saturating_add(1));
                next_col = 0;
                if !empty {
                    row = Some(Row {
                        number: row_no,
                        cells: Vec::new(),
                    });
                }
            }
            Tok::Close("row") => {
                if let Some(r) = row.take() {
                    if !r.cells.is_empty() {
                        rows.push(r);
                    }
                }
            }
            Tok::Open {
                name: "c",
                attrs,
                empty,
            } => {
                let at =
                    attr(attrs, "r").unwrap_or_else(|| format!("{}{row_no}", col_name(next_col)));
                next_col = col_of(&at).unwrap_or(next_col) + 1;
                // A self-closing cell has a style and no value: nothing to show.
                if !empty {
                    cell = Some(Pending {
                        at,
                        kind: attr(attrs, "t").unwrap_or_default(),
                        style: attr(attrs, "s").and_then(|s| s.parse().ok()).unwrap_or(0),
                        ..Pending::default()
                    });
                }
            }
            Tok::Open {
                name: "v",
                empty: false,
                ..
            } if cell.is_some() => capture = Some(Capture::V),
            Tok::Open {
                name: "f", empty, ..
            } if cell.is_some() => {
                if let Some(p) = cell.as_mut() {
                    p.f = Some(String::new());
                }
                if !empty {
                    capture = Some(Capture::F);
                }
            }
            Tok::Open {
                name: "is", empty, ..
            } if cell.is_some() => {
                if let Some(p) = cell.as_mut() {
                    p.inline = Some(String::new());
                }
                in_is = !empty;
            }
            Tok::Close("is") => in_is = false,
            Tok::Open {
                name: "rPh",
                empty: false,
                ..
            } => phonetic = true,
            Tok::Close("rPh") => phonetic = false,
            Tok::Open {
                name: "t",
                empty: false,
                ..
            } if in_is && !phonetic => capture = Some(Capture::T),
            Tok::Close("v" | "f" | "t") => capture = None,
            Tok::Text(text, cdata) => {
                if let (Some(cap), Some(p)) = (capture, cell.as_mut()) {
                    let s = if cdata {
                        text.to_string()
                    } else {
                        unescape(text)
                    };
                    let slot = match cap {
                        Capture::V => &mut p.v,
                        Capture::F => &mut p.f,
                        Capture::T => &mut p.inline,
                    };
                    slot.get_or_insert_with(String::new).push_str(&s);
                }
            }
            Tok::Close("c") => {
                let Some(p) = cell.take() else { continue };
                capture = None;
                in_is = false;
                let value = typed(&p, ctx, lim.cell_chars);
                if value == CellValue::Empty && p.f.is_none() {
                    continue;
                }
                let formula = p.f.as_deref().map(|f| clip(f.trim(), lim.cell_chars));
                let chars =
                    value_chars(&value) + formula.as_deref().map_or(0, |f| f.chars().count());
                if used.cells >= lim.cells {
                    notes.push(format!(
                        "stopped at the {}-cell limit, in sheet \"{name}\" at {}",
                        lim.cells, p.at
                    ));
                    used.stop = true;
                    break;
                }
                if used.chars + chars > lim.text_chars {
                    notes.push(format!(
                        "stopped at the {}-character cell-text limit, in sheet \"{name}\" at {}",
                        lim.text_chars, p.at
                    ));
                    used.stop = true;
                    break;
                }
                used.cells += 1;
                used.chars += chars;
                if let Some(r) = row.as_mut() {
                    r.cells.push(Cell {
                        at: p.at,
                        value,
                        formula,
                    });
                }
            }
            _ => {}
        }
    }
    if let Some(r) = row.take() {
        if !r.cells.is_empty() {
            rows.push(r);
        }
    }
    Sheet { name, rows }
}

fn typed(p: &Pending, ctx: &Ctx, cell_chars: usize) -> CellValue {
    let v = p.v.as_deref().map(str::trim).filter(|v| !v.is_empty());
    match p.kind.as_str() {
        "s" => {
            let Some(raw) = v else {
                return CellValue::Empty;
            };
            match raw.parse::<usize>() {
                Ok(i) => match ctx.shared.get(i) {
                    Some(s) => CellValue::Text(s.clone()),
                    None if ctx.shared_cut => {
                        CellValue::Unresolved(format!("shared string {i}, past the read limit"))
                    }
                    None => CellValue::Unresolved(format!(
                        "shared string {i}, missing from the workbook"
                    )),
                },
                Err(_) => {
                    CellValue::Unresolved(format!("bad shared-string index {}", clip(raw, 20)))
                }
            }
        }
        "inlineStr" => match (&p.inline, &p.v) {
            (Some(s), _) | (None, Some(s)) => CellValue::Text(clip(s, cell_chars)),
            (None, None) => CellValue::Empty,
        },
        // A formula's cached string: whitespace is part of the value.
        "str" => {
            p.v.as_deref()
                .map_or(CellValue::Empty, |s| CellValue::Text(clip(s, cell_chars)))
        }
        "b" => match v {
            Some("1" | "true" | "TRUE") => CellValue::Bool(true),
            Some("0" | "false" | "FALSE") => CellValue::Bool(false),
            Some(other) => CellValue::Unresolved(format!("boolean {}", clip(other, 20))),
            None => CellValue::Empty,
        },
        "e" => v.map_or(CellValue::Empty, |e| CellValue::Error(clip(e, 40))),
        "d" => v.map_or(CellValue::Empty, |d| CellValue::IsoDate(clip(d, 40))),
        _ => match v {
            None => CellValue::Empty,
            Some(n) if ctx.dates.get(p.style).copied().unwrap_or(false) => CellValue::Date {
                serial: clip(n, 40),
                reading: serial_date(n, ctx.date1904),
            },
            Some(n) => CellValue::Number(clip(n, 40)),
        },
    }
}

fn value_chars(v: &CellValue) -> usize {
    match v {
        CellValue::Text(s)
        | CellValue::Number(s)
        | CellValue::Error(s)
        | CellValue::IsoDate(s)
        | CellValue::Unresolved(s) => s.chars().count(),
        CellValue::Date { serial, reading } => {
            serial.len() + reading.as_ref().map_or(0, String::len)
        }
        CellValue::Bool(_) => 5,
        CellValue::Empty => 0,
    }
}

/// `0` → `A`, `25` → `Z`, `26` → `AA`.
fn col_name(mut i: u32) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// The zero-based column of a reference like `AB12`.
fn col_of(at: &str) -> Option<u32> {
    let letters: Vec<u8> = at.bytes().take_while(u8::is_ascii_alphabetic).collect();
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let n = letters.iter().fold(0u32, |n, b| {
        n * 26 + u32::from(b.to_ascii_uppercase() - b'A') + 1
    });
    Some(n - 1)
}

fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &s[..cut]),
        None => s.to_string(),
    }
}

// ── XML ────────────────────────────────────────────────────────────────────

/// Just enough XML for SpreadsheetML: element names lose their namespace
/// prefix, comments, processing instructions and DOCTYPEs are skipped (no
/// entity is ever defined or expanded), and an unfinished tag at the end of a
/// cut part ends the stream.
#[derive(Debug, PartialEq)]
enum Tok<'a> {
    Open {
        name: &'a str,
        attrs: &'a str,
        empty: bool,
    },
    Close(&'a str),
    /// Text, and whether it was CDATA (taken literally).
    Text(&'a str, bool),
}

fn tokens(xml: &str) -> Tokens<'_> {
    Tokens { s: xml, i: 0 }
}

struct Tokens<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Tok<'a>;

    fn next(&mut self) -> Option<Tok<'a>> {
        loop {
            let rest = &self.s[self.i..];
            if rest.is_empty() {
                return None;
            }
            if !rest.starts_with('<') {
                let end = rest.find('<').unwrap_or(rest.len());
                self.i += end;
                return Some(Tok::Text(&rest[..end], false));
            }
            if let Some(body) = rest.strip_prefix("<![CDATA[") {
                let end = body.find("]]>")?;
                self.i += "<![CDATA[".len() + end + "]]>".len();
                return Some(Tok::Text(&body[..end], true));
            }
            if rest.starts_with("<!--") {
                let end = rest.find("-->")?;
                self.i += end + "-->".len();
                continue;
            }
            if rest.starts_with("<?") || rest.starts_with("<!") {
                let end = rest.find('>')?;
                self.i += end + 1;
                continue;
            }
            // The tag ends at the first `>` outside a quoted attribute value.
            let mut quote = 0u8;
            let mut end = None;
            for (k, &b) in rest.as_bytes().iter().enumerate().skip(1) {
                if quote != 0 {
                    if b == quote {
                        quote = 0;
                    }
                } else if b == b'"' || b == b'\'' {
                    quote = b;
                } else if b == b'>' {
                    end = Some(k);
                    break;
                }
            }
            let end = end?;
            let inner = &rest[1..end];
            self.i += end + 1;
            if let Some(name) = inner.strip_prefix('/') {
                return Some(Tok::Close(local(name.trim())));
            }
            let (inner, empty) = match inner.strip_suffix('/') {
                Some(x) => (x, true),
                None => (inner, false),
            };
            let split = inner
                .find(|c: char| c.is_ascii_whitespace())
                .unwrap_or(inner.len());
            return Some(Tok::Open {
                name: local(&inner[..split]),
                attrs: &inner[split..],
                empty,
            });
        }
    }
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// An attribute's unescaped value, by name or by local name (`r:id` → `id`).
fn attr(attrs: &str, want: &str) -> Option<String> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r#"([^\s=]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).expect("attribute pattern")
    });
    re.captures_iter(attrs).find_map(|c| {
        let name = c.get(1)?.as_str();
        if name.starts_with("xmlns") || (name != want && local(name) != want) {
            return None;
        }
        Some(unescape(
            c.get(2).or_else(|| c.get(3)).map_or("", |m| m.as_str()),
        ))
    })
}

/// The five XML entities and character references; anything else stays as written.
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let semi = rest.as_bytes().iter().take(12).position(|&b| b == b';');
        let ch = semi.and_then(|end| {
            let ent = &rest[1..end];
            match ent {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => {
                    let code = if let Some(h) =
                        ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X"))
                    {
                        u32::from_str_radix(h, 16).ok()
                    } else if let Some(d) = ent.strip_prefix('#') {
                        d.parse::<u32>().ok()
                    } else {
                        None
                    };
                    code.and_then(char::from_u32)
                }
            }
        });
        match (ch, semi) {
            (Some(c), Some(end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A workbook from `(part path, XML)` pairs, stored or deflated.
    fn workbook(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            for (name, xml) in parts {
                z.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                z.write_all(xml.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><workbookPr/><sheets><sheet name="Budget" sheetId="1" r:id="rId1"/><sheet name="Notes &amp; misc" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
    const RELS: &str = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="/xl/worksheets/sheet1.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#;
    const STYLES: &str = r#"<styleSheet><numFmts count="1"><numFmt numFmtId="164" formatCode="yyyy\-mm\-dd"/></numFmts><cellXfs count="4"><xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="164"/><xf numFmtId="4"/></cellXfs></styleSheet>"#;
    const SHARED: &str = r#"<sst count="4" uniqueCount="4"><si><t>Item</t></si><si><r><t>Re</t></r><r><t xml:space="preserve">nt</t></r><rPh><t>れんと</t></rPh></si><si><t>Unused &lt;pool&gt; string</t></si><si><t>Total</t></si></sst>"#;
    const SHEET1: &str = r#"<worksheet><sheetData>
<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="inlineStr"><is><t>Cost</t></is></c><c r="C1" t="s" s="0"><v>9</v></c></row>
<row r="2"><c r="A2" t="s"><v>1</v></c><c r="B2"><v>1200</v></c><c r="C2" s="1"><v>46296</v></c><c r="D2" t="b"><v>1</v></c><c r="E2" s="0"/></row>
<row r="4"><c r="A4" t="s"><v>3</v></c><c r="B4"><f>SUM(B2:B3)</f><v>1200</v></c><c r="C4" t="e"><f>1/0</f><v>#DIV/0!</v></c><c r="D4" t="str"><f>A4&amp;"!"</f><v>Total!</v></c><c r="E4"><f>NOW()</f></c><c r="F4" s="2"><v>45658.5</v></c></row>
</sheetData></worksheet>"#;
    const SHEET2: &str = r#"<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><x:sheetData><x:row><x:c t="inlineStr"><x:is><x:t><![CDATA[a < b]]></x:t></x:is></x:c><x:c t="d"><x:v>2026-10-02T00:00:00</x:v></x:c></x:row></x:sheetData></x:worksheet>"#;

    fn fixture() -> Vec<u8> {
        workbook(&[
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", RELS),
            ("xl/styles.xml", STYLES),
            ("xl/sharedStrings.xml", SHARED),
            ("xl/worksheets/sheet1.xml", SHEET1),
            ("xl/worksheets/sheet2.xml", SHEET2),
        ])
    }

    fn cell(wb: &Workbook, sheet: usize, at: &str) -> Cell {
        wb.sheets[sheet]
            .rows
            .iter()
            .flat_map(|r| r.cells.iter())
            .find(|c| c.at == at)
            .cloned()
            .unwrap_or_else(|| panic!("no cell {at}"))
    }

    #[test]
    fn reads_typed_cells_by_sheet_and_row() {
        let wb = read(&fixture(), &LIMITS).unwrap();
        assert!(wb.truncated.is_empty(), "{:?}", wb.truncated);
        let names: Vec<&str> = wb.sheets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["Budget", "Notes & misc"],
            "workbook order, not part order"
        );
        let rows: Vec<u32> = wb.sheets[0].rows.iter().map(|r| r.number).collect();
        assert_eq!(rows, [1, 2, 4]);
        assert_eq!(cell(&wb, 0, "A1").value, CellValue::Text("Item".into()));
        assert_eq!(cell(&wb, 0, "B1").value, CellValue::Text("Cost".into()));
        assert_eq!(
            cell(&wb, 0, "C1").value,
            CellValue::Unresolved("shared string 9, missing from the workbook".into())
        );
        assert_eq!(
            cell(&wb, 0, "A2").value,
            CellValue::Text("Rent".into()),
            "rich runs joined, phonetic run dropped"
        );
        assert_eq!(cell(&wb, 0, "B2").value, CellValue::Number("1200".into()));
        assert_eq!(
            cell(&wb, 0, "C2").value,
            CellValue::Date {
                serial: "46296".into(),
                reading: Some("2026-10-01".into())
            }
        );
        assert_eq!(cell(&wb, 0, "D2").value, CellValue::Bool(true));
        assert!(
            wb.sheets[0].rows[1].cells.iter().all(|c| c.at != "E2"),
            "a style-only cell is not content"
        );
        let b4 = cell(&wb, 0, "B4");
        assert_eq!(
            (b4.value, b4.formula.as_deref()),
            (CellValue::Number("1200".into()), Some("SUM(B2:B3)"))
        );
        assert_eq!(cell(&wb, 0, "C4").value, CellValue::Error("#DIV/0!".into()));
        let d4 = cell(&wb, 0, "D4");
        assert_eq!(
            (d4.value, d4.formula.as_deref()),
            (CellValue::Text("Total!".into()), Some("A4&\"!\""))
        );
        assert_eq!(cell(&wb, 0, "E4").value, CellValue::Empty);
        assert_eq!(
            cell(&wb, 0, "F4").value,
            CellValue::Date {
                serial: "45658.5".into(),
                reading: Some("2025-01-01 12:00:00".into())
            }
        );
        // Prefixed names, no `r` attributes, CDATA, an ISO date cell.
        assert_eq!(cell(&wb, 1, "A1").value, CellValue::Text("a < b".into()));
        assert_eq!(
            cell(&wb, 1, "B1").value,
            CellValue::IsoDate("2026-10-02T00:00:00".into())
        );
    }

    #[test]
    fn render_shows_cells_not_the_string_pool() {
        let text = render(&read(&fixture(), &LIMITS).unwrap());
        assert!(
            !text.contains("Unused"),
            "a pooled string no cell uses is not sheet content:\n{text}"
        );
        assert!(!text.contains("[truncated"), "{text}");
        assert!(text.contains("## Sheet 1: Budget\nA1 \"Item\" | B1 \"Cost\" | C1 [shared string 9, missing from the workbook]\n"), "{text}");
        assert!(
            text.contains("A2 \"Rent\" | B2 1200 | C2 2026-10-01 (date, serial 46296) | D2 TRUE\n"),
            "{text}"
        );
        assert!(text.contains("B4 1200 (formula =SUM(B2:B3))"), "{text}");
        assert!(text.contains("C4 #DIV/0! (error) (formula =1/0)"), "{text}");
        assert!(
            text.contains("E4 (no cached value) (formula =NOW())"),
            "{text}"
        );
        assert!(
            text.contains(
                "## Sheet 2: Notes & misc\nA1 \"a < b\" | B1 2026-10-02T00:00:00 (date)\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn caps_mark_the_read_truncated() {
        let lim = |sheets, rows, cells, xml_bytes| Limits {
            sheets,
            rows,
            cells,
            xml_bytes,
            text_chars: 60_000,
            cell_chars: 2_000,
        };
        let wb = read(&fixture(), &lim(1, 10, 100, 1 << 20)).unwrap();
        assert_eq!(wb.sheets.len(), 1);
        assert_eq!(wb.truncated, ["first 1 of 2 sheets"]);
        assert!(render(&wb).starts_with("[truncated: first 1 of 2 sheets]\n"));

        let wb = read(&fixture(), &lim(32, 2, 100, 1 << 20)).unwrap();
        assert_eq!(wb.sheets[0].rows.len(), 2);
        assert_eq!(
            wb.truncated,
            ["sheet \"Budget\": first 2 rows with cells only"]
        );

        let wb = read(&fixture(), &lim(32, 10, 3, 1 << 20)).unwrap();
        let kept: usize = wb
            .sheets
            .iter()
            .flat_map(|s| &s.rows)
            .map(|r| r.cells.len())
            .sum();
        assert_eq!(kept, 3);
        assert_eq!(
            wb.truncated,
            ["stopped at the 3-cell limit, in sheet \"Budget\" at A2"]
        );
    }

    #[test]
    fn a_zip_bomb_inflates_only_to_the_byte_cap() {
        // ~8 MB of XML that deflates to a few KB.
        let mut sheet = String::from("<worksheet><sheetData>");
        let filler = "<row><c><v>1</v></c></row>".repeat(300_000);
        sheet.push_str(&filler);
        sheet.push_str("</sheetData></worksheet>");
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let deflate = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            z.start_file("xl/worksheets/sheet1.xml", deflate).unwrap();
            z.write_all(sheet.as_bytes()).unwrap();
            z.finish().unwrap();
        }
        let bytes = buf.into_inner();
        assert!(
            bytes.len() < sheet.len() / 20,
            "the fixture really is compressed"
        );
        let lim = Limits {
            sheets: 32,
            rows: 1_000_000,
            cells: 1_000_000,
            xml_bytes: 64 * 1024,
            text_chars: 1_000_000,
            cell_chars: 2_000,
        };
        let wb = read(&bytes, &lim).unwrap();
        assert_eq!(
            wb.sheets[0].name, "Sheet1",
            "no workbook.xml: the sheet parts in order"
        );
        let kept: usize = wb.sheets[0].rows.iter().map(|r| r.cells.len()).sum();
        assert!(kept > 1_000 && kept < 64 * 1024 / 20, "{kept}");
        assert_eq!(
            wb.truncated,
            ["sheet \"Sheet1\" read only up to the 64 KB uncompressed-XML limit"]
        );
    }

    #[test]
    fn serials_convert_in_both_date_systems() {
        assert_eq!(serial_date("1", false).as_deref(), Some("1900-01-01"));
        assert_eq!(serial_date("59", false).as_deref(), Some("1900-02-28"));
        assert_eq!(serial_date("60", false), None, "Excel's phantom 1900-02-29");
        assert_eq!(serial_date("61", false).as_deref(), Some("1900-03-01"));
        assert_eq!(serial_date("0.75", false).as_deref(), Some("18:00:00"));
        assert_eq!(serial_date("0", true).as_deref(), Some("1904-01-01"));
        assert_eq!(serial_date("-1", false), None);
        assert_eq!(serial_date("1e300", false), None);
        assert!(
            is_date_format("[$-409]mmmm d, yyyy;@")
                && is_date_format("h:mm AM/PM")
                && is_date_format("[h]:mm:ss")
        );
        assert!(
            !is_date_format("General")
                && !is_date_format("#,##0.00\\ \"kr\"")
                && !is_date_format("[Red]0.00E+00")
                && !is_date_format("\"days\" 0")
        );
    }

    #[test]
    fn xml_helpers_handle_the_odd_shapes() {
        assert_eq!(
            unescape("a &amp; b &#65;&#x42; &bogus; &"),
            "a & b AB &bogus; &"
        );
        assert_eq!(attr(r#" r="B2" t='s' s="3""#, "t").as_deref(), Some("s"));
        assert_eq!(
            attr(r#" xmlns:r="urn:x" name="S" r:id="rId1""#, "id").as_deref(),
            Some("rId1")
        );
        assert_eq!(
            (
                col_name(0),
                col_name(25),
                col_name(26),
                col_name(701),
                col_name(702)
            ),
            (
                "A".into(),
                "Z".into(),
                "AA".into(),
                "ZZ".into(),
                "AAA".into()
            )
        );
        assert_eq!(
            (col_of("A1"), col_of("AB12"), col_of("12")),
            (Some(0), Some(27), None)
        );
        let toks: Vec<Tok> = tokens(r#"<a x=">"/><!-- c --><b>t</b><c"#).collect();
        assert_eq!(
            toks,
            [
                Tok::Open {
                    name: "a",
                    attrs: r#" x=">""#,
                    empty: true
                },
                Tok::Open {
                    name: "b",
                    attrs: "",
                    empty: false
                },
                Tok::Text("t", false),
                Tok::Close("b")
            ]
        );
        assert!(read(b"not a zip", &LIMITS).is_err());
    }
}
