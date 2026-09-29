// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo

//! `stegobench metrics`: the one place a detection number is computed.
//!
//! WHY A COMMAND AND NOT A LIBRARY CALL
//!
//! The arithmetic already lives in `stegobench-metrics`, and the Rust half of
//! this project reaches it directly. The Python half could not, so it grew its
//! own copy, and two copies of a metric is two answers to the same question
//! with nothing in either output to say which one a reader is holding. This
//! command is the seam that removes the second copy: anything that can start a
//! process and write JSON now gets the same number the Rust scorer gets, from
//! the same code.
//!
//! WHAT IT WILL NOT DO
//!
//! It never invents a figure. Every condition the metrics crate answers `None`
//! to (a length mismatch, a score that is not a number, a budget that is not a
//! rate, a single class) comes back here as a refusal naming which condition
//! fired, because a null in a machine-readable output gets read as a zero far
//! more often than it gets read as an absence.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use serde::Deserialize;
use stegobench_core::exit;

/// How many scores one call will read.
///
/// The largest published tier is Pentimento Core at 344,357 pairs, which is
/// 688,714 answers for one arm. This is roughly thirty times that, so no real
/// corpus meets it, and it bounds what one call can hold: 20 million f64 is
/// 160 MB, plus 20 MB of labels, plus the JSON text they were parsed from.
/// A bound nobody legitimately reaches is still a bound, and without one a
/// malformed input decides this process's memory footprint.
pub const MAX_SCORES: usize = 20_000_000;

/// How many bytes of JSON one call will read.
///
/// The whole document is held as text before it is parsed, so this is the real
/// ceiling on the input and it is stated separately from [`MAX_SCORES`]. At
/// roughly twenty bytes for a score and six for a label, 256 MiB comfortably
/// holds ten times the largest published tier.
pub const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;

/// Scores and labels, as the caller handed them over.
///
/// A score the detector could not produce is written `null` rather than left
/// out. JSON has no NaN, so without a spelling for "no answer" a caller would
/// have to choose between a placeholder number, which is a fabricated
/// measurement, and dropping the item from one list and not the other, which
/// is a length mismatch this would then refuse for the wrong reason.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// One score per item. `null` means the detector produced no number.
    pub scores: Vec<Option<f64>>,
    /// One label per item, `true` for stego.
    pub labels: Vec<bool>,
}

/// Why a set of scores could not be turned into a metric.
#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "{path} holds more than {} MiB of JSON, which is more than this reads \
         in one call. Split the run into arms and score each one: a single set \
         of scores that large is not one measurement",
        MAX_INPUT_BYTES / (1024 * 1024)
    )]
    TooLarge { path: String },
    #[error(
        "{count} scores is more than the {MAX_SCORES} this reads in one call. \
         Split the run into arms and score each one"
    )]
    TooManyScores { count: usize },
    #[error(
        "{path} is not a JSON object of scores and labels: {source}\nIt should \
         look like {{\"scores\": [0.91, 0.02], \"labels\": [true, false]}}, \
         with null for an item the detector could not score"
    )]
    NotJson {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "a false-alarm budget of {text:?} is not a rate. Give a fraction \
         between 0 and 1, such as 0.01 for one per cent"
    )]
    BudgetNotARate { text: String },
    #[error(
        "--at {text} was given twice. One budget is one measurement, and \
         reporting it twice would put the same figure in the output under two \
         names"
    )]
    DuplicateBudget { text: String },
    #[error(
        "{scores} score(s) against {labels} label(s): the two were built from \
         different record sets, so any ranking over them would describe images \
         nobody named"
    )]
    LengthMismatch { scores: usize, labels: usize },
    #[error(
        "nothing to measure: no scores were given. An empty set reads as a \
         detector that caught nothing rather than one that was never run"
    )]
    Empty,
    #[error(
        "{count} of the {total} score(s) are null, so the detector produced no \
         number for them. A value that cannot be ordered cannot be ranked, and \
         a ranking that quietly skipped it would be measured on a subset \
         nobody named"
    )]
    NotANumber { count: usize, total: usize },
    #[error(
        "{clean} clean and {stego} stego image(s), and a ranking needs both. \
         There is nothing to tell apart when every answer carries the same \
         label"
    )]
    OneSided { clean: usize, stego: usize },
    #[error(
        "{count} answer(s) were usable and the ranking over them still could \
         not be formed{at}. This is a bug in stegobench rather than anything \
         you did; please report it with this message"
    )]
    Unrankable { count: usize, at: String },
}

