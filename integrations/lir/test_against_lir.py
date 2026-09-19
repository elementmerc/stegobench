# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Check this implementation against NFI's own `lir` library.

WHY THIS IS A SEPARATE FILE AND AN OPTIONAL DEPENDENCY
--------------------------------------------------------
`lir` is not imported by anything in this directory, on purpose. It pulls in
pymc, pytensor, numba, llvmlite, optuna, scikit-learn and matplotlib, roughly
2 GB, to compute a handful of scalars that numpy and scipy already do. Taking
that dependency to get Cllr would be a poor trade.

But a check against the reference implementation is worth a great deal, and a
check against a library you imported is not a check at all. So `lir` is an
optional test dependency and these tests skip when it is absent.

WHAT THE TWO IMPLEMENTATIONS AGREE AND DISAGREE ON
----------------------------------------------------
`lir.metrics.cllr` is algebraically identical to `likelihood_ratio.cllr`: the
same halved sum of `log2(1 + LR)` over the clean side and `log2(1 + 1/LR)`
over the payload side, citing the same Brümmer and du Preez paper.

`lir.metrics.cllr_min` fits its isotonic floor to **the reported LLRs**, which
is the same correction this module had to make after review: fitting the floor
to the raw scores is wrong, because a calibrator may invert and the floor is
monotone non-decreasing. Arriving at the same answer independently is the most
useful thing in this file.

Two deliberate differences remain, and both are documented rather than
reconciled:

**Base.** `lir` works in base-10 log odds throughout. This module works in
plain likelihood ratios. The conversion is exact and the tests do it.

