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

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use stegobench_core::registry::Entry;

pub mod availability;
pub mod exec;
pub mod parsers;
pub mod runner;
pub mod selftest;

pub use availability::{Availability, Presence};
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

/// Asks a binary plugin what version it is, using the arguments its entry
/// declares. Returns None when it cannot be asked, rather than guessing.
pub fn binary_version(entry: &Entry) -> Option<String> {
    let bin = entry.binary.as_ref()?;
    let program = bin.command.first()?;
    let path = which(program)?;
    let out = Command::new(&path).args(&bin.version_args).output().ok()?;
    let text = String::from_utf8_lossy(if out.stdout.is_empty() {
        &out.stderr
    } else {
        &out.stdout
    });
    text.lines().next().map(|l| l.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn which_finds_something_that_exists_and_not_something_that_does_not() {
        assert!(which("sh").is_some(), "sh should be on PATH");
        assert!(which("definitely-not-a-real-program-xyzzy").is_none());
    }

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
