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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tally {
    /// Items this run scored, not counting ones resumed from a previous run.
    pub scored: u64,
    /// Items already recorded when the run started.
    pub resumed: u64,
    /// Items the tool could not answer about. Counted rather than inferred,
    /// because `n_error` in a result is required precisely so that zero is an
    /// assertion rather than an absence.
    pub errored: u64,
    /// Why the tool's own output stopped being kept, if it did.
    ///
    /// A sidecar that cannot be written does not fail the run, because the
    /// measurement is in the records file and is still good. It travels back
    /// here so the caller can say so once, rather than leaving somebody to
    /// find a short sidecar later and guess whether the tool was quiet or the
    /// disk was full.
    pub raw_problem: Option<String>,
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

/// The sidecar holding what tools printed, opened only if anything asked for
/// one.
///
/// Appended and flushed per line for the same reason the records file is: a
/// run that is killed should still explain the items it got through. Separate
/// from the records file rather than a field on `Record`, so the format other
/// things already read does not change shape, and so a reader who does not
/// care never pays to parse it.
struct RawSink {
    /// Taken from the policy and kept until the first line needs writing.
    ///
    /// Opened lazily because most runs keep nothing: every item answered and
    /// nobody asked for more. Creating the file up front left an empty
    /// sidecar beside every records file, which is a question for whoever
    /// finds it later and an answer to nothing.
    path: Option<PathBuf>,
    file: Option<std::fs::File>,
    every: bool,
}

impl RawSink {
    fn open(policy: RawPolicy<'_>) -> Self {
        Self {
            path: policy.path.map(Path::to_path_buf),
            file: None,
            every: policy.every,
        }
    }

    /// Keep this item's output, if the policy wants it.
    ///
    /// An item the harness could not read an answer from is kept whatever the
    /// policy, because that is the one a reader cannot diagnose without it.
    /// Returns a problem to report the FIRST time writing fails, and closes
    /// itself so a full disk does not produce one warning per image.
    ///
    /// A sidecar that cannot be written must not take the run down with it:
    /// the measurement is in the records file and is still good. It must not
    /// fail silently either, or a reader finds a short sidecar later and has
    /// no idea whether the tool was quiet or the disk was full.
    #[must_use]
    fn keep(&mut self, id: &str, record: &Record, raw: &crate::selftest::Raw) -> Option<String> {
        if raw.is_empty() || (!self.every && record.error.is_none()) {
            return None;
        }
        if self.file.is_none() {
            let path = self.path.clone()?;
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                Ok(f) => self.file = Some(f),
                Err(e) => {
                    self.path = None;
                    return Some(format!(
                        "could not open {} to keep what the tool printed: {e}. \
                         The measurement is unaffected",
                        path.display()
                    ));
                }
            }
        }
        let file = self.file.as_mut()?;
        let line = serde_json::json!({
            "id": id,
            "stdout": raw.stdout,
            "stderr": raw.stderr,
        });
        match writeln!(file, "{line}").and_then(|()| file.flush()) {
            Ok(()) => None,
            Err(e) => {
                self.file = None;
                Some(format!(
                    "could not keep what the tool printed: {e}. The                      measurement is unaffected and nothing further will be                      kept for this run"
                ))
            }
        }
    }
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
/// Everything about a scoring run except the work itself.
///
/// Grouped because they travel together and are decided together, before a
/// single image is read: which subject, against which records file, under
/// what deadline, how many at a time.
pub struct Run<'a> {
    pub entry: &'a Entry,
    /// The record file being appended to. A path that does not exist is a
    /// fresh run.
    pub already: &'a Path,
    /// What one image is given before the detector is killed.
    pub timeout: Duration,
    /// How many images to score at once. 1 is one at a time.
    pub jobs: usize,
    /// The trees a relative `invoke.adapter` is resolved against.
    pub adapter_roots: &'a [PathBuf],
    /// Where to keep what the tool printed, and how much of it.
    pub raw: RawPolicy<'a>,
}

