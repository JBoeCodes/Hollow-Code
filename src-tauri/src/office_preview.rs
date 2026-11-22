//! Read-only previews of Word, Excel and PowerPoint files for the Preview
//! pane (B1b, Linear ENG-104): the text only, never the layout.
//!
//! - **Word** (`.docx`): one part, its paragraphs (at most [`MAX_PARAGRAPHS`]).
//! - **PowerPoint** (`.pptx`): one part per slide, `Slide N`, with its
//!   paragraphs (at most [`MAX_SLIDES`] slides).
//! - **Excel** (`.xlsx`): one part per sheet (at most [`MAX_SHEETS`]), named
//!   by the sheet, as a grid from column A: at most [`MAX_ROWS`] rows with
//!   cells and [`MAX_COLS`] columns, each cell's value as text (a date as
//!   its reading, a formula as its cached value).
//!
//! The file is read through `pdf_preview::read_scoped` with [`SCOPE`]: the
//! PDF preview's guard (inside the vault, a project or the conversation's
//! folders, nothing hidden or secret-named, unchanged while read), at most
//! 20 MB and a zip's signature. The text comes from `doc_text` and
//! `xlsx_cells`, the readers retrieval uses. Every cut is listed in
//! [`OfficePreview::truncated`], so the pane can say so.
use crate::doc_text::DocKind;
use crate::pdf_preview::Scope;
use crate::xlsx_cells::{Cell, CellValue};
use serde::Serialize;
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 20 * 1024 * 1024;
pub(crate) const MAX_PARAGRAPHS: usize = 2000;
pub(crate) const MAX_SLIDES: usize = 200;
pub(crate) const MAX_SHEETS: usize = 16;
pub(crate) const MAX_ROWS: usize = 200;
pub(crate) const MAX_COLS: usize = 20;

pub(crate) const SCOPE: Scope = Scope {
    noun: "Word, Excel or PowerPoint file",
    exts: &["docx", "xlsx", "pptx"],
    max_bytes: MAX_BYTES,
    signature: zip_signature,
};

/// A zip's local file header, which every Office Open XML file starts with.
fn zip_signature(head: &[u8]) -> bool {
    head.starts_with(b"PK\x03\x04")
}

/// What the pane shows: `kind` is `word`, `excel` or `powerpoint`.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OfficePreview {
    pub kind: &'static str,
    pub parts: Vec<Part>,
    /// What a cap cut, one line each; empty when everything is shown.
    pub truncated: Vec<String>,
}

/// A Word document's body, one slide or one sheet.
#[derive(Debug, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    /// `Slide N` or the sheet's name; empty for a Word document.
    pub title: String,
    /// Word and PowerPoint text, one paragraph each.
    pub paragraphs: Vec<String>,
    /// Excel: the column letters shown, from `A`.
    pub columns: Vec<String>,
    /// Excel: the rows with cells, each as wide as `columns`.
    pub rows: Vec<GridRow>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GridRow {
    /// The sheet's row number (1-based).
    pub number: u32,
    pub cells: Vec<String>,
    /// UN1: each cell's formula without its `=`, parallel to `cells` (empty
    /// where a cell has none). Left out when no cell in the row has one, so a
    /// sheet without formulas serializes as before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formulas: Vec<String>,
}

/// The preview of the Office file at `path`, read only inside `roots`
/// (canonical, as `media::roots` makes them). Blocking.
pub fn preview(path: &str, roots: &[PathBuf]) -> Result<OfficePreview, String> {
    let bytes = crate::pdf_preview::read_scoped(path, roots, &SCOPE)?;
    // The kind of what was read: a link named `a.docx` may lead to a deck.
    let canonical = Path::new(path)
        .canonicalize()
        .map_err(|_| "This file was moved or is unavailable.".to_string())?;
    let kind = crate::doc_text::kind(&canonical)
        .filter(|k| *k != DocKind::Pdf)
        .ok_or("This file is not a Word, Excel or PowerPoint file.")?;
    shape(&bytes, kind).map_err(|e| {
        format!("Hollow couldn't read the text in this file: {e}. Open it in its app instead.")
    })
}

