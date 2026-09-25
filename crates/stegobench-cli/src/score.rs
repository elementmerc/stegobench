// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo

//! `stegobench score`: the headline verb, joined up.
//!
//! Three pieces that already existed do the work. The corpus reader streams
//! samples off disk, the runner asks the detector about each one and records
//! the answer as it goes, and the metrics crate turns scores and labels into
//! numbers. This module is the seam between them, and the place the result
//! document is assembled.
//!
//! WHERE THE MEMORY GOES, STATED RATHER THAN IMPLIED
//!
//! The scoring pass holds one item at a time, so a corpus of any size costs
//! the same. The metrics pass does not and cannot: a ROC AUC is defined over
//! the whole set and needs every score before it can rank them. That is one
//! f64 and one bool per item, so a Core tier is a few megabytes, and it is
//! bounded by the corpus rather than by anything this code chooses.
//!
//! It is worth naming because it is the one place a bigger corpus eventually
//! bites, and a reader of this file should meet that fact here rather than
//! discover it at ten times the scale.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use stegobench_core::header::{self, Shape};
use stegobench_core::registry::{Entry, Kind};
use stegobench_core::result::{
    Arm, Configuration, CorpusRef, CorpusSource, Declarations, Determinism, Domain, Metrics,
    Pairing, PluginRef, Provenance, Result1, SplitDiscipline, Subject, SubjectKind,
    RESULT_SCHEMA_ID,
};
use stegobench_core::samples::{Role, Samples};
use stegobench_plugin::runner::{self, JsonLines, Tally};
use stegobench_plugin::{Record, WorkItem};

/// What the caller asked for, gathered so the signature stays readable.
pub struct Request<'a> {
    pub corpus: &'a Path,
    pub records: PathBuf,
    pub timeout: Duration,
    pub limit: Option<u64>,
}

/// Why a run could not produce a result.
#[derive(Debug, thiserror::Error)]
pub enum ScoreError {
    #[error("{0}")]
    Corpus(#[from] stegobench_core::samples::SampleError),
    #[error("{0}")]
    Run(#[from] runner::RunError),
    #[error("could not open the records file at {path}: {source}")]
    Records {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "{name} is registered as an embedder, and an embedder cannot be asked \
         to tell two images apart. Pick a detector: `stegobench list detectors`"
    )]
    NotADetector { name: String },
    #[error(
        "the corpus at {path} puts {count} stego image(s) on a different side \
         of the train and test split from the cover they were made from, for \
         example: {examples}. A cover and its stego twin on opposite sides \
         inflates every number computed from this corpus, and the inflation is \
         invisible in the output, so this refuses rather than reporting a \
         confident wrong answer"
    )]
    SplitLeaks {
        path: String,
        count: usize,
        examples: String,
    },
    #[error(
        "the corpus at {path} holds {clean} clean and {stego} stego image(s), \
         and a measurement needs both. A detector scored on one side of the \
         question has not been measured, it has been asked a leading one"
    )]
    OneSided {
        path: String,
        clean: u64,
        stego: u64,
    },
}

