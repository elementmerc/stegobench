// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//
// Migrated from Stegcore's `stegcore-ops` crate, where it was dual licensed.
// It is relicensed AGPL-only here by its author. The code is unchanged apart
// from this header: it arrives with its thirteen tests, and a metric that has
// been producing published numbers is not the place to start rewriting.
//
// It lives in the benchmark rather than in Stegcore because the benchmark must
// not be built out of the product it judges.

//! Detection metrics for the comparative benchmark.
//!
//! Pure functions over labels and scores: a binary confusion matrix (positive
//! = stego), the rank-based ROC AUC (Mann-Whitney, tie-aware), and the ROC
//! curve points. These are deliberately storage- and tool-agnostic so the
//! same code scores Stegcore's ensemble, any single detector, or an external
//! comparator.

use std::cmp::Ordering;
use std::fmt;

/// Why a confusion count could not be taken honestly.
///
/// Hand written rather than derived, because this crate has no dependencies on
/// purpose: a metric whose number needs an audit of somebody else's crate before
/// it can be trusted is not the metric this benchmark wants to publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TallyError {
    /// The two slices were built from different record sets.
    LengthMismatch {
        /// How many labels the caller handed over.
        labels: usize,
        /// How many predictions the caller handed over.
        predicted_positive: usize,
    },
    /// Nothing was classified at all.
    Empty,
}

impl fmt::Display for TallyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TallyError::LengthMismatch {
                labels,
                predicted_positive,
            } => write!(
                f,
                "{labels} labels against {predicted_positive} predictions: the \
                 two were built from different record sets, so any count over \
                 them would describe images nobody named"
            ),
            TallyError::Empty => write!(
                f,
                "nothing to count: an empty confusion matrix reads as a \
                 detector that got everything wrong rather than one that was \
                 never run"
            ),
        }
    }
}

impl std::error::Error for TallyError {}

/// Binary classification counts. Positive is "stego".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Confusion {
    pub tp: u64,
    pub fp: u64,
    pub tn: u64,
    pub fn_: u64,
}

impl Confusion {
    /// Tally counts from per-sample labels and predicted-positive flags,
    /// **truncating to the shorter slice**.
    ///
    /// Prefer [`Confusion::try_tally`], which refuses that case instead. This
    /// one walks the two slices pairwise and stops at whichever runs out first,
    /// so a caller that built the labels and the predictions from different
    /// record sets gets a count over a subset nobody named, with nothing in the
    /// return value to say so. It is kept, undeprecated, only because it is
    /// published and callers depend on the signature.
    pub fn tally(labels: &[bool], predicted_positive: &[bool]) -> Self {
        let mut c = Confusion::default();
        for (&label, &pred) in labels.iter().zip(predicted_positive) {
            match (label, pred) {
                (true, true) => c.tp += 1,
                (false, true) => c.fp += 1,
                (false, false) => c.tn += 1,
                (true, false) => c.fn_ += 1,
            }
        }
        c
    }

    /// Tally counts from per-sample labels and predicted-positive flags,
    /// refusing anything that cannot honestly produce a count.
    ///
    /// This is [`Confusion::tally`] with the truncation taken out. A length
    /// mismatch means the labels and the predictions came from different record
    /// sets, and walking the shorter of the two publishes a figure measured on
    /// images nobody named. An empty pair of slices is refused too: every rate
    /// on [`Confusion`] answers 0.0 on a zero denominator, so an empty count
    /// reads as a detector that caught nothing rather than one that was never
    /// run.
    ///
    /// A single class is **not** refused, unlike [`roc_auc`] and [`roc_curve`].
    /// A ranking needs both a positive and a negative to rank against each
    /// other; a confusion count does not, and a one-sided run is a real
    /// measurement here. The `must_clear` half of a detector's self test is
    /// exactly that: clean images only, where the whole question is how many
    /// false positives came back.
    ///
    /// NaN has no analogue to refuse: both inputs are `bool`, so there is no
    /// unordered or absent value a detector could hand over. A detector that
    /// failed to score an image must be left out of both slices rather than
    /// given a placeholder flag, and the length check is what catches a caller
    /// that dropped it from one slice and not the other.
    pub fn try_tally(labels: &[bool], predicted_positive: &[bool]) -> Result<Self, TallyError> {
        if labels.len() != predicted_positive.len() {
            return Err(TallyError::LengthMismatch {
                labels: labels.len(),
                predicted_positive: predicted_positive.len(),
            });
        }
        if labels.is_empty() {
            return Err(TallyError::Empty);
        }
        Ok(Self::tally(labels, predicted_positive))
    }

