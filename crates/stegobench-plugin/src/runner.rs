// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo

//! Scoring a corpus: one tool, many items, and a run that survives being
//! interrupted.
//!
//! WHY THIS IS THIN
//!
//! Asking a detector about one file already exists, because `doctor` has to do
//! it: an [`Entry`] and a path go in, a [`Reading`] comes out, and a `Reading`
//! already knows how to become a [`Record`]. So a run is that, in a loop, with
//! three things added that only matter once the loop is long: it must not hold
//! the corpus in memory, it must survive an interruption, and it must say it
//! is alive.
//!
//! WHY RESUME IS POSITIONAL RATHER THAN A SET
//!
//! The obvious resume is a set of finished ids, skipping any item already in
//! it. That set grows with the corpus, and a Core tier is 344,357 pairs, so
//! the set runs to hundreds of thousands of ids: exactly the unbounded
//! collection baseline Section 12 forbids. It also
//! reads the whole record file before the first item is scored.
//!
//! So records are appended IN ITEM ORDER, and a resumed run walks the existing
//! records alongside the items, one at a time, in constant memory. Position
//! plus a check that the ids still agree gives the same guarantee, and gives a
//! second one for free: if the corpus changed under the run, the ids diverge
//! and the run says so instead of writing records against the wrong files.
//!
//! That divergence check is the part worth keeping. A resumed run whose corpus
//! grew by one file at the front would otherwise attribute every score to the
//! wrong image, and every number would be wrong in a way nothing downstream
//! could detect.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use stegobench_core::registry::Entry;

use crate::{Record, WorkItem};

/// How often a long run says it is alive, per baseline Section 2.1.
///
/// A corpus run is hours. Silence for hours is indistinguishable from a hang,
/// and this project has now lost time three times to a long job that stopped
/// without announcing it.
pub const HEARTBEAT: Duration = Duration::from_secs(30);

/// What a run did, for the caller that turns it into metrics.
///
/// Counts rather than the records themselves, because the records went to the
/// sink as they were produced and holding them here would put the corpus back
/// in memory one layer up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tally {
    /// Items this run scored, not counting ones resumed from a previous run.
    pub scored: u64,
    /// Items already recorded when the run started.
    pub resumed: u64,
    /// Items the tool could not answer about. Counted rather than inferred,
    /// because `n_error` in a result is required precisely so that zero is an
    /// assertion rather than an absence.
    pub errored: u64,
}

impl Tally {
    /// Every item accounted for, however it was accounted for.
    pub fn seen(&self) -> u64 {
        self.scored + self.resumed
    }
}

/// Why a run stopped early.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The corpus and the existing records disagree, so resuming would write
    /// answers against the wrong files.
    #[error(
        "this run was resumed, and at position {position} the record file \
         names {recorded:?} while the corpus offers {offered:?}. The corpus \
         has changed since the run that wrote those records, so resuming \
         would score one image and file the answer under another. Start a \
         fresh record file, or point at the corpus the records were made \
         against."
    )]
    Diverged {
        position: u64,
        recorded: String,
        offered: String,
    },
    #[error("could not read the existing records at {path}: {source}")]
    ReadRecords {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write a record to {path}: {source}")]
    WriteRecord {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("a record already written to {path} could not be read back: {reason}")]
    BadRecord { path: String, reason: String },
}

/// Somewhere completed records go, one at a time.
///
/// A trait rather than a file path so the logic never touches storage
/// directly, per baseline Section 12: a run that can write to a local file can
/// write to anything else without this module learning a second way to do it.
pub trait Sink {
    fn write(&mut self, record: &Record) -> std::io::Result<()>;
}

/// Appends records as JSON lines.
///
/// One line per record, flushed as it is written. Flushing every line is the
/// point rather than an oversight: a buffered run that is killed loses exactly
/// the work a resume was supposed to save, and the cost is one small write per
/// item against a tool invocation measured in hundreds of milliseconds.
pub struct JsonLines<W: Write> {
    out: W,
}

impl<W: Write> JsonLines<W> {
    pub fn new(out: W) -> Self {
        Self { out }
    }
}

