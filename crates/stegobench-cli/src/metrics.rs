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
        "{count} of the {total} answer(s) are not numbers. A score written \
         `null`, or one that is not a number, cannot be ordered, and a ranking \
         that quietly skipped it would be measured on a subset nobody named"
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
    /// DeLong's 95 per cent interval, where both classes have two or more.
    ///
    /// `None` rather than a fabricated range for a run too small to estimate
    /// one: a variance over a single observation is not an estimate, and an
    /// invented interval beside a real AUC is worse than no interval.
    pub auc_ci95: Option<[f64; 2]>,
    /// Detection rate keyed by the budget TEXT the caller typed.
    ///
    /// Keyed by the text rather than by a reformatted number so a caller can
    /// look up what it asked for without knowing how this renders a float:
    /// `--at 0.10` answers under "0.10" and `--at 0.1` under "0.1". Both are
    /// the same budget and both are honoured; neither is silently renamed.
    pub tpr_at_fpr: BTreeMap<String, f64>,
    /// The false-alarm rate each figure above actually came from.
    ///
    /// Keyed identically to `tpr_at_fpr`. A value below its key means the
    /// budget could not be spent: with six clean images the only rates that
    /// exist are multiples of 1/6, so a request for 0.01 is answered at 0.0
    /// and calling the result "TPR at 1% FA" claims a resolution this sample
    /// never had. The figure is right; the label would be wrong without this
    /// beside it.
    pub achieved_fpr: BTreeMap<String, f64>,
    /// The detector score to compare against to reach each budget.
    ///
    /// Keyed identically to `tpr_at_fpr`. Flag an image when its score is at or
    /// above this. Every other figure here says how good the detector is; this
    /// is the one that says what to put in the `if`, and it is what a developer
    /// choosing a cutoff came for.
    ///
    /// A key is absent where the chosen point flags nothing, so there is no
    /// cutoff to give. Raw scores in the detector's own units, which do not
    /// transfer to another tool or another version of the same one.
    pub threshold_at_fpr: BTreeMap<String, f64>,
    pub n_clean: usize,
    pub n_stego: usize,
    /// What the metrics crate found wrong with these scores, if anything.
    ///
    /// Held rather than printed, so the caller decides where it goes: a
    /// diagnostic belongs on stderr beside human output and inside the
    /// document in `--json`, and a function that printed it would get one of
    /// those two wrong.
    pub findings: Vec<stegobench_metrics::Finding>,
}

