//! Line-by-line JSONL streaming for the import readers: one line in memory at
//! a time, a line over [`MAX_LINE_BYTES`] skipped without being held, and
//! the last line marked when the file ends without a newline (a transcript
//! cut off mid-write).

use serde_json::Value;
use std::io::{BufRead, BufReader, Read};

/// A line longer than this is skipped (a pasted file or a giant tool
/// result) and counted; the rest of the file still reads.
pub(crate) const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

/// One line of a JSONL file, as a reader sees it.
pub(crate) enum Line {
    /// Parsed JSON.
    Json(Value),
    /// Not JSON (a malformed line). `last` when it is the file's final line
    /// with no newline after it: a cut-off write.
    Malformed { last: bool },
    /// Longer than [`MAX_LINE_BYTES`].
    TooLong,
}

/// Stream `reader`, calling `f` for each non-empty line. Read errors end the
/// stream (what was read stays).
pub(crate) fn for_each(reader: impl Read, mut f: impl FnMut(Line)) {
    let mut reader = BufReader::with_capacity(64 * 1024, reader);
    let mut line: Vec<u8> = Vec::new();
    let mut skipping = false;
    let emit = |bytes: &[u8], last: bool, f: &mut dyn FnMut(Line)| {
        let text = String::from_utf8_lossy(bytes);
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        match serde_json::from_str::<Value>(text) {
            Ok(value) => f(Line::Json(value)),
            Err(_) => f(Line::Malformed { last }),
        }
    };
    loop {
        let (done, used) = {
            let Ok(buf) = reader.fill_buf() else { break };
            if buf.is_empty() {
                (true, 0)
            } else if let Some(i) = buf.iter().position(|&b| b == b'\n') {
                if skipping || line.len() + i > MAX_LINE_BYTES {
                    f(Line::TooLong);
                } else {
                    line.extend_from_slice(&buf[..i]);
                    emit(&line, false, &mut f);
                }
                line.clear();
                skipping = false;
                (false, i + 1)
            } else {
                let n = buf.len();
                if !skipping {
                    if line.len() + n > MAX_LINE_BYTES {
                        line.clear();
                        skipping = true;
                    } else {
                        line.extend_from_slice(buf);
                    }
                }
                (false, n)
            }
        };
        if done {
            if skipping {
                f(Line::TooLong);
            } else if !line.is_empty() {
                emit(&line, true, &mut f);
            }
            return;
        }
        reader.consume(used);
    }
}