    fn ratio(num: u64, den: u64) -> f64 {
        if den == 0 {
            0.0
        } else {
            num as f64 / den as f64
        }
    }

    /// True-positive rate (detection rate / recall): tp / (tp + fn).
    pub fn tpr(&self) -> f64 {
        Self::ratio(self.tp, self.tp + self.fn_)
    }

    /// False-positive rate: fp / (fp + tn).
    pub fn fpr(&self) -> f64 {
        Self::ratio(self.fp, self.fp + self.tn)
    }

    /// Precision: tp / (tp + fp).
    pub fn precision(&self) -> f64 {
        Self::ratio(self.tp, self.tp + self.fp)
    }

    /// Accuracy: (tp + tn) / total.
    pub fn accuracy(&self) -> f64 {
        Self::ratio(self.tp + self.tn, self.tp + self.tn + self.fp + self.fn_)
    }

    /// F1 score: harmonic mean of precision and recall; 0 when both are 0.
    pub fn f1(&self) -> f64 {
        let (p, r) = (self.precision(), self.tpr());
        if p + r == 0.0 {
            0.0
        } else {
            2.0 * p * r / (p + r)
        }
    }
}

fn cmp_f64(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

/// Whether these two slices can be ranked against each other at all.
///
/// A length mismatch means the caller built the scores and the labels from
/// different record sets, and a ranking over the shorter of the two is a number
/// measured on a subset nobody named. A score that is not a number cannot be
/// ordered, so every comparison against it is false and the ranking silently
/// stops meaning anything. Both answer `None` rather than a figure, because in
/// a measurement tool a quietly wrong number is worse than no number: it gets
/// published.
fn rankable(scores: &[f64], labels: &[bool]) -> bool {
    scores.len() == labels.len() && scores.iter().all(|s| !s.is_nan())
}

/// Rank-based ROC AUC (equivalent to the Mann-Whitney U statistic), tie-aware
/// via average ranks.
///
/// Returns `None` when one class is absent, since AUC is undefined without both
/// a positive and a negative sample; when the two slices are of different
/// lengths; and when any score is NaN.
pub fn roc_auc(scores: &[f64], labels: &[bool]) -> Option<f64> {
    if !rankable(scores, labels) {
        return None;
    }
    let n_pos = labels.iter().filter(|&&l| l).count();
    let n_neg = labels.len() - n_pos;
    if n_pos == 0 || n_neg == 0 {
        return None;
    }

    // Indices sorted by ascending score.
    let mut idx: Vec<usize> = (0..scores.len()).collect();
    idx.sort_by(|&a, &b| cmp_f64(scores[a], scores[b]));

    // Average ranks (1-based), tie groups share the mean of their positions.
    let mut ranks = vec![0.0f64; scores.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && scores[idx[j + 1]] == scores[idx[i]] {
            j += 1;
        }
        let avg = ((i + 1) + (j + 1)) as f64 / 2.0;
        for &k in &idx[i..=j] {
            ranks[k] = avg;
        }
        i = j + 1;
    }

    let sum_pos: f64 = labels
        .iter()
        .zip(&ranks)
        .filter(|(&l, _)| l)
        .map(|(_, &r)| r)
        .sum();
    let auc = (sum_pos - (n_pos * (n_pos + 1)) as f64 / 2.0) / (n_pos as f64 * n_neg as f64);
    Some(auc)
}

/// ROC curve as `(fpr, tpr)` points, swept from the highest score downward
/// (predicted positive when `score >= threshold`). Begins at `(0, 0)` and ends
/// at `(1, 1)`.
///
/// Empty when either class is absent, when the two slices are of different
/// lengths, and when any score is NaN. See [`roc_auc`].
pub fn roc_curve(scores: &[f64], labels: &[bool]) -> Vec<(f64, f64)> {
    if !rankable(scores, labels) {
        return Vec::new();
    }
    let n_pos = labels.iter().filter(|&&l| l).count() as f64;
    let n_neg = labels.len() as f64 - n_pos;
    if n_pos == 0.0 || n_neg == 0.0 {
        return Vec::new();
    }

    let mut pairs: Vec<(f64, bool)> = scores.iter().copied().zip(labels.iter().copied()).collect();
    pairs.sort_by(|a, b| cmp_f64(b.0, a.0)); // descending score

    let mut curve = vec![(0.0, 0.0)];
    let (mut tp, mut fp) = (0.0f64, 0.0f64);
    let mut i = 0;
    while i < pairs.len() {
        let thr = pairs[i].0;
        // Advance over every sample at this threshold before recording a point,
        // so tied scores collapse to a single ROC vertex.
        while i < pairs.len() && pairs[i].0 == thr {
            if pairs[i].1 {
                tp += 1.0;
            } else {
                fp += 1.0;
            }
            i += 1;
        }
        curve.push((fp / n_neg, tp / n_pos));
    }
    curve
}

/// The best true-positive rate this detector reaches without exceeding
/// `max_fpr`.
///
/// `None` when the curve cannot be drawn (see [`roc_curve`]) and when `max_fpr`
/// is not a false-alarm budget: a negative, a NaN or a figure above 1 is a
/// caller's mistake rather than a strict budget, and answering 0.0 to it would
/// read as a detector that caught nothing.
///
/// ONE SWEEP, NOT ONE PASS PER THRESHOLD
///
/// The obvious shape of this is a loop over every distinct score that counts
/// the hits above it, and a copy of this metric elsewhere in the project was
/// written that way. It is quadratic, and the scores are floats a detector
/// produced, so "distinct" means nearly all of them: an arm of 344,357 images
/// made it around a hundred billion comparisons, which is not slow, it is a
/// run that never ends. [`roc_curve`] walks the scores once in descending
/// order and accumulates counts, which gives the same answer, ties included,
/// for the cost of the sort.
///
/// This is the headline number for an external evaluation and accuracy is not.
/// A detector facing a corpus that is mostly clean can score 95% accuracy by
/// answering "clean" every time, and a false-positive rate chosen after seeing
/// the results is not a measurement. Pinning the false-positive budget first and
/// asking what detection it buys is the comparison that survives review.
pub fn tpr_at_fpr(scores: &[f64], labels: &[bool], max_fpr: f64) -> Option<f64> {
    if !(0.0..=1.0).contains(&max_fpr) {
        return None;
    }
    let curve = roc_curve(scores, labels);
    if curve.is_empty() {
        return None;
    }
    // The curve is a step function, so the answer is the highest point whose
    // false-positive rate is still inside the budget. The epsilon absorbs the
    // representation error in ratios like 1/3, which would otherwise drop an
    // operating point that is exactly on the limit.
    let best = curve
        .iter()
        .filter(|&&(fpr, _)| fpr <= max_fpr + 1e-12)
        .map(|&(_, tpr)| tpr)
        .fold(f64::NEG_INFINITY, f64::max);
    Some(if best.is_finite() { best } else { 0.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tpr_at_fpr_perfect_separation_is_one_at_zero_budget() {
        let scores = [0.1, 0.2, 0.8, 0.9];
        let labels = [false, false, true, true];
        assert_eq!(tpr_at_fpr(&scores, &labels, 0.0), Some(1.0));
    }

    #[test]
    fn tpr_at_fpr_tightens_as_the_budget_shrinks() {
        // 2 pos (0.9, 0.4), 2 neg (0.5, 0.1). Catching the 0.4 positive means
        // first admitting the 0.5 negative, so a zero-FPR budget buys half.
        let scores = [0.9, 0.4, 0.5, 0.1];
        let labels = [true, true, false, false];
        assert_eq!(tpr_at_fpr(&scores, &labels, 0.0), Some(0.5));
        assert_eq!(tpr_at_fpr(&scores, &labels, 0.5), Some(1.0));
    }

    #[test]
    fn tpr_at_fpr_is_zero_when_the_budget_buys_nothing() {
        // Every negative outranks every positive: no detection at zero FPR.
        let scores = [0.1, 0.2, 0.8, 0.9];
        let labels = [true, true, false, false];
        assert_eq!(tpr_at_fpr(&scores, &labels, 0.0), Some(0.0));
    }

    #[test]
    fn tpr_at_fpr_undefined_with_single_class() {
        assert_eq!(tpr_at_fpr(&[0.1, 0.2], &[true, true], 0.01), None);
    }

    /// A detector that only answers yes or no has two operating points, so the
    /// budget either admits its false positives or refuses them outright. This
    /// is exactly why `Detector::is_graded` exists: the number is real, but it
    /// cannot be traded off the way a graded detector's can.
    #[test]
    fn tpr_at_fpr_handles_a_binary_detector() {
        let scores = [1.0, 1.0, 1.0, 0.0];
        let labels = [true, true, false, false];
        assert_eq!(tpr_at_fpr(&scores, &labels, 0.0), Some(0.0));
        assert_eq!(tpr_at_fpr(&scores, &labels, 0.5), Some(1.0));
    }

    #[test]
    fn confusion_counts_and_rates() {
        // labels:  S S S C C   (3 stego, 2 clean)
        // preds:   1 1 0 1 0
        let labels = [true, true, true, false, false];
        let preds = [true, true, false, true, false];
        let c = Confusion::tally(&labels, &preds);
        assert_eq!((c.tp, c.fn_, c.fp, c.tn), (2, 1, 1, 1));
        assert!((c.tpr() - 2.0 / 3.0).abs() < 1e-12);
        assert!((c.fpr() - 0.5).abs() < 1e-12);
        assert!((c.precision() - 2.0 / 3.0).abs() < 1e-12);
        assert!((c.accuracy() - 3.0 / 5.0).abs() < 1e-12);
        assert!((c.f1() - 2.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn confusion_zero_denominators_are_zero_not_nan() {
        let c = Confusion::default();
        assert_eq!(c.tpr(), 0.0);
        assert_eq!(c.fpr(), 0.0);
        assert_eq!(c.precision(), 0.0);
        assert_eq!(c.accuracy(), 0.0);
        assert_eq!(c.f1(), 0.0);
    }

    /// The honest counterpart agrees with the truncating one whenever the
    /// truncating one had nothing to truncate.
    #[test]
    fn try_tally_matches_tally_on_well_formed_input() {
        let labels = [true, true, true, false, false];
        let preds = [true, true, false, true, false];
        assert_eq!(
            Confusion::try_tally(&labels, &preds),
            Ok(Confusion::tally(&labels, &preds))
        );
    }

    /// Labels and predictions of different lengths came from different record
    /// sets. `tally` walks the shorter one; this refuses, which is the whole
    /// reason for having both.
    #[test]
    fn try_tally_refuses_a_length_mismatch_in_either_direction() {
        let labels = [true, true, false, false];
        assert_eq!(
            Confusion::try_tally(&labels, &[true, false, true]),
            Err(TallyError::LengthMismatch {
                labels: 4,
                predicted_positive: 3
            })
        );
        assert_eq!(
            Confusion::try_tally(&labels[..3], &[true, false, true, false]),
            Err(TallyError::LengthMismatch {
                labels: 3,
                predicted_positive: 4
            })
        );
    }

    /// Zero samples is not a score of zero. Every rate on `Confusion` answers
    /// 0.0 on a zero denominator, so an empty count is indistinguishable from a
    /// detector that missed everything.
    #[test]
    fn try_tally_refuses_an_empty_run() {
        assert_eq!(Confusion::try_tally(&[], &[]), Err(TallyError::Empty));
    }

    /// One class is a real measurement here, unlike a ranking. `must_clear` in
    /// a detector's self test hands over clean images only, and the false
    /// positives that come back are the answer it wanted.
    #[test]
    fn try_tally_accepts_a_single_class() {
        let clean_only = Confusion::try_tally(&[false, false, false], &[true, false, false]);
        assert_eq!(
            clean_only,
            Ok(Confusion {
                tp: 0,
                fp: 1,
                tn: 2,
                fn_: 0
            })
        );
        let stego_only = Confusion::try_tally(&[true, true], &[true, false]);
        assert_eq!(
            stego_only,
            Ok(Confusion {
                tp: 1,
                fp: 0,
                tn: 0,
                fn_: 1
            })
        );
    }

    /// The point of adding a second function rather than changing the first:
    /// `tally` still truncates, and a caller depending on that still gets it.
    #[test]
    fn tally_still_truncates_exactly_as_before() {
        let labels = [true, true, false, false];
        let preds = [true, false, true];
        let c = Confusion::tally(&labels, &preds);
        assert_eq!((c.tp, c.fn_, c.fp, c.tn), (1, 1, 1, 0));
        assert_eq!(Confusion::tally(&[], &[]), Confusion::default());
        assert_eq!(
            Confusion::tally(&labels, &[]),
            Confusion::default(),
            "an empty prediction slice truncates to nothing rather than panicking"
        );
    }

    /// The error has to survive being turned into a message a user reads, so
    /// each variant says which condition fired and names the figures.
    #[test]
    fn a_tally_error_says_which_condition_fired() {
        let mismatch = TallyError::LengthMismatch {
            labels: 9_882,
            predicted_positive: 10_000,
        };
        let text = mismatch.to_string();
        assert!(text.contains("9882"), "{text}");
        assert!(text.contains("10000"), "{text}");
        assert!(text.contains("different record sets"), "{text}");

        let empty = TallyError::Empty.to_string();
        assert!(empty.contains("nothing to count"), "{empty}");

        // Usable as a `std::error::Error`, so a caller can box it like any
        // other failure rather than matching on it by hand.
        let boxed: Box<dyn std::error::Error> = Box::new(mismatch);
        assert_eq!(boxed.to_string(), text);
    }

    #[test]
    fn auc_perfect_separation_is_one() {
        let scores = [0.1, 0.2, 0.8, 0.9];
        let labels = [false, false, true, true];
        assert_eq!(roc_auc(&scores, &labels), Some(1.0));
    }

    #[test]
    fn auc_reversed_is_zero_and_random_is_half() {
        let scores = [0.9, 0.8, 0.2, 0.1];
        let labels = [false, false, true, true];
        assert_eq!(roc_auc(&scores, &labels), Some(0.0));

        // Perfectly interleaved with ties handled by average rank → 0.5.
        let s = [0.5, 0.5, 0.5, 0.5];
        let l = [true, false, true, false];
        assert_eq!(roc_auc(&s, &l), Some(0.5));
    }

    #[test]
    fn auc_undefined_with_single_class() {
        assert_eq!(roc_auc(&[0.1, 0.2], &[true, true]), None);
        assert_eq!(roc_auc(&[0.1, 0.2], &[false, false]), None);
    }

    #[test]
    fn auc_matches_manual_small_case() {
        // 2 pos, 2 neg; one pos below one neg → AUC = 3/4.
        let scores = [0.3, 0.4, 0.35, 0.2];
        let labels = [true, true, false, false];
        let auc = roc_auc(&scores, &labels).unwrap();
        assert!((auc - 0.75).abs() < 1e-12, "auc={auc}");
    }

    #[test]
    fn roc_curve_reaches_corners() {
        let scores = [0.1, 0.2, 0.8, 0.9];
        let labels = [false, false, true, true];
        let curve = roc_curve(&scores, &labels);
        assert_eq!(curve.first(), Some(&(0.0, 0.0)));
        assert_eq!(curve.last(), Some(&(1.0, 1.0)));
        // Perfect separation: tpr hits 1.0 before any fpr accrues.
        assert!(curve.iter().any(|&(fpr, tpr)| fpr == 0.0 && tpr == 1.0));
    }

    #[test]
    fn roc_curve_empty_without_both_classes() {
        assert!(roc_curve(&[0.1, 0.2], &[true, true]).is_empty());
    }

    /// Scores and labels of different lengths mean they were built from
    /// different record sets. Walking the shorter of the two would answer with a
    /// figure measured on a subset nobody named, and the caller would have no
    /// way of telling that from a real one.
    #[test]
    fn a_length_mismatch_is_refused_rather_than_truncated() {
        let scores = [0.9, 0.8, 0.2, 0.1];
        assert_eq!(roc_auc(&scores, &[true, false, true]), None);
        assert_eq!(roc_auc(&scores[..3], &[true, false, true, false]), None);
        assert!(roc_curve(&scores, &[true, false, true]).is_empty());
        assert_eq!(tpr_at_fpr(&scores, &[true, false, true], 0.01), None);
    }

    /// A detector that answers NaN for an image has not scored it. NaN compares
    /// false against everything, so it lands wherever the sort happens to leave
    /// it and the rank sum built on top of that is arbitrary.
    #[test]
    fn a_nan_score_is_refused_rather_than_ranked() {
        let scores = [0.9, f64::NAN, 0.2, 0.1];
        let labels = [true, true, false, false];
        assert_eq!(roc_auc(&scores, &labels), None);
        assert!(roc_curve(&scores, &labels).is_empty());
        assert_eq!(tpr_at_fpr(&scores, &labels, 0.01), None);
    }

    /// A budget that is not a rate is a caller's mistake, and 0.0 would read as
    /// a detector that caught nothing at a budget it was never asked about.
    #[test]
    fn a_budget_that_is_not_a_rate_is_refused() {
        let scores = [0.9, 0.4, 0.5, 0.1];
        let labels = [true, true, false, false];
        assert_eq!(tpr_at_fpr(&scores, &labels, -0.1), None);
        assert_eq!(tpr_at_fpr(&scores, &labels, 1.5), None);
        assert_eq!(tpr_at_fpr(&scores, &labels, f64::NAN), None);
        // The ends of the range are budgets, not mistakes.
        assert_eq!(tpr_at_fpr(&scores, &labels, 0.0), Some(0.5));
        assert_eq!(tpr_at_fpr(&scores, &labels, 1.0), Some(1.0));
    }
}