impl MetricsError {
    /// Which documented exit code this is.
    ///
    /// Two groups, and the split is whether retrying the same command
    /// unchanged could work. A malformed input or a budget that is not a rate
    /// is a usage error: fix the command. Numbers that are well formed and
    /// cannot be ranked are a refusal, code 3, and will refuse again.
    pub fn exit_code(&self) -> i32 {
        match self {
            MetricsError::Read { .. } => exit::FAILURE,
            MetricsError::NotJson { .. }
            | MetricsError::BudgetNotARate { .. }
            | MetricsError::DuplicateBudget { .. }
            | MetricsError::LengthMismatch { .. } => exit::USAGE,
            MetricsError::TooLarge { .. }
            | MetricsError::TooManyScores { .. }
            | MetricsError::Empty
            | MetricsError::NotANumber { .. }
            | MetricsError::OneSided { .. } => exit::PREFLIGHT_REFUSED,
            MetricsError::Unrankable { .. } => exit::FAILURE,
        }
    }

    /// A stable word for this condition, for a caller that has to branch.
    ///
    /// The message is for a person and will be reworded; this will not. A
    /// script reading the JSON error needs to tell "one class only", which is
    /// a fact about the corpus it may want to skip past, from "that file is
    /// not JSON", which is its own bug, and matching on English prose to do it
    /// is how a caller silently stops noticing the difference.
    pub fn reason(&self) -> &'static str {
        match self {
            MetricsError::Read { .. } => "read-failed",
            MetricsError::TooLarge { .. } => "input-too-large",
            MetricsError::TooManyScores { .. } => "too-many-scores",
            MetricsError::NotJson { .. } => "not-json",
            MetricsError::BudgetNotARate { .. } => "budget-not-a-rate",
            MetricsError::DuplicateBudget { .. } => "duplicate-budget",
            MetricsError::LengthMismatch { .. } => "length-mismatch",
            MetricsError::Empty => "empty",
            MetricsError::NotANumber { .. } => "not-a-number",
            MetricsError::OneSided { .. } => "one-sided",
            MetricsError::Unrankable { .. } => "unrankable",
        }
    }
}

/// Every number one call produces.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub auc: f64,
    /// Detection rate keyed by the budget TEXT the caller typed.
    ///
    /// Keyed by the text rather than by a reformatted number so a caller can
    /// look up what it asked for without knowing how this renders a float:
    /// `--at 0.10` answers under "0.10" and `--at 0.1` under "0.1". Both are
    /// the same budget and both are honoured; neither is silently renamed.
    pub tpr_at_fpr: BTreeMap<String, f64>,
    pub n_clean: usize,
    pub n_stego: usize,
}

impl Report {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ok": true,
            "auc": self.auc,
            "tpr_at_fpr": self.tpr_at_fpr,
            "n_clean": self.n_clean,
            "n_stego": self.n_stego,
            "n": self.n_clean + self.n_stego,
        })
    }

    /// What a person reads. Full precision, because rounding belongs to
    /// whoever is writing the report and this is not that.
    pub fn human(&self) -> String {
        let mut text = format!(
            "{} answer(s): {} clean, {} stego\nAUC            {}",
            self.n_clean + self.n_stego,
            self.n_clean,
            self.n_stego,
            self.auc
        );
        for (budget, tpr) in &self.tpr_at_fpr {
            text.push_str(&format!("\nTPR at {budget:<7} {tpr}"));
        }
        text
    }
}