**Bounding.** `lir`'s floor is unbounded by default; its `add_misleading`
argument exists to tame extreme values and is off. This module clips the floor
to the same bound as the system, because it reports a bounded system and
subtracting an unbounded floor from a bounded total inflates the calibration
loss by a median 70% on the cells where a detector discriminates. Where `lir`
is used as intended,
with an unbounded system as well, the comparison is like for like and the
issue does not arise.
"""
from __future__ import annotations

import numpy as np
import pytest

from likelihood_ratio import cllr, cross_validated_lrs, decompose, pav

lir_metrics = pytest.importorskip("lir.metrics", reason="lir is an optional check")
from lir.data.models import LLRData  # noqa: E402


def as_llr_data(lrs: np.ndarray, labels: np.ndarray) -> LLRData:
    """Our likelihood ratios in the shape `lir` expects: base-10 log odds."""
    with np.errstate(divide="ignore"):
        llrs = np.log10(np.asarray(lrs, dtype=float))
    return LLRData(features=llrs.reshape(-1, 1), hypothesis=np.asarray(labels, dtype=int))


def sample(separation: float, n: int = 400, seed: int = 0):
    rng = np.random.default_rng(seed)
    labels = np.array([0] * (n // 2) + [1] * (n // 2))
    scores = rng.normal(loc=separation * labels, scale=1.0)
    return scores, labels


class TestCllrAgrees:
    """The headline number, on which there must be no daylight at all."""

    @pytest.mark.parametrize("separation", [-3.0, -1.0, 0.0, 0.5, 1.0, 2.0, 4.0])
    def test_cross_validated_ratios(self, separation):
        scores, labels = sample(separation)
        lrs = cross_validated_lrs(scores, labels, folds=10)
        ours = cllr(lrs[labels == 1], lrs[labels == 0])
        theirs = lir_metrics.cllr(as_llr_data(lrs, labels))
        assert ours == pytest.approx(theirs, rel=1e-12)

    def test_the_reference_point(self):
        """LR = 1 everywhere is exactly 1.0 in both."""
        lrs = np.ones(200)
        labels = np.array([0] * 100 + [1] * 100)
        assert cllr(lrs[labels == 1], lrs[labels == 0]) == pytest.approx(1.0)
        assert lir_metrics.cllr(as_llr_data(lrs, labels)) == pytest.approx(1.0)

    @pytest.mark.parametrize("value", [0.01, 0.5, 2.0, 100.0])
    def test_constant_ratios(self, value):
        labels = np.array([0] * 50 + [1] * 50)
        lrs = np.full(100, value)
        ours = cllr(lrs[labels == 1], lrs[labels == 0])
        theirs = lir_metrics.cllr(as_llr_data(lrs, labels))
        assert ours == pytest.approx(theirs, rel=1e-12)

    def test_an_inverted_system(self):
        """The StegaShield case: a detector pointing the wrong way."""
        labels = np.array([0] * 100 + [1] * 100)
        lrs = np.where(labels == 1, 0.05, 20.0).astype(float)
        ours = cllr(lrs[labels == 1], lrs[labels == 0])
        theirs = lir_metrics.cllr(as_llr_data(lrs, labels))
        assert ours == pytest.approx(theirs, rel=1e-12)
        assert ours > 1.0


class TestTheFloorAgrees:
    """`cllr_min`, compared with the bound taken off so it is like for like."""

    #: Large enough that our clip never binds, so the comparison is of the
    #: isotonic fit rather than of the bounding policy.
    UNBOUNDED = 1e11

    @pytest.mark.parametrize("separation", [-2.0, 0.0, 1.0, 3.0])
    def test_cllr_min_matches_when_neither_is_bounded(self, separation):
        scores, labels = sample(separation)
        lrs = cross_validated_lrs(scores, labels, folds=10, bound=self.UNBOUNDED)
        ours = decompose(lrs, labels, bound=self.UNBOUNDED).cllr_min
        theirs = lir_metrics.cllr_min(as_llr_data(lrs, labels))
        assert ours == pytest.approx(theirs, abs=2e-3)

    @pytest.mark.parametrize("separation", [-2.0, 0.0, 1.0, 3.0])
    def test_cllr_cal_matches_when_neither_is_bounded(self, separation):
        scores, labels = sample(separation)
        lrs = cross_validated_lrs(scores, labels, folds=10, bound=self.UNBOUNDED)
        ours = decompose(lrs, labels, bound=self.UNBOUNDED).cllr_cal
        theirs = lir_metrics.cllr_cal(as_llr_data(lrs, labels))
        assert ours == pytest.approx(theirs, abs=2e-3)

    def test_both_treat_an_inverted_system_as_having_discrimination(self):
        """The property that was broken here before review.

        A detector pointing the wrong way still separates the classes, so the
        floor must be below 1.0 and the calibration loss must stay positive.
        Both implementations have to agree on that or one of them is wrong.
        """
        scores, labels = sample(-3.0)
        lrs = cross_validated_lrs(scores, labels, folds=10, bound=self.UNBOUNDED)
        ours = decompose(lrs, labels, bound=self.UNBOUNDED)
        theirs_min = lir_metrics.cllr_min(as_llr_data(lrs, labels))
        assert ours.cllr_min < 1.0
        assert theirs_min < 1.0
        assert ours.cllr_cal >= -1e-9
        assert ours.cllr - theirs_min >= -1e-9


class TestTheIsotonicFitAgrees:
    """Our PAV against theirs, which wraps scikit-learn."""

    def test_the_orderings_match(self):
        """Posterior ordering must be identical, whatever the parameterisation.

        `lir` balances the classes with sample weights and we divide by the
        prior odds. Those are different routes to the same place, so the
        values can differ on unbalanced data while the ordering cannot.
        """
        from lir.algorithms.isotonic_regression import IsotonicCalibrator

        rng = np.random.default_rng(3)
        for seed in range(10):
            rng = np.random.default_rng(seed)
            n = 200
            labels = rng.integers(0, 2, size=n)
            if labels.min() == labels.max():
                continue
            scores = rng.normal(loc=labels, scale=1.0)

            ours = pav(scores, labels)
            theirs = IsotonicCalibrator().fit_apply(
                LLRData(features=scores.reshape(-1, 1), hypothesis=labels)
            ).llrs

            # Both are monotone in the score, so ordering one by the other
            # must be non-decreasing.
            #
            # Compared pairwise rather than through `np.diff`, because their
            # output legitimately contains plus and minus infinity: an
            # isotonic block of pure zeros or pure ones is a probability of 0
            # or 1, and log odds of that is infinite. `inf - inf` is nan and
            # would fail a diff based check on correct output.
            order = np.argsort(scores, kind="mergesort")
            for series in (ours[order], theirs[order]):
                assert np.all(series[:-1] <= series[1:] + 1e-9)

    def test_on_balanced_data_the_values_agree_too(self):
        """With equal class sizes the two parameterisations coincide."""
        from lir.algorithms.isotonic_regression import IsotonicCalibrator

        scores, labels = sample(1.5, n=400, seed=5)
        ours_posterior = pav(scores, labels)
        prior_odds = (labels == 1).sum() / (labels == 0).sum()
        eps = 1e-12
        ours_lr = (ours_posterior + eps) / (1 - ours_posterior + eps) / prior_odds

        theirs_llr = IsotonicCalibrator().fit_apply(
            LLRData(features=scores.reshape(-1, 1), hypothesis=labels)
        ).llrs
        theirs_lr = 10.0**theirs_llr

        finite = np.isfinite(theirs_lr) & (theirs_lr > 0) & (ours_lr < 1e10)
        assert finite.sum() > len(scores) * 0.5
        assert np.allclose(
            np.log10(ours_lr[finite]), np.log10(theirs_lr[finite]), atol=1e-6
        )


class TestOnTheRealCorpus:
    """The JPEG panel, where the published table comes from."""

    def test_the_published_verdict_survives_the_reference_implementation(self, tmp_path):
        """If `lir` disagreed about these, the table would be wrong.

        Synthesised to the same shape as a round3-q95 arm rather than read from
        the corpus, which is licence restricted and not in the repository.
        """
        rng = np.random.default_rng(0)
        n = 160
        cover = rng.normal(size=n)
        scores = np.concatenate([cover, cover])  # the structural arm: identical
        labels = np.concatenate([np.ones(n, int), np.zeros(n, int)])

        lrs = cross_validated_lrs(scores, labels, folds=10)
        ours = cllr(lrs[labels == 1], lrs[labels == 0])
        theirs = lir_metrics.cllr(as_llr_data(lrs, labels))
        assert ours == pytest.approx(theirs, rel=1e-12)
        # Both must sit just above the reference point, which is the bias the
        # permutation null exists to absorb.
        assert 1.0 < ours < 1.02
