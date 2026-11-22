//! Process recorder → skill, stage 1 (master plan #10; overnight B5): turn a
//! recording of a process into a timestamped step list with frames. Local
//! only: no model sees anything at this stage, so a payroll screen never
//! leaves the Mac.
//!
//! **Input.** A screen recording already in a workspace's `sources/`
//! (macOS's built-in recorder, ⇧⌘5, then copy the file into the workspace's
//! `sources/` folder in Finder: Add source stops at 20 MB, less than a minute
//! of screen recording; no new permission either way),
//! and optionally its transcript: a `.srt`, `.vtt`, Whisper `.json` or
//! timestamped `.txt`/`.md`, given by path or found beside the video with
//! the same name. Hollow doesn't transcribe yet (that needs a speech model:
//! F9); without a transcript the steps are one per frame interval.
//!
//! **Output**, all under `outputs/process-captures/<video name>/`:
//! - `frames/frame-NNN.jpg`: one frame every [`DEFAULT_EVERY_S`] seconds,
//!   widened so at most [`MAX_FRAMES`] cover the whole recording (`ffprobe`'s
//!   length), 1280 px wide, by `ffmpeg` (local files only) when installed.
//!   Taken in a temp folder outside the vault with no lock held; only
//!   copying them in and writing the note run inside the run's History
//!   lease ([`commit`]). Not Results or Activity ([`is_frame`]).
//! - `capture.md`: the draft step list. Each step has its time, the words
//!   said then and the frame on screen when it starts (none past the last
//!   frame). A new step starts at a pause of
//!   [`PAUSE_S`] seconds or more, at a cue word ("then", "next", "now",
//!   "first", "finally", "step N"), or past [`STEP_CHARS`] characters.
//!
//! Stage 2 (a model drafting `SKILL.md`, the inputs and outputs, fixtures and
//! the pass/fail table; saving stays a confirm) builds on this note.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const DEFAULT_EVERY_S: u32 = 10;
pub const MAX_FRAMES: u32 = 60;
const PAUSE_S: f64 = 1.5;
const STEP_CHARS: usize = 400;
const MAX_STEPS: usize = 60;
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(180);
const VIDEO_EXTS: &[&str] = &["mov", "mp4", "m4v", "webm", "mkv"];
const TRANSCRIPT_EXTS: &[&str] = &["srt", "vtt", "json", "txt", "md"];
const TRANSCRIPT_MAX_BYTES: u64 = 5 * 1024 * 1024;
/// Under a workspace: where captures go. `frames/` inside is not a Result.
pub const CAPTURE_DIR: &str = "outputs/process-captures/";

/// `outputs/process-captures/<capture>/frames/…`: a frame, not a Result.
pub fn is_frame(rel: &str) -> bool {
    rel.strip_prefix(CAPTURE_DIR)
        .is_some_and(|rest| rest.split('/').nth(1) == Some("frames"))
}

/// One stretch of speech; `start`/`end` in seconds when the transcript had them.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub text: String,
}

/// `HH:MM:SS,mmm`, `HH:MM:SS.mmm`, `MM:SS(.mmm)` or plain seconds.
fn seconds(t: &str) -> Option<f64> {
    let t = t.trim().replace(',', ".");
    let parts: Vec<&str> = t.split(':').collect();
    let mut total = 0.0;
    for p in &parts {
        total = total * 60.0 + p.parse::<f64>().ok()?;
    }
    (parts.len() <= 3).then_some(total)
}

/// Parse a transcript by its extension.
pub fn parse_transcript(name: &str, text: &str) -> Vec<Segment> {
    let text = text.trim_start_matches('\u{feff}');
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "json" => parse_whisper(text),
        "srt" | "vtt" => parse_cues(text),
        _ => parse_lines(text),
    }
}

