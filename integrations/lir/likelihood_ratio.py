# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Turn detector scores into likelihood ratios, the way forensic science asks for them.

THE TRANSLATION PROBLEM
-----------------------
Steganalysis reports AUC and true positive rate at a fixed false positive
rate. Forensic science reports a **likelihood ratio**: how much more probable
this evidence is if the picture carries a payload than if it does not.

    LR = P(score | it carries a payload) / P(score | it does not)

They are two languages for one question, and the difference is not cosmetic.
An AUC of 0.95 describes how a detector behaves *across a corpus*. It says
nothing about the picture in front of the examiner. An LR of 40 says something
about *this* picture: the evidence is forty times more probable under one
account than the other. That is the form a court can combine with everything
else in the case, and it is the form a steganalysis paper almost never
produces.

WHY A SCORE IS NOT ALREADY A LIKELIHOOD RATIO
----------------------------------------------
A detector's output is an arbitrary number that happens to be larger when it
is more suspicious. Sample Pair Analysis returning 0.43 does not mean the odds
are 0.43, or 43, or anything at all until it has been mapped onto a ratio of
two probability densities estimated from data. That mapping is calibration,
and it is the whole job.

Doing it badly is worse than not doing it. An overstated LR handed to a court
is a miscarriage waiting to happen, which is why this module reports Cllr
alongside every LR it produces and splits it into the part that is genuine
discrimination and the part that is calibration error.

THE COST FUNCTION, AND WHY BOTH HALVES OF IT MATTER
-----------------------------------------------------
Cllr (log-likelihood-ratio cost) penalises an LR by how wrong it was and how
confidently it was wrong:

    Cllr = 1/2 [ mean over payload cases of log2(1 + 1/LR)
               + mean over clean cases of log2(1 + LR) ]

A system that always answers LR = 1, meaning "this tells you nothing", scores
exactly 1.0. **Anything above 1.0 is worse than useless**: it would have been
better to say nothing. That property is why forensic scientists use it and
AUC cannot replace it. AUC has no such point of reference; an AUC of 0.5 is
uninformative but an AUC of 0.4 is not "worse than silence", it is a detector
pointing the wrong way, which is information.

Cllr splits in two, via the pool adjacent violators algorithm:

    Cllr_min   what the scores could achieve if calibrated perfectly.
               This is discrimination, and it is the detector's fault.
    Cllr_cal   Cllr - Cllr_min. The price paid for the calibration being
               imperfect. This is the statistician's fault, and it is
               fixable without touching the detector.

