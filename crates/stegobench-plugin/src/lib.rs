// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Running a plugin, and finding out whether one can be run at all.
//!
//! THE PROTOCOL, IN FULL
//! ---------------------
//! A plugin is a program, or a container image holding one, that answers two
//! verbs and nothing else.
//!
//! `describe` prints one JSON object and exits 0. `run` reads newline
//! delimited work items on stdin and prints one record per item on stdout,
//! with progress on stderr.
//!
//! Two verbs is the whole contract. Sharding, timeouts, memory caps, container
//! lifecycle, retries and resumption are the host's job, and no plugin author
//! ever thinks about them. That is what makes "contribute a detector" mean
//! writing thirty lines in any language rather than learning our internals.
//!
//! AN ERROR IS A RECORD, NOT AN ABSENCE
//! ------------------------------------
//! A plugin that cannot score an image says so, in a record carrying the same
//! id. On 2026-09-17 resume logic here treated an empty failed record as
//! complete, so a re-run scored nothing while printing "resuming: 1480 files
//! already scored". A record is done when it carries a score **or** an error,
//! never when it merely exists.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use stegobench_core::registry::Entry;

pub mod adapter;
pub mod availability;
pub mod embed;
pub mod exec;
pub mod parsers;
pub mod runner;
pub mod selftest;

pub use availability::{registry_reach, Availability, Presence, Reach};
pub use parsers::Reading;
pub use selftest::Verified;

/// One item of work handed to a plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkItem {
    /// Stable across the run, and the only thing tying a record back to a file.
    pub id: String,
    pub path: PathBuf,
}

/// What a plugin says about one item.
///
/// Exactly one of `score`, `verdict` or `error` is meaningful, and the
/// [`Record::is_complete`] rule is what resume depends on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
}

impl Record {
    /// Whether this item needs doing again.
    ///
    /// An answer OR an error counts. A record with neither is a placeholder
    /// the plugin wrote before it worked, and treating that as done is the
    /// exact bug that made a re-run score nothing while reporting success.
    pub fn is_complete(&self) -> bool {
        self.score.is_some() || self.verdict.is_some() || self.error.is_some()
    }
}

/// How a plugin describes itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Describe {
    pub protocol: u32,
    pub name: String,
    pub version: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("{name} is not installed: {reason}")]
    NotInstalled { name: String, reason: String },
    #[error("{name} could not be run: {source}")]
    Spawn {
        name: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{name} produced output this version cannot read: {reason}")]
    BadOutput { name: String, reason: String },
}

/// What runs a host adapter when its entry names no entrypoint.
///
/// One constant rather than two literals, because the availability check and
/// the runner have to agree about which program they are talking about: a
/// `doctor` line saying the tool is here, against an interpreter a run then
/// fails to launch, is the exact drift this file's neighbours exist to stop.
pub const DEFAULT_HOST_ENTRYPOINT: &str = "python3";

/// The SHA256 of a file, which is how a binary plugin is pinned.
///
/// Stronger than a tag: a tag can be moved under you by whoever published it,
/// and a hash of the bytes actually executed cannot.
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(format!("sha256:{:x}", h.finalize()))
}

/// Finds an executable the way a shell would, so a registry entry can name
/// `stegcore` rather than an absolute path that differs per machine.
pub fn which(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return p.is_file().then_some(p);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(program))
            .find(|p| p.is_file())
    })
}

/// How long a tool gets to say what version it is.
///
/// A version probe is the cheapest question there is, so the ceiling is for
/// the tool that does not answer it rather than for the one that is slow.
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// How many lines of a version probe's output are read before giving up.
///
/// Real tools print a banner and put the version on the second or third line.
/// Nothing prints it on the fortieth, and an unbounded scan over a tool that
/// answered with its whole help text is a parser with no resource cap.
const VERSION_LINES_SCANNED: usize = 10;

/// The longest a line may be and still be a version.
///
/// Generous on purpose: `zsteg 0.2.13 (c) 2019 ...` is a real version line and
/// a cap tight enough to be elegant would throw it away. A line longer than
/// this is prose, a traceback, or a usage message.
const VERSION_LINE_MAX: usize = 200;