/// Whisper's `{"segments": [{"start", "end", "text"}]}`.
fn parse_whisper(text: &str) -> Vec<Segment> {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    v.get("segments")
        .and_then(Value::as_array)
        .map(|segs| {
            segs.iter()
                .filter_map(|s| {
                    let text = s.get("text")?.as_str()?.trim().to_string();
                    (!text.is_empty()).then(|| Segment {
                        start: s.get("start").and_then(Value::as_f64),
                        end: s.get("end").and_then(Value::as_f64),
                        text,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// SRT and WebVTT: a `start --> end` line, then text lines up to a blank.
fn parse_cues(text: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((a, b)) = line.split_once("-->") else {
            continue;
        };
        // WebVTT may add cue settings after the end time.
        let b = b.split_whitespace().next().unwrap_or("");
        let (start, end) = (seconds(a), seconds(b));
        let mut words = Vec::new();
        while let Some(next) = lines.peek() {
            if next.trim().is_empty() {
                break;
            }
            words.push(strip_tags(lines.next().unwrap().trim()));
        }
        let text = words.join(" ").trim().to_string();
        if !text.is_empty() && start.is_some() {
            out.push(Segment { start, end, text });
        }
    }
    out
}

/// `<v Speaker>` and other WebVTT tags out.
fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

/// Plain text: a leading `[mm:ss]`, `(mm:ss)` or `mm:ss` on a line is its
/// start; other lines carry no time.
fn parse_lines(text: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    // A time alone on its line (YouTube's copied transcripts) dates the next text.
    let mut pending: Option<f64> = None;
    for line in text.lines() {
        let line = line.trim().trim_start_matches(['-', '*']).trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (start, rest) = leading_time(line);
        let rest = rest.trim_start_matches([':', '-', '–', ' ']).trim();
        if rest.is_empty() {
            pending = start.or(pending);
            continue;
        }
        out.push(Segment {
            start: start.or(pending.take()),
            end: None,
            text: rest.to_string(),
        });
    }
    out
}

fn leading_time(line: &str) -> (Option<f64>, &str) {
    let inner = line.trim_start_matches(['[', '(']);
    let end = inner
        .find(|c: char| !(c.is_ascii_digit() || c == ':' || c == '.'))
        .unwrap_or(inner.len());
    let candidate = &inner[..end];
    if candidate.contains(':') {
        if let Some(t) = seconds(candidate) {
            let rest = inner[end..].trim_start_matches([']', ')']);
            return (Some(t), rest);
        }
    }
    (None, line)
}

/// One drafted step.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub start: Option<f64>,
    pub text: String,
    /// `frames/frame-NNN.jpg`, the frame nearest its start.
    pub frame: Option<String>,
}

fn starts_with_cue(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    // The first word, cut at any punctuation ("Then, open HRIS"); a word
    // running on into a name ("Next.js") is not a cue.
    let word: String = lower.chars().take_while(|c| c.is_alphanumeric()).collect();
    let after = &lower[word.len()..];
    if after.starts_with('.') && after[1..].starts_with(|c: char| c.is_alphanumeric()) {
        return false;
    }
    matches!(
        word.as_str(),
        "then" | "next" | "now" | "first" | "finally" | "step"
    ) || (word == "after" && after.trim_start().starts_with("that"))
}

/// Group segments into steps (see the module doc for the rules).
pub fn steps(segments: &[Segment]) -> Vec<Step> {
    let mut out: Vec<Step> = Vec::new();
    let mut last_end: Option<f64> = None;
    for seg in segments {
        let pause = matches!((last_end, seg.start), (Some(e), Some(s)) if s - e >= PAUSE_S);
        let split = match out.last() {
            None => true,
            Some(cur) => pause || starts_with_cue(&seg.text) || cur.text.len() >= STEP_CHARS,
        };
        if split && out.len() < MAX_STEPS {
            out.push(Step {
                start: seg.start,
                text: seg.text.clone(),
                frame: None,
            });
        } else if let Some(cur) = out.last_mut() {
            // Past the step cap the rest joins the last step, up to a limit.
            if cur.text.len() >= STEP_CHARS * 10 {
                if !cur.text.ends_with('…') {
                    cur.text.push_str(" …");
                }
                continue;
            }
            cur.text.push(' ');
            cur.text.push_str(&seg.text);
            cur.start = cur.start.or(seg.start);
        }
        last_end = seg.end.or(seg.start).or(last_end);
    }
    out
}

/// The frame on screen when a step starts (the last one taken at or before
/// `t`), when frames were taken every `every` seconds; none past the last.
fn frame_for(t: f64, every: u32, frames: usize) -> Option<String> {
    let i = (t.max(0.0) / f64::from(every.max(1))).floor() as usize;
    (i < frames).then(|| format!("frames/frame-{:03}.jpg", i + 1))
}

fn clock(t: f64) -> String {
    let s = t.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// `ffmpeg` where Homebrew or /usr/local put it.
pub fn find_ffmpeg() -> Option<PathBuf> {
    ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

/// Run a tool with a time limit, keeping the end of its stderr (read on its
/// own thread, so a chatty tool can't fill the pipe and stall). Its own
/// process group, ended when it exits or times out, so nothing it started
/// outlives it.
fn run_bounded(
    cmd: &mut std::process::Command,
    limit: Duration,
) -> Result<(bool, String, String), String> {
    run_bounded_until(cmd, limit, &|| false)
}

/// [`run_bounded`], also ended when `stop()` turns true (checked every 2 s:
/// the run was stopped or its turn ended).
pub(crate) fn run_bounded_until(
    cmd: &mut std::process::Command,
    limit: Duration,
    stop: &dyn Fn() -> bool,
) -> Result<(bool, String, String), String> {
    use std::io::Read;
    crate::process_group::isolate_probe(cmd);
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("didn't start ({e})"))?;
    // Each reader hands its tail over a channel, so a descendant that left
    // the group and still holds a pipe can't keep this waiting.
    let tail = |mut r: Box<dyn Read + Send>| {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut kept = Vec::new();
            let mut buf = [0u8; 8192];
            while let Ok(n) = r.read(&mut buf) {
                if n == 0 {
                    break;
                }
                kept.extend_from_slice(&buf[..n]);
                if kept.len() > 16_384 {
                    kept.drain(..kept.len() - 16_384);
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&kept).to_string());
        });
        rx
    };
    let out = tail(Box::new(child.stdout.take().unwrap()));
    let err = tail(Box::new(child.stderr.take().unwrap()));
    let started = Instant::now();
    let mut checked = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        let stopped = checked.elapsed() >= Duration::from_secs(2) && {
            checked = Instant::now();
            stop()
        };
        if started.elapsed() > limit || stopped {
            crate::process_group::end_probe_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
            return Err(if stopped {
                "was stopped with its run".to_string()
            } else {
                format!("took longer than {} s; stopped", limit.as_secs())
            });
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    // Whatever it left running in its group (a background child holding the pipes).
    crate::process_group::end_probe_group(child.id());
    let grace = Duration::from_secs(2);
    Ok((
        status.success(),
        out.recv_timeout(grace).unwrap_or_default(),
        err.recv_timeout(grace).unwrap_or_default(),
    ))
}

/// The recording's length in seconds, from `ffprobe` beside `ffmpeg`.
fn duration_of(ffmpeg: &Path, video: &Path) -> Option<f64> {
    let probe = ffmpeg.with_file_name("ffprobe");
    if !probe.is_file() {
        return None;
    }
    let mut cmd = std::process::Command::new(probe);
    cmd.args([
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-show_entries",
        "format=duration",
        "-of",
        "default=nw=1:nk=1",
    ])
    .arg(video);
    let (ok, out, _) = run_bounded(&mut cmd, Duration::from_secs(30)).ok()?;
    ok.then(|| out.trim().parse::<f64>().ok()).flatten()
}

/// One frame every `every` seconds into `dir` (a temp folder outside the
/// vault); how many were written.
fn extract_frames(ffmpeg: &Path, video: &Path, dir: &Path, every: u32) -> Result<usize, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("frames folder: {e}"))?;
    let mut cmd = std::process::Command::new(ffmpeg);
    // Local files only: no network protocol, whatever the container says.
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-protocol_whitelist",
        "file",
        "-i",
    ])
    .arg(video)
    .args([
        "-vf",
        &format!("fps=1/{every},scale=1280:-2"),
        "-frames:v",
        &MAX_FRAMES.to_string(),
        "-q:v",
        "4",
    ])
    .arg(dir.join("frame-%03d.jpg"));
    let (ok, _, err) = run_bounded(&mut cmd, FFMPEG_TIMEOUT).map_err(|e| format!("ffmpeg {e}"))?;
    if !ok {
        let err: String = err.lines().rev().take(3).collect::<Vec<_>>().join(" ");
        return Err(format!("ffmpeg couldn't read the video: {err}"));
    }
    let mut n = 0;
    while dir.join(format!("frame-{:03}.jpg", n + 1)).is_file() {
        n += 1;
    }
    Ok(n)
}

fn slug(stem: &str) -> String {
    let mut s: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-').chars().take(60).collect::<String>();
    if s.is_empty() || s == "frames" {
        "capture".into()
    } else {
        s
    }
}

/// A capture ready to write: everything slow is done (frames in a temp
/// folder outside the vault), so [`commit`] holds History's lock only to
/// copy and write.
pub struct Prepared {
    /// The workspace's vault-relative path and the capture's folder in it.
    ws_path: String,
    out_rel: String,
    note_rel: String,
    note: String,
    frames_tmp: Option<PathBuf>,
    frames: usize,
    summary: Value,
}

impl Drop for Prepared {
    fn drop(&mut self) {
        // Only the temp folder this capture made.
        if let Some(tmp) = &self.frames_tmp {
            let _ = std::fs::remove_dir_all(tmp);
        }
    }
}

/// Validate, read the transcript, take the frames and draft the note. No
/// History lock, nothing written in the vault.
pub fn prepare(
    vault: &Path,
    workspace: &str,
    video: &str,
    transcript: Option<&str>,
    every: Option<u32>,
    ffmpeg: Option<&Path>,
) -> Result<Prepared, String> {
    let ws = crate::workspaces::summary(vault, workspace)?;
    let folder = crate::workspaces::folder(vault, workspace)?;
    let inside = |p: &str| -> Result<String, String> {
        let rel = p
            .trim_start_matches('/')
            .strip_prefix(&format!("{}/", ws.path))
            .unwrap_or(p.trim_start_matches('/'))
            .to_string();
        if rel.split('/').any(|seg| seg == ".." || seg.is_empty()) {
            return Err(format!("{p} isn't a path inside workspace {workspace}."));
        }
        Ok(rel)
    };
    // Through the vault's own path check, so a symlink out of the vault is refused.
    let resolve = |rel: &str| crate::vault::safe_join(vault, &format!("{}/{rel}", ws.path));
    let video_rel = inside(video)?;
    if !video_rel.starts_with("sources/") {
        return Err(format!(
            "The recording must be in the workspace's sources/ (copy it into that folder); got {video_rel}."
        ));
    }
    let ext = video_rel
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if !VIDEO_EXTS.contains(&ext.as_str()) {
        return Err(format!(
            "{video_rel} isn't a video ({}).",
            VIDEO_EXTS.join(", ")
        ));
    }
    let video_abs = resolve(&video_rel)?;
    if !video_abs.is_file() {
        return Err(format!(
            "{video_rel} doesn't exist in workspace {workspace}."
        ));
    }
    // The transcript: given, else beside the video with the same name.
    let transcript_rel = match transcript {
        Some(t) => {
            let t = inside(t)?;
            let ext = t.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            if !TRANSCRIPT_EXTS.contains(&ext.as_str()) {
                return Err(format!(
                    "{t} isn't a transcript ({}).",
                    TRANSCRIPT_EXTS.join(", ")
                ));
            }
            Some(t)
        }
        None => {
            let stem = video_rel
                .rsplit_once('.')
                .map_or(video_rel.as_str(), |(s, _)| s);
            TRANSCRIPT_EXTS
                .iter()
                .map(|e| format!("{stem}.{e}"))
                .find(|p| resolve(p).is_ok_and(|abs| abs.is_file()))
        }
    };
    let segments = match &transcript_rel {
        Some(t) => {
            let abs = resolve(t)?;
            let size = std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0);
            if size > TRANSCRIPT_MAX_BYTES {
                return Err(format!(
                    "{t} is over 5 MB; that's not a transcript Hollow will read."
                ));
            }
            let text = std::fs::read_to_string(&abs)
                .map_err(|e| format!("Couldn't read the transcript {t}: {e}"))?;
            let segs = parse_transcript(t, &text);
            if segs.is_empty() {
                return Err(format!(
                    "{t} has no lines Hollow could read as a transcript."
                ));
            }
            segs
        }
        None => Vec::new(),
    };
    // Frames spread over the whole recording: at most MAX_FRAMES of them.
    let mut every = every.unwrap_or(DEFAULT_EVERY_S).clamp(1, 600);
    if let Some(d) = ffmpeg.and_then(|f| duration_of(f, &video_abs)) {
        every = every
            .max((d / f64::from(MAX_FRAMES)).ceil() as u32)
            .min(600);
    }
    let stem = Path::new(&video_rel)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let base = format!("{CAPTURE_DIR}{}", slug(&stem));
    let mut out_rel = base.clone();
    let mut n = 2;
    while folder.join(&out_rel).exists() {
        out_rel = format!("{base}-{n}");
        n += 1;
    }
    let tmp = std::env::temp_dir().join(format!(
        "hollow-capture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let (frames, frame_note, frames_tmp) = match ffmpeg {
        Some(bin) => match extract_frames(bin, &video_abs, &tmp, every) {
            Ok(n) => (n, None, Some(tmp)),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                (0, Some(e), None)
            }
        },
        None => (
            0,
            Some(
                "ffmpeg isn't installed, so there are no frames (brew install ffmpeg adds them)."
                    .to_string(),
            ),
            None,
        ),
    };
    let mut drafted = steps(&segments);
    if drafted.is_empty() {
        // No transcript: one step per frame, for the person to describe.
        drafted = (0..frames)
            .map(|i| Step {
                start: Some(f64::from(every) * i as f64),
                text: String::new(),
                frame: None,
            })
            .collect();
    }
    for s in &mut drafted {
        s.frame = s.start.and_then(|t| frame_for(t, every, frames));
    }
    let mut note = format!(
        "# Process capture: {stem}\n\n\
         Recording: `{video_rel}` in {title}. Transcript: {transcript}. Frames: {frames} (one every {every} s).\n\n\
         Drafted locally by `process.capture` (stage 1): no model saw this recording when it was captured. The \
         frames are now workspace files, so an agent working here can open them. Edit the steps, then stage 2 \
         drafts the skill from them.\n\n## Steps (draft)\n\n",
        title = ws.title,
        transcript = transcript_rel
            .as_deref()
            .map(|t| format!("`{t}`"))
            .unwrap_or_else(|| "none (one step per frame)".into()),
    );
    if let Some(n) = &frame_note {
        note.push_str(&format!("> {n}\n\n"));
    }
    if drafted.is_empty() {
        note.push_str("No steps: there was no transcript and no frames.\n");
    }
    for (i, s) in drafted.iter().enumerate() {
        let when = s
            .start
            .map(|t| format!(" ({})", clock(t)))
            .unwrap_or_default();
        let text = if s.text.is_empty() {
            "_What happens here?_".to_string()
        } else {
            s.text.clone()
        };
        note.push_str(&format!("{}. **Step {}{when}.** {text}\n", i + 1, i + 1));
        if let Some(f) = &s.frame {
            note.push_str(&format!("   ![step {}]({f})\n", i + 1));
        }
    }
    note.push_str("\n## Inputs and outputs (to fill in)\n\n- Inputs:\n- Outputs:\n");
    let note_rel = format!("{}/{out_rel}/capture.md", ws.path);
    let summary = json!({
        "capture": note_rel,
        "folder": format!("{}/{out_rel}", ws.path),
        "steps": drafted.len(),
        "frames": frames,
        "everySeconds": every,
        "transcript": transcript_rel,
        "framesNote": frame_note,
        "note": "Local only: no model saw the recording. Review the steps in capture.md.",
    });
    Ok(Prepared {
        ws_path: ws.path.clone(),
        out_rel,
        note_rel,
        note,
        frames_tmp,
        frames,
        summary,
    })
}