/// What to do with a tool's own output.
///
/// The default keeps the output of items the harness could not read an answer
/// from, which is close to free because it is rare, and is exactly the case
/// where a reader cannot otherwise tell a detector that found nothing from a
/// harness that misread it. `every` keeps all of it, which is what somebody
/// debugging an adapter wants and what nobody wants by default.
#[derive(Debug, Default, Clone, Copy)]
pub struct RawPolicy<'a> {
    /// Where the sidecar goes. `None` keeps nothing whatever `every` says.
    pub path: Option<&'a Path>,
    /// Keep the output of items that answered, not only the ones that did not.
    pub every: bool,
}

pub fn score<I, S, P>(
    run: Run<'_>,
    items: I,
    sink: &mut S,
    mut progress: P,
) -> Result<Tally, RunError>
where
    I: IntoIterator<Item = WorkItem>,
    S: Sink,
    P: FnMut(&Tally),
{
    let Run {
        entry,
        already,
        timeout,
        jobs,
        adapter_roots,
        raw: raw_policy,
    } = run;
    let mut raw_sink = RawSink::open(raw_policy);
    let mut prior = Prior::open(already)?;
    // One id at a time, walked in step with the items. Holding the finished
    // ids in a collection would be a collection that grows with the corpus,
    // which is the thing Section 12 forbids and which this module's own
    // preamble promises not to do.
    let mut recorded = prior.next_id()?;
    let mut tally = Tally::default();
    let mut last_beat = Instant::now();

    let mut it = items.into_iter().enumerate();

    // RESUME IS ALWAYS A PREFIX, WHICH IS WHAT MAKES THE PARALLEL PHASE SAFE
    //
    // Records are appended in corpus order, so what a previous run finished is
    // a run of items from the start and never a scattering through the middle.
    // Draining that prefix sequentially leaves a tail that is entirely
    // unscored, and items in that tail have no ordering constraint between
    // them: they only have to be WRITTEN in order. So the resume check keeps
    // its exact previous behaviour and the concurrency is confined to work
    // that no earlier run touched.
    while recorded.is_some() {
        let Some((position, item)) = it.next() else {
            break;
        };
        let done = recorded.as_deref().unwrap_or_default();
        if done != item.id {
            return Err(RunError::Diverged {
                position: position as u64,
                recorded: done.to_string(),
                offered: item.id,
            });
        }
        tally.resumed += 1;
        recorded = prior.next_id()?;
    }

    let mut finish = |record: Record, tally: &mut Tally| -> Result<(), RunError> {
        if record.error.is_some() {
            tally.errored += 1;
        }
        sink.write(&record).map_err(|e| RunError::WriteRecord {
            path: already.display().to_string(),
            source: e,
        })?;
        tally.scored += 1;
        Ok(())
    };

    if jobs <= 1 {
        for (_, item) in it {
            let mut raw = crate::selftest::Raw::default();
            let record = crate::selftest::read_one_observed(
                entry,
                &item.path,
                timeout,
                adapter_roots,
                &mut raw,
            )
            .into_record(item.id.as_str());
            if let Some(problem) = raw_sink.keep(&item.id, &record, &raw) {
                tally.raw_problem.get_or_insert(problem);
            }
            finish(record, &mut tally)?;
            if last_beat.elapsed() >= HEARTBEAT {
                progress(&tally);
                last_beat = Instant::now();
            }
        }
        return Ok(tally);
    }

    // A WINDOW OF `jobs`, NOT A QUEUE OF EVERY ITEM
    //
    // The whole corpus is 344,357 items at Core, so nothing here may hold a
    // collection that grows with it. One chunk of at most `jobs` items is in
    // memory at a time, and the records come back in the order they were
    // handed out, so the file this appends to is byte for byte the file a
    // serial run would have written. That is what keeps resume working after
    // an interrupted parallel run.
    //
    // Chunked rather than a continuously fed pool: a slow item stalls its own
    // chunk, which costs a little throughput, and in exchange the ordering is
    // a property of the structure instead of something a reassembly buffer
    // has to be trusted to get right.
    let mut chunk: Vec<WorkItem> = Vec::with_capacity(jobs);
    loop {
        chunk.clear();
        chunk.extend(it.by_ref().take(jobs).map(|(_, item)| item));
        if chunk.is_empty() {
            break;
        }

        let scored: Vec<(Record, crate::selftest::Raw)> = std::thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|item| {
                    scope.spawn(move || {
                        let mut raw = crate::selftest::Raw::default();
                        let record = crate::selftest::read_one_observed(
                            entry,
                            &item.path,
                            timeout,
                            adapter_roots,
                            &mut raw,
                        )
                        .into_record(item.id.as_str());
                        (record, raw)
                    })
                })
                .collect();
            handles
                .into_iter()
                .zip(chunk.iter())
                .map(|(handle, item)| {
                    // A panicking worker is recorded as an error against its
                    // own item rather than taken as the end of the run. The
                    // alternative loses every answer in the chunk, including
                    // the ones that were fine, and leaves a records file the
                    // next run cannot resume from.
                    handle.join().unwrap_or_else(|_| {
                        (
                            Record {
                                id: item.id.clone(),
                                score: None,
                                verdict: None,
                                error: Some(
                                    "the worker scoring this item panicked; the run \
                                     continued and this item was not measured"
                                        .into(),
                                ),
                                elapsed_ms: None,
                            },
                            crate::selftest::Raw::default(),
                        )
                    })
                })
                .collect()
        });

        for ((record, raw), item) in scored.into_iter().zip(chunk.iter()) {
            if let Some(problem) = raw_sink.keep(&item.id, &record, &raw) {
                tally.raw_problem.get_or_insert(problem);
            }
            finish(record, &mut tally)?;
        }
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

    /// A parallel run must produce the file a serial run would have.
    ///
    /// Resume reads the records positionally, so an out of order write does
    /// not merely look untidy: the next run compares recorded ids against
    /// corpus ids, finds them disagreeing, and refuses the whole corpus as
    /// having changed. Byte equality is therefore the real contract, not an
    /// aesthetic one.
    #[test]
    fn a_parallel_run_writes_what_a_serial_run_would_have() {
        let dir = tempfile::tempdir().expect("tmp");
        // Sleeps in reverse order of id, so the workers finish back to front
        // and any reliance on completion order shows up rather than passing
        // by luck on a fast machine.
        let e = scripted(&tool(
            dir.path(),
            "n=$(basename \"$1\" .png); sleep 0.$((9 - 10#$n % 10)); echo 0.5",
        ));

        let mut serial_out = Cursor::new(Vec::new());
        let mut sink = JsonLines::new(&mut serial_out);
        let serial = score(
            Run {
                entry: &e,
                already: &dir.path().join("none.jsonl"),
                timeout: Duration::from_secs(30),
                jobs: 1,
                adapter_roots: &[],
                raw: Default::default(),
            },
            items(dir.path(), 8),
            &mut sink,
            |_| {},
        )
        .expect("the serial run");

        let mut parallel_out = Cursor::new(Vec::new());
        let mut sink = JsonLines::new(&mut parallel_out);
        let parallel = score(
            Run {
                entry: &e,
                already: &dir.path().join("none.jsonl"),
                timeout: Duration::from_secs(30),
                jobs: 4,
                adapter_roots: &[],
                raw: Default::default(),
            },
            items(dir.path(), 8),
            &mut sink,
            |_| {},
        )
        .expect("the parallel run");

        assert_eq!(serial_out.get_ref(), parallel_out.get_ref());
        assert_eq!(parallel.scored, serial.scored);
        assert_eq!(parallel.errored, serial.errored);
    }

    /// Interrupting a parallel run and finishing it must give the same file
    /// as never having been interrupted.
    #[test]
    fn a_parallel_run_resumes_from_what_a_parallel_run_left() {
        let dir = tempfile::tempdir().expect("tmp");
        let e = scripted(&tool(dir.path(), "echo 0.5"));

        let whole = {
            let mut out = Cursor::new(Vec::new());
            let mut sink = JsonLines::new(&mut out);
            score(
                Run {
                    entry: &e,
                    already: &dir.path().join("none.jsonl"),
                    timeout: Duration::from_secs(30),
                    jobs: 4,
                    adapter_roots: &[],
                    raw: Default::default(),
                },
                items(dir.path(), 9),
                &mut sink,
                |_| {},
            )
            .expect("the uninterrupted run");
            out.into_inner()
        };

        // Stand in for an interruption: the first five items are already on
        // disk, written by an earlier parallel run.
        let part = dir.path().join("part.jsonl");
        let five: Vec<u8> = whole
            .split_inclusive(|b| *b == b'\n')
            .take(5)
            .flatten()
            .copied()
            .collect();
        std::fs::write(&part, &five).expect("the partial records");

        let mut rest = Cursor::new(Vec::new());
        let mut sink = JsonLines::new(&mut rest);
        let tally = score(
            Run {
                entry: &e,
                already: &part,
                timeout: Duration::from_secs(30),
                jobs: 4,
                adapter_roots: &[],
                raw: Default::default(),
            },
            items(dir.path(), 9),
            &mut sink,
            |_| {},
        )
        .expect("the resumed run");

        assert_eq!(tally.resumed, 5, "the prefix already on disk");
        assert_eq!(tally.scored, 4, "only the tail is rescored");
        let mut joined = five;
        joined.extend_from_slice(rest.get_ref());
        assert_eq!(joined, whole, "resuming produced a different file");
    }

    #[test]
    fn every_item_is_scored_once_and_recorded_in_order() {
        let dir = tempfile::tempdir().expect("tmp");
        let e = scripted(&tool(dir.path(), "echo 0.5"));
        let mut out = Cursor::new(Vec::new());
        let mut sink = JsonLines::new(&mut out);
        let tally = score(
            Run {
                entry: &e,
                already: &dir.path().join("none.jsonl"),
                timeout: Duration::from_secs(10),
                jobs: 1,
                adapter_roots: &[],
                raw: Default::default(),
            },
            items(dir.path(), 4),
            &mut sink,
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
            Run {
                entry: &e,
                already: &record_path,
                timeout: Duration::from_secs(10),
                jobs: 1,
                adapter_roots: &[],
                raw: Default::default(),
            },
            items(dir.path(), 4),
            &mut sink,
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
            Run {
                entry: &e,
                already: &record_path,
                timeout: Duration::from_secs(10),
                jobs: 1,
                adapter_roots: &[],
                raw: Default::default(),
            },
            items(dir.path(), 2),
            &mut sink,
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
            Run {
                entry: &e,
                already: &record_path,
                timeout: Duration::from_secs(10),
                jobs: 1,
                adapter_roots: &[],
                raw: Default::default(),
            },
            items(dir.path(), 1),
            &mut sink,
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
            Run {
                entry: &e,
                already: &record_path,
                timeout: Duration::from_secs(10),
                jobs: 1,
                adapter_roots: &[],
                raw: Default::default(),
            },
            items(dir.path(), 2),
            &mut sink,
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
            Run {
                entry: &e,
                already: &dir.path().join("none.jsonl"),
                timeout: Duration::from_secs(10),
                jobs: 1,
                adapter_roots: &[],
                raw: Default::default(),
            },
            items(dir.path(), 3),
            &mut sink,
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
            Run {
                entry: &e,
                already: &dir.path().join("none.jsonl"),
                timeout: Duration::from_millis(200),
                jobs: 1,
                adapter_roots: &[],
                raw: Default::default(),
            },
            items(dir.path(), 1),
            &mut sink,
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
            Run {
                entry: &e,
                already: &dir.path().join("none.jsonl"),
                timeout: Duration::from_secs(10),
                jobs: 1,
                adapter_roots: &[],
                raw: Default::default(),
            },
            items(dir.path(), 3),
            &mut sink,
            |_| {},
        )
        .expect("ran");
        let on_disk = std::fs::read(&record_path).expect("read back");
        assert_eq!(lines(&on_disk).len(), 3);
    }
}
