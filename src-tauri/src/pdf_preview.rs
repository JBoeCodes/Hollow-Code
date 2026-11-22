//! Read-only PDF pages for the existing Files/Results panes. Native PDFKit
//! returns pixels and bounded text, never a live PDF plugin, script or URL.
use serde::Serialize;
use std::io::Read;
use std::path::PathBuf;
use std::path::{Component, Path};

#[cfg(any(target_os = "macos", test))]
const MAX_BYTES: u64 = 20 * 1024 * 1024;
#[cfg(target_os = "macos")]
const MAX_PAGES: usize = 2000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfPage {
    pub page: u32,
    pub pages: u32,
    pub width: u32,
    pub height: u32,
    pub data_url: String,
    pub text: String,
    pub text_truncated: bool,
}

/// What a scoped preview read admits: the noun its messages use ("PDF"),
/// its extensions, its size cap and a check of the file's first bytes.
pub(crate) struct Scope {
    pub noun: &'static str,
    pub exts: &'static [&'static str],
    pub max_bytes: u64,
    pub signature: fn(&[u8]) -> bool,
}

#[cfg(any(target_os = "macos", test))]
const PDF: Scope = Scope {
    noun: "PDF",
    exts: &["pdf"],
    max_bytes: MAX_BYTES,
    signature: pdf_signature,
};

/// `%PDF-` within the first 1 KB.
#[cfg(any(target_os = "macos", test))]
fn pdf_signature(head: &[u8]) -> bool {
    head[..head.len().min(1024)]
        .windows(5)
        .any(|v| v == b"%PDF-")
}

#[cfg(any(target_os = "macos", test))]
fn read(path: &str, roots: &[PathBuf]) -> Result<Vec<u8>, String> {
    read_scoped(path, roots, &PDF)
}

/// The bytes of one file the preview may show: an absolute path whose
/// canonical target is inside one of `roots` (canonical, `media::roots`),
/// with no hidden or secret-named folder on the way and no hidden name, of
/// `scope`'s extension and signature, at most its size cap, and unchanged
/// while it was read. Shared by the PDF and Office previews (B1b).
pub(crate) fn read_scoped(path: &str, roots: &[PathBuf], scope: &Scope) -> Result<Vec<u8>, String> {
    let noun = scope.noun;
    let mb = scope.max_bytes / (1024 * 1024);
    let input = Path::new(path);
    if !input.is_absolute() || path.len() > 8192 || path.contains('\0') {
        return Err(format!(
            "Choose a {noun} from this conversation or project."
        ));
    }
    let canonical = input
        .canonicalize()
        .map_err(|_| format!("This {noun} was moved or is unavailable."))?;
    let root = roots
        .iter()
        .filter(|r| canonical.starts_with(r))
        .max_by_key(|r| r.as_os_str().len())
        .ok_or_else(|| format!("This {noun} is outside the conversation and project folders."))?;
    let relative = canonical
        .strip_prefix(root)
        .map_err(|_| format!("Invalid {noun} location."))?;
    // Like media previews, reject hidden/secret directories but allow a
    // non-hidden filename such as token-budget.pdf or password-policy.pdf.
    // The extension and signature checks below still apply to the file.
    let private_parent = relative.parent().is_some_and(|parent| {
        parent.components().any(|c| {
            matches!(c, Component::Normal(s) if crate::source_evidence::forbidden_component(&s.to_string_lossy()))
        })
    });
    let hidden_file = relative
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with('.'));
    if private_parent || hidden_file {
        return Err(format!("This {noun} is in a private or hidden location."));
    }
    if !canonical
        .extension()
        .is_some_and(|e| scope.exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
    {
        return Err(format!("This file is not a {noun}."));
    }
    let mut file = crate::media::open_regular(&canonical)
        .ok_or_else(|| format!("This {noun} could not be opened."))?;
    let before = file
        .metadata()
        .map_err(|_| format!("This {noun} could not be read."))?;
    if !before.is_file() {
        return Err(format!("Choose a {noun} file, not a folder."));
    }
    if before.len() > scope.max_bytes {
        return Err(format!(
            "This {noun} is too large to preview here (limit {mb} MB). Open it externally instead."
        ));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(scope.max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("This {noun} could not be read."))?;
    if bytes.len() as u64 > scope.max_bytes {
        return Err(format!(
            "This {noun} grew beyond the {mb} MB preview limit."
        ));
    }
    let changed = || format!("This {noun} changed while it was read. Retry.");
    let after = std::fs::metadata(&canonical).map_err(|_| changed())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err(changed());
        }
    }
    if input.canonicalize().ok().as_ref() != Some(&canonical)
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err(changed());
    }
    if !(scope.signature)(&bytes) {
        return Err(format!("This file does not contain a readable {noun}."));
    }
    Ok(bytes)
}

