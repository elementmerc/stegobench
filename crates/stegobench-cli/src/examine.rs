// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//
// Running a registered detector over images the user brought.
//
// WHY THIS EXISTS, GIVEN THAT THE PROJECT USED TO REFUSE IT
//
// `check` and `scan` were signposts that said Stegobench measures detectors
// against labelled images and cannot tell you whether something is hidden in
// an image of yours, and sent the reader to `describe` to copy a container
// command out by hand. The argument behind the refusal is sound and is kept
// in `help scope`: a detector run raw over one photograph prints pages of
// confident-looking candidate hits that are nearly all noise, and a detector
// right nine times in ten still calls one clean image in ten a hit.
//
// But that is an argument about how an answer must be presented, not about
// whether the command should exist, and `embed` had already crossed the same
// line in the other direction: it takes the user's own image and runs a
// registered embedder on it. So the project accepted "run one registered tool
// on one file of yours" for hiding and refused it for detecting, and the
// refusal was the only thing standing between a reader and the registry's
// whole point, which is not having to go and find these tools one at a time.
//
// WHAT KEEPS IT HONEST
//
// No result document. A `result-v1` is the output of a measurement against
// images whose answers were known in advance, and an examination is not one:
// there are no labels here, so there is no accuracy to report and nothing to
// put a confidence interval around. Keeping the two apart in the type system
// rather than in a warning is what stops an examination being quoted as a
// benchmark figure.

use std::path::{Path, PathBuf};
use std::time::Duration;

use stegobench_core::registry::{Entry, Kind};
use stegobench_plugin::{runner, Record, WorkItem};

/// What one detector said about one image.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// A number. Higher means more suspicious, and the scale is the tool's
    /// own: two detectors' numbers are not comparable with each other.
    Score(f64),
    /// Yes or no, from a tool that gives no number.
    Verdict(bool),
    /// The tool ran and produced nothing readable about this image.
    Failed(String),
}

impl Answer {
    /// The cell as a reader sees it.
    fn cell(&self) -> String {
        match self {
            Answer::Score(s) => format!("{s:.4}"),
            Answer::Verdict(true) => "stego".to_string(),
            Answer::Verdict(false) => "clean".to_string(),
            Answer::Failed(_) => "failed".to_string(),
        }
    }
}

/// One detector's column.
#[derive(Debug, Clone)]
pub struct Column {
    pub detector: String,
    /// `None` when the detector is not installed here, carrying why. The
    /// column is kept rather than dropped: a reader has to see that the tool
    /// was asked and could not answer, which is different from not asking.
    pub unavailable: Option<String>,
    /// One per image, in the order the images were given. Empty when the
    /// detector was unavailable.
    pub answers: Vec<Answer>,
}