/// `bytes` (an Office file of `kind`) as the pane's parts.
pub(crate) fn shape(bytes: &[u8], kind: DocKind) -> Result<OfficePreview, String> {
    match kind {
        DocKind::Xlsx => excel(bytes),
        DocKind::Docx | DocKind::Pptx => {
            let limits = crate::doc_text::Limits {
                file_bytes: MAX_BYTES,
                units: MAX_SLIDES,
                text_bytes: 1024 * 1024,
                xml_bytes: 50 * 1024 * 1024,
            };
            let text = crate::doc_text::extract_bytes(bytes, kind, &limits)?.text;
            let (mut truncated, body) = split_notes(&text);
            let parts = if kind == DocKind::Docx {
                let all: Vec<String> = body
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(str::to_string)
                    .collect();
                if all.len() > MAX_PARAGRAPHS {
                    truncated.push(format!(
                        "first {MAX_PARAGRAPHS} of {} paragraphs",
                        all.len()
                    ));
                }
                vec![Part {
                    paragraphs: all.into_iter().take(MAX_PARAGRAPHS).collect(),
                    ..Part::default()
                }]
            } else {
                slides(body)
            };
            Ok(OfficePreview {
                kind: if kind == DocKind::Docx {
                    "word"
                } else {
                    "powerpoint"
                },
                parts,
                truncated,
            })
        }
        DocKind::Pdf => Err("not an Office file".into()),
    }
}

/// `doc_text`'s `[truncated: a; b]` first line as notes, and the rest.
fn split_notes(text: &str) -> (Vec<String>, &str) {
    if let Some(rest) = text.strip_prefix("[truncated: ") {
        if let Some(end) = rest.find("]\n\n") {
            let notes = rest[..end].split("; ").map(str::to_string).collect();
            return (notes, &rest[end + 3..]);
        }
    }
    (Vec::new(), text)
}

/// A deck's text (`## Slide N` before each slide's paragraphs) as parts.
fn slides(body: &str) -> Vec<Part> {
    let mut parts: Vec<Part> = Vec::new();
    for line in body.lines() {
        if let Some(n) = line.strip_prefix("## Slide ") {
            parts.push(Part {
                title: format!("Slide {n}"),
                ..Part::default()
            });
        } else if !line.trim().is_empty() {
            if let Some(slide) = parts.last_mut() {
                slide.paragraphs.push(line.to_string());
            }
        }
    }
    parts
}

fn excel(bytes: &[u8]) -> Result<OfficePreview, String> {
    let limits = crate::xlsx_cells::Limits {
        sheets: MAX_SHEETS,
        rows: MAX_ROWS,
        cells: 20_000,
        xml_bytes: 50 * 1024 * 1024,
        text_chars: 200_000,
        cell_chars: 500,
    };
    let wb = crate::xlsx_cells::read(bytes, &limits)?;
    let mut truncated = wb.truncated;
    let mut parts = Vec::new();
    for sheet in wb.sheets {
        let width = sheet
            .rows
            .iter()
            .flat_map(|r| r.cells.iter())
            .filter_map(|c| column_index(&c.at))
            .max()
            .map_or(0, |i| i + 1);
        if width > MAX_COLS {
            truncated.push(format!(
                "sheet \"{}\": first {MAX_COLS} of {width} columns",
                sheet.name
            ));
        }
        let shown = width.min(MAX_COLS);
        let rows = sheet
            .rows
            .iter()
            .map(|row| {
                let mut cells = vec![String::new(); shown];
                let mut formulas = vec![String::new(); shown];
                for cell in &row.cells {
                    if let Some(i) = column_index(&cell.at).filter(|i| *i < shown) {
                        cells[i] = cell_text(cell);
                        if let Some(f) = cell.formula.as_deref().filter(|f| !f.is_empty()) {
                            formulas[i] = f.to_string();
                        }
                    }
                }
                if formulas.iter().all(String::is_empty) {
                    formulas.clear();
                }
                GridRow {
                    number: row.number,
                    cells,
                    formulas,
                }
            })
            .collect();
        parts.push(Part {
            title: sheet.name,
            columns: (0..shown).map(column_name).collect(),
            rows,
            ..Part::default()
        });
    }
    Ok(OfficePreview {
        kind: "excel",
        parts,
        truncated,
    })
}

/// `B7` → 1 (0-based), from the reference's letters.
fn column_index(at: &str) -> Option<usize> {
    let letters: Vec<u8> = at.bytes().take_while(u8::is_ascii_alphabetic).collect();
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let n = letters.iter().fold(0usize, |n, b| {
        n * 26 + (b.to_ascii_uppercase() - b'A') as usize + 1
    });
    Some(n - 1)
}

