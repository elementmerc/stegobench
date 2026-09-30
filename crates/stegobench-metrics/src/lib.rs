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

/// An AUC with the uncertainty that belongs beside it.
///
/// A bare AUC printed to sixteen digits invites a reader to compare two
/// numbers that differ in the third, on corpora small enough that one image
/// moves the figure further than that. The interval is the tool saying how
/// much of its own answer is real.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AucInterval {
    /// The point estimate, identical to [`roc_auc`].
    pub auc: f64,
    /// DeLong's standard error.
    pub standard_error: f64,
    /// Lower bound, clamped into `[0, 1]` because AUC cannot leave it.
    pub low: f64,
    /// Upper bound, clamped the same way.
    pub high: f64,
    /// How many standard errors wide, so a reader knows what was asked for.
    pub z: f64,
}

/// 1.959964, the two-sided normal quantile for 95 per cent.
///
/// Written out rather than computed: there is no inverse normal in the
/// standard library, and this crate has no dependencies on purpose (a metric
/// with a dependency graph is one somebody has to audit before trusting a
/// number).
pub const Z_95: f64 = 1.959_963_984_540_054;

/// Average ranks of `values` among themselves, 1-based, ties sharing the mean
/// of the positions they span.
///
/// Split out because DeLong needs the same computation three times over three
/// different sets, and a second copy of a tie rule is how two of them come to
/// disagree.
fn midranks(values: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..values.len()).collect();
    idx.sort_by(|&a, &b| cmp_f64(values[a], values[b]));
    let mut ranks = vec![0.0f64; values.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && values[idx[j + 1]] == values[idx[i]] {
            j += 1;
        }
        let avg = ((i + 1) + (j + 1)) as f64 / 2.0;
        for &k in &idx[i..=j] {
            ranks[k] = avg;
        }
        i = j + 1;
    }
    ranks
}

/// AUC with a confidence interval, by DeLong's method.
///
/// WHY DELONG AND NOT HANLEY AND MCNEIL
///
/// The Hanley and McNeil standard error is two lines and assumes the scores
/// are exponentially distributed within each class. Detector scores are not:
/// several of the tools in this registry emit a bounded statistic, one emits
/// a count, and one answers the same number to everything. Under those the
/// parametric approximation is wrong in a direction nobody can predict from
/// the output. DeLong's estimator makes no such assumption; it is computed
/// from the data's own ranks and is what the statistical literature treats as
/// the default for comparing classifiers.
///
/// Computed through midranks rather than the O(n_pos * n_neg) definition, so
/// it stays O(n log n) and a Core tier run pays sorting rather than a hundred
/// billion comparisons.
///
/// `z` is how many standard errors wide the interval is: [`Z_95`] for the
/// usual 95 per cent.
///
/// Returns `None` for everything [`roc_auc`] returns `None` for, and also
/// when either class has fewer than two members, because a variance over one
/// observation is not an estimate of anything.
pub fn roc_auc_interval(scores: &[f64], labels: &[bool], z: f64) -> Option<AucInterval> {
    let auc = roc_auc(scores, labels)?;
    if !z.is_finite() || z < 0.0 {
        return None;
    }

    let pos: Vec<f64> = scores
        .iter()
        .zip(labels)
        .filter(|(_, &l)| l)
        .map(|(&s, _)| s)
        .collect();
    let neg: Vec<f64> = scores
        .iter()
        .zip(labels)
        .filter(|(_, &l)| !l)
        .map(|(&s, _)| s)
        .collect();
    let (m, n) = (pos.len(), neg.len());
    if m < 2 || n < 2 {
        return None;
    }

    // Ranks of each class within itself, and of everything within everything.
    let t_x = midranks(&pos);
    let t_y = midranks(&neg);
    let mut all = Vec::with_capacity(m + n);
    all.extend_from_slice(&pos);
    all.extend_from_slice(&neg);
    let t_z = midranks(&all);

    // DeLong's structural components. V10 is, for each positive, the fraction
    // of negatives it beats; V01 the mirror. Both fall out of the difference
    // between a point's rank among everything and its rank among its own
    // class, which is why this costs a sort rather than a product.
    let v10: Vec<f64> = (0..m).map(|i| (t_z[i] - t_x[i]) / n as f64).collect();
    let v01: Vec<f64> = (0..n)
        .map(|j| 1.0 - (t_z[m + j] - t_y[j]) / m as f64)
        .collect();

    let s10 = sample_variance(&v10)?;
    let s01 = sample_variance(&v01)?;
    let var = s10 / m as f64 + s01 / n as f64;
    if !var.is_finite() || var < 0.0 {
        return None;
    }
    let se = var.sqrt();

    Some(AucInterval {
        auc,
        standard_error: se,
        // Clamped, because the normal interval runs past the ends of the
        // scale near 1.0 and an upper bound of 1.03 is not a thing anybody
        // should print beside a measurement.
        low: (auc - z * se).clamp(0.0, 1.0),
        high: (auc + z * se).clamp(0.0, 1.0),
        z,
    })
}