/// Everything a run of this command produced.
#[derive(Debug, Clone)]
pub struct Examination {
    pub images: Vec<PathBuf>,
    pub columns: Vec<Column>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExamineError {
    #[error(
        "no detector called {name} is registered. `stegobench list detectors` \
         shows the ones that are"
    )]
    NotRegistered { name: String },
    #[error(
        "{name} is registered as an embedder, which hides a payload in an \
         image rather than answering questions about one. `stegobench list \
         detectors` shows what can answer"
    )]
    NotADetector { name: String },
    #[error("{} is not a file or a directory here", path.display())]
    NotAFile { path: PathBuf },
    #[error("could not read the directory {}: {source}", path.display())]
    UnreadableDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "{} holds no image files, so there is nothing to ask about. Name the \
         images themselves if they are somewhere else, or check the extension: \
         a file this does not recognise as an image is skipped rather than \
         guessed at",
        path.display()
    )]
    NoImagesInDirectory { path: PathBuf },
    #[error(
        "{} holds more than {most} images. That is more than this will start a \
         detector for on one command, because a run of that size belongs in \
         `score` against a labelled corpus where it can be resumed. Name fewer \
         images, or a subdirectory",
        path.display()
    )]
    TooManyImages { path: PathBuf, most: usize },
    #[error("the same detector was asked for twice: {name}")]
    AskedTwice { name: String },
    #[error("could not make a scratch directory for the run: {source}")]
    NoScratch {
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Run(#[from] runner::RunError),
}

impl ExamineError {
    pub fn exit_code(&self) -> i32 {
        use stegobench_core::exit;
        match self {
            ExamineError::NotRegistered { .. }
            | ExamineError::NotADetector { .. }
            | ExamineError::AskedTwice { .. } => exit::USAGE,
            ExamineError::NotAFile { .. }
            | ExamineError::NoImagesInDirectory { .. }
            | ExamineError::TooManyImages { .. } => exit::PREFLIGHT_REFUSED,
            ExamineError::UnreadableDirectory { .. } => exit::FAILURE,
            ExamineError::NoScratch { .. } => exit::FAILURE,
            ExamineError::Run(_) => exit::PLUGIN_FAILED,
        }
    }

    /// A stable word a script can branch on, since the prose will be reworded
    /// the first time somebody is confused by it.
    pub fn reason(&self) -> &'static str {
        match self {
            ExamineError::NotRegistered { .. } => "not-registered",
            ExamineError::NotADetector { .. } => "not-a-detector",
            ExamineError::AskedTwice { .. } => "asked-twice",
            ExamineError::NotAFile { .. } => "not-a-file",
            ExamineError::UnreadableDirectory { .. } => "unreadable-directory",
            ExamineError::NoImagesInDirectory { .. } => "no-images-there",
            ExamineError::TooManyImages { .. } => "too-many-images",
            ExamineError::NoScratch { .. } => "no-scratch",
            ExamineError::Run(_) => "run-failed",
        }
    }
}

/// Everything the run needs that is not the images themselves.
pub struct Request<'a> {
    pub detectors: &'a [String],
    pub images: &'a [PathBuf],
    pub timeout: Duration,
    pub jobs: usize,
    pub adapter_roots: &'a [PathBuf],
    /// Where to keep what the tools printed, if anywhere.
    pub raw: Option<&'a Path>,
}

/// Collects the records in memory.
///
/// An examination is a handful of images rather than a corpus, so holding the
/// answers is bounded by what the user typed on the command line. A scoring
/// run streams to a file precisely because it is not.
struct Collect(Vec<Record>);

impl runner::Sink for Collect {
    fn write(&mut self, record: &Record) -> std::io::Result<()> {
        self.0.push(record.clone());
        Ok(())
    }
}

/// What a cell says when the detector could not be asked.
///
/// Not "not installed", because that is only one of the reasons: a tool can
/// also be here and unable to run on this machine, or here and declare no
/// command to drive it. The footnote under the table carries the specific
/// reason; the cell says only that there is no answer in it.
const UNAVAILABLE: &str = "unavailable";

/// How many entries a named directory is read before it is refused.
///
/// Bounded because a detector may take a second an image and somebody can
/// name a directory holding a hundred thousand of them by accident. The limit
/// refuses loudly rather than silently examining a prefix, which would be a
/// table that looks complete and is not.
const MOST_IMAGES_IN_A_DIRECTORY: usize = 10_000;

/// The images a named path stands for.
///
/// A file is itself. A directory is the image files directly inside it, in
/// sorted order, and nothing from any subdirectory: somebody who names a
/// folder means the pictures in it, and walking into subdirectories turns one
/// mistyped path into an hour of container starts.
///
/// Sorted so two runs over one directory produce the same table in the same
/// order, which readdir alone does not promise.
fn images_under(path: &Path) -> Result<Vec<PathBuf>, ExamineError> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.is_dir() {
        return Err(ExamineError::NotAFile {
            path: path.to_path_buf(),
        });
    }
    let entries = std::fs::read_dir(path).map_err(|source| ExamineError::UnreadableDirectory {
        path: path.to_path_buf(),
        source,
    })?;
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ExamineError::UnreadableDirectory {
            path: path.to_path_buf(),
            source,
        })?;
        let candidate = entry.path();
        if !candidate.is_file() {
            continue;
        }
        let looks_right = candidate
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .is_some_and(|e| crate::LOOKS_LIKE_AN_IMAGE.contains(&e.as_str()));
        if looks_right {
            found.push(candidate);
        }
        if found.len() > MOST_IMAGES_IN_A_DIRECTORY {
            return Err(ExamineError::TooManyImages {
                path: path.to_path_buf(),
                most: MOST_IMAGES_IN_A_DIRECTORY,
            });
        }
    }
    if found.is_empty() {
        return Err(ExamineError::NoImagesInDirectory {
            path: path.to_path_buf(),
        });
    }
    found.sort();
    Ok(found)
}