/// Run the detector over the corpus and build the result document.
pub fn score<P>(
    entry: &Entry,
    request: &Request,
    mut progress: P,
) -> Result<(Result1, Tally), ScoreError>
where
    P: FnMut(&str),
{
    if entry.kind == Kind::Embedder {
        return Err(ScoreError::NotADetector {
            name: entry.name.clone(),
        });
    }

    let started = Instant::now();
    let started_utc = now_utc();

    // The scoring pass. Streams, and every answer is on disk before the next
    // item begins, so an interrupted run resumes rather than restarts.
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&request.records)
        .map_err(|e| ScoreError::Records {
            path: request.records.display().to_string(),
            source: e,
        })?;
    let mut sink = JsonLines::new(file);

    // Streamed, not collected. Building a Vec of 344,357 items here would put
    // the corpus back in memory one layer above the runner that was written
    // specifically not to hold it.
    let mut feed = Feed::open(request)?;
    let tally = runner::score(
        entry,
        &mut feed,
        &request.records,
        &mut sink,
        request.timeout,
        |t| progress(&format!("{} scored, {} errored", t.scored, t.errored)),
    )?;
    // A corpus defect part way through is a failure, not a short run. Checked
    // after the loop because the iterator cannot return one.
    if let Some(e) = feed.fault {
        return Err(ScoreError::Corpus(e));
    }

    // The metrics pass. Labels come from the corpus and scores from the
    // records, joined by position, because both are produced in the same
    // deterministic order and the runner refuses to continue when they
    // disagree.
    let (scores, labels, errored) = join(request)?;
    let n_stego = labels.iter().filter(|l| **l).count() as u64;
    let n_clean = labels.len() as u64 - n_stego;
    if n_clean == 0 || n_stego == 0 {
        return Err(ScoreError::OneSided {
            path: request.corpus.display().to_string(),
            clean: n_clean,
            stego: n_stego,
        });
    }

    // Checked before a number is computed from the corpus, because a corpus
    // that leaks a cover across the boundary produces a confident wrong
    // answer, and producing it first and mentioning the problem afterwards is
    // how a bad number gets quoted.
    let checks = check(request)?;
    if !checks.split_leaks.is_empty() {
        return Err(ScoreError::SplitLeaks {
            path: request.corpus.display().to_string(),
            count: checks.split_leaks.count,
            examples: checks.split_leaks.examples(),
        });
    }

    // A confounded corpus is reported rather than refused, which is the
    // opposite of what a split leak gets, and the difference is deliberate.
    // A leak makes a number wrong while looking right. A second variable
    // between the clean and stego halves is a real property of some arms, kept
    // on purpose to demonstrate what it does, so the run happens and the
    // document says what it measured.
    match checks.pairing {
        Pairing::Confounded => progress(&format!(
            "WARNING: {} stego image(s) differ from their cover in more than \
             the payload, for example: {}. This run measures that difference \
             as well as the payload, and the result says so",
            checks.pairing_breaks.count,
            checks.pairing_breaks.examples()
        )),
        Pairing::Unverified => progress(
            "the pairing rule could not be checked: no stego image here names \
             a cover this could read alongside it. The result says unverified \
             rather than claiming the rule held",
        ),
        Pairing::SingleVariable => {
            if checks.unreadable > 0 {
                progress(&format!(
                    "{} of {} stego image(s) could not be compared with their \
                     cover, so the pairing check covered the rest",
                    checks.unreadable,
                    checks.unreadable + checks.compared
                ));
            }
        }
    }

    let auc = stegobench_metrics::roc_auc(&scores, &labels).unwrap_or(0.5);
    let mut tpr_at_fpr = BTreeMap::new();
    for fpr in [0.01, 0.05, 0.10] {
        if let Some(tpr) = stegobench_metrics::tpr_at_fpr(&scores, &labels, fpr) {
            tpr_at_fpr.insert(format!("{fpr:.2}"), tpr);
        }
    }

    let result = Result1 {
        schema: RESULT_SCHEMA_ID.to_string(),
        subject: Subject {
            name: entry.name.clone(),
            version: subject_version(entry),
            kind: SubjectKind::Detector,
        },
        corpus: CorpusRef {
            name: corpus_name(request.corpus),
            tier: None,
            // The user pointed at a directory. Nothing here downloaded it, and
            // saying otherwise would be the harness vouching for bytes it
            // never saw arrive.
            source: CorpusSource::Supplied,
            digest: String::new(),
            pairs: labels.len() as u64,
            split: None,
        },
        arm: Arm {
            // A directory of samples does not say which scheme made it. The
            // honest answer is the one the corpus gave, and it gave none.
            embedder: "unknown".into(),
            rate: None,
            domain: Domain::Spatial,
            format: "png".into(),
        },
        metrics: Metrics {
            auc,
            auc_ci95: None,
            tpr_at_fpr,
            verdict_rate: None,
            n_clean,
            n_stego,
            n_error: errored,
        },
        provenance: Provenance {
            seed: None,
            plugins: vec![PluginRef {
                name: entry.name.clone(),
                image: subject_version(entry),
                determinism: Determinism::Nondeterministic,
            }],
            harness_version: env!("CARGO_PKG_VERSION").to_string(),
            started_utc,
            elapsed_seconds: started.elapsed().as_secs_f64(),
            // Containers are run with --network=none; a binary entry is a
            // program the operator installed and this cannot speak for it.
            network_reachable: entry.binary.is_some(),
            host: None,
        },
        declarations: Declarations {
            split_discipline: checks.split,
            pairing: checks.pairing,
            // Always custom, and this is the honest answer rather than a
            // placeholder. A directory of samples is not a registered tier: it
            // carries no digest anybody can check and no arm anybody can name,
            // so it is comparable with itself and nothing else. Scoring a
            // named tier is a separate route and will say so.
            configuration: Configuration::Custom,
            trained_on: None,
            self_reported: false,
        },
    };
    Ok((result, tally))
}

/// The items to score, one at a time, stopping at `limit` where one is set.
///
/// An iterator cannot return an error, and a corpus defect must not read as
/// the end of the corpus: that is the difference between "scored everything"
/// and "scored as far as the broken file", and they have the same shape from
/// the outside. So a fault stops the feed and is picked up by the caller.
struct Feed {
    samples: Samples,
    limit: Option<u64>,
    taken: u64,
    fault: Option<stegobench_core::samples::SampleError>,
}