impl<W: Write> Sink for JsonLines<W> {
    fn write(&mut self, record: &Record) -> std::io::Result<()> {
        let line = serde_json::to_string(record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.out.write_all(line.as_bytes())?;
        self.out.write_all(b"\n")?;
        self.out.flush()
    }
}

/// The ids already recorded, in the order they were written.
///
/// Reads one line at a time and hands back one id at a time, so a resumed run
/// costs one record of memory rather than one corpus of it.
///
/// A trailing partial line is ignored rather than treated as corruption: a run
/// killed mid-write leaves one, it is the ordinary case rather than the
/// alarming one, and the item it belongs to is simply scored again.
struct Prior {
    lines: Option<std::io::Lines<BufReader<std::fs::File>>>,
    path: String,
}

impl Prior {
    fn open(path: &Path) -> Result<Self, RunError> {
        let lines = match std::fs::File::open(path) {
            Ok(f) => Some(BufReader::new(f).lines()),
            // No record file is a fresh run, which is the common case and not
            // an error.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(RunError::ReadRecords {
                    path: path.display().to_string(),
                    source: e,
                })
            }
        };
        Ok(Self {
            lines,
            path: path.display().to_string(),
        })
    }

    /// The next finished id, or None once the file runs out.
    fn next_id(&mut self) -> Result<Option<String>, RunError> {
        let Some(lines) = self.lines.as_mut() else {
            return Ok(None);
        };
        for line in lines.by_ref() {
            let line = line.map_err(|e| RunError::ReadRecords {
                path: self.path.clone(),
                source: e,
            })?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Record>(&line) {
                Ok(record) if record.is_complete() => return Ok(Some(record.id)),
                // A placeholder written before the tool answered. Treating it
                // as done is the bug that made a re-run score nothing while
                // reporting success, so it is left to be scored again.
                Ok(_) => return Ok(None),
                // The last line of a killed run. Anything after it is
                // unreachable anyway, because records are written in item
                // order, so stopping here is the same answer as skipping and
                // costs no guesswork about which malformed line is which.
                Err(_) => return Ok(None),
            }
        }
        Ok(None)
    }
}