/// What a version probe produced, and whether it can be believed.
///
/// Three answers rather than an `Option`, because "it refused to say" and "it
/// said something that cannot be a version" send a reader to different
/// problems, and the second used to be recorded as though it were the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionProbe {
    /// It answered, and the answer can be a version.
    Reported(String),
    /// It answered, and the answer cannot be recorded as a version. The string
    /// is a short reason that names what the tool actually printed.
    Refused(String),
    /// It could not be asked at all: no binary, not on PATH, no answer inside
    /// the deadline.
    NotAsked,
}

impl VersionProbe {
    /// The short string that goes in front of the hash in a presence pin.
    ///
    /// Kept under forty characters because `doctor` renders it in a column,
    /// and it carries a digit wherever the tool's own answer did, so the
    /// renderer prints it rather than collapsing it to "probe failed".
    pub fn pin_label(&self) -> &str {
        match self {
            VersionProbe::Reported(v) => v,
            VersionProbe::Refused(why) => why,
            VersionProbe::NotAsked => "unknown version",
        }
    }
}

/// Asks a binary plugin what version it is, using the arguments its entry
/// declares, and refuses an answer that cannot be a version.
///
/// WHY THE ANSWER IS CHECKED AND NOT JUST RECORDED
/// -----------------------------------------------
/// An adversarial researcher walked this build on 2026-10-02 with a shell
/// script that printed a score for every input it was given. Asked `--version`
/// it printed `0.010`, and the harness wrote `0.010` down as the version of the
/// thing that produced a number. Nothing anywhere asked whether a version
/// string looked like a version.
///
/// Where the line sits, and why it sits there. Real tools answer in wildly
/// different shapes (`zsteg 0.2.13`, `v1.4`, `StegExpose 0.0.2`, a bare `1.0`,
/// a banner with the version on the third line, sometimes on stderr), and a
/// check strict enough to catch every cheat would throw honest versions away,
/// which is the worse failure. So only two things are refused:
///
/// 1. Output with no version-shaped token anywhere in the lines scanned. A
///    Python tool once answered `Traceback (most recent call last):` and that
///    sat in the column every other row uses for a digest. A token is dotted
///    digits, optionally `v` prefixed, which is what every shape above has and
///    what a traceback, a usage line and a copyright year do not.
/// 2. An answer with no letter in it that the tool's OWN output parser reads as
///    a score. `0.010` is equally a version and this tool's answer about an
///    image, nothing here can tell which was meant, and guessing is how a
///    misread becomes a published figure. The cost is that a tool whose entire
///    version output is a bare `1.0` is refused too; it keeps working, and its
///    result is pinned by the hash of its bytes rather than by what it says
///    about itself, which is the stronger pin anyway.
///
/// Bounded through [`exec::captured`] rather than `Command::output()`, which
/// reads both pipes to end of file with no deadline. Measured 2026-09-29: a
/// tool that prints its version, exits 0, and leaves one background child
/// holding the write end of the pipe makes `output()` block forever, because
/// end of file arrives when the LAST holder closes it rather than when the
/// process we spawned exits. The tool had already answered. Forking a helper
/// and returning is ordinary behaviour for a shell wrapper, which is what
/// several registry entries point at.
pub fn binary_version(entry: &Entry) -> VersionProbe {
    let Some(bin) = entry.binary.as_ref() else {
        return VersionProbe::NotAsked;
    };
    let Some(program) = bin.command.first() else {
        return VersionProbe::NotAsked;
    };
    let Some(path) = which(program) else {
        return VersionProbe::NotAsked;
    };
    let mut cmd = Command::new(&path);
    cmd.args(&bin.version_args);
    let Ok(out) = exec::captured(cmd, program, VERSION_TIMEOUT) else {
        return VersionProbe::NotAsked;
    };
    let text = String::from_utf8_lossy(if out.stdout.is_empty() {
        &out.stderr
    } else {
        &out.stdout
    });
    let parser = entry.invoke.as_ref().map(|i| i.parser.as_str());
    read_version(&text, parser)
}