impl Feed {
    fn open(request: &Request) -> Result<Self, ScoreError> {
        Ok(Self {
            samples: Samples::open(request.corpus)?,
            limit: request.limit,
            taken: 0,
            fault: None,
        })
    }
}

impl Iterator for Feed {
    type Item = WorkItem;

    fn next(&mut self) -> Option<WorkItem> {
        if self.limit.is_some_and(|n| self.taken >= n) {
            return None;
        }
        match self.samples.next()? {
            Ok(sample) => {
                self.taken += 1;
                Some(WorkItem {
                    id: sample.id,
                    path: sample.image,
                })
            }
            Err(e) => {
                self.fault = Some(e);
                None
            }
        }
    }
}

/// How many example violations are kept to show the user.
///
/// The count is exact and the examples are capped, which is the only shape
/// that works: a corpus built by a broken script fails on every one of its
/// 344,357 rows, and a list of them would put the whole corpus in memory to
/// print three lines of it.
const MAX_EXAMPLES: usize = 3;

/// A tally of one kind of corpus defect, with a few examples kept to show.
#[derive(Default)]
struct Violations {
    count: usize,
    examples: Vec<String>,
}

impl Violations {
    fn note(&mut self, what: String) {
        self.count += 1;
        if self.examples.len() < MAX_EXAMPLES {
            self.examples.push(what);
        }
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn examples(&self) -> String {
        self.examples.join("; ")
    }
}

/// What the corpus turned out to be, as opposed to what it claims.
struct Checks {
    split: SplitDiscipline,
    split_leaks: Violations,
    pairing: Pairing,
    pairing_breaks: Violations,
    /// Stego images whose cover could be found and compared.
    compared: u64,
    /// Stego images naming a cover that could not be compared, because one of
    /// the two images could not be read or is in a format this does not
    /// measure.
    unreadable: u64,
}

/// What a cover contributes to both checks, gathered in one pass.
struct Cover {
    split: Option<String>,
    shape: Option<Shape>,
}

/// Checks the two reliability claims the result document carries.
///
/// SPLIT DISCIPLINE, WHICH FILES ON DISK CAN PROVE
///
/// A split label travels with a cover, and a stego image names the cover it was
/// made from, so a stego image carrying a DIFFERENT split from its own cover is
/// a corpus that will leak a photograph across the train and test boundary.
/// Violating it inflates every number computed from the corpus and is invisible
/// in the output, which is exactly why the harness must not take it on trust.
///
/// PAIRING, WHERE ONLY THE NEGATIVE IS PROVABLE
///
/// The rule is that a clean image and its stego twin differ in nothing but the
/// payload. Nothing short of decoding both images could prove that, and even
/// that would miss a cover re-encoded before the payload went in. The reachable
/// half is the refutation: if the two differ in format, width, height, bit depth
/// or channel count, then something other than the payload changed, and the
/// number about to be computed is measuring that too.
///
/// So this returns `SingleVariable` where every comparable pair matched,
/// `Confounded` where any did not, and `Unverified` where nothing could be
/// compared. The third answer is the point. A benchmark that says "single
/// variable" having looked at nothing is making the unchecked claim this
/// project exists to stop repeating.
///
/// WHAT THIS HOLDS IN MEMORY, AND WHY THAT IS BOUNDED
///
/// One entry per COVER, not one per sample, and the stego rows are streamed
/// against it rather than gathered. A Core tier is 10,000 covers against
/// 344,357 samples, so this is an order of magnitude below the corpus and
/// bounded by a number the corpus states rather than by anything this code
/// chooses. It costs a second walk of the directory tree, because a cover can
/// sort after the arm that used it and the map has to be complete before any
/// row is judged against it.
fn check(request: &Request) -> Result<Checks, ScoreError> {
    let mut covers: HashMap<String, Cover> = HashMap::new();
    let mut any_split = false;

    for sample in Samples::open(request.corpus)? {
        let sample = sample?;
        if sample.role != Role::Clean {
            continue;
        }
        any_split |= sample.split.is_some();
        let Some(name) = sample.image.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        covers.insert(
            name.to_string(),
            Cover {
                split: sample.split,
                // An unreadable cover is not a corpus defect this should refuse
                // over. The sample reader has already accepted the file, the
                // detector will be asked about it regardless, and the honest
                // consequence is that this pair cannot be compared rather than
                // that the run cannot happen.
                shape: header::read(&sample.image).ok(),
            },
        );
    }

    let mut checks = Checks {
        split: SplitDiscipline::NotApplicable,
        split_leaks: Violations::default(),
        pairing: Pairing::Unverified,
        pairing_breaks: Violations::default(),
        compared: 0,
        unreadable: 0,
    };

    for sample in Samples::open(request.corpus)? {
        let sample = sample?;
        if sample.role != Role::Stego {
            continue;
        }
        let Some(cover_name) = sample.cover else {
            continue;
        };
        let Some(cover) = covers.get(&cover_name) else {
            continue;
        };

        // A stego row with no split of its own inherits its cover's, which is
        // the corpus doing the right thing by construction and cannot be a
        // violation. Only a row that states one can contradict.
        if let (Some(split), Some(cover_split)) = (&sample.split, &cover.split) {
            any_split = true;
            if split != cover_split {
                checks.split_leaks.note(format!(
                    "{} is in the {split} split while its cover {cover_name} is \
                     in the {cover_split} split",
                    sample.id
                ));
            }
        }

        match (cover.shape, header::read(&sample.image).ok()) {
            (Some(cover_shape), Some(stego_shape)) => match difference(cover_shape, stego_shape) {
                None => checks.compared += 1,
                Some(what) => {
                    checks.compared += 1;
                    checks
                        .pairing_breaks
                        .note(format!("{} and its cover {cover_name} {what}", sample.id));
                }
            },
            _ => checks.unreadable += 1,
        }
    }

    if any_split {
        checks.split = SplitDiscipline::ByCover;
    }
    checks.pairing = if !checks.pairing_breaks.is_empty() {
        Pairing::Confounded
    } else if checks.compared > 0 {
        Pairing::SingleVariable
    } else {
        Pairing::Unverified
    };
    Ok(checks)
}

/// The first way two images differ other than in their payload, in words.
///
/// Ordered so the reader is told the most fundamental difference rather than
/// the first one an arbitrary field order happens to reach: a PNG against a
/// JPEG is a different fact from a PNG one pixel wider than another.
fn difference(cover: Shape, stego: Shape) -> Option<String> {
    if cover.format != stego.format {
        return Some(format!(
            "are different formats: {} against {}",
            cover.format.name(),
            stego.format.name()
        ));
    }
    let (Some(c), Some(s)) = (cover.geometry, stego.geometry) else {
        return None;
    };
    if (c.width, c.height) != (s.width, s.height) {
        return Some(format!(
            "are different sizes: {} by {} against {} by {}",
            c.width, c.height, s.width, s.height
        ));
    }
    if c.depth != s.depth {
        return Some(format!(
            "have different bit depths: {} against {}",
            c.depth, s.depth
        ));
    }
    if c.channels != s.channels {
        return Some(format!(
            "have different channel counts: {} against {}",
            c.channels, s.channels
        ));
    }
    None
}

/// Scores and labels, joined by position.
fn join(request: &Request) -> Result<(Vec<f64>, Vec<bool>, u64), ScoreError> {
    let mut labels = Vec::new();
    for sample in Samples::open(request.corpus)? {
        let sample = sample?;
        labels.push(sample.role == Role::Stego);
        if request.limit.is_some_and(|n| labels.len() as u64 >= n) {
            break;
        }
    }

    let file = std::fs::File::open(&request.records).map_err(|e| ScoreError::Records {
        path: request.records.display().to_string(),
        source: e,
    })?;
    let mut scores = Vec::new();
    let mut errored = 0;
    let mut kept = Vec::new();
    for (line, label) in BufReader::new(file).lines().zip(labels.iter()) {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Record>(&line) else {
            break;
        };
        match record
            .score
            .or(record.verdict.map(|v| if v { 1.0 } else { 0.0 }))
        {
            Some(s) => {
                scores.push(s);
                kept.push(*label);
            }
            // An item the tool could not answer about is counted, never
            // guessed at. A metric computed over the survivors of a partly
            // failed run is how an evaluation misleads without anyone
            // intending it, which is why n_error is required.
            None => errored += 1,
        }
    }
    Ok((scores, kept, errored))
}

/// How the subject identifies itself: an image digest, or the binary's hash.
fn subject_version(entry: &Entry) -> String {
    if let Some(image) = &entry.image {
        return image.reference.clone();
    }
    entry
        .binary
        .as_ref()
        .and_then(|b| b.command.first())
        .and_then(|program| stegobench_plugin::which(program))
        .and_then(|path| stegobench_plugin::hash_file(&path).ok())
        .unwrap_or_else(|| "unknown".into())
}

/// The corpus directory's own name, which is the only name it has.
fn corpus_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("corpus")
        .to_string()
}