Reporting one number hides which of the two is failing.
"""
from __future__ import annotations

import dataclasses

import numpy as np

#: LRs are bounded before they are reported. An unbounded LR from a finite
#: sample is an artefact: with 200 clean pictures, the data cannot support a
#: claim of "ten thousand times more probable", because the tail was never
#: observed. The empirical lower and upper bound (ELUB) idea is to refuse to
#: state more than the sample size can carry. This is the crude version of it,
#: and the bound is reported next to every number that hits it.
DEFAULT_BOUND = 100.0


def _check_labels(labels) -> np.ndarray:
    """Labels must be exactly {0, 1}, and saying so beats a downstream puzzle.

    Passing labels of {0, 2} used to reach `cllr` and come back as "one side
    is empty", which points at the wrong problem entirely.
    """
    labels = np.asarray(labels)
    if labels.ndim != 1:
        raise ValueError("labels must be one dimensional")
    extra = set(np.unique(labels).tolist()) - {0, 1}
    if extra:
        raise ValueError(f"labels must be 0 or 1; found {sorted(extra)}")
    return labels.astype(int)


@dataclasses.dataclass(frozen=True)
class CllrDecomposition:
    """Cllr and the parts it is made of."""

    cllr: float
    #: Discrimination. What these reported ratios could achieve if optimally
    #: recalibrated, under the same bound.
    cllr_min: float
    #: Calibration loss. Zero when the reported ratios are already the best
    #: monotone recalibration of themselves. Cannot be negative.
    cllr_cal: float
    #: What a flawless bounded system still pays. Part of `cllr_min`, quoted
    #: separately so a reader can see how much of the total is the clip.
    bound_cost: float
    n_payload: int
    n_clean: int

    @property
    def informative(self) -> bool:
        """False when the system would have done better saying nothing at all."""
        return self.cllr < 1.0


def cllr(lrs_payload: np.ndarray, lrs_clean: np.ndarray) -> float:
    """The log-likelihood-ratio cost.

    :param lrs_payload: LRs for pictures that do carry a payload.
    :param lrs_clean: LRs for pictures that do not.
    """
    if len(lrs_payload) == 0 or len(lrs_clean) == 0:
        raise ValueError("Cllr needs cases on both sides; one side is empty")
    if np.any(lrs_payload < 0) or np.any(lrs_clean < 0):
        raise ValueError("a likelihood ratio cannot be negative")

    # log2(1 + 1/LR) and log2(1 + LR), written through log1p so that an LR
    # near zero or very large does not lose precision where it matters most.
    with np.errstate(divide="ignore"):
        loss_payload = np.log1p(1.0 / lrs_payload) / np.log(2.0)
    loss_clean = np.log1p(lrs_clean) / np.log(2.0)
    return float(0.5 * (np.mean(loss_payload) + np.mean(loss_clean)))


def pav(scores: np.ndarray, labels: np.ndarray) -> np.ndarray:
    """Pool adjacent violators: the best monotone calibration of these scores.

    Returns the posterior probability assigned to each input, under the
    isotonic fit. This is the optimal calibration *in hindsight*, which is
    precisely why it gives the floor Cllr_min and must never be used to
    produce an LR that is then reported: it has seen the answers.

    **Tied scores are pooled before fitting, and that is not an optimisation.**
    Two pictures that produced the same detector output are, as far as this
    evidence goes, indistinguishable. Without pooling, which of them lands
    first in the sort decides which gets the higher value, so two identical
    measurements would be assigned different likelihood ratios by an accident
    of input order. That is indefensible in a report and it is also
    non-deterministic, which the fleet's own rules forbid.

    :param scores: detector outputs, higher meaning more suspicious.
    :param labels: 1 for a picture carrying a payload, 0 for a clean one.
    """
    scores = np.asarray(scores, dtype=float)
    labels = _check_labels(labels)
    if len(scores) != len(labels):
        raise ValueError(f"{len(scores)} scores against {len(labels)} labels")

    order = np.argsort(scores, kind="mergesort")
    sorted_scores = scores[order]
    y = labels[order].astype(float)

    # Collapse each run of equal scores to its mean, carrying the run length
    # as a weight, so the fit cannot distinguish within a tie.
    group = np.concatenate(([0], np.flatnonzero(np.diff(sorted_scores) != 0) + 1, [len(y)]))
    tie_values = np.array([y[group[i]:group[i + 1]].mean() for i in range(len(group) - 1)])
    tie_counts = np.diff(group).astype(float)

    # Each block holds a running mean and the count it is a mean of. Adjacent
    # blocks are merged whenever the sequence stops being non-decreasing.
    values: list[float] = []
    weights: list[float] = []
    for value, count in zip(tie_values, tie_counts):
        values.append(float(value))
        weights.append(float(count))
        while len(values) > 1 and values[-2] > values[-1]:
            v2, w2 = values.pop(), weights.pop()
            v1, w1 = values.pop(), weights.pop()
            values.append((v1 * w1 + v2 * w2) / (w1 + w2))
            weights.append(w1 + w2)

    fitted = np.repeat(values, [int(w) for w in weights])
    out = np.empty(len(y), dtype=float)
    out[order] = fitted
    return out


def bound_cost(bound: float = DEFAULT_BOUND) -> float:
    """What a flawless system still pays because its answers are bounded.

    A system that reports `bound` for every payload case and `1/bound` for
    every clean one is as right as a bounded system can be, and it does not
    score zero. At a bound of 100 it scores 0.0144. Quoting a total Cllr
    without this alongside invites the reader to attribute the whole of it to
    the detector, when a third to a half of it can be the clip.
    """
    return cllr(np.array([bound]), np.array([1.0 / bound]))


def decompose(
    lrs: np.ndarray, labels: np.ndarray, bound: float = DEFAULT_BOUND
) -> CllrDecomposition:
    """Split Cllr into discrimination and calibration loss.

    THE FLOOR IS TAKEN FROM THE REPORTED RATIOS, NOT FROM THE RAW SCORES
    ---------------------------------------------------------------------
    An earlier version fitted the isotonic floor to the raw detector scores
    and left it unbounded, while the reported ratios were clipped. Two
    consequences, both found by review rather than by reasoning, and both
    capable of putting a false number in a forensic table:

    **The bound leaked into the calibration loss.** The floor was free to be
    ten orders of magnitude more confident than the system it was the floor
    for, so `cllr_cal` measured the clip as much as the calibrator. Measured
    across all 45 cells of the spatial corpus by `bound_attribution.py`: a
    median 23% of the reported calibration loss was the bound, and on the 18
    cells where the detector actually discriminates (AUC above 0.99) it was
    38% to 82%, median 70%. The single figure quoted before, "68 to 71%",
    was that subset's middle presented as though it described the corpus.

    **The floor was not a floor.** Isotonic regression is monotone *non
    decreasing* in whatever it is fitted to. A calibrator is free to fit a
    negative slope, and on a detector that points the wrong way it does. The
    system then beats its own floor and `cllr_cal` goes negative, which is a
    quantity that cannot exist. How far depends on how hard the inversion is:
    -0.05 at AUC 0.325, -0.51 at AUC 0.071, -0.89 at AUC 0.003 on Gaussian
    data. A weakly inverted detector, which is what the JPEG arms are at AUC
    0.476 to 0.493, does not show it at all.

    Fitting the floor to the reported ratios fixes both. The identity map is
    in the feasible set, so the floor cannot be beaten; and clipping the
    isotonic solution to the same bound keeps the comparison on one scale,
    which is legitimate because each per-case loss is convex in the assigned
    ratio, so clipping to the nearest allowed value is the constrained
    optimum and preserves monotonicity.

    :param lrs: the likelihood ratios actually reported.
    :param labels: 1 for payload, 0 for clean.
    :param bound: the same bound the reported ratios were held to.
    """
    lrs = np.asarray(lrs, dtype=float)
    labels = _check_labels(labels)
    payload = labels == 1
    total = cllr(lrs[payload], lrs[~payload])

    # Ordered by the REPORTED ratio, so an inverted system is measured
    # against the best monotone recalibration of what it actually said.
    posterior = pav(lrs, labels)
    prior_odds = payload.sum() / (~payload).sum()
    eps = 1e-12
    pav_lrs = (posterior + eps) / (1.0 - posterior + eps) / prior_odds
    pav_lrs = np.clip(pav_lrs, 1.0 / bound, bound)
    floor = cllr(pav_lrs[payload], pav_lrs[~payload])

    return CllrDecomposition(
        cllr=total,
        cllr_min=floor,
        cllr_cal=total - floor,
        bound_cost=bound_cost(bound),
        n_payload=int(payload.sum()),
        n_clean=int((~payload).sum()),
    )


def cllr_null(
    scores: np.ndarray,
    labels: np.ndarray,
    permutations: int = 100,
    folds: int = 10,
    bound: float = DEFAULT_BOUND,
    seed: int = 0,
    pairs: np.ndarray | None = None,
    seeds: int = 20,
) -> np.ndarray:
    """Cllr values this pipeline produces when the labels mean nothing.

    WHY 1.0 IS THE WRONG REFERENCE POINT IN PRACTICE
    -------------------------------------------------
    In theory a system with no discriminating power scores exactly 1.0. In
    practice a *cross validated* pipeline does not, and it errs upwards: each
    fold fits a calibrator to noise, the fitted coefficient is small but not
    zero, and the resulting likelihood ratios scatter around 1 instead of
    sitting on it. Cllr is convex with its minimum at LR = 1 for null data, so
    any scatter costs something. The bias is small, around 0.003 to 0.005 on a
    few hundred cases, and it is systematic rather than random, which means a
    bootstrap over the likelihood ratios cannot see it: the bootstrap
    resamples the output, and the bias is in the procedure.

    This was found rather than anticipated. The `structural` arm of the
    round3-q95 corpus appends data after the end of a JPEG, which changes no
    pixel, so the pixel domain detectors return byte identical scores on it
    and on the clean arm. The true Cllr there is exactly 1.000 and there is no
    argument about it. The pipeline reported 1.003 to 1.005 with a bootstrap
    interval that excluded 1.0, which would have been published as three
    detectors performing worse than silence.

    So the reference is measured instead: permute the labels, run the whole
    pipeline again, and see what it produces when there is provably nothing to
    find. A real result has to beat that, not beat 1.0.

    PERMUTE WITHIN PAIRS WHEN THE DESIGN IS PAIRED
    ------------------------------------------------
    `pairs` makes the permutation respect the cover pairing that the corpus
    was built around. A stego picture and the clean cover it came from are one
    unit; the null hypothesis is that the *label within that unit* is
    arbitrary, not that labels are arbitrary across the whole corpus. Shuffling
    freely destroys the pairing the analysis deliberately constructs and gives
    the wrong null.

    Measured on the structural arm, where the two sides carry byte identical
    scores and the answer is known exactly:

        observed                     1.00445
        within pair, matched   1.00415, 5-95% [1.00344, 1.00473], p 0.762
        within pair, 1 seed    1.00418, 5-95% [1.00149, 1.00750], p 0.564
        free, matched          1.00153, 5-95% [0.99437, 1.00456], p 0.931

    The paired null lands on the observed value to four decimals. The free
    null is centred 0.003 low and is 7.9 times as wide, erring conservative by
    accident rather than design. Regenerate with `null_comparison.py`; the
    figures above are from the matched estimator and an earlier version of
    this docstring quoted the single-seed ones.

    :param pairs: an identifier per case, equal for the two members of a pair.
        When given, labels are flipped within each pair rather than shuffled
        across the corpus.
    :param seeds: fold seeds averaged into each draw. Must match the value
        used for the observed statistic, or the two are not comparable.
    """
    scores = np.asarray(scores, dtype=float)
    labels = _check_labels(labels)
    rng = np.random.default_rng(seed)

    pair_index = None
    if pairs is not None:
        pairs = np.asarray(pairs)
        if len(pairs) != len(labels):
            raise ValueError(f"{len(pairs)} pair ids against {len(labels)} labels")
        _, pair_index = np.unique(pairs, return_inverse=True)

    out = np.empty(permutations)
    for i in range(permutations):
        if pair_index is None:
            shuffled = rng.permutation(labels)
        else:
            # One coin per pair. Where it comes up heads the two members swap
            # labels, which is exactly the exchange the null allows.
            flip = rng.integers(0, 2, size=pair_index.max() + 1)[pair_index].astype(bool)
            shuffled = np.where(flip, 1 - labels, labels)

        # Each draw is averaged over the SAME number of fold seeds as the
        # observed statistic. This is not an optimisation; a mean over 20
        # seeds and a single draw are different estimators with different
        # variance, and comparing one against the other inflated this null
        # by a factor of 4.6, which is sqrt(20), in the direction that
        # made results look
        # indistinguishable from it. See `observed_cllr`.
        draws = np.empty(seeds)
        for s in range(seeds):
            lrs = cross_validated_lrs(scores, shuffled, folds=folds, bound=bound, seed=s)
            draws[s] = cllr(lrs[shuffled == 1], lrs[shuffled == 0])
        out[i] = draws.mean()
    return out


def observed_decomposition(
    scores: np.ndarray,
    labels: np.ndarray,
    folds: int = 10,
    bound: float = DEFAULT_BOUND,
    seeds: int = 20,
) -> tuple[CllrDecomposition, float]:
    """The whole decomposition averaged over fold seeds, not just the total.

    WHY THIS EXISTS RATHER THAN A CALL TO `decompose` ON ONE SEED
    --------------------------------------------------------------
    An earlier version of the reporting scripts printed a Cllr averaged over
    20 fold seeds next to a Cllr_min taken from a single seed. Both numbers
    were individually correct and their difference was not a quantity anything
    had computed: the reader subtracts one column from the other and gets a
    calibration loss that mixes two estimators.

    Averaging the decomposition term by term keeps the arithmetic true, because
    the mean of the differences is the difference of the means. Every column in
    a printed row then comes from the same 20 draws.

    :returns: (the averaged decomposition, the standard deviation of the total)
    """
    labels = _check_labels(labels)
    totals = np.empty(seeds)
    floors = np.empty(seeds)
    for i in range(seeds):
        lrs = cross_validated_lrs(scores, labels, folds=folds, bound=bound, seed=i)
        d = decompose(lrs, labels, bound=bound)
        totals[i] = d.cllr
        floors[i] = d.cllr_min
    return (
        CllrDecomposition(
            cllr=float(totals.mean()),
            cllr_min=float(floors.mean()),
            cllr_cal=float((totals - floors).mean()),
            bound_cost=bound_cost(bound),
            n_payload=int((labels == 1).sum()),
            n_clean=int((labels == 0).sum()),
        ),
        float(totals.std()),
    )


def observed_cllr(
    scores: np.ndarray,
    labels: np.ndarray,
    folds: int = 10,
    bound: float = DEFAULT_BOUND,
    seeds: int = 20,
) -> tuple[float, float, np.ndarray]:
    """Cllr averaged over fold seeds, with its spread.

    A single cross validation is one draw from a distribution whose width is
    set by which cases landed in which fold. On the structural arm that spread
    runs from 1.00084 to 1.00786 across 20 seeds: wider than the bias the
    permutation null exists to correct, and wider than the gap between any two
    numbers in the published table.

    Printing one draw to three decimal places presents a random variable as a
    measurement, so the mean and the spread are returned together and the
    caller is expected to show both.

    :returns: (mean, standard deviation, every draw)
    """
    draws = np.empty(seeds)
    labels = _check_labels(labels)
    for i in range(seeds):
        lrs = cross_validated_lrs(scores, labels, folds=folds, bound=bound, seed=i)
        draws[i] = cllr(lrs[labels == 1], lrs[labels == 0])
    return float(draws.mean()), float(draws.std()), draws


class LogisticCalibrator:
    """Map scores to LRs through a logistic fit.

    The workhorse of forensic LR calibration, because it is monotone, has two
    parameters, and therefore cannot invent structure that a few hundred cases
    do not support. A kernel density fit is more flexible and, on a sample
    this size, more likely to produce a confident number out of a gap in the
    data.
    """

    def __init__(self, bound: float = DEFAULT_BOUND):
        self.bound = bound
        self.coef_: float | None = None
        self.intercept_: float | None = None
        self._centre: float = 0.0
        self._scale: float = 1.0

    def fit(self, scores: np.ndarray, labels: np.ndarray) -> "LogisticCalibrator":
        """Fit on standardised scores, because the raw scale breaks the optimiser.

        Without standardisation, a detector reporting large numbers (byte
        counts, chi-square statistics, StegExpose's raw payload estimate)
        saturates `logaddexp`, the gradient underflows, and BFGS stops at the
        starting point and reports success. Measured: scores offset by 1e5
        with a genuine one sigma separation and AUC 0.76 fitted a coefficient
        of -7e-07 and returned Cllr exactly 1.000. A false negative made of
        arithmetic rather than of evidence.
        """
        from scipy.optimize import minimize

        x_raw = np.asarray(scores, dtype=float)
        y = _check_labels(labels).astype(float)
        if not np.all(np.isfinite(x_raw)):
            raise ValueError("scores must all be finite")

        self._centre = float(np.mean(x_raw))
        spread = float(np.std(x_raw))
        # A constant score carries no information; keep the scale at 1 so the
        # fit degenerates to an intercept instead of dividing by zero.
        self._scale = spread if spread > 0 else 1.0
        x = (x_raw - self._centre) / self._scale

        def negative_log_likelihood(params):
            a, b = params
            z = a * x + b
            return float(np.sum(np.logaddexp(0.0, z) - y * z))

        # A constant score cannot be calibrated against. The honest fit is
        # intercept only, which after the prior odds are divided out reports
        # LR = 1 for everything: this evidence does not move the odds.
        if spread == 0:
            rate = float(np.clip(y.mean(), 1e-12, 1 - 1e-12))
            self.coef_, self.intercept_ = 0.0, float(np.log(rate / (1 - rate)))
            return self

        def gradient(params):
            a, b = params
            p = 1.0 / (1.0 + np.exp(-np.clip(a * x + b, -700, 700)))
            return np.array([np.sum((p - y) * x), np.sum(p - y)])

        result = minimize(negative_log_likelihood, x0=[0.0, 0.0], method="BFGS", jac=gradient)

        # `result.success` is the wrong test in both directions. It is False on
        # well separated data, where BFGS reports precision loss because the
        # maximum likelihood estimate genuinely diverges, and the large finite
        # coefficient it returns is perfectly usable. It is True in the failure
        # that matters, where the optimiser stopped at the starting point and
        # said so cheerfully. So the check is on the answer: finite, and
        # actually at an optimum.
        if not np.all(np.isfinite(result.x)):
            raise RuntimeError(f"the logistic fit returned non-finite parameters: {result.x}")
        residual = float(np.linalg.norm(gradient(result.x))) / len(y)
        if residual > 1e-3:
            raise RuntimeError(
                f"the logistic fit stopped away from an optimum (scaled gradient "
                f"{residual:.2e}); check the scale of the scores"
            )

        self.coef_, self.intercept_ = float(result.x[0]), float(result.x[1])
        return self

    def transform(self, scores: np.ndarray, prior_odds: float) -> np.ndarray:
        """Posterior odds divided by prior odds, which is the LR.

        `prior_odds` is the ratio of payload to clean cases in the data the
        calibrator was fitted on. Dividing it out is what turns a posterior
        into a likelihood ratio, and forgetting it is the most common way to
        report a number that silently carries the training set's base rate
        into a case where that base rate is meaningless.
        """
        if self.coef_ is None:
            raise RuntimeError("fit before transform")
        x = (np.asarray(scores, dtype=float) - self._centre) / self._scale
        z = self.coef_ * x + self.intercept_
        posterior_odds = np.exp(np.clip(z, -700, 700))
        lrs = posterior_odds / prior_odds
        return np.clip(lrs, 1.0 / self.bound, self.bound)


def cross_validated_lrs(
    scores: np.ndarray,
    labels: np.ndarray,
    folds: int = 10,
    bound: float = DEFAULT_BOUND,
    seed: int = 0,
) -> np.ndarray:
    """LRs for every case, each produced by a calibrator that never saw it.

    Fitting a calibrator and then scoring the same data with it produces LRs
    that are too good, and the optimism grows as the calibrator gets more
    flexible. For an LR that is going to be quoted as evidence, that optimism
    is not a statistical nicety; it is an overstatement of the strength of
    evidence against someone.
    """
    scores = np.asarray(scores, dtype=float)
    labels = np.asarray(labels, dtype=int)
    if folds < 2:
        raise ValueError("cross validation needs at least two folds")

    rng = np.random.default_rng(seed)
    assignment = np.empty(len(scores), dtype=int)
    # Stratify, so that a fold cannot come out with no clean cases in it and
    # leave the calibrator with nothing to estimate the denominator from.
    for value in (0, 1):
        idx = np.flatnonzero(labels == value)
        if len(idx) < folds:
            raise ValueError(
                f"only {len(idx)} cases with label {value}, which cannot fill {folds} folds"
            )
        shuffled = rng.permutation(idx)
        assignment[shuffled] = np.arange(len(shuffled)) % folds

    out = np.empty(len(scores), dtype=float)
    for fold in range(folds):
        test = assignment == fold
        train = ~test
        prior_odds = (labels[train] == 1).sum() / (labels[train] == 0).sum()
        calibrator = LogisticCalibrator(bound=bound).fit(scores[train], labels[train])
        out[test] = calibrator.transform(scores[test], prior_odds)
    return out