/// The version in a probe's output, or the reason there is not one.
///
/// Separated from the running so that every shape a real tool prints is a test
/// rather than a claim. See [`binary_version`] for where the line sits.
fn read_version(text: &str, parser: Option<&str>) -> VersionProbe {
    let candidate = text
        .lines()
        .take(VERSION_LINES_SCANNED)
        .map(str::trim)
        .filter(|line| line.len() <= VERSION_LINE_MAX && !line.chars().any(char::is_control))
        .find(|line| line.split_whitespace().any(is_version_token));

    let Some(line) = candidate else {
        // Named rather than summarised: the operator has to be able to see
        // that the tool answered and what it said, or they go looking for an
        // installation problem that is not there.
        let first = text.lines().next().unwrap_or("").trim();
        return VersionProbe::Refused(format!("no version in: {}", clip(first, 22)));
    };

    // The cheat's answer: a number and nothing else, which this tool's own
    // parser reads as its answer about an image. Refused rather than resolved,
    // and the message says which two readings it could not choose between.
    let bare_number = !line.chars().any(|c| c.is_ascii_alphabetic());
    if bare_number {
        let reads_as_an_answer = parser.is_some_and(|p| {
            matches!(
                parsers::parse(p, line, ""),
                Reading::Score(_) | Reading::Verdict(true)
            )
        });
        if reads_as_an_answer {
            return VersionProbe::Refused(format!("{} is a score, not a version", clip(line, 12)));
        }
    }
    VersionProbe::Reported(line.to_string())
}

/// Is this token shaped like a version: `1.4`, `v2.0.0`, `0.2.13-rc1`?
///
/// Dotted digits, because that is the one thing every real shape has in common
/// and the thing a traceback, a usage line and a copyright year all lack. A
/// bare integer and a date are not accepted, and a tool versioned that way is
/// refused with its answer named rather than having the answer guessed at.
fn is_version_token(token: &str) -> bool {
    let trimmed = token.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    let stripped = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    let head: String = stripped
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut groups = head.split('.').peekable();
    let mut count = 0usize;
    while let Some(group) = groups.next() {
        // An empty group means two dots in a row or a gap: `1..2` is not a
        // number anybody versions with. A trailing dot never reaches here,
        // because the punctuation at either end was trimmed above.
        if group.is_empty() {
            return groups.peek().is_none() && count >= 2;
        }
        if !group.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        count += 1;
    }
    count >= 2
}