/// Reads scores and labels, and reports the metrics over them.
///
/// `file` of `None`, or of `-`, reads standard input, so a few hundred
/// thousand scores never have to fit in an argv.
pub fn run(file: Option<&Path>, at: &[String]) -> Result<Report, MetricsError> {
    let budgets = parse_budgets(at)?;
    let (label, text) = read_input(file)?;
    let input: Input = serde_json::from_str(&text).map_err(|source| MetricsError::NotJson {
        path: label,
        source,
    })?;
    compute(input, &budgets)
}

/// The budgets to report at, as `(text the caller typed, value)`.
fn parse_budgets(at: &[String]) -> Result<Vec<(String, f64)>, MetricsError> {
    let mut out: Vec<(String, f64)> = Vec::new();
    for text in at {
        if out.iter().any(|(seen, _)| seen == text) {
            return Err(MetricsError::DuplicateBudget { text: text.clone() });
        }
        let value: f64 = text
            .parse()
            .map_err(|_| MetricsError::BudgetNotARate { text: text.clone() })?;
        // The same window `tpr_at_fpr` accepts, checked here so the refusal
        // names the budget rather than arriving as a bare `None` from a
        // function that was asked about four of them.
        if !(0.0..=1.0).contains(&value) {
            return Err(MetricsError::BudgetNotARate { text: text.clone() });
        }
        out.push((text.clone(), value));
    }
    Ok(out)
}

/// The whole input as text, bounded twice: by what the file claims and by what
/// is actually read.
///
/// The metadata check is the cheap one and it fires before a byte is copied.
/// It is not sufficient on its own: standard input has no length to ask about,
/// and a file can grow between the two calls, so the read is capped as well.
fn read_input(file: Option<&Path>) -> Result<(String, String), MetricsError> {
    let (label, mut reader): (String, Box<dyn Read>) = match file {
        None => (
            "standard input".to_string(),
            Box::new(std::io::stdin().lock()),
        ),
        Some(path) => {
            let handle = std::fs::File::open(path).map_err(|source| MetricsError::Read {
                path: path.display().to_string(),
                source,
            })?;
            if handle.metadata().map(|m| m.len()).unwrap_or(0) > MAX_INPUT_BYTES {
                return Err(MetricsError::TooLarge {
                    path: path.display().to_string(),
                });
            }
            (path.display().to_string(), Box::new(handle))
        }
    };
    let mut text = String::new();
    // One byte past the cap, so a file exactly at the limit is read and one
    // byte over is refused rather than silently truncated into a parse error
    // that names the wrong problem.
    let read = reader
        .by_ref()
        .take(MAX_INPUT_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|source| MetricsError::Read {
            path: label.clone(),
            source,
        })?;
    if read as u64 > MAX_INPUT_BYTES {
        return Err(MetricsError::TooLarge { path: label });
    }
    Ok((label, text))
}

