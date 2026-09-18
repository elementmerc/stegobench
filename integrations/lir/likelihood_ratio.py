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


@dataclasses.dataclass(frozen=True)
class CllrDecomposition:
    """Cllr and the two things it is made of."""

    cllr: float
    cllr_min: float
    #: Calibration loss. Zero when the LRs are perfectly calibrated.
    cllr_cal: float
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
    order = np.argsort(scores, kind="mergesort")
    sorted_scores = np.asarray(scores, dtype=float)[order]
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


def decompose(lrs: np.ndarray, labels: np.ndarray, scores: np.ndarray) -> CllrDecomposition:
    """Split Cllr into discrimination and calibration loss.

    :param lrs: the likelihood ratios actually reported.
    :param labels: 1 for payload, 0 for clean.
    :param scores: the raw detector scores the LRs came from, used for the
        isotonic floor.
    """
    payload = labels == 1
    total = cllr(lrs[payload], lrs[~payload])

    # The PAV posteriors, converted to LRs at the prior odds of this sample,
    # give the best any monotone calibration of these scores could do.
    posterior = pav(scores, labels)
    prior_odds = payload.sum() / (~payload).sum()
    eps = 1e-12
    pav_lrs = (posterior + eps) / (1.0 - posterior + eps) / prior_odds
    floor = cllr(pav_lrs[payload], pav_lrs[~payload])

    return CllrDecomposition(
        cllr=total,
        cllr_min=floor,
        cllr_cal=total - floor,
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
    """
    scores = np.asarray(scores, dtype=float)
    labels = np.asarray(labels, dtype=int)
    rng = np.random.default_rng(seed)
    out = np.empty(permutations)
    for i in range(permutations):
        shuffled = rng.permutation(labels)
        lrs = cross_validated_lrs(scores, shuffled, folds=folds, bound=bound, seed=int(rng.integers(1 << 31)))
        out[i] = cllr(lrs[shuffled == 1], lrs[shuffled == 0])
    return out


def cllr_interval(
    lrs: np.ndarray,
    labels: np.ndarray,
    resamples: int = 2000,
    level: float = 0.95,
    seed: int = 0,
) -> tuple[float, float]:
    """A percentile bootstrap interval for Cllr.

    Without this, a Cllr of 1.004 reads as "worse than saying nothing", when
    on a few hundred cases it is indistinguishable from exactly 1. Calling
    that a finding would be the same overstatement this module exists to stop,
    pointed at ourselves instead of at a defendant.

    Resampling is stratified, because the two sides enter Cllr as separate
    means and a resample that thins one of them is not the same experiment.
    """
    rng = np.random.default_rng(seed)
    payload = np.flatnonzero(labels == 1)
    clean = np.flatnonzero(labels == 0)
    if len(payload) == 0 or len(clean) == 0:
        raise ValueError("Cllr needs cases on both sides")

    draws = np.empty(resamples)
    for i in range(resamples):
        p = rng.choice(payload, size=len(payload), replace=True)
        c = rng.choice(clean, size=len(clean), replace=True)
        draws[i] = cllr(lrs[p], lrs[c])

    tail = (1.0 - level) / 2.0
    return float(np.quantile(draws, tail)), float(np.quantile(draws, 1.0 - tail))


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

    def fit(self, scores: np.ndarray, labels: np.ndarray) -> "LogisticCalibrator":
        from scipy.optimize import minimize

        x = np.asarray(scores, dtype=float)
        y = np.asarray(labels, dtype=float)

        def negative_log_likelihood(params):
            a, b = params
            z = a * x + b
            # log(1 + exp(z)) written stably for large |z|.
            return float(np.sum(np.logaddexp(0.0, z) - y * z))

        result = minimize(negative_log_likelihood, x0=[1.0, 0.0], method="BFGS")
        if not result.success and not np.all(np.isfinite(result.x)):
            raise RuntimeError(f"the logistic fit did not converge: {result.message}")
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
        z = self.coef_ * np.asarray(scores, dtype=float) + self.intercept_
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