/// Unbiased sample variance, or `None` for fewer than two observations.
fn sample_variance(v: &[f64]) -> Option<f64> {
    if v.len() < 2 {
        return None;
    }
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    Some(v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0))
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

/// The point on the curve that [`tpr_at_fpr`] actually reported.
///
/// WHY THIS EXISTS, AND IT IS NOT A CONVENIENCE
///
/// A false-alarm budget finer than one clean image cannot be spent. With six
/// clean images the only false-alarm rates that exist are 0, 1/6, 2/6 and so
/// on, so a budget of 0.01 buys exactly what a budget of 0 buys, and
/// `tpr_at_fpr` correctly returns the zero-budget answer. What it cannot do
/// is say so, and the caller then prints that number under the heading
/// "TPR@1%FA", which claims a resolution the measurement never had.
///
/// An engineer picking a review threshold reads that column and nothing else.
/// So this returns the rate that was actually achieved beside the one that
/// was asked for, and the two differing is the signal that the corpus is too
/// small for the question.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OperatingPoint {
    /// The detection rate at this point.
    pub tpr: f64,
    /// The false-alarm rate this point actually sits at, which is a multiple
    /// of `1 / n_clean` and is never above `requested`.
    pub achieved_fpr: f64,
    /// The budget the caller asked for.
    pub requested_fpr: f64,
}

impl OperatingPoint {
    /// Whether the corpus could express the budget that was asked for.
    ///
    /// False means the answer is real but the label is not: the figure is the
    /// detection rate at `achieved_fpr`, not at `requested_fpr`.
    pub fn budget_was_expressible(&self, n_clean: usize) -> bool {
        n_clean > 0 && fpr_resolution(n_clean) <= self.requested_fpr + 1e-12
    }
}

/// The smallest false-alarm rate a corpus of this many clean images can show.
///
/// One clean image wrongly flagged out of `n_clean`. Anything finer is a
/// budget the sample cannot spend.
pub fn fpr_resolution(n_clean: usize) -> f64 {
    if n_clean == 0 {
        return f64::INFINITY;
    }
    1.0 / n_clean as f64
}