/// Copy the frames in and write the note: the only part that changes the
/// vault (run inside the caller's History lease).
pub fn commit(vault: &Path, prepared: &Prepared) -> Result<Value, String> {
    if let Some(tmp) = &prepared.frames_tmp {
        let rel = format!("{}/{}/frames", prepared.ws_path, prepared.out_rel);
        let dest = crate::vault::safe_join(vault, &rel)?;
        std::fs::create_dir_all(&dest).map_err(|e| format!("frames folder: {e}"))?;
        for i in 1..=prepared.frames {
            let name = format!("frame-{i:03}.jpg");
            std::fs::copy(tmp.join(&name), dest.join(&name))
                .map_err(|e| format!("copy {name}: {e}"))?;
        }
    }
    crate::vault::create_note(vault, &prepared.note_rel, &prepared.note)?;
    Ok(prepared.summary.clone())
}

/// [`prepare`] then [`commit`], for callers already inside a lease (tests).
#[cfg(test)]
pub fn capture(
    vault: &Path,
    workspace: &str,
    video: &str,
    transcript: Option<&str>,
    every: Option<u32>,
    ffmpeg: Option<&Path>,
) -> Result<Value, String> {
    let prepared = prepare(vault, workspace, video, transcript, every, ffmpeg)?;
    commit(vault, &prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcripts_of_every_kind_parse_to_timed_segments() {
        let srt = "1\n00:00:01,000 --> 00:00:04,500\nOpen the push file.\n\n2\n00:00:07,000 --> 00:00:09,000\nThen run the plan step.\n";
        let s = parse_transcript("a.srt", srt);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].start, s[0].end), (Some(1.0), Some(4.5)));
        assert_eq!(s[1].text, "Then run the plan step.");
        let vtt = "WEBVTT\n\n00:01.000 --> 00:03.000 align:start\n<v Alex>Open HRIS.</v>\n\n";
        assert_eq!(parse_transcript("a.vtt", vtt)[0].text, "Open HRIS.");
        assert_eq!(parse_transcript("a.vtt", vtt)[0].start, Some(1.0));
        let json = r#"{"segments":[{"start":0.0,"end":2.0,"text":" First, export."},{"start":2.5,"end":3.0,"text":" "}]}"#;
        assert_eq!(parse_transcript("a.json", json).len(), 1);
        let txt = "# Notes\n[0:05] Pull the report\n- 1:10 - Filter by code\nplain line\n";
        let t = parse_transcript("a.txt", txt);
        assert_eq!(
            t.iter().map(|s| s.start).collect::<Vec<_>>(),
            [Some(5.0), Some(70.0), None]
        );
        assert_eq!(t[1].text, "Filter by code");
    }

    #[test]
    fn steps_split_at_pauses_and_cue_words_and_find_their_frame() {
        let seg = |s: f64, e: f64, t: &str| Segment {
            start: Some(s),
            end: Some(e),
            text: t.into(),
        };
        let segs = [
            seg(0.0, 2.0, "Open the push file"),
            seg(2.2, 4.0, "from the email."),
            seg(4.3, 6.0, "Then run the plan step."),
            seg(9.0, 12.0, "Pull the HRIS report for that range."),
            seg(12.1, 13.0, "next, run it."),
        ];
        let st = steps(&segs);
        let texts: Vec<&str> = st.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Open the push file from the email.",
                "Then run the plan step.",
                "Pull the HRIS report for that range.",
                "next, run it."
            ]
        );
        assert_eq!(
            frame_for(12.1, 10, 3).as_deref(),
            Some("frames/frame-002.jpg")
        );
        assert_eq!(frame_for(999.0, 10, 3), None, "no frame past the last one");
        assert_eq!(
            frame_for(19.9, 10, 3).as_deref(),
            Some("frames/frame-002.jpg"),
            "the one on screen then"
        );
        assert_eq!(frame_for(1.0, 10, 0), None);
        assert_eq!(clock(70.4), "1:10");
        // Cue words with Whisper's punctuation; a name is not a cue.
        assert!(starts_with_cue("Then, open HRIS"));
        assert!(starts_with_cue("Now: filter by code"));
        assert!(starts_with_cue("After that we save"));
        assert!(!starts_with_cue("Next.js handles the page"));
        assert!(!starts_with_cue("Nowhere else"));
        assert!(!starts_with_cue("Thenceforth"));
    }

    #[test]
    fn a_bom_and_times_on_their_own_lines_still_date_each_line() {
        let t = parse_transcript(
            "a.txt",
            "\u{feff}[0:05] Pull the report\n0:30\nFilter by code\n",
        );
        assert_eq!(
            t.iter().map(|s| s.start).collect::<Vec<_>>(),
            [Some(5.0), Some(30.0)]
        );
        let j = parse_transcript(
            "a.json",
            "\u{feff}{\"segments\":[{\"start\":1.0,\"text\":\"Go\"}]}",
        );
        assert_eq!(j.len(), 1);
        assert!(is_frame(
            "outputs/process-captures/tip-run/frames/frame-001.jpg"
        ));
        assert!(!is_frame("outputs/process-captures/frames/capture.md"));
        assert!(!is_frame("outputs/process-captures/tip-run/capture.md"));
        assert_eq!(slug("frames"), "capture");
    }

    fn workspace(v: &Path) -> (String, PathBuf) {
        let w = crate::workspaces::create(v, "TipFlow capture", "x").unwrap();
        let folder = crate::workspaces::folder(v, &w.id).unwrap();
        std::fs::create_dir_all(folder.join("sources")).unwrap();
        (w.id, folder)
    }

    #[test]
    fn a_capture_writes_a_step_list_beside_the_recording_without_any_model() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let (id, folder) = workspace(v);
        std::fs::write(folder.join("sources/tip run.mov"), b"not really a video").unwrap();
        std::fs::write(
            folder.join("sources/tip run.srt"),
            "1\n00:00:01,000 --> 00:00:03,000\nOpen the push file.\n\n2\n00:00:25,000 --> 00:00:27,000\nThen run plan.\n",
        )
        .unwrap();
        let out = capture(v, &id, "sources/tip run.mov", None, None, None).unwrap();
        assert_eq!(out["steps"], 2);
        assert_eq!(out["frames"], 0);
        assert_eq!(
            out["transcript"], "sources/tip run.srt",
            "the sidecar is found"
        );
        assert!(out["framesNote"]
            .as_str()
            .unwrap()
            .contains("ffmpeg isn't installed"));
        let note = std::fs::read_to_string(v.join(out["capture"].as_str().unwrap())).unwrap();
        assert!(
            note.contains("1. **Step 1 (0:01).** Open the push file."),
            "{note}"
        );
        assert!(
            note.contains("2. **Step 2 (0:25).** Then run plan."),
            "{note}"
        );
        assert!(note.contains("no model saw this recording"));
        // A second capture of the same recording gets its own folder.
        let again = capture(v, &id, "sources/tip run.mov", None, None, None).unwrap();
        assert!(again["folder"]
            .as_str()
            .unwrap()
            .ends_with("process-captures/tip-run-2"));
    }

    #[test]
    fn it_refuses_paths_outside_sources_and_non_videos() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let (id, folder) = workspace(v);
        std::fs::write(folder.join("sources/notes.txt"), "x").unwrap();
        std::fs::write(folder.join("outputs-clip.mov"), "x").unwrap();
        assert!(capture(v, &id, "sources/notes.txt", None, None, None)
            .unwrap_err()
            .contains("isn't a video"));
        assert!(capture(v, &id, "outputs-clip.mov", None, None, None)
            .unwrap_err()
            .contains("sources/"));
        assert!(capture(v, &id, "sources/../../x.mov", None, None, None)
            .unwrap_err()
            .contains("inside workspace"));
        assert!(capture(v, &id, "sources/missing.mov", None, None, None)
            .unwrap_err()
            .contains("doesn't exist"));
        std::fs::write(folder.join("sources/clip.mov"), "x").unwrap();
        assert!(capture(
            v,
            &id,
            "sources/clip.mov",
            Some("sources/clip.mov"),
            None,
            None
        )
        .unwrap_err()
        .contains("isn't a transcript"));
    }

    #[cfg(unix)]
    #[test]
    fn frames_come_from_ffmpeg_and_each_step_shows_the_nearest_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let (id, folder) = workspace(v);
        std::fs::write(folder.join("sources/run.mp4"), b"x").unwrap();
        std::fs::write(
            folder.join("sources/run.txt"),
            "[0:02] Open it\n[0:31] Then save it\n",
        )
        .unwrap();
        // A fake ffmpeg: writes four frames to the pattern it was given (the last argument).
        let fake = dir.path().join("fake-ffmpeg");
        std::fs::write(
            &fake,
            "#!/bin/sh\nfor last; do :; done\nfor i in 1 2 3 4; do printf jpg > \"$(printf \"$last\" $i)\"; done\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = capture(v, &id, "sources/run.mp4", None, Some(10), Some(&fake)).unwrap();
        assert_eq!(out["frames"], 4);
        let note = std::fs::read_to_string(v.join(out["capture"].as_str().unwrap())).unwrap();
        assert!(note.contains("![step 1](frames/frame-001.jpg)"), "{note}");
        assert!(note.contains("![step 2](frames/frame-004.jpg)"), "{note}");
        // A 20-minute recording (ffprobe beside ffmpeg says so): 60 frames
        // spread over all of it, one every 20 s rather than 10.
        let probe = dir.path().join("ffprobe");
        std::fs::write(&probe, "#!/bin/sh\necho 1200.0\n").unwrap();
        std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = capture(v, &id, "sources/run.mp4", None, Some(10), Some(&fake)).unwrap();
        assert_eq!(out["everySeconds"], 20);
        std::fs::remove_file(&probe).unwrap();
        // A failing ffmpeg leaves the steps and says why.
        std::fs::write(&fake, "#!/bin/sh\necho 'Invalid data found' >&2\nexit 1\n").unwrap();
        let out = capture(v, &id, "sources/run.mp4", None, Some(10), Some(&fake)).unwrap();
        assert_eq!(out["frames"], 0);
        assert!(out["framesNote"]
            .as_str()
            .unwrap()
            .contains("Invalid data found"));
        assert_eq!(out["steps"], 2);
    }
}
