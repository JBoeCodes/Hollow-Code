//! Secrets handed to an agent child on inherited file descriptors, never on
//! argv or in the environment (R5b, Q-R33). `ps` shows every process's argv,
//! and `ps -E` shows its environment to the same user for any non-Apple
//! binary (Claude, Codex, Grok, node), so the appended system prompt and an
//! text travels as an unlinked private file the child reads by number:
//! `--append-system-prompt-file /dev/fd/<n>`. Proven against a fake endpoint
//! with Claude Code 2.1.325, with and without a keychain login present (the
//! request carried the appended text). The account token does NOT go this way:
//! with a login present Claude ignores
//! `CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR` and uses the login, so the token
//! stays in `CLAUDE_CODE_OAUTH_TOKEN` until a probe passes with a login.
//!
//! The files have no name once created (so no other process can open them
//! and nothing is left to clean up) and are regular files (so an unread one
//! never blocks the child or Hollow). The child's own descriptors are the
//! only way in.

use std::fs::File;

/// The descriptor a runner run's appended system prompt is opened on there
/// (`remote_session`; the runner's supervisor picks it, not this module).
pub(crate) const APPEND_FD: i32 = 7;

/// An unlinked private file holding `text`, rewound.
pub(crate) fn private_file(text: &str) -> std::io::Result<File> {
    use std::io::{Seek, Write};
    let mut file = tempfile::tempfile()?;
    file.write_all(text.as_bytes())?;
    file.flush()?;
    file.rewind()?;
    Ok(file)
}

#[cfg(test)]
thread_local! {
    static PASSED: std::cell::RefCell<Vec<(i32, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Test-only: what [`pass`] has handed out since the last build on this
/// thread, as `(fd, text)`.
#[cfg(test)]
pub(crate) fn passed() -> Vec<(i32, String)> {
    PASSED.with(|p| p.borrow().clone())
}

/// Test-only: forget what was handed out (each command build starts here).
#[cfg(test)]
pub(crate) fn reset() {
    PASSED.with(|p| p.borrow_mut().clear());
}

/// Test-only: `args` with a private appended system prompt shown the way an
/// inline one reads (`--append-system-prompt <text>`), so a test of the
/// prompt's content reads the same whichever way it travels.
#[cfg(test)]
pub(crate) fn expand_append(args: Vec<String>) -> Vec<String> {
    let handed = passed();
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.into_iter().peekable();
    while let Some(arg) = it.next() {
        let text = (arg == "--append-system-prompt-file")
            .then(|| it.peek().cloned())
            .flatten()
            .and_then(|path| {
                handed
                    .iter()
                    .find(|(fd, _)| path == format!("/dev/fd/{fd}"))
                    .map(|(_, text)| text.clone())
            });
        match text {
            Some(text) => {
                it.next();
                out.push("--append-system-prompt".to_string());
                out.push(text);
            }
            None => out.push(arg),
        }
    }
    out
}

/// Give the child each text as an open, rewound, unlinked file on a
/// descriptor, and return the descriptor numbers (same order).
///
/// The file is moved to a free descriptor of 10 or more in this process and
/// stays open here (close-on-exec) until `cmd` is dropped. That number is the
/// target too: Rust's spawn creates its error-report pipe at the lowest free
/// descriptor, and ours is occupied, so the pipe can never land on it, and
/// no `dup2` over anything is needed; the `pre_exec` hook only clears
/// close-on-exec in the child. Fails (leaving `cmd` as it was) when a file
/// can't be made, so the caller can keep the old route.
#[cfg(unix)]
pub(crate) fn pass(cmd: &mut tokio::process::Command, texts: &[&str]) -> std::io::Result<Vec<i32>> {
    use std::os::unix::io::{AsRawFd, FromRawFd};
    let mut files: Vec<File> = Vec::with_capacity(texts.len());
    for text in texts {
        let plain = private_file(text)?;
        // SAFETY: plain is open; F_DUPFD_CLOEXEC returns a new descriptor
        // this process then owns.
        let high = unsafe { libc::fcntl(plain.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
        if high < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `high` was just returned by fcntl and is owned by no one else.
        files.push(unsafe { File::from_raw_fd(high) });
    }
    let fds: Vec<i32> = files.iter().map(AsRawFd::as_raw_fd).collect();
    #[cfg(test)]
    PASSED.with(|p| {
        p.borrow_mut()
            .extend(fds.iter().copied().zip(texts.iter().map(|t| t.to_string())));
    });
    // SAFETY: the closure runs between fork and exec and calls only lseek and
    // fcntl, which are async-signal-safe, and allocates nothing.
    unsafe {
        cmd.pre_exec(move || {
            for file in &files {
                let fd = file.as_raw_fd();
                libc::lseek(fd, 0, libc::SEEK_SET);
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    Ok(fds)
}

#[cfg(not(unix))]
pub(crate) fn pass(_: &mut tokio::process::Command, _: &[&str]) -> std::io::Result<Vec<i32>> {
    Err(std::io::Error::other("descriptors need a Unix system"))
}