/// [`tpr_at_fpr`], plus the rate the answer actually came from.
pub fn operating_point(scores: &[f64], labels: &[bool], max_fpr: f64) -> Option<OperatingPoint> {
    if !(0.0..=1.0).contains(&max_fpr) {
        return None;
    }
    let curve = roc_curve(scores, labels);
    if curve.is_empty() {
        return None;
    }
    // The same point `tpr_at_fpr` picks: highest detection rate inside the
    // budget. Ties on tpr take the lowest fpr, because two points with the
    // same detection rate are the same answer bought more or less cheaply,
    // and reporting the dearer one would overstate what the budget cost.
    let mut best: Option<(f64, f64)> = None;
    for &(fpr, tpr) in curve.iter().filter(|&&(f, _)| f <= max_fpr + 1e-12) {
        best = Some(match best {
            None => (fpr, tpr),
            Some((bf, bt)) if tpr > bt || (tpr == bt && fpr < bf) => (fpr, tpr),
            Some(b) => b,
        });
    }
    let (achieved_fpr, tpr) = best.unwrap_or((0.0, 0.0));
    Some(OperatingPoint {
        tpr,
        achieved_fpr,
        requested_fpr: max_fpr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_budget_finer_than_one_clean_image_is_reported_as_not_expressible() {
        // The finding this exists for. Six clean images, a 1% budget, and the
        // honest answer is the zero-budget answer: 1/6 is 16.7%, so 1% buys
        // nothing that 0% did not already buy. The figure is correct and the
        // label "TPR@1%FA" is not.
        let scores = [
            0.9, 0.8, 0.7, 0.6, 0.1, 0.2, 0.3, 0.4, 0.5, 0.05, 0.06, 0.07,
        ];
        let labels = [
            true, true, true, true, true, true, false, false, false, false, false, false,
        ];
        let p = operating_point(&scores, &labels, 0.01).expect("a point");
        assert_eq!(p.requested_fpr, 0.01);
        assert_eq!(
            p.achieved_fpr, 0.0,
            "1% cannot be spent on six clean images"
        );
        assert!(!p.budget_was_expressible(6));
        // And the same corpus CAN express a budget of one image in six.
        let coarse = operating_point(&scores, &labels, 1.0 / 6.0).expect("a point");
        assert!(coarse.budget_was_expressible(6));
    }

    #[test]
    fn a_budget_the_corpus_can_spend_is_reported_as_expressible() {
        // Two hundred clean images make 1% a real budget: two of them.
        assert!(fpr_resolution(200) <= 0.01);
        let mut scores = Vec::new();
        let mut labels = Vec::new();
        for i in 0..200 {
            scores.push(i as f64 / 200.0);
            labels.push(false);
        }
        for i in 0..200 {
            scores.push(1.0 + i as f64);
            labels.push(true);
        }
        let p = operating_point(&scores, &labels, 0.01).expect("a point");
        assert!(p.budget_was_expressible(200));
        assert_eq!(p.tpr, 1.0);
    }

    #[test]
    fn the_operating_point_agrees_with_the_figure_tpr_at_fpr_reports() {
        // Two ways of asking one question must not drift apart.
        let scores = [0.9, 0.4, 0.5, 0.1];
        let labels = [true, true, false, false];
        for budget in [0.0, 0.25, 0.5, 1.0] {
            let a = tpr_at_fpr(&scores, &labels, budget);
            let b = operating_point(&scores, &labels, budget).map(|p| p.tpr);
            assert_eq!(a, b, "at a budget of {budget}");
        }
    }

    #[test]
    fn the_achieved_rate_never_exceeds_the_budget_that_was_asked_for() {
        let scores = [0.9, 0.4, 0.5, 0.1];
        let labels = [true, true, false, false];
        for budget in [0.0, 0.1, 0.25, 0.4, 0.5, 0.75, 1.0] {
            let p = operating_point(&scores, &labels, budget).expect("a point");
            assert!(
                p.achieved_fpr <= budget + 1e-12,
                "spent {} of a {budget} budget",
                p.achieved_fpr
            );
        }
    }

    #[test]
    fn the_resolution_of_a_corpus_with_no_clean_images_is_not_a_number_to_divide_by() {
        assert!(fpr_resolution(0).is_infinite());
        // Which means no budget is ever expressible, rather than all of them.
        let p = OperatingPoint {
            tpr: 1.0,
            achieved_fpr: 0.0,
            requested_fpr: 0.5,
        };
        assert!(!p.budget_was_expressible(0));
    }

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

#[cfg(test)]
mod delong_tests {
    use super::*;

    /// Hanley and McNeil's 1982 worked example, which is the one every
    /// implementation is checked against.
    ///
    /// Ratings 1 to 5 for 58 normal and 51 abnormal cases. The published AUC
    /// is 0.893 and the published standard error 0.029, computed by their own
    /// parametric method; DeLong on the same data gives an SE close to but
    /// not identical with it, which is the point of preferring DeLong. The
    /// assertion is therefore that the AUC matches to three places and the SE
    /// lands in the range every published DeLong implementation agrees on.
    fn hanley_mcneil() -> (Vec<f64>, Vec<bool>) {
        // (rating, n_normal, n_abnormal)
        let table = [
            (1.0, 33, 3),
            (2.0, 6, 2),
            (3.0, 6, 2),
            (4.0, 11, 11),
            (5.0, 2, 33),
        ];
        let mut scores = Vec::new();
        let mut labels = Vec::new();
        for (rating, n_norm, n_abn) in table {
            for _ in 0..n_norm {
                scores.push(rating);
                labels.push(false);
            }
            for _ in 0..n_abn {
                scores.push(rating);
                labels.push(true);
            }
        }
        (scores, labels)
    }

    #[test]
    fn the_published_worked_example_comes_out_where_it_is_published() {
        let (scores, labels) = hanley_mcneil();
        let got = roc_auc_interval(&scores, &labels, Z_95).expect("both classes, enough of each");
        assert!(
            (got.auc - 0.893).abs() < 0.001,
            "AUC {} is not the published 0.893",
            got.auc
        );
        assert!(
            (0.025..0.035).contains(&got.standard_error),
            "standard error {} is outside what every DeLong implementation gives here",
            got.standard_error
        );
        assert!(got.low < got.auc && got.auc < got.high);
    }

    #[test]
    fn a_perfect_separation_has_a_zero_width_interval() {
        // Every positive above every negative. There is no sampling noise in
        // the ordering, so DeLong's variance is exactly zero, and the
        // interval must not be reported as wider than the measurement.
        let scores = vec![1.0, 2.0, 3.0, 10.0, 11.0, 12.0];
        let labels = vec![false, false, false, true, true, true];
        let got = roc_auc_interval(&scores, &labels, Z_95).expect("valid");
        assert_eq!(got.auc, 1.0);
        assert_eq!(got.standard_error, 0.0);
        assert_eq!((got.low, got.high), (1.0, 1.0));
    }

    #[test]
    fn a_detector_answering_one_number_to_everything_has_no_spread_either() {
        // AUC 0.5 by construction. Every comparison is a tie, so every
        // structural component is 0.5 and the variance is zero: the tool is
        // not uncertain about this AUC, it is certain the detector said
        // nothing. The warning about that lives in `score`, not here.
        let scores = vec![7.0; 8];
        let labels = vec![true, false, true, false, true, false, true, false];
        let got = roc_auc_interval(&scores, &labels, Z_95).expect("valid");
        assert_eq!(got.auc, 0.5);
        assert_eq!(got.standard_error, 0.0);
    }

    #[test]
    fn the_interval_is_clamped_to_the_scale_it_is_measured_on() {
        // Near 1.0 the normal interval runs off the end, and an upper bound
        // of 1.04 beside a measurement is nonsense a reader would quote.
        let mut scores: Vec<f64> = (0..20).map(|i| i as f64).collect();
        let mut labels = vec![false; 20];
        scores.extend((0..20).map(|i| 100.0 + i as f64));
        labels.extend(vec![true; 20]);
        scores[0] = 200.0; // one negative above everything, so AUC is high but not 1
        let got = roc_auc_interval(&scores, &labels, Z_95).expect("valid");
        assert!(got.high <= 1.0, "upper bound {} left the scale", got.high);
        assert!(got.low >= 0.0, "lower bound {} left the scale", got.low);
    }

    #[test]
    fn one_of_a_class_is_refused_because_a_variance_needs_two() {
        let scores = vec![1.0, 2.0, 3.0];
        let labels = vec![true, false, false];
        assert_eq!(roc_auc_interval(&scores, &labels, Z_95), None);
        // And the point estimate is still available, because that one IS
        // defined with a single positive.
        assert!(roc_auc(&scores, &labels).is_some());
    }

    #[test]
    fn everything_roc_auc_refuses_this_refuses_too() {
        assert_eq!(roc_auc_interval(&[1.0, 2.0], &[true], Z_95), None);
        assert_eq!(
            roc_auc_interval(
                &[1.0, f64::NAN, 3.0, 4.0],
                &[true, true, false, false],
                Z_95
            ),
            None
        );
        assert_eq!(roc_auc_interval(&[1.0, 2.0], &[true, true], Z_95), None);
    }

    #[test]
    fn a_nonsense_width_is_refused_rather_than_producing_a_nonsense_interval() {
        let scores = vec![1.0, 2.0, 3.0, 4.0];
        let labels = vec![false, false, true, true];
        assert_eq!(roc_auc_interval(&scores, &labels, f64::NAN), None);
        assert_eq!(roc_auc_interval(&scores, &labels, -1.0), None);
    }

    #[test]
    fn a_wider_z_gives_a_wider_interval_around_the_same_point() {
        let (scores, labels) = hanley_mcneil();
        let narrow = roc_auc_interval(&scores, &labels, 1.0).expect("valid");
        let wide = roc_auc_interval(&scores, &labels, Z_95).expect("valid");
        assert_eq!(narrow.auc, wide.auc);
        assert_eq!(narrow.standard_error, wide.standard_error);
        assert!(wide.high - wide.low > narrow.high - narrow.low);
    }

    #[test]
    fn the_point_estimate_is_the_same_number_roc_auc_gives() {
        // Two implementations of one metric is the thing this crate exists to
        // avoid, so the interval must not quietly compute its own AUC.
        let (scores, labels) = hanley_mcneil();
        let plain = roc_auc(&scores, &labels).expect("valid");
        let with_ci = roc_auc_interval(&scores, &labels, Z_95).expect("valid");
        assert_eq!(plain, with_ci.auc);
    }
}