/// Checks the names and the files before any tool is started.
fn preflight<'r>(
    registry: &'r stegobench_core::registry::Registry,
    request: &Request<'_>,
) -> Result<Vec<&'r Entry>, ExamineError> {
    let mut entries = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for name in request.detectors {
        if seen.contains(&name.as_str()) {
            return Err(ExamineError::AskedTwice { name: name.clone() });
        }
        seen.push(name);
        let Some(entry) = registry.entries.get(name) else {
            return Err(ExamineError::NotRegistered { name: name.clone() });
        };
        if entry.kind != Kind::Detector {
            return Err(ExamineError::NotADetector { name: name.clone() });
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// Every path the user named, expanded, with duplicates kept out.
///
/// A duplicate is dropped rather than refused: naming a file and the
/// directory holding it is an easy thing to type and asking twice about one
/// image would put the same row in the table twice, which reads as two pieces
/// of evidence.
fn expand(named: &[PathBuf]) -> Result<Vec<PathBuf>, ExamineError> {
    let mut images: Vec<PathBuf> = Vec::new();
    for path in named {
        for image in images_under(path)? {
            let canonical = image.canonicalize().unwrap_or_else(|_| image.clone());
            if !images
                .iter()
                .any(|seen| seen.canonicalize().unwrap_or_else(|_| seen.clone()) == canonical)
            {
                images.push(image);
            }
        }
    }
    Ok(images)
}

/// Runs every named detector over every named image.
///
/// `progress` is called with a line per detector as it starts, because a
/// container pull or a cold start is long enough that silence reads as a hang.
pub fn run<P>(
    registry: &stegobench_core::registry::Registry,
    request: Request<'_>,
    mut progress: P,
) -> Result<Examination, ExamineError>
where
    P: FnMut(&str),
{
    let entries = preflight(registry, &request)?;
    let images = expand(request.images)?;

    let scratch = tempfile::tempdir().map_err(|source| ExamineError::NoScratch { source })?;
    let mut columns = Vec::with_capacity(entries.len());

    for entry in entries {
        // The same question `score` and `doctor` ask, through the same
        // function, which also catches the tool that IS here and declares no
        // command to drive it. Asking only whether the code is present would
        // start that one once per image and record nothing against every one.
        if let Some(why) = crate::unavailable_reason(entry, request.adapter_roots) {
            progress(&format!("{} {why}, so it answered nothing", entry.name));
            columns.push(Column {
                detector: entry.name.clone(),
                unavailable: Some(why),
                answers: Vec::new(),
            });
            continue;
        }

        progress(&format!("{} over {} image(s)", entry.name, images.len()));

        // A path inside a fresh scratch directory, so nothing resumes: an
        // examination is never continued from a previous one, and a stale
        // record would answer about a file the user is not asking about now.
        let records = scratch.path().join(format!("{}.records.jsonl", entry.name));
        let items: Vec<WorkItem> = images
            .iter()
            .map(|path| WorkItem {
                id: path.display().to_string(),
                path: path.clone(),
            })
            .collect();

        let mut sink = Collect(Vec::with_capacity(images.len()));
        let tally = runner::score(
            runner::Run {
                entry,
                already: &records,
                timeout: request.timeout,
                jobs: request.jobs,
                adapter_roots: request.adapter_roots,
                raw: runner::RawPolicy {
                    path: request.raw,
                    every: request.raw.is_some(),
                },
            },
            items,
            &mut sink,
            // The runner calls this at most once every thirty seconds, so a
            // handful of images never prints it and a directory of ten
            // thousand does not go silent for hours. Discarding it was the
            // whole observability rule missed in one empty closure.
            |t| progress(&heartbeat(&entry.name, t.seen(), images.len(), t.errored)),
        )?;
        if let Some(problem) = &tally.raw_problem {
            progress(problem);
        }

        columns.push(Column {
            detector: entry.name.clone(),
            unavailable: None,
            answers: sink.0.iter().map(answer_of).collect(),
        });
    }

    Ok(Examination { images, columns })
}

/// The line a long examination prints while it works.
///
/// A function rather than a closure body so it can be read and tested. The
/// wiring behind it cannot be unit tested without waiting thirty seconds for
/// the runner's heartbeat, so it was verified by running: forty images
/// against a detector sleeping a second each printed exactly one of these, at
/// thirty, reading "slow: 30 of 40 answered, 0 errored".
fn heartbeat(name: &str, answered: u64, total: usize, errored: u64) -> String {
    format!("{name}: {answered} of {total} answered, {errored} errored")
}

/// A record as one cell.
///
/// A record carrying neither an answer nor an error cannot be written by the
/// runner, which treats that as incomplete, so the last arm is the record
/// having said only that it failed without saying why.
fn answer_of(record: &Record) -> Answer {
    match (record.score, record.verdict, &record.error) {
        (Some(s), _, _) => Answer::Score(s),
        (None, Some(v), _) => Answer::Verdict(v),
        (None, None, Some(e)) => Answer::Failed(e.clone()),
        (None, None, None) => Answer::Failed("no answer and no reason".to_string()),
    }
}

impl Examination {
    /// Whether every detector answered about every image.
    pub fn complete(&self) -> bool {
        self.columns.iter().all(|c| {
            c.unavailable.is_none()
                && c.answers.len() == self.images.len()
                && !c.answers.iter().any(|a| matches!(a, Answer::Failed(_)))
        })
    }

    /// The exit code this examination earns.
    ///
    /// A detector that is not here is a refusal: nothing is broken and
    /// installing it fixes it. A detector that ran and answered nothing is a
    /// failure, and the two are not the same thing to anybody automating this.
    ///
    /// NONE OF THEM BEING HERE IS A THIRD THING, AND IT USED TO LOOK LIKE THE
    /// FIRST.
    ///
    /// One detector of five missing and all five missing both returned 3, so
    /// a script could not tell a partial answer from an empty table. The
    /// empty table is the machine being unfit for the request rather than the
    /// request being partly refused, which is what 8 already means elsewhere
    /// in this binary: `doctor` returns it when nothing here is usable.
    pub fn exit_code(&self) -> i32 {
        use stegobench_core::exit;
        if self.columns.iter().any(|c| {
            c.answers.iter().any(|a| matches!(a, Answer::Failed(_)))
                || (c.unavailable.is_none() && c.answers.len() != self.images.len())
        }) {
            return exit::PLUGIN_FAILED;
        }
        let unavailable = self
            .columns
            .iter()
            .filter(|c| c.unavailable.is_some())
            .count();
        if unavailable > 0 && unavailable == self.columns.len() {
            return exit::ENVIRONMENT_UNFIT;
        }
        if unavailable > 0 {
            return exit::PREFLIGHT_REFUSED;
        }
        exit::OK
    }

    /// The table, one row per image and one column per detector.
    pub fn table(&self) -> String {
        let mut name_width = "image".len();
        let labels: Vec<String> = self
            .images
            .iter()
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.display().to_string())
            })
            .collect();
        for label in &labels {
            name_width = name_width.max(label.chars().count());
        }

        let mut widths = Vec::with_capacity(self.columns.len());
        for column in &self.columns {
            let mut w = column.detector.chars().count();
            if column.unavailable.is_some() {
                w = w.max(UNAVAILABLE.len());
            }
            for answer in &column.answers {
                w = w.max(answer.cell().chars().count());
            }
            widths.push(w);
        }

        // Every line is trimmed at its end. Padding the last column leaves
        // trailing spaces on every row, which a diff flags, a linter flags,
        // and a reader copying the table out carries with them.
        let mut out = String::new();
        let mut line = format!("{:<name_width$}", "image");
        for (column, width) in self.columns.iter().zip(&widths) {
            line.push_str(&format!("  {:<width$}", column.detector, width = width));
        }
        out.push_str(line.trim_end());
        out.push('\n');

        for (row, label) in labels.iter().enumerate() {
            let mut line = format!("{label:<name_width$}");
            for (column, width) in self.columns.iter().zip(&widths) {
                let cell = match (&column.unavailable, column.answers.get(row)) {
                    (Some(_), _) => UNAVAILABLE.to_string(),
                    (None, Some(answer)) => answer.cell(),
                    (None, None) => "failed".to_string(),
                };
                line.push_str(&format!("  {cell:<width$}"));
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    /// The lines that go under the table.
    ///
    /// Short on purpose. The argument for why an examination is weak evidence
    /// is long and it lives in `help scope`, where somebody can read it when
    /// they want it; repeating it under every table trains people to skip it.
    pub fn footnotes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        notes.push(
            "NOT A MEASUREMENT. No image here was labelled, so nothing above is an \
             accuracy, and a figure from it is not quotable. `stegobench help scope` \
             says why, and `stegobench score` is what produces a number you can defend."
                .to_string(),
        );
        if self
            .columns
            .iter()
            .filter(|c| c.unavailable.is_none())
            .count()
            > 1
        {
            notes.push(
                "Each column is on its own scale. A higher number means more suspicious \
                 to that one detector, and two detectors' numbers do not compare with \
                 each other."
                    .to_string(),
            );
        }
        for column in &self.columns {
            if let Some(why) = &column.unavailable {
                notes.push(format!("{}: {why}", column.detector));
            }
        }
        for column in &self.columns {
            let failures = column
                .answers
                .iter()
                .filter(|a| matches!(a, Answer::Failed(_)))
                .count();
            if failures > 0 {
                notes.push(format!(
                    "{}: answered nothing about {failures} of {} image(s). \
                     `--raw <FILE>` keeps what it printed.",
                    column.detector,
                    self.images.len()
                ));
            }
        }
        notes
    }

    /// The JSON block, which is deliberately not a result document.
    pub fn to_json(&self) -> serde_json::Value {
        let images: Vec<String> = self
            .images
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        let columns: Vec<serde_json::Value> = self
            .columns
            .iter()
            .map(|column| {
                let answers: Vec<serde_json::Value> = column
                    .answers
                    .iter()
                    .map(|a| match a {
                        Answer::Score(s) => serde_json::json!({ "score": s }),
                        Answer::Verdict(v) => serde_json::json!({ "verdict": v }),
                        Answer::Failed(why) => serde_json::json!({ "error": why }),
                    })
                    .collect();
                serde_json::json!({
                    "detector": column.detector,
                    "unavailable": column.unavailable,
                    "answers": answers,
                })
            })
            .collect();
        serde_json::json!({
            // Named so nothing mistakes this for a `result-v1`, which is the
            // output of a measurement against labelled images and carries an
            // accuracy. This carries answers and no accuracy.
            "kind": "examination",
            "measurement": false,
            "images": images,
            "columns": columns,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn examination(columns: Vec<Column>, images: usize) -> Examination {
        Examination {
            images: (0..images)
                .map(|i| PathBuf::from(format!("img{i}.png")))
                .collect(),
            columns,
        }
    }

    fn answered(detector: &str, answers: Vec<Answer>) -> Column {
        Column {
            detector: detector.to_string(),
            unavailable: None,
            answers,
        }
    }

    #[test]
    fn the_heartbeat_names_the_detector_and_both_counts() {
        // All four values, because a progress line that drops the total is
        // the one that leaves somebody unable to tell a slow run from a
        // stuck one.
        let line = heartbeat("zsteg", 30, 40, 2);
        assert!(line.contains("zsteg"), "{line}");
        assert!(line.contains("30 of 40"), "{line}");
        assert!(line.contains("2 errored"), "{line}");
    }

    #[test]
    fn a_score_and_a_verdict_render_differently() {
        assert_eq!(Answer::Score(0.5).cell(), "0.5000");
        assert_eq!(Answer::Verdict(true).cell(), "stego");
        assert_eq!(Answer::Verdict(false).cell(), "clean");
        assert_eq!(Answer::Failed("why".into()).cell(), "failed");
    }

    #[test]
    fn a_record_becomes_the_answer_it_carries() {
        let base = Record {
            id: "a".into(),
            score: None,
            verdict: None,
            error: None,
            elapsed_ms: None,
        };
        assert_eq!(
            answer_of(&Record {
                score: Some(0.25),
                ..base.clone()
            }),
            Answer::Score(0.25)
        );
        assert_eq!(
            answer_of(&Record {
                verdict: Some(true),
                ..base.clone()
            }),
            Answer::Verdict(true)
        );
        assert_eq!(
            answer_of(&Record {
                error: Some("boom".into()),
                ..base.clone()
            }),
            Answer::Failed("boom".into())
        );
        // A record with neither cannot be written by the runner, and if one
        // ever is, it says so rather than reading as an answer.
        assert_eq!(
            answer_of(&base),
            Answer::Failed("no answer and no reason".into())
        );
    }

    #[test]
    fn a_score_wins_over_a_verdict_in_the_same_record() {
        // A plugin should send one or the other. Given both, the number is
        // kept, because a number carries strictly more than the yes or no a
        // threshold would turn it into.
        let both = Record {
            id: "a".into(),
            score: Some(0.9),
            verdict: Some(false),
            error: None,
            elapsed_ms: None,
        };
        assert_eq!(answer_of(&both), Answer::Score(0.9));
    }

    #[test]
    fn every_detector_answering_exits_zero() {
        let e = examination(
            vec![
                answered("alfa", vec![Answer::Score(0.1), Answer::Score(0.2)]),
                answered("bravo", vec![Answer::Verdict(true), Answer::Verdict(false)]),
            ],
            2,
        );
        assert!(e.complete());
        assert_eq!(e.exit_code(), stegobench_core::exit::OK);
    }

    #[test]
    fn a_detector_that_is_not_here_is_a_refusal_rather_than_a_failure() {
        let e = examination(
            vec![
                answered("alfa", vec![Answer::Score(0.1)]),
                Column {
                    detector: "bravo".to_string(),
                    unavailable: Some("no container runtime".to_string()),
                    answers: Vec::new(),
                },
            ],
            1,
        );
        assert!(!e.complete());
        assert_eq!(e.exit_code(), stegobench_core::exit::PREFLIGHT_REFUSED);
        assert!(e
            .footnotes()
            .iter()
            .any(|n| n.contains("no container runtime")));
    }

    #[test]
    fn a_detector_that_answered_nothing_is_a_failure() {
        let e = examination(
            vec![answered(
                "alfa",
                vec![Answer::Score(0.1), Answer::Failed("timed out".into())],
            )],
            2,
        );
        assert!(!e.complete());
        assert_eq!(e.exit_code(), stegobench_core::exit::PLUGIN_FAILED);
    }

    #[test]
    fn a_short_column_is_a_failure_even_with_no_error_recorded() {
        // Fewer answers than images means the run stopped early, and reading
        // the missing rows as "nothing suspicious" is the worst thing this
        // command could do.
        let e = examination(vec![answered("alfa", vec![Answer::Score(0.1)])], 3);
        assert!(!e.complete());
        assert_eq!(e.exit_code(), stegobench_core::exit::PLUGIN_FAILED);
    }

    /// The distinction a script needs and did not have: some missing against
    /// all missing.
    #[test]
    fn nothing_being_installed_is_told_apart_from_something_being_installed() {
        let missing = |name: &str| Column {
            detector: name.to_string(),
            unavailable: Some("not installed".to_string()),
            answers: Vec::new(),
        };

        let some = examination(
            vec![answered("alfa", vec![Answer::Score(0.1)]), missing("bravo")],
            1,
        );
        assert_eq!(some.exit_code(), stegobench_core::exit::PREFLIGHT_REFUSED);

        let none = examination(vec![missing("alfa"), missing("bravo")], 1);
        assert_eq!(none.exit_code(), stegobench_core::exit::ENVIRONMENT_UNFIT);
        assert_ne!(some.exit_code(), none.exit_code());
    }

    #[test]
    fn a_failure_outranks_an_unavailable_detector_in_the_exit_code() {
        let e = examination(
            vec![
                answered("alfa", vec![Answer::Failed("boom".into())]),
                Column {
                    detector: "bravo".to_string(),
                    unavailable: Some("not installed".to_string()),
                    answers: Vec::new(),
                },
            ],
            1,
        );
        assert_eq!(e.exit_code(), stegobench_core::exit::PLUGIN_FAILED);
    }

    #[test]
    fn the_table_aligns_on_the_longest_name_and_cell() {
        let e = Examination {
            images: vec![
                PathBuf::from("/some/where/a-long-filename.png"),
                PathBuf::from("/some/where/b.png"),
            ],
            columns: vec![answered(
                "aletheia-spa",
                vec![Answer::Score(0.1234), Answer::Verdict(true)],
            )],
        };
        let table = e.table();
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 3);
        // The row label is the file name, not the whole path: a reader gave
        // these paths and a column of identical directory prefixes is noise.
        assert!(lines[1].starts_with("a-long-filename.png"));
        assert!(lines[2].starts_with("b.png"));
        // Every line is padded to the same width before the first cell.
        let at = |line: &str| line.find("0.1234").or_else(|| line.find("stego"));
        assert_eq!(at(lines[1]), at(lines[2]));
        // And no line carries trailing whitespace, which the last column's
        // padding produced until it was trimmed.
        for line in &lines {
            assert_eq!(*line, line.trim_end(), "trailing space on {line:?}");
        }
    }

    #[test]
    fn the_not_a_measurement_note_is_always_there() {
        let e = examination(vec![answered("alfa", vec![Answer::Score(0.1)])], 1);
        let notes = e.footnotes();
        assert!(notes[0].contains("NOT A MEASUREMENT"));
        assert!(notes[0].contains("not quotable"));
        // One detector needs no warning about comparing scales.
        assert!(!notes.iter().any(|n| n.contains("own scale")));
    }

    #[test]
    fn two_detectors_are_warned_not_to_be_compared_directly() {
        let e = examination(
            vec![
                answered("alfa", vec![Answer::Score(0.1)]),
                answered("bravo", vec![Answer::Score(0.9)]),
            ],
            1,
        );
        assert!(e.footnotes().iter().any(|n| n.contains("own scale")));
    }

    #[test]
    fn one_detector_plus_an_unavailable_one_is_not_two_scales() {
        // The warning is about comparing numbers, and a column with no
        // numbers in it gives nobody anything to compare.
        let e = examination(
            vec![
                answered("alfa", vec![Answer::Score(0.1)]),
                Column {
                    detector: "bravo".to_string(),
                    unavailable: Some("not installed".to_string()),
                    answers: Vec::new(),
                },
            ],
            1,
        );
        assert!(!e.footnotes().iter().any(|n| n.contains("own scale")));
    }

    #[test]
    fn the_json_says_it_is_not_a_measurement() {
        let e = examination(vec![answered("alfa", vec![Answer::Score(0.1)])], 1);
        let json = e.to_json();
        assert_eq!(json["kind"], "examination");
        assert_eq!(json["measurement"], false);
        // And carries no field a result document would carry.
        assert!(json.get("metrics").is_none());
        assert!(json.get("auc").is_none());
    }

    #[test]
    fn every_error_carries_its_own_exit_code_and_stable_word() {
        use stegobench_core::exit;
        let cases = [
            (
                ExamineError::NotRegistered { name: "x".into() },
                exit::USAGE,
                "not-registered",
            ),
            (
                ExamineError::NotADetector { name: "x".into() },
                exit::USAGE,
                "not-a-detector",
            ),
            (
                ExamineError::AskedTwice { name: "x".into() },
                exit::USAGE,
                "asked-twice",
            ),
            (
                ExamineError::NotAFile {
                    path: PathBuf::from("x"),
                },
                exit::PREFLIGHT_REFUSED,
                "not-a-file",
            ),
        ];
        for (error, code, reason) in cases {
            assert_eq!(error.exit_code(), code, "{error}");
            assert_eq!(error.reason(), reason);
            // And every one says something, since a refusal with no reason is
            // the same as a crash to whoever reads it.
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn asking_for_a_detector_twice_is_refused_rather_than_run_twice() {
        // Two identical columns would make the table look like corroboration
        // from two independent tools.
        let e = ExamineError::AskedTwice {
            name: "zsteg".into(),
        };
        assert!(e.to_string().contains("twice"));
        assert_eq!(e.exit_code(), stegobench_core::exit::USAGE);
    }
}

#[cfg(test)]
mod expansion {
    use super::*;

    fn tree(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tmp");
        for name in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("mkdir");
            }
            std::fs::write(&path, b"x").expect("write");
        }
        dir
    }

    #[test]
    fn a_named_file_is_itself() {
        let dir = tree(&["a.png"]);
        let file = dir.path().join("a.png");
        assert_eq!(images_under(&file).expect("found"), vec![file.clone()]);
    }

    #[test]
    fn a_named_directory_becomes_the_images_in_it_sorted() {
        let dir = tree(&["c.png", "a.jpg", "b.bmp", "notes.txt", "run.sh"]);
        let found = images_under(dir.path()).expect("found");
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        // Sorted, so two runs over one directory give the same table in the
        // same order. Readdir alone does not promise that.
        assert_eq!(names, vec!["a.jpg", "b.bmp", "c.png"]);
    }

    #[test]
    fn an_extension_in_capitals_is_still_an_image() {
        let dir = tree(&["PHOTO.JPG", "other.PNG"]);
        assert_eq!(images_under(dir.path()).expect("found").len(), 2);
    }

    #[test]
    fn a_subdirectory_is_not_walked_into() {
        // One mistyped path should not become an hour of container starts.
        let dir = tree(&["top.png", "deeper/inside.png"]);
        let found = images_under(dir.path()).expect("found");
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("top.png"));
    }

    #[test]
    fn a_directory_with_no_images_is_refused_rather_than_examined_as_empty() {
        let dir = tree(&["notes.txt"]);
        let e = images_under(dir.path()).expect_err("refused");
        assert_eq!(e.reason(), "no-images-there");
        assert_eq!(e.exit_code(), stegobench_core::exit::PREFLIGHT_REFUSED);
    }

    #[test]
    fn a_path_that_is_neither_is_the_same_refusal_as_before() {
        let dir = tree(&[]);
        let e = images_under(&dir.path().join("nothing-here.png")).expect_err("refused");
        assert_eq!(e.reason(), "not-a-file");
    }

    #[test]
    fn naming_a_file_and_its_directory_asks_about_it_once() {
        // Twice in the table would read as two pieces of evidence.
        let dir = tree(&["a.png", "b.png"]);
        let images =
            expand(&[dir.path().to_path_buf(), dir.path().join("a.png")]).expect("expanded");
        assert_eq!(images.len(), 2, "{images:?}");
    }

    #[test]
    fn the_same_file_named_twice_is_asked_about_once() {
        let dir = tree(&["a.png"]);
        let file = dir.path().join("a.png");
        let images = expand(&[file.clone(), file]).expect("expanded");
        assert_eq!(images.len(), 1);
    }

    #[test]
    fn two_directories_are_both_expanded_in_the_order_they_were_named() {
        let first = tree(&["z.png"]);
        let second = tree(&["a.png"]);
        let images =
            expand(&[first.path().to_path_buf(), second.path().to_path_buf()]).expect("expanded");
        // Within a directory it sorts; across directories it keeps what the
        // user typed, because that is the order they listed them in.
        assert!(images[0].ends_with("z.png"), "{images:?}");
        assert!(images[1].ends_with("a.png"), "{images:?}");
    }

    #[test]
    fn one_bad_path_refuses_the_whole_command_rather_than_examining_the_rest() {
        // A partial table that looks complete is the worst outcome here.
        let dir = tree(&["a.png"]);
        let e = expand(&[dir.path().to_path_buf(), PathBuf::from("/no/such/place")])
            .expect_err("refused");
        assert_eq!(e.reason(), "not-a-file");
    }
}
