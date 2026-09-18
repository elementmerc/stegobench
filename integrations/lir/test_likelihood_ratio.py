# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the likelihood ratio machinery.

Cllr has properties that hold by construction rather than by convention, and
those are what is tested here: a system that says nothing scores exactly 1, a
system that is confidently wrong scores worse than one that says nothing, and
the isotonic floor is a floor. If any of those break, the number is not Cllr
any more, whatever it is called.
"""
from __future__ import annotations

import numpy as np
import pytest

from likelihood_ratio import (
    LogisticCalibrator,
    cllr,
    cross_validated_lrs,
    decompose,
    pav,
)


class TestCllrHasAReferencePoint:
    def test_saying_nothing_costs_exactly_one(self):
        """LR = 1 everywhere means 'this evidence does not move the odds'."""
        ones = np.ones(100)
        assert cllr(ones, ones) == pytest.approx(1.0)

    def test_a_confidently_correct_system_costs_almost_nothing(self):
        assert cllr(np.full(100, 1000.0), np.full(100, 0.001)) < 0.01

    def test_a_confidently_wrong_system_costs_more_than_silence(self):
        """The property AUC cannot express.

        Here every payload case is given a strong LR against carrying a
        payload and vice versa. Cllr must exceed 1, because the examiner would
        have been better served by a system that declined to answer.
        """
        assert cllr(np.full(100, 0.01), np.full(100, 100.0)) > 1.0

    def test_both_sides_are_required(self):
        with pytest.raises(ValueError):
            cllr(np.array([2.0]), np.array([]))

    def test_a_negative_likelihood_ratio_is_refused(self):
        with pytest.raises(ValueError, match="cannot be negative"):
            cllr(np.array([-1.0]), np.array([1.0]))


class TestPav:
    def test_output_is_monotone_in_the_score(self):
        rng = np.random.default_rng(0)
        scores = rng.normal(size=200)
        labels = (scores + rng.normal(scale=0.5, size=200) > 0).astype(int)
        fitted = pav(scores, labels)
        order = np.argsort(scores)
        assert np.all(np.diff(fitted[order]) >= -1e-12)

    def test_perfectly_separated_scores_are_fitted_exactly(self):
        scores = np.array([1.0, 2.0, 3.0, 4.0])
        labels = np.array([0, 0, 1, 1])
        assert pav(scores, labels) == pytest.approx([0.0, 0.0, 1.0, 1.0])

    def test_a_useless_score_is_flattened_to_the_base_rate(self):
        """Scores that carry no information collapse to the base rate.

        The two endpoints are genuinely 0 and 1 and that is the correct
        isotonic fit, not an artefact: the lowest score really was a clean
        picture and the highest really did carry a payload, and a monotone fit
        is allowed to say so. Everything between them flattens, which is the
        system admitting the ordering told it nothing.
        """
        scores = np.arange(100, dtype=float)
        labels = np.tile([0, 1], 50)
        fitted = pav(scores, labels)
        assert np.allclose(fitted[1:-1], 0.5, atol=0.02)
        assert fitted[0] == 0.0 and fitted[-1] == 1.0

    def test_tied_scores_all_receive_the_same_value(self):
        """Identical evidence must produce identical likelihood ratios.

        Two pictures that gave the same detector output cannot be told apart
        by that output, so a calibration that assigns them different numbers
        is deciding on the basis of where they sat in a file.
        """
        scores = np.array([1.0, 1.0, 1.0, 2.0])
        labels = np.array([0, 1, 0, 1])
        fitted = pav(scores, labels)
        assert fitted[0] == fitted[1] == fitted[2]

    def test_ties_do_not_depend_on_input_order(self):
        scores = np.array([1.0, 1.0, 1.0, 2.0])
        labels = np.array([0, 1, 0, 1])
        a = pav(scores, labels)
        idx = np.array([2, 0, 1, 3])
        b = pav(scores[idx], labels[idx])
        # Same multiset of fitted values, whatever order the rows arrived in.
        assert sorted(a) == pytest.approx(sorted(b))
        # And each row keeps its own value through the permutation.
        assert a[idx] == pytest.approx(b)


class TestDecomposition:
    @staticmethod
    def _sample(separation: float, n: int = 400, seed: int = 1):
        rng = np.random.default_rng(seed)
        labels = np.array([0] * (n // 2) + [1] * (n // 2))
        scores = rng.normal(loc=separation * labels, scale=1.0)
        return scores, labels

    def test_calibration_loss_is_never_negative(self):
        """Cllr_min is a floor, so the remainder cannot be below zero.

        Checked across a range rather than once, because a floor that holds
        for one sample and not another is an implementation bug in PAV.
        """
        for separation in (0.0, 0.5, 1.0, 2.0, 4.0):
            scores, labels = self._sample(separation)
            lrs = cross_validated_lrs(scores, labels, folds=5)
            d = decompose(lrs, labels, scores)
            assert d.cllr_cal >= -1e-9, f"separation {separation} gave {d.cllr_cal}"

    def test_better_separated_scores_discriminate_better(self):
        floors = []
        for separation in (0.0, 1.0, 3.0):
            scores, labels = self._sample(separation)
            lrs = cross_validated_lrs(scores, labels, folds=5)
            floors.append(decompose(lrs, labels, scores).cllr_min)
        assert floors[0] > floors[1] > floors[2]

    def test_scores_carrying_nothing_are_reported_as_uninformative(self):
        scores, labels = self._sample(0.0)
        lrs = cross_validated_lrs(scores, labels, folds=5)
        d = decompose(lrs, labels, scores)
        assert d.cllr == pytest.approx(1.0, abs=0.15)

    def test_the_counts_are_carried_through(self):
        scores, labels = self._sample(1.0, n=300)
        lrs = cross_validated_lrs(scores, labels, folds=5)
        d = decompose(lrs, labels, scores)
        assert d.n_payload == 150 and d.n_clean == 150


class TestCalibrator:
    def test_the_prior_odds_are_divided_out(self):
        """An LR must not carry the training set's base rate.

        Same scores, same separation, different proportion of payload cases.
        The LRs must agree, because the likelihood ratio is a property of the
        evidence and not of how many guilty pictures happened to be collected.
        """
        rng = np.random.default_rng(4)
        query = np.array([-1.0, 0.0, 1.0, 2.0])

        lrs = []
        for n_clean, n_payload in ((800, 200), (200, 800)):
            labels = np.array([0] * n_clean + [1] * n_payload)
            scores = rng.normal(loc=2.0 * labels, scale=1.0)
            prior_odds = n_payload / n_clean
            cal = LogisticCalibrator().fit(scores, labels)
            lrs.append(cal.transform(query, prior_odds))

        # Not identical, because the two fits saw different samples, but the
        # base rate must not be what separates them.
        assert np.allclose(np.log(lrs[0]), np.log(lrs[1]), atol=0.6)

    def test_likelihood_ratios_are_bounded(self):
        rng = np.random.default_rng(5)
        labels = np.array([0] * 100 + [1] * 100)
        scores = rng.normal(loc=8.0 * labels, scale=0.5)
        cal = LogisticCalibrator(bound=50.0).fit(scores, labels)
        lrs = cal.transform(np.array([-50.0, 50.0]), 1.0)
        assert lrs.min() >= 1 / 50.0 and lrs.max() <= 50.0

    def test_transform_before_fit_is_an_error(self):
        with pytest.raises(RuntimeError):
            LogisticCalibrator().transform(np.array([1.0]), 1.0)


class TestCrossValidation:
    def test_it_is_more_honest_than_fitting_on_everything(self):
        """The reason cross validation is not optional here.

        Fitting a calibrator and scoring the same cases with it gives LRs that
        overstate the evidence. On data with no signal at all, that shows up
        as a Cllr better than 1.0, which would be a system claiming to be
        informative about nothing.
        """
        rng = np.random.default_rng(7)
        labels = np.array([0] * 100 + [1] * 100)
        scores = rng.normal(size=200)  # deliberately unrelated to the labels

        cal = LogisticCalibrator().fit(scores, labels)
        in_sample = cal.transform(scores, 1.0)
        honest = cross_validated_lrs(scores, labels, folds=10)

        assert cllr(in_sample[labels == 1], in_sample[labels == 0]) < cllr(
            honest[labels == 1], honest[labels == 0]
        )

    def test_folds_are_stratified(self):
        labels = np.array([0] * 100 + [1] * 12)
        scores = np.arange(112, dtype=float)
        # 12 payload cases cannot fill 20 folds, and pretending otherwise
        # leaves a fold with no positives to estimate from.
        with pytest.raises(ValueError, match="cannot fill"):
            cross_validated_lrs(scores, labels, folds=20)

    def test_it_is_reproducible(self):
        rng = np.random.default_rng(9)
        labels = np.array([0] * 100 + [1] * 100)
        scores = rng.normal(loc=labels, scale=1.0)
        a = cross_validated_lrs(scores, labels, folds=5, seed=3)
        b = cross_validated_lrs(scores, labels, folds=5, seed=3)
        assert np.array_equal(a, b)


class TestPermutationNull:
    """The reference point, which is measured rather than assumed.

    These exist because the textbook reference of 1.0 produced three false
    findings on real data. See `cllr_null` for the incident.
    """

    @staticmethod
    def _identical_distributions(n: int = 200, seed: int = 0):
        """The structural-arm situation: the two sides are literally the same.

        Appending bytes after the end of a JPEG changes no pixel, so a pixel
        domain detector returns byte identical scores for the stego arm and
        the clean arm. The true Cllr is exactly 1.000 and there is nothing to
        argue about.
        """
        rng = np.random.default_rng(seed)
        s = rng.normal(size=n)
        return np.concatenate([s, s]), np.concatenate([np.ones(n, int), np.zeros(n, int)])

    def test_the_pipeline_is_biased_above_one_on_null_data(self):
        """Pinning the bias, so nobody later reads it as a finding."""
        from likelihood_ratio import cllr_null

        scores, labels = self._identical_distributions()
        lrs = cross_validated_lrs(scores, labels, folds=10)
        observed = cllr(lrs[labels == 1], lrs[labels == 0])
        assert observed > 1.0
        # Small, but well outside what a bootstrap over the LRs would flag.
        assert observed < 1.02

    def test_the_null_covers_the_observed_value_on_null_data(self):
        from likelihood_ratio import cllr_null

        scores, labels = self._identical_distributions()
        lrs = cross_validated_lrs(scores, labels, folds=10)
        observed = cllr(lrs[labels == 1], lrs[labels == 0])
        null = cllr_null(scores, labels, permutations=100, folds=10)
        p = (null <= observed).mean()
        assert 0.05 < p < 0.95, f"a provably null arm was called a finding at p={p}"

    def test_a_real_signal_clears_the_null(self):
        """The control that has to be able to fail.

        A pipeline that could only ever answer 'no evidential value' would
        produce exactly the table this module produced on JPEG arms, so it has
        to be shown answering the other way on data that does carry a signal.
        """
        from likelihood_ratio import cllr_null

        rng = np.random.default_rng(2)
        labels = np.array([0] * 300 + [1] * 300)
        scores = rng.normal(loc=2.5 * labels, scale=1.0)
        lrs = cross_validated_lrs(scores, labels, folds=10)
        observed = cllr(lrs[labels == 1], lrs[labels == 0])
        null = cllr_null(scores, labels, permutations=50, folds=10)
        assert observed < 0.6
        assert (null <= observed).mean() == 0.0

    def test_it_is_reproducible(self):
        from likelihood_ratio import cllr_null

        scores, labels = self._identical_distributions(n=100)
        a = cllr_null(scores, labels, permutations=20, folds=5, seed=1)
        b = cllr_null(scores, labels, permutations=20, folds=5, seed=1)
        assert np.array_equal(a, b)