/// A string cut to a length that fits a column, with the cut made visible.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max.saturating_sub(3)).collect();
    format!("{head}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hang this whole module's timeout discipline exists to stop, driven
    /// rather than described.
    ///
    /// A tool that prints its version, exits 0, and leaves one background
    /// child alive is a shell wrapper doing something ordinary. The write end
    /// of the pipe is inherited by that child, so end of file never arrives,
    /// and `Command::output()` waits on it for as long as the machine stays
    /// up. Measured 2026-09-29 with a five line script: `output()` had not
    /// returned after 25 seconds although the tool answered immediately.
    ///
    /// Written to fail rather than to hang if the bound is ever removed: the
    /// probe runs on its own thread and the assertion is on a channel with a
    /// deadline, so a regression reports a failure in seconds instead of
    /// wedging the suite, which is what a test for a hang has to do.
    #[cfg(unix)]
    #[test]
    fn a_version_probe_returns_even_when_the_tool_leaves_a_child_holding_the_pipe() {
        let dir = tempfile::tempdir().expect("tmp");
        let script = dir.path().join("forker.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nsleep 120 &\necho 'forker 9.9'\nexit 0\n",
        )
        .expect("written");
        let mut perms = std::fs::metadata(&script).expect("stat").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&script, perms).expect("chmod");

        let entry: Entry = toml::from_str(&format!(
            "name = \"forker\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [\"{}\"]\nversion_args = [\"--version\"]\n\
             [selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n",
            script.display()
        ))
        .expect("the entry parses");

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(binary_version(&entry));
        });

        // Comfortably above VERSION_TIMEOUT and far below the 120s the
        // grandchild lives for, so this can only pass because the probe is
        // bounded.
        match rx.recv_timeout(Duration::from_secs(VERSION_TIMEOUT.as_secs() + 20)) {
            Ok(version) => assert_eq!(
                version,
                VersionProbe::Reported("forker 9.9".into()),
                "the tool did answer, so its answer should survive the bound"
            ),
            Err(_) => panic!(
                "the version probe never returned: a child holding the pipe \
                 open is enough to hang it again"
            ),
        }
    }

    /// Every shape a real tool was observed to answer in, kept together so
    /// that tightening the check has to face all of them at once.
    #[test]
    fn the_shapes_real_tools_answer_in_are_all_recorded() {
        for answer in [
            "zsteg 0.2.13",
            "v1.4",
            "StegExpose 0.0.2",
            "stegcore 1.2.3-rc1",
            "zsteg 0.2.13 (c) 2019 Andrey Zaikin, all rights reserved",
            "tool version 1.4.",
        ] {
            assert_eq!(
                read_version(answer, Some("number")),
                VersionProbe::Reported(answer.trim().to_string()),
                "an honest version was refused: {answer:?}"
            );
        }
    }

    #[test]
    fn a_version_on_the_third_line_of_a_banner_is_found() {
        // A banner is ordinary. The first line carries no version-shaped
        // token, so the scan goes on rather than recording the product name.
        assert_eq!(
            read_version("MyTool\nCopyright (c) 2019 nobody\n1.4.2\n", Some("number")),
            VersionProbe::Reported("1.4.2".into()),
            "the copyright year must not be mistaken for the version either"
        );
    }

    #[test]
    fn a_traceback_is_refused_rather_than_recorded_as_a_version() {
        // The failure this check was written beside: a Python tool answered
        // with a traceback and it sat in the column every other row uses for
        // a digest. The second line carries a digit, so a looser rule would
        // have recorded `File "x.py", line 1` as the version.
        match read_version(
            "Traceback (most recent call last):\n  File \"x.py\", line 1, in <module>\n",
            Some("number"),
        ) {
            VersionProbe::Refused(why) => assert!(why.contains("Traceback"), "got {why}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_tool_that_answers_the_version_probe_with_its_score_is_refused() {
        // Rung 5, 2026-10-02: a shell script that printed a score for every
        // input printed `0.010` for `--version` too, and the harness wrote
        // that down as the version of the thing that produced a number.
        match read_version("0.010\n", Some("number")) {
            VersionProbe::Refused(why) => {
                assert!(why.contains("0.010"), "the answer is named: {why}");
                assert!(why.contains("score"), "and why it was refused: {why}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_number_is_refused_only_where_the_tools_own_parser_reads_it() {
        // The line is drawn at ambiguity, not at shape. `1.0` is a perfectly
        // ordinary version, and it is refused for an entry whose parser reads
        // a bare number as an answer about an image, because nothing here can
        // tell which of the two readings was meant. For a tool that answers in
        // JSON there is no second reading, so it is recorded.
        assert!(matches!(
            read_version("1.0\n", Some("number")),
            VersionProbe::Refused(_)
        ));
        assert_eq!(
            read_version("1.0\n", Some("stegcore")),
            VersionProbe::Reported("1.0".into())
        );
    }

    #[test]
    fn a_usage_message_and_an_empty_answer_are_both_refused() {
        for answer in ["", "usage: tool [-h] [--version]\n", "   \n"] {
            assert!(
                matches!(
                    read_version(answer, Some("number")),
                    VersionProbe::Refused(_)
                ),
                "this is not a version: {answer:?}"
            );
        }
    }

    #[test]
    fn a_probe_that_floods_the_output_is_bounded() {
        // A tool that answers the version flag with its whole manual. The
        // scan stops after a fixed number of lines, so the version buried
        // below is not found and the answer is refused rather than the parser
        // reading megabytes to find out.
        let mut flood = "no version here\n".repeat(VERSION_LINES_SCANNED + 5);
        flood.push_str("1.2.3\n");
        assert!(matches!(
            read_version(&flood, Some("number")),
            VersionProbe::Refused(_)
        ));
    }

    #[test]
    fn an_overlong_line_is_not_a_version_even_with_a_number_in_it() {
        let long = format!("{} 1.2.3", "x".repeat(VERSION_LINE_MAX));
        assert!(matches!(
            read_version(&long, Some("number")),
            VersionProbe::Refused(_)
        ));
    }

    #[test]
    fn a_pin_label_is_short_enough_for_the_column_it_is_printed_in() {
        // `doctor` renders this in a forty character column. A refusal that
        // overflows it is a refusal nobody reads.
        for probe in [
            read_version("0.010\n", Some("number")),
            read_version("Traceback (most recent call last):\n", Some("number")),
            VersionProbe::NotAsked,
        ] {
            let label = probe.pin_label();
            assert!(
                label.chars().count() <= 40,
                "{label:?} is {} characters",
                label.chars().count()
            );
        }
    }

    #[test]
    fn a_record_with_a_score_is_complete() {
        let r = Record {
            id: "1".into(),
            score: Some(0.7),
            verdict: None,
            error: None,
            elapsed_ms: None,
        };
        assert!(r.is_complete());
    }

    #[test]
    fn a_record_with_an_error_is_also_complete() {
        // An error is an answer. Re-running it would fail the same way, and
        // treating it as outstanding makes a resume loop forever.
        let r = Record {
            id: "1".into(),
            score: None,
            verdict: None,
            error: Some("decode failed".into()),
            elapsed_ms: None,
        };
        assert!(r.is_complete());
    }

    #[test]
    fn an_empty_placeholder_is_not_complete() {
        // The 2026-09-17 bug, pinned. The panel writes a record before it
        // scores, and counting that as done made a re-run print "resuming:
        // 1480 files already scored" and then score nothing.
        let r = Record {
            id: "1".into(),
            score: None,
            verdict: None,
            error: None,
            elapsed_ms: None,
        };
        assert!(!r.is_complete());
    }

    #[test]
    fn a_verdict_only_plugin_still_produces_complete_records() {
        // zsteg answers yes or no and never a number. That is a property of
        // the tool, not a failure, and resume must not treat it as unfinished.
        let r = Record {
            id: "1".into(),
            score: None,
            verdict: Some(true),
            error: None,
            elapsed_ms: None,
        };
        assert!(r.is_complete());
    }

    #[test]
    fn hashing_a_file_is_stable_and_prefixed() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x");
        std::fs::write(&p, b"hello").unwrap();
        let a = hash_file(&p).unwrap();
        assert_eq!(a, hash_file(&p).unwrap());
        assert!(a.starts_with("sha256:"));
    }

    #[test]
    fn hashing_notices_a_changed_byte() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x");
        std::fs::write(&p, b"hello").unwrap();
        let before = hash_file(&p).unwrap();
        std::fs::write(&p, b"hellp").unwrap();
        assert_ne!(before, hash_file(&p).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn which_finds_something_that_exists_and_not_something_that_does_not() {
        assert!(which("sh").is_some(), "sh should be on PATH");
        assert!(which("definitely-not-a-real-program-xyzzy").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn an_absolute_path_is_used_directly() {
        assert!(which("/bin/sh").is_some() || which("/usr/bin/sh").is_some());
        assert!(which("/nope/nothing").is_none());
    }

    #[test]
    fn records_round_trip_and_omit_what_is_absent() {
        let r = Record {
            id: "42".into(),
            score: Some(0.5),
            verdict: None,
            error: None,
            elapsed_ms: Some(10),
        };
        let text = serde_json::to_string(&r).unwrap();
        assert!(
            !text.contains("verdict"),
            "absent fields should not be written"
        );
        assert_eq!(serde_json::from_str::<Record>(&text).unwrap(), r);
    }
}