/// A UTC timestamp in ISO 8601, captured once per run and reused.
///
/// Written out by hand rather than by pulling in a date library. A result
/// document is read by people and by other tools, so a bare count of seconds
/// would be a number nobody can check against a log, and a dependency whose
/// only job is printing one string is a dependency to audit for ever.
///
/// The civil-date arithmetic is Howard Hinnant's days-from-civil, inverted.
/// It is exact for every date this will ever see and does not need a table.
fn now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso8601(secs)
}

fn iso8601(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // Shift the epoch to 0000-03-01 so a leap day lands at the end of the
    // cycle and the month arithmetic has no special cases.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timestamp_is_iso_8601_and_correct() {
        // A result document is read by people and by tools. This is written
        // by hand rather than by a dependency, so it is checked against dates
        // somebody can verify independently.
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_000_000_000), "2001-09-09T01:46:40Z");
        // A leap day, which is where hand-rolled date arithmetic goes wrong.
        assert_eq!(iso8601(1_709_164_800), "2024-02-29T00:00:00Z");
        // The turn of a century that is not a leap year.
        assert_eq!(iso8601(4_102_444_800), "2100-01-01T00:00:00Z");
    }

    /// A PNG that is a real header and nothing after it.
    ///
    /// The pairing check reads headers, so a fixture of eight magic bytes would
    /// exercise only the branch where nothing can be compared. `padding` puts
    /// bytes after the header without changing what the header says, which is
    /// what a payload does to a file's size.
    fn png(width: u32, height: u32, depth: u8, colour: u8, padding: usize) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&width.to_be_bytes());
        v.extend_from_slice(&height.to_be_bytes());
        v.extend_from_slice(&[depth, colour, 0, 0, 0]);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v.extend_from_slice(&vec![0u8; padding]);
        v
    }

    fn corpus(root: &Path, clean: usize, stego: usize) {
        std::fs::create_dir_all(root).expect("corpus");
        for i in 0..clean {
            std::fs::write(root.join(format!("c{i:03}.png")), png(32, 32, 8, 2, 0)).unwrap();
            std::fs::write(
                root.join(format!("c{i:03}.json")),
                r#"{"role":"clean","sha256":"0"}"#,
            )
            .unwrap();
        }
        for i in 0..stego {
            // Same header, more bytes. A payload changes the file, not its
            // shape, which is the whole premise of the pairing check.
            std::fs::write(root.join(format!("s{i:03}.png")), png(32, 32, 8, 2, 64)).unwrap();
            std::fs::write(
                root.join(format!("s{i:03}.json")),
                r#"{"role":"stego","source_png":"c000.png","sha256":"0"}"#,
            )
            .unwrap();
        }
    }

    fn request(dir: &Path, limit: Option<u64>) -> Request<'_> {
        Request {
            corpus: dir,
            records: dir.with_extension("records.jsonl"),
            timeout: Duration::from_secs(5),
            limit,
        }
    }

    #[test]
    fn the_feed_hands_out_one_item_at_a_time_in_corpus_order() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 2, 2);
        let feed = Feed::open(&request(&root, None)).expect("opens");
        let ids: Vec<String> = feed.map(|w| w.id).collect();
        assert_eq!(ids, ["c000", "c001", "s000", "s001"]);
    }

    #[test]
    fn a_limit_stops_the_feed_early() {
        // The smoke-test path. It is also why such a run is marked custom: a
        // prefix of a corpus is not the corpus.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 4, 4);
        let feed = Feed::open(&request(&root, Some(3))).expect("opens");
        assert_eq!(feed.count(), 3);
    }

    #[test]
    fn a_corpus_defect_stops_the_feed_and_is_kept_rather_than_read_as_the_end() {
        // The difference between "scored everything" and "scored as far as
        // the broken file" is invisible from outside an iterator, and they
        // are very different claims about a number.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 2, 2);
        // An image with no record beside it.
        std::fs::write(root.join("orphan.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        let mut feed = Feed::open(&request(&root, None)).expect("opens");
        let seen = feed.by_ref().count();
        assert!(
            feed.fault.is_some(),
            "a defect was read as the end, after {seen}"
        );
    }

    /// A detector that scores by file size, so a test can measure without
    /// installing anything.
    fn sizing_detector(dir: &Path) -> Entry {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("size.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo v1; exit 0; }\n\
             wc -c < \"$1\" | tr -d ' ' | awk '{print $1/1000}'\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();
        toml::from_str(&format!(
            "name = \"sizer\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [{:?}]\nversion_args = [\"--version\"]\n\
             [emits]\noutput = \"score\"\nhigher_means_stego = true\n\
             [invoke]\nargv = [\"{{file}}\"]\nparser = \"number\"\n\
             [selftest]\nmust_detect = \"fixtures/a.png\"\nmust_clear = \"fixtures/b.png\"\n",
            script.display().to_string()
        ))
        .expect("parses")
    }

    #[test]
    fn two_runs_over_the_same_corpus_agree_on_every_number() {
        // Baseline Section 2.1 asks for byte-identical output from two runs,
        // and a result document cannot give that: it records when the run
        // started and how long it took, and those genuinely differ. So what
        // is asserted is the part a reader acts on. If the metrics moved
        // between two runs over unchanged bytes, the measurement would not be
        // a measurement.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 3, 3);
        let entry = sizing_detector(tmp.path());

        let first = score(&entry, &request(&root, None), |_| {})
            .expect("first run")
            .0;
        // A second records file, so the second run genuinely re-scores rather
        // than resuming and agreeing with itself by construction.
        let mut second_request = request(&root, None);
        second_request.records = root.with_extension("second.jsonl");
        let second = score(&entry, &second_request, |_| {})
            .expect("second run")
            .0;

        assert_eq!(first.metrics, second.metrics);
        assert_eq!(first.corpus, second.corpus);
        assert_eq!(first.declarations, second.declarations);
        assert_eq!(first.subject, second.subject);
    }

    #[test]
    fn a_resumed_run_reports_the_same_numbers_as_an_uninterrupted_one() {
        // The failure this prevents is the worst kind: a resumed run that
        // quietly measures less than it claims, because the resumed items
        // were counted as done but never joined to their labels.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 3, 3);
        let entry = sizing_detector(tmp.path());

        let whole = score(&entry, &request(&root, None), |_| {})
            .expect("whole")
            .0;

        // Score part, then finish, against one records file. The limit has to
        // reach past the clean images: enumeration is sorted, so a limit of
        // three here would score `c000` to `c002` and nothing else, and a
        // one-sided run is correctly refused before it can be resumed.
        let mut partial = request(&root, Some(4));
        partial.records = root.with_extension("resumed.jsonl");
        score(&entry, &partial, |_| {}).expect("partial");
        let mut rest = request(&root, None);
        rest.records = partial.records.clone();
        let (resumed, tally) = score(&entry, &rest, |_| {}).expect("resumed");

        assert_eq!(tally.resumed, 4, "the scored part should have been reused");
        assert_eq!(whole.metrics, resumed.metrics);
    }

    /// A corpus carrying split labels, with one knob: whether the stego rows
    /// agree with their covers.
    fn split_corpus(root: &Path, leak: bool) {
        std::fs::create_dir_all(root).expect("corpus");
        for i in 0..3 {
            let split = if i == 0 { "test" } else { "train" };
            std::fs::write(root.join(format!("c{i}.png")), png(32, 32, 8, 2, 0)).unwrap();
            std::fs::write(
                root.join(format!("c{i}.json")),
                format!(r#"{{"role":"clean","split":"{split}","sha256":"0"}}"#),
            )
            .unwrap();

            // The leak puts the stego twin of cover 0 on the other side.
            let stego_split = if leak && i == 0 { "train" } else { split };
            std::fs::write(root.join(format!("s{i}.png")), png(32, 32, 8, 2, 64)).unwrap();
            std::fs::write(
                root.join(format!("s{i}.json")),
                format!(
                    r#"{{"role":"stego","source_png":"c{i}.png","split":"{stego_split}","sha256":"0"}}"#
                ),
            )
            .unwrap();
        }
    }

    #[test]
    fn a_corpus_that_keeps_twins_together_is_reported_as_by_cover() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        split_corpus(&root, false);
        let checks = check(&request(&root, None)).expect("checked");
        assert!(
            checks.split_leaks.is_empty(),
            "{}",
            checks.split_leaks.examples()
        );
        assert_eq!(checks.split, SplitDiscipline::ByCover);
    }

    #[test]
    fn a_cover_split_from_its_twin_stops_the_run() {
        // The whole reason the check exists. A photograph on both sides of the
        // boundary inflates every number computed from the corpus, and the
        // inflation is invisible in the output, so a confident wrong answer is
        // the alternative to refusing.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        split_corpus(&root, true);

        let checks = check(&request(&root, None)).expect("checked");
        assert_eq!(checks.split_leaks.count, 1);
        assert!(
            checks.split_leaks.examples().contains("c0.png"),
            "{}",
            checks.split_leaks.examples()
        );

        let entry = sizing_detector(tmp.path());
        let err = score(&entry, &request(&root, None), |_| {}).expect_err("refused");
        assert!(err.to_string().contains("different side"), "{err}");
    }

    #[test]
    fn a_corpus_with_no_split_labels_says_not_applicable_rather_than_by_cover() {
        // "No split applies" and "the split was checked and held" are
        // different facts, and a directory of loose samples is an ordinary
        // thing to score.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 2, 2);
        let checks = check(&request(&root, None)).expect("checked");
        assert!(checks.split_leaks.is_empty());
        assert_eq!(checks.split, SplitDiscipline::NotApplicable);
    }

    #[test]
    fn a_stego_row_inheriting_its_cover_split_is_not_a_violation() {
        // Arm rows in the real corpus carry no split of their own, precisely
        // because the split is a property of the cover. Reading that absence
        // as a disagreement would refuse every correctly built corpus.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("c0.png"), png(32, 32, 8, 2, 0)).unwrap();
        std::fs::write(
            root.join("c0.json"),
            r#"{"role":"clean","split":"test","sha256":"0"}"#,
        )
        .unwrap();
        std::fs::write(root.join("s0.png"), png(32, 32, 8, 2, 64)).unwrap();
        std::fs::write(
            root.join("s0.json"),
            r#"{"role":"stego","source_png":"c0.png","sha256":"0"}"#,
        )
        .unwrap();
        let checks = check(&request(&root, None)).expect("checked");
        assert!(
            checks.split_leaks.is_empty(),
            "{}",
            checks.split_leaks.examples()
        );
        assert_eq!(checks.split, SplitDiscipline::ByCover);
    }

    /// A corpus of one pair, where the test chooses what the stego image is.
    fn pair(root: &Path, stego_image: &[u8]) {
        std::fs::create_dir_all(root).expect("corpus");
        std::fs::write(root.join("c0.png"), png(64, 64, 8, 2, 0)).unwrap();
        std::fs::write(root.join("c0.json"), r#"{"role":"clean","sha256":"0"}"#).unwrap();
        std::fs::write(root.join("s0.png"), stego_image).unwrap();
        std::fs::write(
            root.join("s0.json"),
            r#"{"role":"stego","source_png":"c0.png","sha256":"0"}"#,
        )
        .unwrap();
    }

    #[test]
    fn a_stego_image_shaped_like_its_cover_is_reported_as_single_variable() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        pair(&root, &png(64, 64, 8, 2, 128));
        let checks = check(&request(&root, None)).expect("checked");
        assert!(checks.pairing_breaks.is_empty());
        assert_eq!(checks.compared, 1);
        assert_eq!(checks.pairing, Pairing::SingleVariable);
    }

    #[test]
    fn a_stego_image_a_different_size_from_its_cover_is_confounded() {
        // The failure the check exists for. A resized stego half means the
        // detector is being asked to tell two SIZES apart, and it will do it
        // well, and the number will look like detection.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        pair(&root, &png(63, 64, 8, 2, 0));
        let checks = check(&request(&root, None)).expect("checked");
        assert_eq!(checks.pairing, Pairing::Confounded);
        assert_eq!(checks.pairing_breaks.count, 1);
        assert!(
            checks.pairing_breaks.examples().contains("different sizes"),
            "{}",
            checks.pairing_breaks.examples()
        );
    }

    #[test]
    fn a_stego_image_in_another_format_is_confounded_and_says_which_two() {
        // The real case that voided a measurement round: one half written as
        // JPEG against a clean half that was not.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        jpeg.extend_from_slice(&64u16.to_be_bytes());
        jpeg.extend_from_slice(&64u16.to_be_bytes());
        jpeg.push(3);
        jpeg.extend_from_slice(&[1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);
        pair(&root, &jpeg);
        let checks = check(&request(&root, None)).expect("checked");
        assert_eq!(checks.pairing, Pairing::Confounded);
        let seen = checks.pairing_breaks.examples();
        assert!(seen.contains("PNG") && seen.contains("JPEG"), "{seen}");
    }

    #[test]
    fn a_stego_image_of_another_bit_depth_or_channel_count_is_confounded() {
        for stego in [png(64, 64, 16, 2, 0), png(64, 64, 8, 6, 0)] {
            let tmp = tempfile::tempdir().expect("tmp");
            let root = tmp.path().join("corpus");
            pair(&root, &stego);
            let checks = check(&request(&root, None)).expect("checked");
            assert_eq!(checks.pairing, Pairing::Confounded);
        }
    }

    #[test]
    fn a_confounded_corpus_is_scored_and_the_document_says_so() {
        // Deliberately the opposite of what a split leak gets. A second
        // variable is a real property of some arms, kept on purpose to show
        // what it does, so the run happens and the result carries the fact.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..2 {
            std::fs::write(root.join(format!("c{i}.png")), png(64, 64, 8, 2, i)).unwrap();
            std::fs::write(
                root.join(format!("c{i}.json")),
                r#"{"role":"clean","sha256":"0"}"#,
            )
            .unwrap();
            std::fs::write(root.join(format!("s{i}.png")), png(48, 48, 8, 2, 64 + i)).unwrap();
            std::fs::write(
                root.join(format!("s{i}.json")),
                format!(r#"{{"role":"stego","source_png":"c{i}.png","sha256":"0"}}"#),
            )
            .unwrap();
        }

        let entry = sizing_detector(tmp.path());
        let mut warnings = Vec::new();
        let (result, _) = score(&entry, &request(&root, None), |line| {
            warnings.push(line.to_string())
        })
        .expect("scored");
        assert_eq!(result.declarations.pairing, Pairing::Confounded);
        assert!(
            warnings.iter().any(|w| w.starts_with("WARNING")),
            "the run said nothing about it: {warnings:?}"
        );
    }

    #[test]
    fn a_corpus_nothing_can_be_compared_in_says_unverified() {
        // Three ways to end up here, and none of them may read as the rule
        // holding: unreadable images, stego rows that name no cover, and a
        // cover this cannot measure.
        let tmp = tempfile::tempdir().expect("tmp");

        let unreadable = tmp.path().join("unreadable");
        std::fs::create_dir_all(&unreadable).unwrap();
        std::fs::write(unreadable.join("c0.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        std::fs::write(
            unreadable.join("c0.json"),
            r#"{"role":"clean","sha256":"0"}"#,
        )
        .unwrap();
        std::fs::write(unreadable.join("s0.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        std::fs::write(
            unreadable.join("s0.json"),
            r#"{"role":"stego","source_png":"c0.png","sha256":"0"}"#,
        )
        .unwrap();
        let checks = check(&request(&unreadable, None)).expect("checked");
        assert_eq!(checks.pairing, Pairing::Unverified);
        assert_eq!(checks.unreadable, 1);
        assert_eq!(checks.compared, 0);

        let unlinked = tmp.path().join("unlinked");
        std::fs::create_dir_all(&unlinked).unwrap();
        std::fs::write(unlinked.join("c0.png"), png(64, 64, 8, 2, 0)).unwrap();
        std::fs::write(unlinked.join("c0.json"), r#"{"role":"clean","sha256":"0"}"#).unwrap();
        std::fs::write(unlinked.join("s0.png"), png(64, 64, 8, 2, 1)).unwrap();
        std::fs::write(unlinked.join("s0.json"), r#"{"role":"stego","sha256":"0"}"#).unwrap();
        let checks = check(&request(&unlinked, None)).expect("checked");
        assert_eq!(checks.pairing, Pairing::Unverified);
        assert_eq!(checks.compared, 0);
    }

    #[test]
    fn a_format_this_cannot_measure_is_compared_as_far_as_its_format_and_no_further() {
        // Two BMPs of unknown size are not evidence that they match, but they
        // are evidence that neither is a JPEG. Claiming more than that would
        // be the unchecked claim in a smaller costume.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("c0.bmp"), b"BM\x40\x00\x40\x00").unwrap();
        std::fs::write(root.join("c0.json"), r#"{"role":"clean","sha256":"0"}"#).unwrap();
        std::fs::write(root.join("s0.bmp"), b"BM\x20\x00\x20\x00").unwrap();
        std::fs::write(
            root.join("s0.json"),
            r#"{"role":"stego","source_png":"c0.bmp","sha256":"0"}"#,
        )
        .unwrap();
        let checks = check(&request(&root, None)).expect("checked");
        assert_eq!(checks.compared, 1);
        assert_eq!(checks.pairing, Pairing::SingleVariable);
    }

    #[test]
    fn the_count_of_violations_is_exact_while_the_examples_are_capped() {
        // A corpus built by a broken script fails on every row. The count has
        // to be the real one and the list must not grow with the corpus.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("c0.png"), png(64, 64, 8, 2, 0)).unwrap();
        std::fs::write(root.join("c0.json"), r#"{"role":"clean","sha256":"0"}"#).unwrap();
        for i in 0..12 {
            std::fs::write(root.join(format!("s{i:02}.png")), png(48, 48, 8, 2, i)).unwrap();
            std::fs::write(
                root.join(format!("s{i:02}.json")),
                r#"{"role":"stego","source_png":"c0.png","sha256":"0"}"#,
            )
            .unwrap();
        }
        let checks = check(&request(&root, None)).expect("checked");
        assert_eq!(checks.pairing_breaks.count, 12);
        assert_eq!(checks.pairing_breaks.examples.len(), MAX_EXAMPLES);
    }

    #[test]
    fn an_embedder_is_refused_rather_than_scored() {
        // Asking an embedder to tell two images apart produces a number that
        // means nothing, and it would look exactly like a weak detector.
        let entry: Entry = toml::from_str(
            "name = \"x\"\nkind = \"embedder\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"a@sha256:b\"\nsize_mb = 1\nbundled = true\n\
             [selftest]\nmust_detect = \"fixtures/a.png\"\nmust_clear = \"fixtures/b.png\"\n",
        )
        .expect("parses");
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 1, 1);
        let err = score(&entry, &request(&root, None), |_| {}).expect_err("refused");
        assert!(err.to_string().contains("cannot be asked"), "{err}");
    }
}
