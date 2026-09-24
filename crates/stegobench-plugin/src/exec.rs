// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo

//! Running a plugin and waiting for it, but never for ever.
//!
//! Every path that asks a tool a question goes through here. It exists as one
//! module rather than three call sites because the waiting is the part that
//! goes wrong, and it goes wrong the same way each time: a tool stops for a
//! prompt nobody will answer, or fills a pipe nobody is reading, and the
//! harness waits on it until somebody notices.
//!
//! `doctor` waiting for ever is an annoyance. `score` waiting for ever, part
//! way through a corpus of 344,357 pairs, is a run nobody can tell from a slow
//! one.

use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How long a single plugin invocation is given by default.
///
/// One image through one detector. The slowest registered tool costs about a
/// second per image on the recorded figures, so a minute is roughly sixty
/// times the worst measured case: long enough that a loaded machine does not
/// produce false failures, short enough that a stuck item cannot eat a run.
pub const ITEM_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a self-test phase is given.
///
/// Longer than an item because a container may be doing first-run work, and a
/// self-test runs once rather than once per image.
pub const PHASE_TIMEOUT: Duration = Duration::from_secs(300);

/// How long to wait for the output pipes after the child is gone.
///
/// Bounded for a specific reason, found by measurement rather than by
/// reasoning: a tool that forks leaves a grandchild holding the write end, so
/// the pipe stays open after the process we killed has gone, and joining the
/// reader threads unbounded reintroduces the hang one line after fixing it.
/// What is lost by not waiting is the tail of a message from a tool that has
/// already failed; what would be lost by waiting is the whole run.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// How often the child is checked. Short enough to be imperceptible, long
/// enough that waiting costs no measurable CPU across a long corpus.
const POLL: Duration = Duration::from_millis(20);

/// Run `command` to completion, or kill it when `timeout` expires.
///
/// Both output streams are drained on their own threads. That is not tidiness:
/// a child that fills a pipe blocks on the write while the parent waits for it
/// to exit, and that deadlock looks exactly like the hang this module exists
/// to prevent.
///
/// The error is a sentence a person can act on, naming the tool and what it
/// did, because it reaches a user through `doctor` and through a result
/// record.
pub fn captured(mut command: Command, label: &str, timeout: Duration) -> Result<Output, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not run {label}: {e}"))?;

    let out_rx = drain(child.stdout.take());
    let err_rx = drain(child.stderr.take());

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => return Err(format!("could not wait for {label}: {e}")),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            break child.wait().unwrap_or_else(|_| exited_nonzero());
        }
        thread::sleep(POLL);
    };

    let stdout = out_rx.recv_timeout(DRAIN_GRACE).unwrap_or_default();
    let stderr = err_rx.recv_timeout(DRAIN_GRACE).unwrap_or_default();

    if timed_out {
        // The hint is worth the words. A tool that never answers is almost
        // always waiting on input: a passphrase prompt, a confirmation, a
        // terminal nobody is watching. Saying so sends the reader to the
        // likely cause rather than to a bug report.
        return Err(format!(
            "{label} gave no answer in {}s and was killed, which usually means \
             it is waiting on something nobody is going to type{}",
            timeout.as_secs(),
            tail(&stderr)
        ));
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Read a pipe to the end on its own thread, handing back what it read.
fn drain(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    rx
}

/// The last of a tool's complaint, for an error message a person reads.
///
/// Bounded, because a tool that failed by printing a million lines should not
/// put a million lines into a result record.
fn tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let shown: String = trimmed
        .chars()
        .rev()
        .take(160)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!(". stderr: {shown}")
}

/// Stands in for the status of a child we killed and could not reap.
///
/// Only reachable when the operating system refuses to report on a process we
/// have already signalled, and the caller is about to return a timeout error
/// regardless, so the value is never read as a real exit status.
fn exited_nonzero() -> std::process::ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(9)
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new("cmd")
            .args(["/C", "exit 9"])
            .status()
            .expect("a shell that can exit")
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.args(["-c", script]);
        c
    }

    #[test]
    fn a_tool_that_answers_is_answered() {
        let out = captured(sh("echo hello"), "sh", Duration::from_secs(5)).expect("ran");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
        assert!(out.status.success());
    }

    #[test]
    fn both_streams_come_back() {
        // Parsers read stdout and report stderr, so losing either turns a
        // diagnosable failure into a blank one.
        let out =
            captured(sh("echo out; echo err >&2"), "sh", Duration::from_secs(5)).expect("ran");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "out");
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "err");
    }

    #[test]
    fn a_failing_tool_is_not_an_error_here() {
        // A non-zero exit is the tool's answer, and the parser decides what it
        // means. Treating it as a harness failure would lose the output that
        // says why.
        let out = captured(sh("echo why >&2; exit 3"), "sh", Duration::from_secs(5)).expect("ran");
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "why");
    }

    #[test]
    fn a_tool_that_never_answers_is_killed_and_says_so() {
        let started = Instant::now();
        let err = captured(sh("sleep 30"), "sh", Duration::from_millis(150))
            .expect_err("it should have been killed");
        assert!(started.elapsed() < Duration::from_secs(10), "it waited");
        assert!(err.contains("gave no answer in"), "got {err}");
    }

    #[test]
    fn a_tool_that_waits_on_input_does_not_hold_the_run() {
        // stdin is closed rather than inherited, so a tool that reads it gets
        // an immediate end of file instead of waiting on a terminal nobody is
        // watching. This is the usual way a plugin hangs.
        let out = captured(sh("cat; echo done"), "sh", Duration::from_secs(5)).expect("ran");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "done");
    }

    #[test]
    fn a_noisy_tool_does_not_deadlock_on_a_full_pipe() {
        // The deadlock this prevents: the child blocks writing to a pipe
        // nobody is reading while the parent blocks waiting for the child to
        // exit. It looks identical to the hang the timeout exists for, and it
        // happens at a volume a self-test never reaches but a corpus does.
        let out = captured(
            sh("i=0; while [ $i -lt 20000 ]; do echo 'a line of output'; i=$((i+1)); done"),
            "sh",
            Duration::from_secs(20),
        )
        .expect("ran");
        assert_eq!(String::from_utf8_lossy(&out.stdout).lines().count(), 20000);
    }

    #[test]
    fn a_missing_program_is_reported_with_its_name() {
        let err = captured(
            Command::new("definitely-not-installed-anywhere"),
            "definitely-not-installed-anywhere",
            Duration::from_secs(5),
        )
        .expect_err("there is no such program");
        assert!(err.contains("could not run"), "got {err}");
        assert!(
            err.contains("definitely-not-installed-anywhere"),
            "got {err}"
        );
    }
}