impl Report {
    /// The findings as sentences, in the register `score` uses for the same
    /// two conditions.
    ///
    /// `metrics` is the documented way to bring your own detector, so it is
    /// the path most likely to be carrying a sign-flipped adapter, and it used
    /// to print `AUC 0` with nothing beside it. The wording lives here rather
    /// than in the metrics crate because that crate has no dependencies and no
    /// opinion about presentation.
    pub fn warnings(&self) -> Vec<String> {
        self.findings
            .iter()
            .map(|f| match *f {
                stegobench_metrics::Finding::EveryScoreIdentical { count } => format!(
                    "every one of the {count} answers was identical, so this AUC is \
                     0.5 by construction and not by measurement: nothing separated \
                     these images"
                ),
                stegobench_metrics::Finding::ScoresRunBackwards { auc, inverted } => format!(
                    "this scored {auc:.4}, which is below the 0.5 a coin flip gets. \
                     That usually means the scores run the wrong way round rather \
                     than that the detector cannot see anything: at {inverted:.4} \
                     they separate these images about as well inverted as they \
                     would upright. Check the adapter's sign before reading this as \
                     a measurement"
                ),
            })
            .collect()
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ok": true,
            "warnings": self.warnings(),
            "auc": self.auc,
            "auc_ci95": self.auc_ci95,
            "tpr_at_fpr": self.tpr_at_fpr,
            "achieved_fpr": self.achieved_fpr,
            "threshold_at_fpr": self.threshold_at_fpr,
            "fpr_resolution": stegobench_metrics::fpr_resolution(self.n_clean),
            "n_clean": self.n_clean,
            "n_stego": self.n_stego,
            "n": self.n_clean + self.n_stego,
        })
    }

    /// What a person reads.
    ///
    /// Four decimals and a bracketed interval, which is what `score` prints
    /// for the identical arithmetic. The two used to disagree: `AUC 1` and
    /// `95% interval 0.5 to 0.5` here against `AUC 1.0000 [1.0000, 1.0000]`
    /// there, so the same number read as two different measurements
    /// depending on which command produced it. Full precision is in `--json`,
    /// which is where a caller doing arithmetic on it should be reading.
    pub fn human(&self) -> String {
        let mut text = format!(
            "{} answer(s): {} clean, {} stego\nAUC            {:.4}{}",
            self.n_clean + self.n_stego,
            self.n_clean,
            self.n_stego,
            self.auc,
            match self.auc_ci95 {
                Some([lo, hi]) => format!("\n95% interval   [{lo:.4}, {hi:.4}]"),
                // Two different reasons, and naming the wrong one sends a
                // reader to fix a sample size that is not the problem.
                None if self.n_clean < 2 || self.n_stego < 2 => {
                    "\n95% interval   not estimated: one class has fewer \
                     than two members"
                        .to_string()
                }
                None => "\n95% interval   not estimated: these answers \
                         separate with nothing left over, so the estimator \
                         has no width to report"
                    .to_string(),
            }
        );
        for (budget, tpr) in &self.tpr_at_fpr {
            text.push_str(&format!("\nTPR at {budget:<7} {tpr:.4}"));
            // The cutoff, printed beside the rate it buys rather than left for
            // the reader to derive. A developer whose whole task was picking one
            // searched for it, found nothing, and reimplemented this sweep.
            if let Some(threshold) = self.threshold_at_fpr.get(budget) {
                text.push_str(&format!("  flag at score >= {threshold}"));
            }
            // Only where the two differ, so an adequate sample reads exactly
            // as it did before and the note means something when it appears.
            if let Some(got) = self.achieved_fpr.get(budget) {
                if let Ok(asked) = budget.parse::<f64>() {
                    if *got + 1e-12 < asked {
                        text.push_str(&format!(
                            "  (actually at a false-alarm rate of {got}: {} clean \
                             image(s) cannot express {budget})",
                            self.n_clean
                        ));
                    }
                }
            }
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
    let report = compute(input, &budgets)?;
    // Said here rather than by the caller because the caller renders either
    // the human text or the JSON and never both, and a warning that only
    // appears in one of the two modes is one somebody integrating a detector
    // will not see. The numbers stay on stdout; this is a diagnostic.
    for warning in report.warnings() {
        eprintln!("{warning}");
    }
    Ok(report)
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

/// Which condition stops a ranking being formed over these scores, if any.
///
/// The ladder every caller checks, in one place. `roc_auc` answers `None` and
/// does not say why, which is the right shape for a metrics crate with no
/// dependencies and the wrong thing to hand a user, so the conditions are told
/// apart here. A second copy of these sentences somewhere else is a second copy
/// that drifts, and one already had.
fn unrankable(scores: &[f64], labels: &[bool]) -> Option<MetricsError> {
    if scores.len() != labels.len() {
        return Some(MetricsError::LengthMismatch {
            scores: scores.len(),
            labels: labels.len(),
        });
    }
    if scores.is_empty() {
        return Some(MetricsError::Empty);
    }
    let nan = scores.iter().filter(|s| s.is_nan()).count();
    if nan > 0 {
        return Some(MetricsError::NotANumber {
            count: nan,
            total: scores.len(),
        });
    }
    let stego = labels.iter().filter(|l| **l).count();
    let clean = labels.len() - stego;
    if clean == 0 || stego == 0 {
        return Some(MetricsError::OneSided { clean, stego });
    }
    None
}

/// Why a ranking could not be formed, for a caller that already knows it could
/// not be.
///
/// [`unrankable`] names the four conditions that can be diagnosed. Anything
/// else is a case this did not anticipate and is reported as the bug it would
/// be rather than dressed up as one of the four. `at` names the operating
/// point, where there is one, and is empty for the AUC itself.
pub fn why_unrankable(scores: &[f64], labels: &[bool], at: &str) -> MetricsError {
    unrankable(scores, labels).unwrap_or_else(|| MetricsError::Unrankable {
        count: scores.len(),
        at: at.to_string(),
    })
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
    // JSON cannot spell NaN, so `null` is how an absent answer arrives, and it
    // becomes the NaN the ladder below refuses by count. `compute` is also
    // callable without going through JSON, where a real NaN arrives instead,
    // and the two reach the same refusal by the same route.
    let scores: Vec<f64> = input
        .scores
        .into_iter()
        .map(|s| s.unwrap_or(f64::NAN))
        .collect();
    let labels = input.labels;
    if let Some(e) = unrankable(&scores, &labels) {
        return Err(e);
    }
    let n_stego = labels.iter().filter(|l| **l).count();
    let n_clean = labels.len() - n_stego;

    let auc_ci95 = stegobench_metrics::roc_auc_interval(&scores, &labels, stegobench_metrics::Z_95)
        .map(|ci| [ci.low, ci.high]);
    let auc = stegobench_metrics::roc_auc(&scores, &labels)
        .ok_or_else(|| why_unrankable(&scores, &labels, ""))?;
    let mut tpr_at_fpr = BTreeMap::new();
    let mut achieved_fpr = BTreeMap::new();
    let mut threshold_at_fpr = BTreeMap::new();
    for (text, value) in budgets {
        let tpr = stegobench_metrics::tpr_at_fpr(&scores, &labels, *value).ok_or_else(|| {
            why_unrankable(
                &scores,
                &labels,
                &format!(" at a false-alarm budget of {text}"),
            )
        })?;
        tpr_at_fpr.insert(text.clone(), tpr);
        if let Some(point) = stegobench_metrics::operating_point(&scores, &labels, *value) {
            achieved_fpr.insert(text.clone(), point.achieved_fpr);
            if let Some(threshold) = point.threshold {
                threshold_at_fpr.insert(text.clone(), threshold);
            }
        }
    }
    Ok(Report {
        auc,
        auc_ci95,
        tpr_at_fpr,
        achieved_fpr,
        threshold_at_fpr,
        n_clean,
        n_stego,
        findings: stegobench_metrics::findings(&scores, &labels),
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

    /// The path a stranger's detector arrives on used to print `AUC 0` with
    /// nothing beside it, while `score` said at length that an AUC that low
    /// means an adapter wired up backwards.
    #[test]
    fn an_inverted_detector_is_warned_about_the_way_score_warns_about_it() {
        let report = compute(
            input(&[0.1, 0.2, 0.8, 0.9], &[true, true, false, false]),
            &[],
        )
        .expect("ranked");
        assert_eq!(report.auc, 0.0);
        let said = report.warnings();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].contains("wrong way round"), "{said:?}");
        assert!(said[0].contains("0.0000"), "{said:?}");
        assert!(said[0].contains("1.0000"), "{said:?}");
        // And the JSON carries it, because a caller reading `--json` reads
        // nothing else.
        assert_eq!(
            report.to_json()["warnings"],
            serde_json::json!(said),
            "the warning is missing from the machine-readable output"
        );
    }

    #[test]
    fn one_answer_to_everything_is_warned_about_rather_than_read_as_chance() {
        let report = compute(
            input(&[0.5, 0.5, 0.5, 0.5], &[true, true, false, false]),
            &[],
        )
        .expect("ranked");
        assert_eq!(report.auc, 0.5);
        let said = report.warnings();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].contains("identical"), "{said:?}");
        assert!(said[0].contains("by construction"), "{said:?}");
        assert!(said[0].contains('4'), "{said:?}");
    }

    /// A warning that fires every time is one a reader stops seeing.
    #[test]
    fn a_detector_that_separates_the_images_is_not_warned_about() {
        let report = compute(
            input(&[0.9, 0.8, 0.2, 0.1], &[true, true, false, false]),
            &budgets(&["0.5"]),
        )
        .expect("ranked");
        assert!(report.warnings().is_empty(), "{:?}", report.warnings());
        assert_eq!(report.to_json()["warnings"], serde_json::json!([]));
        // The numbers are the payload and stay clear of the diagnostics.
        assert!(!report.human().contains("way round"), "{}", report.human());
    }

    /// `metrics` printed `AUC 1` and `95% interval 0.5 to 0.5` where `score`
    /// printed `AUC 1.0000 [1.0000, 1.0000]` for the identical arithmetic, so
    /// the same number read as two different measurements depending on which
    /// command produced it.
    #[test]
    fn a_number_is_rendered_the_way_score_renders_it() {
        let input = Input {
            scores: vec![Some(0.9), Some(0.1), Some(0.8), Some(0.2)],
            labels: vec![true, false, true, false],
        };
        let report = compute(input, &[("0.50".to_string(), 0.5)]).expect("rankable");
        let text = report.human();
        assert!(text.contains("AUC            1.0000"), "{text}");
        assert!(text.contains("TPR at 0.50    1.0000"), "{text}");
        // These four answers separate completely, so there is no interval to
        // print. What must NOT appear is a 95% interval of zero width, which
        // is what both commands used to show.
        assert!(text.contains("95% interval   not estimated"), "{text}");
        assert!(text.contains("separate with nothing left over"), "{text}");
        assert!(!text.contains("fewer than two members"), "{text}");

        // A sample with room in it still prints a real interval here, so the
        // rendering itself is still the same as `score`'s.
        let spread = compute(
            Input {
                // Deliberately overlapping: one stego image scores below
                // two clean ones, so the classes do not separate and the
                // estimator has something to measure.
                scores: vec![
                    Some(0.9),
                    Some(0.5),
                    Some(0.4),
                    Some(0.2),
                    Some(0.8),
                    Some(0.7),
                ],
                labels: vec![true, false, true, false, true, false],
            },
            &[],
        )
        .expect("rankable");
        assert!(
            spread.human().contains("95% interval   ["),
            "{}",
            spread.human()
        );
        // Full precision is still one flag away, for anybody doing
        // arithmetic on it.
        assert_eq!(report.to_json()["auc"], serde_json::json!(1.0));
    }
}