/// Score every item with `entry`, writing each record as it is produced.
///
/// `already` is the record file being appended to. Pass a path that does not
/// exist for a fresh run.
///
/// `progress` is called at most once per [`HEARTBEAT`] with the tally so far,
/// so a caller can print a line without this module deciding what a line looks
/// like.
///
/// `adapter_roots` are the trees a relative `invoke.adapter` is resolved
/// against, and are passed straight through to [`crate::selftest::read_one`]
/// so a scoring run asks the same question the self-test asked.
pub fn score<I, S, P>(
    entry: &Entry,
    items: I,
    already: &Path,
    sink: &mut S,
    timeout: Duration,
    adapter_roots: &[PathBuf],
    mut progress: P,
) -> Result<Tally, RunError>
where
    I: IntoIterator<Item = WorkItem>,
    S: Sink,
    P: FnMut(&Tally),
{
    let mut prior = Prior::open(already)?;
    // One id at a time, walked in step with the items. Holding the finished
    // ids in a collection would be a collection that grows with the corpus,
    // which is the thing Section 12 forbids and which this module's own
    // preamble promises not to do.
    let mut recorded = prior.next_id()?;
    let mut tally = Tally::default();
    let mut last_beat = Instant::now();

    for (position, item) in items.into_iter().enumerate() {
        // Resume, positionally. The ids must agree or the corpus moved under
        // the records, and scoring on regardless would file every answer
        // against the wrong image.
        if let Some(done) = recorded.as_deref() {
            if done != item.id {
                return Err(RunError::Diverged {
                    position: position as u64,
                    recorded: done.to_string(),
                    offered: item.id,
                });
            }
            tally.resumed += 1;
            recorded = prior.next_id()?;
            continue;
        }

        let record = crate::selftest::read_one(entry, &item.path, timeout, adapter_roots)
            .into_record(item.id.as_str());
        if record.error.is_some() {
            tally.errored += 1;
        }
        sink.write(&record).map_err(|e| RunError::WriteRecord {
            path: already.display().to_string(),
            source: e,
        })?;
        tally.scored += 1;

        if last_beat.elapsed() >= HEARTBEAT {
            progress(&tally);
            last_beat = Instant::now();
        }
    }
    Ok(tally)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn entry(toml: &str) -> Entry {
        toml::from_str(toml).expect("parses")
    }

    /// An entry that runs a shell script, so a test can decide what the tool
    /// says without installing one.
    fn scripted(script_path: &str) -> Entry {
        entry(&format!(
            "name = \"x\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [{script_path:?}]\n\
             [emits]\noutput = \"score\"\n\
             [invoke]\nargv = [\"{{file}}\"]\nparser = \"number\"\n\
             [selftest]\nmust_detect = \"fixtures/a.png\"\nmust_clear = \"fixtures/b.png\"\n"
        ))
    }

    fn tool(dir: &Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("tool.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("script");
        let mut perms = std::fs::metadata(&path).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
        path.display().to_string()
    }

    fn items(dir: &Path, n: usize) -> Vec<WorkItem> {
        (0..n)
            .map(|i| {
                let path = dir.join(format!("{i:05}.png"));
                std::fs::write(&path, b"\x89PNG\r\n\x1a\n").expect("image");
                WorkItem {
                    id: format!("{i:05}"),
                    path,
                }
            })
            .collect()
    }

    fn lines(buf: &[u8]) -> Vec<Record> {
        String::from_utf8_lossy(buf)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("a record"))
            .collect()
    }

    #[test]
    fn every_item_is_scored_once_and_recorded_in_order() {
        let dir = tempfile::tempdir().expect("tmp");
        let e = scripted(&tool(dir.path(), "echo 0.5"));
        let mut out = Cursor::new(Vec::new());
        let mut sink = JsonLines::new(&mut out);
        let tally = score(
            &e,
            items(dir.path(), 4),
            &dir.path().join("none.jsonl"),
            &mut sink,
            Duration::from_secs(10),
            &[],
            |_| {},
        )
        .expect("ran");
        assert_eq!(tally.scored, 4);
        assert_eq!(tally.resumed, 0);
        let written = lines(out.get_ref());
        assert_eq!(written.len(), 4);
        let ids: Vec<&str> = written.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["00000", "00001", "00002", "00003"]);
    }

    #[test]
    fn a_resumed_run_skips_what_is_already_recorded() {
        // The whole point of the record file. A 22 hour run that has to start
        // again from nothing after an interruption is one nobody restarts.
        let dir = tempfile::tempdir().expect("tmp");
        let e = scripted(&tool(dir.path(), "echo 0.5"));
        let record_path = dir.path().join("records.jsonl");
        std::fs::write(
            &record_path,
            "{\"id\":\"00000\",\"score\":0.5}\n{\"id\":\"00001\",\"score\":0.5}\n",
        )
        .expect("records");

        let mut out = Cursor::new(Vec::new());
        let mut sink = JsonLines::new(&mut out);
        let tally = score(
            &e,
            items(dir.path(), 4),
            &record_path,
            &mut sink,
            Duration::from_secs(10),
            &[],
            |_| {},
        )
        .expect("ran");
        assert_eq!(tally.resumed, 2);
        assert_eq!(tally.scored, 2);
        assert_eq!(tally.seen(), 4);
        let ids: Vec<String> = lines(out.get_ref()).into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["00002", "00003"]);
    }

    #[test]
    fn a_corpus_that_changed_under_the_records_is_refused() {
        // Without this the run scores one image and files the answer under
        // another, and every number downstream is wrong in a way nothing can
        // detect afterwards.
        let dir = tempfile::tempdir().expect("tmp");
        let e = scripted(&tool(dir.path(), "echo 0.5"));
        let record_path = dir.path().join("records.jsonl");
        std::fs::write(&record_path, "{\"id\":\"something-else\",\"score\":0.5}\n")
            .expect("records");

        let mut out = Cursor::new(Vec::new());
        let mut sink = JsonLines::new(&mut out);
        let err = score(
            &e,
            items(dir.path(), 2),
            &record_path,
            &mut sink,
            Duration::from_secs(10),
            &[],
            |_| {},
        )
        .expect_err("it should refuse");
        let message = err.to_string();
        assert!(message.contains("something-else"), "got {message}");
        assert!(message.contains("00000"), "got {message}");
    }

    #[test]
    fn an_incomplete_record_is_scored_again_rather_than_counted_as_done() {
        // A record with neither an answer nor an error is a placeholder the
        // run wrote before the tool spoke. Treating it as finished is the bug
        // that made a re-run score nothing and report success.
        let dir = tempfile::tempdir().expect("tmp");
        let e = scripted(&tool(dir.path(), "echo 0.5"));
        let record_path = dir.path().join("records.jsonl");
        std::fs::write(&record_path, "{\"id\":\"00000\"}\n").expect("records");

        let mut out = Cursor::new(Vec::new());
        let mut sink = JsonLines::new(&mut out);
        let tally = score(
            &e,
            items(dir.path(), 1),
            &record_path,
            &mut sink,
            Duration::from_secs(10),
            &[],
            |_| {},
        )
        .expect("ran");
        assert_eq!(tally.resumed, 0);
        assert_eq!(tally.scored, 1);
    }

    #[test]
    fn a_partial_last_line_does_not_stop_a_resume() {
        // A run killed mid-write leaves one. It is the ordinary case, and
        // refusing to resume over it would punish exactly the interruption
        // the record file exists to survive.
        let dir = tempfile::tempdir().expect("tmp");
        let e = scripted(&tool(dir.path(), "echo 0.5"));
        let record_path = dir.path().join("records.jsonl");
        std::fs::write(
            &record_path,
            "{\"id\":\"00000\",\"score\":0.5}\n{\"id\":\"0000",
        )
        .expect("records");

        let mut out = Cursor::new(Vec::new());
        let mut sink = JsonLines::new(&mut out);
        let tally = score(
            &e,
            items(dir.path(), 2),
            &record_path,
            &mut sink,
            Duration::from_secs(10),
            &[],
            |_| {},
        )
        .expect("ran");
        assert_eq!(tally.resumed, 1);
        assert_eq!(tally.scored, 1);
    }

    #[test]
    fn a_tool_that_fails_on_one_item_is_counted_rather_than_stopping_the_run() {
        // Graceful degradation, Section 2.1. One unreadable image must not
        // throw away the other 344,356, and the count is what makes `n_error`
        // an assertion rather than an absence.
        let dir = tempfile::tempdir().expect("tmp");
        let e = scripted(&tool(
            dir.path(),
            "case \"$1\" in *00001.png) exit 1 ;; *) echo 0.5 ;; esac",
        ));
        let mut out = Cursor::new(Vec::new());
        let mut sink = JsonLines::new(&mut out);
        let tally = score(
            &e,
            items(dir.path(), 3),
            &dir.path().join("none.jsonl"),
            &mut sink,
            Duration::from_secs(10),
            &[],
            |_| {},
        )
        .expect("ran");
        assert_eq!(tally.scored, 3);
        assert_eq!(tally.errored, 1);
        assert_eq!(lines(out.get_ref()).len(), 3);
    }

    #[test]
    fn a_stuck_item_is_recorded_as_an_error_rather_than_holding_the_run() {
        // One item out of a Core tier's hundreds of thousands that never
        // answers must not be indistinguishable from a slow run.
        let dir = tempfile::tempdir().expect("tmp");
        let e = scripted(&tool(dir.path(), "sleep 30"));
        let mut out = Cursor::new(Vec::new());
        let mut sink = JsonLines::new(&mut out);
        let started = Instant::now();
        let tally = score(
            &e,
            items(dir.path(), 1),
            &dir.path().join("none.jsonl"),
            &mut sink,
            Duration::from_millis(200),
            &[],
            |_| {},
        )
        .expect("ran");
        assert!(started.elapsed() < Duration::from_secs(10), "it waited");
        assert_eq!(tally.errored, 1);
        let written = lines(out.get_ref());
        assert!(
            written[0]
                .error
                .as_deref()
                .unwrap_or("")
                .contains("no answer"),
            "got {:?}",
            written[0].error
        );
    }

    #[test]
    fn a_record_is_on_disk_before_the_next_item_starts() {
        // Flushing per line is what makes a resume worth having. If the run
        // is killed, the work already done has to be already saved.
        let dir = tempfile::tempdir().expect("tmp");
        let e = scripted(&tool(dir.path(), "echo 0.5"));
        let record_path = dir.path().join("records.jsonl");
        let file = std::fs::File::create(&record_path).expect("create");
        let mut sink = JsonLines::new(file);
        score(
            &e,
            items(dir.path(), 3),
            &dir.path().join("none.jsonl"),
            &mut sink,
            Duration::from_secs(10),
            &[],
            |_| {},
        )
        .expect("ran");
        let on_disk = std::fs::read(&record_path).expect("read back");
        assert_eq!(lines(&on_disk).len(), 3);
    }
}