/// The metrics over one set of scores and labels.
///
/// Every refusal the metrics crate expresses as `None` is raised here as a
/// named condition first, so the crate's `None` can only mean a case this did
/// not anticipate, which is reported as the bug it would be.
pub fn compute(input: Input, budgets: &[(String, f64)]) -> Result<Report, MetricsError> {
    if input.scores.len() != input.labels.len() {
        return Err(MetricsError::LengthMismatch {
            scores: input.scores.len(),
            labels: input.labels.len(),
        });
    }
    if input.scores.len() > MAX_SCORES {
        return Err(MetricsError::TooManyScores {
            count: input.scores.len(),
        });
    }
    if input.scores.is_empty() {
        return Err(MetricsError::Empty);
    }
    let absent = input.scores.iter().filter(|s| s.is_none()).count();
    if absent > 0 {
        return Err(MetricsError::NotANumber {
            count: absent,
            total: input.scores.len(),
        });
    }
    // JSON cannot spell NaN, so `null` is the only way one arrives and it has
    // already been refused. The guard stays because `compute` is callable
    // without going through JSON and the crate would answer `None` to it,
    // which this function's contract says cannot happen.
    let scores: Vec<f64> = input
        .scores
        .into_iter()
        .map(|s| s.unwrap_or(f64::NAN))
        .collect();
    let nan = scores.iter().filter(|s| s.is_nan()).count();
    if nan > 0 {
        return Err(MetricsError::NotANumber {
            count: nan,
            total: scores.len(),
        });
    }
    let labels = input.labels;
    let n_stego = labels.iter().filter(|l| **l).count();
    let n_clean = labels.len() - n_stego;
    if n_stego == 0 || n_clean == 0 {
        return Err(MetricsError::OneSided {
            clean: n_clean,
            stego: n_stego,
        });
    }

    let auc = stegobench_metrics::roc_auc(&scores, &labels).ok_or(MetricsError::Unrankable {
        count: scores.len(),
        at: String::new(),
    })?;
    let mut tpr_at_fpr = BTreeMap::new();
    for (text, value) in budgets {
        let tpr = stegobench_metrics::tpr_at_fpr(&scores, &labels, *value).ok_or_else(|| {
            MetricsError::Unrankable {
                count: scores.len(),
                at: format!(" at a false-alarm budget of {text}"),
            }
        })?;
        tpr_at_fpr.insert(text.clone(), tpr);
    }
    Ok(Report {
        auc,
        tpr_at_fpr,
        n_clean,
        n_stego,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(scores: &[f64], labels: &[bool]) -> Input {
        Input {
            scores: scores.iter().map(|s| Some(*s)).collect(),
            labels: labels.to_vec(),
        }
    }

    fn budgets(at: &[&str]) -> Vec<(String, f64)> {
        parse_budgets(&at.iter().map(|s| s.to_string()).collect::<Vec<_>>()).expect("rates")
    }

    #[test]
    fn it_reports_the_same_numbers_the_crate_does() {
        let scores = [0.9, 0.4, 0.5, 0.1];
        let labels = [true, true, false, false];
        let report = compute(input(&scores, &labels), &budgets(&["0.0", "0.5"])).expect("ranked");
        assert_eq!(
            report.auc,
            stegobench_metrics::roc_auc(&scores, &labels).unwrap()
        );
        assert_eq!(report.tpr_at_fpr["0.0"], 0.5);
        assert_eq!(report.tpr_at_fpr["0.5"], 1.0);
        assert_eq!((report.n_clean, report.n_stego), (2, 2));
    }

    /// The budget is reported back under the text the caller typed. `0.10` and
    /// `0.1` are the same rate and a caller that asked for one should not have
    /// to guess which spelling came back.
    #[test]
    fn a_budget_answers_under_the_spelling_it_was_asked_in() {
        let report = compute(
            input(&[0.9, 0.4, 0.5, 0.1], &[true, true, false, false]),
            &budgets(&["0.5", "0.50"]),
        )
        .expect("ranked");
        assert_eq!(report.tpr_at_fpr["0.5"], 1.0);
        assert_eq!(report.tpr_at_fpr["0.50"], 1.0);
    }

    #[test]
    fn a_budget_that_is_not_a_rate_is_refused_by_name() {
        for text in ["-0.1", "1.5", "NaN", "one per cent", ""] {
            let e = parse_budgets(&[text.to_string()]).expect_err("refused");
            assert_eq!(e.reason(), "budget-not-a-rate", "{text}");
            assert_eq!(e.exit_code(), exit::USAGE);
            assert!(e.to_string().contains("between 0 and 1"), "{e}");
        }
        // The ends of the range are budgets, not mistakes.
        assert_eq!(budgets(&["0", "1"]).len(), 2);
    }

    #[test]
    fn the_same_budget_twice_is_refused_rather_than_reported_twice() {
        let e = parse_budgets(&["0.01".into(), "0.01".into()]).expect_err("refused");
        assert_eq!(e.reason(), "duplicate-budget");
        assert_eq!(e.exit_code(), exit::USAGE);
    }

    #[test]
    fn a_length_mismatch_is_refused_rather_than_truncated() {
        let e = compute(
            Input {
                scores: vec![Some(0.1), Some(0.2), Some(0.3)],
                labels: vec![true, false],
            },
            &budgets(&["0.01"]),
        )
        .expect_err("refused");
        assert_eq!(e.reason(), "length-mismatch");
        assert_eq!(e.exit_code(), exit::USAGE);
        assert!(
            e.to_string().contains('3') && e.to_string().contains('2'),
            "{e}"
        );
    }

    #[test]
    fn an_empty_set_is_refused_rather_than_scored_zero() {
        let e = compute(
            Input {
                scores: vec![],
                labels: vec![],
            },
            &budgets(&["0.01"]),
        )
        .expect_err("refused");
        assert_eq!(e.reason(), "empty");
        assert_eq!(e.exit_code(), exit::PREFLIGHT_REFUSED);
    }

    #[test]
    fn a_score_the_detector_could_not_produce_is_refused_by_count() {
        let e = compute(
            Input {
                scores: vec![Some(0.9), None, Some(0.2), None],
                labels: vec![true, true, false, false],
            },
            &budgets(&["0.01"]),
        )
        .expect_err("refused");
        assert_eq!(e.reason(), "not-a-number");
        assert_eq!(e.exit_code(), exit::PREFLIGHT_REFUSED);
        assert!(e.to_string().contains("2 of the 4"), "{e}");
    }

    /// `compute` is callable without going through JSON, and JSON is the only
    /// thing that cannot carry a NaN. The guard has to hold for the other path.
    #[test]
    fn a_nan_reaching_compute_directly_is_refused_too() {
        let e = compute(
            input(&[0.9, f64::NAN, 0.2, 0.1], &[true, true, false, false]),
            &budgets(&["0.01"]),
        )
        .expect_err("refused");
        assert_eq!(e.reason(), "not-a-number");
    }

    #[test]
    fn one_class_is_refused_with_both_counts() {
        let e =
            compute(input(&[0.1, 0.2], &[true, true]), &budgets(&["0.01"])).expect_err("refused");
        assert_eq!(e.reason(), "one-sided");
        assert_eq!(e.exit_code(), exit::PREFLIGHT_REFUSED);
        assert!(e.to_string().contains("0 clean and 2 stego"), "{e}");
    }

    #[test]
    fn more_scores_than_the_cap_are_refused_before_they_are_ranked() {
        let e = compute(
            Input {
                scores: vec![Some(0.0); MAX_SCORES + 1],
                labels: vec![true; MAX_SCORES + 1],
            },
            &budgets(&["0.01"]),
        )
        .expect_err("refused");
        assert_eq!(e.reason(), "too-many-scores");
        assert_eq!(e.exit_code(), exit::PREFLIGHT_REFUSED);
    }

    #[test]
    fn a_file_that_is_not_scores_and_labels_says_what_one_looks_like() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("not-scores.json");
        std::fs::write(&path, r#"{"auc": 0.5}"#).expect("write");
        let e = run(Some(&path), &["0.01".to_string()]).expect_err("refused");
        assert_eq!(e.reason(), "not-json");
        assert_eq!(e.exit_code(), exit::USAGE);
        assert!(e.to_string().contains("\"scores\""), "{e}");
    }

    #[test]
    fn a_file_that_is_not_there_names_itself() {
        let e = run(
            Some(Path::new("/nonexistent/scores.json")),
            &["0.01".into()],
        )
        .expect_err("refused");
        assert_eq!(e.reason(), "read-failed");
        assert_eq!(e.exit_code(), exit::FAILURE);
        assert!(e.to_string().contains("scores.json"), "{e}");
    }

    #[test]
    fn a_file_over_the_byte_cap_is_refused_before_it_is_parsed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("huge.json");
        let handle = std::fs::File::create(&path).expect("create");
        // Sparse: the length is what the check reads, and writing a quarter of
        // a gigabyte of real bytes to prove a bound is a slow test.
        handle.set_len(MAX_INPUT_BYTES + 1).expect("set_len");
        drop(handle);
        let e = run(Some(&path), &["0.01".into()]).expect_err("refused");
        assert_eq!(e.reason(), "input-too-large");
        assert_eq!(e.exit_code(), exit::PREFLIGHT_REFUSED);
    }

    #[test]
    fn a_whole_run_from_a_file_produces_the_numbers_and_the_counts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scores.json");
        std::fs::write(
            &path,
            r#"{"scores": [0.9, 0.4, 0.5, 0.1], "labels": [true, true, false, false]}"#,
        )
        .expect("write");
        let report = run(Some(&path), &["0.0".into(), "0.5".into()]).expect("ranked");
        assert_eq!(report.tpr_at_fpr["0.0"], 0.5);
        assert_eq!(report.n_clean, 2);
        let json = report.to_json();
        assert_eq!(json["n"], 4);
        assert_eq!(json["ok"], true);
        assert!(report.human().contains("AUC"), "{}", report.human());
    }

    /// A field nobody meant to send is a mistake worth naming. Ignoring it
    /// silently is how a caller spends an afternoon wondering why `--at` in
    /// the JSON did nothing.
    #[test]
    fn an_unknown_field_in_the_input_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("extra.json");
        std::fs::write(
            &path,
            r#"{"scores": [0.9, 0.1], "labels": [true, false], "at": [0.01]}"#,
        )
        .expect("write");
        let e = run(Some(&path), &["0.01".into()]).expect_err("refused");
        assert_eq!(e.reason(), "not-json");
    }

    /// The operating point that sits exactly on the budget. 1 false alarm in
    /// 100 clean images IS one per cent, and a comparison that dropped it
    /// would report the detection rate of a stricter threshold than the one
    /// the caller asked for.
    #[test]
    fn an_operating_point_exactly_on_the_budget_is_inside_it() {
        // 100 clean, 100 stego. One clean image outranks everything; then 40
        // stego; then the rest. At a budget of 0.01 the single false alarm is
        // affordable and buys those 40.
        let mut scores = Vec::new();
        let mut labels = Vec::new();
        scores.push(1.0);
        labels.push(false);
        for _ in 0..40 {
            scores.push(0.9);
            labels.push(true);
        }
        for _ in 0..99 {
            scores.push(0.5);
            labels.push(false);
        }
        for _ in 0..60 {
            scores.push(0.1);
            labels.push(true);
        }
        let report = compute(input(&scores, &labels), &budgets(&["0.01"])).expect("ranked");
        assert_eq!(report.tpr_at_fpr["0.01"], 0.4);
        assert_eq!((report.n_clean, report.n_stego), (100, 100));
    }

    #[test]
    fn every_reason_is_a_distinct_word() {
        let reasons = [
            MetricsError::Read {
                path: "x".into(),
                source: std::io::Error::other("x"),
            },
            MetricsError::TooLarge { path: "x".into() },
            MetricsError::TooManyScores { count: 1 },
            MetricsError::BudgetNotARate { text: "x".into() },
            MetricsError::DuplicateBudget { text: "x".into() },
            MetricsError::LengthMismatch {
                scores: 1,
                labels: 2,
            },
            MetricsError::Empty,
            MetricsError::NotANumber { count: 1, total: 2 },
            MetricsError::OneSided { clean: 0, stego: 2 },
            MetricsError::Unrankable {
                count: 2,
                at: String::new(),
            },
        ]
        .iter()
        .map(|e| e.reason())
        .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(reasons.len(), 10);
    }
}