#[cfg(target_os = "macos")]
pub fn page(path: &str, index: u32, roots: &[PathBuf]) -> Result<PdfPage, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use objc2::AllocAnyThread;
    use objc2_foundation::{NSData, NSSize};
    use objc2_pdf_kit::{PDFDisplayBox, PDFDocument};
    let bytes = read(path, roots)?;
    objc2::rc::autoreleasepool(|_| {
        let data = NSData::from_vec(bytes);
        // SAFETY: this document and its retained page live only on this thread.
        let doc = unsafe { PDFDocument::initWithData(PDFDocument::alloc(), &data) }
            .ok_or("This PDF could not be decoded.")?;
        if unsafe { doc.isLocked() } {
            return Err("This PDF is password-protected. Open it externally to unlock it.".into());
        }
        let pages = unsafe { doc.pageCount() };
        if pages == 0 || pages > MAX_PAGES {
            return Err("This PDF has no pages or exceeds the 2,000-page preview limit.".into());
        }
        if index as usize >= pages {
            return Err("That PDF page is no longer available. Retry from the first page.".into());
        }
        let page =
            unsafe { doc.pageAtIndex(index as usize) }.ok_or("This PDF page is unavailable.")?;
        let bounds = unsafe { page.boundsForBox(PDFDisplayBox::CropBox) };
        let (w, h) = (bounds.size.width, bounds.size.height);
        if !w.is_finite()
            || !h.is_finite()
            || w <= 0.0
            || h <= 0.0
            || w > 1_000_000.0
            || h > 1_000_000.0
        {
            return Err("This PDF page has invalid dimensions.".into());
        }
        // Fit ordinary pages at 1,200px wide, while long exported plans stay
        // legible on zoom. Pixel area/edges bound native allocation and IPC.
        let scale = (1200.0 / w)
            .min(16384.0 / h)
            .min((16_000_000.0 / (w * h)).sqrt());
        let (width, height) = (
            (w * scale).round().max(1.0) as u32,
            (h * scale).round().max(1.0) as u32,
        );
        let image = unsafe {
            page.thumbnailOfSize_forBox(
                NSSize::new(width as f64, height as f64),
                PDFDisplayBox::CropBox,
            )
        };
        let png = crate::webkit_capture::png(&image)?;
        if png.len() > 16 * 1024 * 1024 {
            return Err(
                "This PDF page is too detailed to preview here. Open it externally instead.".into(),
            );
        }
        let text = unsafe { page.string() }
            .map(|s| s.to_string())
            .unwrap_or_default();
        let text_truncated = text.chars().count() > 40_000;
        Ok(PdfPage {
            page: index,
            pages: pages as u32,
            width,
            height,
            data_url: format!("data:image/png;base64,{}", STANDARD.encode(png)),
            text: text.chars().take(40_000).collect(),
            text_truncated,
        })
    })
}

#[cfg(not(target_os = "macos"))]
pub fn page(_: &str, _: u32, _: &[PathBuf]) -> Result<PdfPage, String> {
    Err("PDF preview currently needs macOS. Open it externally instead.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_text_one_page.pdf")
    }
    #[test]
    fn accepts_descriptive_pdf_names_but_refuses_private_directories() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for name in [
            "token-budget.pdf",
            "password-policy.pdf",
            "secret-garden.pdf",
        ] {
            let path = root.join(name);
            std::fs::copy(fixture(), &path).unwrap();
            assert!(read(path.to_str().unwrap(), std::slice::from_ref(&root)).is_ok());
        }
        for folder in [".hidden", "secrets"] {
            let path = root.join(folder).join("document.pdf");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::copy(fixture(), &path).unwrap();
            assert!(read(path.to_str().unwrap(), std::slice::from_ref(&root)).is_err());
        }
    }
    #[test]
    fn refuses_unscoped_hidden_non_pdf_and_large_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        assert!(read("relative.pdf", std::slice::from_ref(&root)).is_err());
        assert!(read(fixture().to_str().unwrap(), std::slice::from_ref(&root)).is_err());
        for name in [".hidden.pdf", "text.txt", "bad.pdf"] {
            let p = root.join(name);
            std::fs::write(&p, b"not a PDF").unwrap();
            assert!(read(p.to_str().unwrap(), std::slice::from_ref(&root)).is_err());
        }
        let large = root.join("large.pdf");
        let f = std::fs::File::create(&large).unwrap();
        f.set_len(MAX_BYTES + 1).unwrap();
        assert!(read(large.to_str().unwrap(), std::slice::from_ref(&root))
            .unwrap_err()
            .contains("20 MB"));
        #[cfg(unix)]
        {
            let link = root.join("escape.pdf");
            std::os::unix::fs::symlink(fixture(), &link).unwrap();
            assert!(read(link.to_str().unwrap(), std::slice::from_ref(&root)).is_err());
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn renders_a_real_pdf_page_and_rejects_invalid_page() {
        let p = fixture();
        let roots = vec![p.parent().unwrap().canonicalize().unwrap()];
        let result = page(p.to_str().unwrap(), 0, &roots).unwrap();
        assert_eq!((result.page, result.pages), (0, 1));
        assert!(result.width > 0 && result.height > 0);
        assert!(result.data_url.starts_with("data:image/png;base64,iVBOR"));
        assert!(result.text.contains("Garage 14 escalator"));
        assert!(page(p.to_str().unwrap(), 1, &roots).is_err());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn real_multi_page_pdf_renders_each_page_without_running_actions() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let p =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf_preview_two_pages.pdf");
        let roots = vec![p.parent().unwrap().canonicalize().unwrap()];
        for i in 0..2 {
            let rendered = page(p.to_str().unwrap(), i, &roots).unwrap();
            assert_eq!(rendered.pages, 2);
            assert!(rendered.text.contains(&format!("page {}", i + 1)));
            let bytes = STANDARD
                .decode(rendered.data_url.split_once(',').unwrap().1)
                .unwrap();
            assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            if let Ok(output) = std::env::var("HOLLOW_PDF_PREVIEW_PROOF_DIR") {
                let dir = Path::new(&output);
                std::fs::create_dir_all(dir).unwrap();
                std::fs::write(dir.join(format!("page-{}.png", i + 1)), bytes).unwrap();
                std::fs::write(
                    dir.join(format!("page-{}.json", i + 1)),
                    serde_json::to_vec(&rendered).unwrap(),
                )
                .unwrap();
            }
        }
        assert!(page(p.to_str().unwrap(), 2, &roots).is_err());
    }
}