/// 0 → `A`, 26 → `AA`.
fn column_name(mut i: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// A cell as the person reads it: a date as its date, a formula as its value.
fn cell_text(cell: &Cell) -> String {
    match &cell.value {
        CellValue::Text(s) | CellValue::Number(s) | CellValue::Error(s) => s.clone(),
        CellValue::IsoDate(d) => d.clone(),
        CellValue::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).to_string(),
        CellValue::Date {
            reading: Some(r), ..
        } => r.clone(),
        CellValue::Date {
            serial,
            reading: None,
        } => serial.clone(),
        CellValue::Unresolved(why) => format!("[{why}]"),
        CellValue::Empty => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc_text::tests::{docx_with, pptx_with, xlsx_with, zip_of};

    #[test]
    fn word_is_one_part_of_paragraphs() {
        let p = shape(&docx_with(&["Hello", "Fish &amp; chips"]), DocKind::Docx).unwrap();
        assert_eq!(p.kind, "word");
        assert_eq!(p.parts.len(), 1);
        assert_eq!(p.parts[0].title, "");
        assert_eq!(p.parts[0].paragraphs, vec!["Hello", "Fish & chips"]);
        assert!(p.parts[0].rows.is_empty());
        assert!(p.truncated.is_empty(), "{:?}", p.truncated);
    }

    #[test]
    fn word_past_the_paragraph_cap_says_so() {
        let paras: Vec<String> = (0..MAX_PARAGRAPHS + 5).map(|i| format!("p{i}")).collect();
        let refs: Vec<&str> = paras.iter().map(String::as_str).collect();
        let p = shape(&docx_with(&refs), DocKind::Docx).unwrap();
        assert_eq!(p.parts[0].paragraphs.len(), MAX_PARAGRAPHS);
        assert_eq!(
            p.truncated,
            vec![format!(
                "first {MAX_PARAGRAPHS} of {} paragraphs",
                MAX_PARAGRAPHS + 5
            )]
        );
    }

    #[test]
    fn powerpoint_is_one_part_per_slide_in_order() {
        let titles: Vec<String> = (1..=10).map(|i| format!("Title {i}")).collect();
        let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
        let p = shape(&pptx_with(&refs), DocKind::Pptx).unwrap();
        assert_eq!(p.kind, "powerpoint");
        assert_eq!(p.parts.len(), 10);
        assert_eq!(p.parts[0].title, "Slide 1");
        assert_eq!(p.parts[0].paragraphs, vec!["Title 1"]);
        assert_eq!(p.parts[9].title, "Slide 10");
        assert_eq!(p.parts[9].paragraphs, vec!["Title 10"]);
        assert!(p.truncated.is_empty(), "{:?}", p.truncated);
    }

    #[test]
    fn powerpoint_past_the_slide_cap_says_so() {
        let many: Vec<String> = (0..MAX_SLIDES + 2).map(|i| format!("s{i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let p = shape(&pptx_with(&refs), DocKind::Pptx).unwrap();
        assert_eq!(p.parts.len(), MAX_SLIDES);
        assert_eq!(
            p.truncated,
            vec![format!("first {MAX_SLIDES} of {} slides", MAX_SLIDES + 2)]
        );
    }

    #[test]
    fn excel_is_a_grid_per_sheet() {
        let p = shape(&xlsx_with("Q3", "Revenue"), DocKind::Xlsx).unwrap();
        assert_eq!(p.kind, "excel");
        assert_eq!(p.parts.len(), 1);
        assert_eq!(p.parts[0].title, "Q3");
        assert_eq!(p.parts[0].columns, vec!["A"]);
        assert_eq!(
            p.parts[0].rows,
            vec![GridRow {
                number: 1,
                cells: vec!["Revenue".to_string()],
                formulas: Vec::new(),
            }]
        );
        assert!(p.truncated.is_empty(), "{:?}", p.truncated);
    }

    #[test]
    fn excel_rows_carry_formulas_beside_their_values() {
        // UN1: B1 holds a value, C1 `=B1*2` with a cached 85; row 2 has none.
        let sheet = r#"<worksheet><sheetData><row r="1"><c r="B1"><v>42.5</v></c><c r="C1"><f>B1*2</f><v>85</v></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>plain</t></is></c></row></sheetData></worksheet>"#;
        let book = zip_of(&[
            (
                "xl/workbook.xml",
                r#"<workbook xmlns:r="urn:r"><sheets><sheet name="F" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="rId1" Type="urn:worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            ("xl/worksheets/sheet1.xml", sheet),
        ]);
        let p = shape(&book, DocKind::Xlsx).unwrap();
        let rows = &p.parts[0].rows;
        assert_eq!(rows[0].cells, vec!["", "42.5", "85"]);
        assert_eq!(rows[0].formulas, vec!["", "", "B1*2"]);
        assert!(
            rows[1].formulas.is_empty(),
            "a row without formulas sends none"
        );
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["parts"][0]["rows"][0]["formulas"][2], "B1*2");
        assert!(v["parts"][0]["rows"][1].get("formulas").is_none());
    }

    #[test]
    fn excel_cells_land_in_their_columns_and_wide_sheets_are_cut() {
        // B2 a number, D2 a boolean; then a row out to column Y (25 wide).
        let wide: String = (0..25)
            .map(|i| format!(r#"<c r="{}3"><v>{i}</v></c>"#, column_name(i)))
            .collect();
        let sheet = format!(
            r#"<worksheet><sheetData><row r="2"><c r="B2"><v>42.5</v></c><c r="D2" t="b"><v>1</v></c></row><row r="3">{wide}</row></sheetData></worksheet>"#
        );
        let book = zip_of(&[
            (
                "xl/workbook.xml",
                r#"<workbook xmlns:r="urn:r"><sheets><sheet name="Wide" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="rId1" Type="urn:worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            ("xl/worksheets/sheet1.xml", &sheet),
        ]);
        let p = shape(&book, DocKind::Xlsx).unwrap();
        let part = &p.parts[0];
        assert_eq!(part.columns.len(), MAX_COLS);
        assert_eq!(part.columns[0], "A");
        assert_eq!(part.columns[MAX_COLS - 1], "T");
        assert_eq!(part.rows[0].number, 2);
        assert_eq!(part.rows[0].cells[..4], ["", "42.5", "", "TRUE"]);
        assert_eq!(part.rows[1].cells.len(), MAX_COLS);
        assert_eq!(part.rows[1].cells[19], "19");
        assert_eq!(
            p.truncated,
            vec![format!("sheet \"Wide\": first {MAX_COLS} of 25 columns")]
        );
    }

    #[test]
    fn column_letters_round_trip() {
        for (i, name) in [(0, "A"), (25, "Z"), (26, "AA"), (27, "AB"), (701, "ZZ")] {
            assert_eq!(column_name(i), name);
            assert_eq!(column_index(&format!("{name}12")), Some(i));
        }
        assert_eq!(column_index("12"), None);
    }

    #[test]
    fn preview_reads_only_inside_the_roots_and_only_office_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let roots = std::slice::from_ref(&root);
        let doc = root.join("report.docx");
        std::fs::write(&doc, docx_with(&["Inside"])).unwrap();
        let p = preview(doc.to_str().unwrap(), roots).unwrap();
        assert_eq!(p.parts[0].paragraphs, vec!["Inside"]);

        // Outside the roots, relative, hidden, a secret folder, not a zip.
        let other = tempfile::tempdir().unwrap();
        let outside = other.path().join("report.docx");
        std::fs::write(&outside, docx_with(&["Outside"])).unwrap();
        assert!(preview(outside.to_str().unwrap(), roots).is_err());
        assert!(preview("report.docx", roots).is_err());
        let hidden = root.join(".hidden.docx");
        std::fs::write(&hidden, docx_with(&["Hidden"])).unwrap();
        assert!(preview(hidden.to_str().unwrap(), roots).is_err());
        let secret = root.join("secrets").join("deck.pptx");
        std::fs::create_dir_all(secret.parent().unwrap()).unwrap();
        std::fs::write(&secret, pptx_with(&["Secret"])).unwrap();
        assert!(preview(secret.to_str().unwrap(), roots).is_err());
        let fake = root.join("fake.xlsx");
        std::fs::write(&fake, b"not a workbook").unwrap();
        assert!(preview(fake.to_str().unwrap(), roots)
            .unwrap_err()
            .contains("does not contain a readable"));
        let text = root.join("notes.txt");
        std::fs::write(&text, b"PK\x03\x04").unwrap();
        assert!(preview(text.to_str().unwrap(), roots).is_err());
        // A zip header and nothing readable after it: the fallback's words.
        let broken = root.join("broken.docx");
        std::fs::write(&broken, b"PK\x03\x04 truncated").unwrap();
        assert!(preview(broken.to_str().unwrap(), roots)
            .unwrap_err()
            .contains("Open it in its app instead"));
        #[cfg(unix)]
        {
            let link = root.join("escape.docx");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(preview(link.to_str().unwrap(), roots).is_err());
        }
    }

    #[test]
    fn a_file_past_the_size_cap_is_refused_with_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let big = root.join("big.xlsx");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(MAX_BYTES + 1).unwrap();
        let err = preview(big.to_str().unwrap(), std::slice::from_ref(&root)).unwrap_err();
        assert!(err.contains("20 MB"), "{err}");
    }

    #[test]
    fn the_preview_serializes_in_camel_case() {
        let p = shape(&xlsx_with("S", "x"), DocKind::Xlsx).unwrap();
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["kind"], "excel");
        assert_eq!(v["parts"][0]["rows"][0]["number"], 1);
        assert_eq!(v["parts"][0]["columns"][0], "A");
        assert!(v["truncated"].as_array().unwrap().is_empty());
    }
}
