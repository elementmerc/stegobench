# Author:  Daniel Iwugo
# Comment: Christ is King
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
    bound_cost,
    cllr,
    cllr_null,
    cross_validated_lrs,
    decompose,
    observed_cllr,
    observed_decomposition,
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

        **The separations here run negative on purpose.** The earlier version
        of this test iterated (0.0, 0.5, 1.0, 2.0, 4.0), all non-negative, and
        so could not fail in the direction the bug lived: an inverted detector
        made the reported ratios beat their own floor and drove cllr_cal to
        minus 0.5. A guard that cannot fire is not a guard.
        """
        for separation in (-4.0, -2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0, 4.0):
            scores, labels = self._sample(separation)
            lrs = cross_validated_lrs(scores, labels, folds=5)
            d = decompose(lrs, labels)
            assert d.cllr_cal >= -1e-9, f"separation {separation} gave {d.cllr_cal}"

    def test_the_floor_holds_on_cross_validated_noise(self):
        """The other way the floor used to leak.

        Cross validated ratios are not one monotone function of the score,
        because every fold has its own calibrator. Fitting the floor to the
        score therefore leaked even when the orientation was right: 9 of 200
        seeds went negative, worst minus 0.0105. Fitting it to the reported
        ratios removes the whole class.
        """
        worst = 0.0
        for seed in range(40):
            rng = np.random.default_rng(seed)
            labels = np.array([0] * 100 + [1] * 100)
            scores = rng.normal(size=200)
            lrs = cross_validated_lrs(scores, labels, folds=10, seed=seed)
            worst = min(worst, decompose(lrs, labels).cllr_cal)
        assert worst >= -1e-9, f"floor leaked by {worst}"

    def test_the_floor_and_the_system_share_a_bound(self):
        """C1: the clip must not leak into the calibration loss.

        An unbounded floor is allowed to be ten orders of magnitude more
        confident than the bounded system it is the floor for. Measured across
        all 45 cells of the spatial corpus, that inflated the reported
        calibration loss by a median 70% on the 18 cells where the detector
        discriminates. See `bound_attribution.py`.
        """
        rng = np.random.default_rng(3)
        labels = np.array([0] * 400 + [1] * 400)
        scores = rng.normal(loc=4.0 * labels, scale=1.0)
        lrs = cross_validated_lrs(scores, labels, folds=10, bound=100.0)
        d = decompose(lrs, labels, bound=100.0)
        # Nothing may be cheaper than a flawless system under the same clip.
        assert d.cllr_min >= d.bound_cost - 1e-9
        assert d.cllr_cal >= -1e-9

    def test_bound_cost_is_reported_and_correct(self):
        """A flawless bounded system does not score zero, and must say so."""
        assert bound_cost(100.0) == pytest.approx(0.0144, abs=5e-4)
        assert bound_cost(1000.0) < bound_cost(100.0) < bound_cost(10.0)
        rng = np.random.default_rng(1)
        labels = np.array([0] * 100 + [1] * 100)
        scores = rng.normal(loc=labels)
        d = decompose(cross_validated_lrs(scores, labels, folds=5), labels)
        assert d.bound_cost == pytest.approx(bound_cost(100.0))

    def test_better_separated_scores_discriminate_better(self):
        floors = []
        for separation in (0.0, 1.0, 3.0):
            scores, labels = self._sample(separation)
            lrs = cross_validated_lrs(scores, labels, folds=5)
            floors.append(decompose(lrs, labels).cllr_min)
        assert floors[0] > floors[1] > floors[2]

    def test_scores_carrying_nothing_are_reported_as_uninformative(self):
        scores, labels = self._sample(0.0)
        lrs = cross_validated_lrs(scores, labels, folds=5)
        d = decompose(lrs, labels)
        assert d.cllr == pytest.approx(1.0, abs=0.15)

    def test_the_counts_are_carried_through(self):
        scores, labels = self._sample(1.0, n=300)
        lrs = cross_validated_lrs(scores, labels, folds=5)
        d = decompose(lrs, labels)
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

    @staticmethod
    def _pairs(n):
        """Cover identifiers for `_identical_distributions`: i and i+n are one cover."""
        return np.concatenate([np.arange(n), np.arange(n)])

    def test_pair_aware_folds_score_exactly_one_on_null_data(self):
        """The correctness anchor for the whole pipeline.

        Both sides carry identical scores, so the true cost is exactly 1.000
        and there is nothing to argue about. With both members of every
        training pair present the logistic likelihood is symmetric, the fitted
        coefficient is zero, every ratio is 1, and the cost is 1.000 exactly
        with no seed to seed spread.
        """
        n = 200
        scores, labels = self._identical_distributions(n)
        groups = self._pairs(n)
        for seed in range(5):
            lrs = cross_validated_lrs(scores, labels, folds=10, seed=seed, groups=groups)
            observed = cllr(lrs[labels == 1], lrs[labels == 0])
            assert observed == pytest.approx(1.0, abs=1e-9), (
                f"seed {seed} gave {observed!r} on an arm whose true cost is 1.000"
            )

    def test_splitting_pairs_across_folds_biases_the_cost_upward(self):
        """Why `groups` is not optional, pinned so the fix cannot be undone.

        This is a real defect that reached a draft paper: folds assigned on the
        label alone separate a cover from its twin about nine times in ten, the
        calibrator fits the resulting imbalance, and the cost drifts upward by
        enough to turn "tells you nothing" into "worse than silence". The test
        asserts the gap between the two schemes, not the broken value, so that
        a future reader meets the reason rather than the symptom.
        """
        n = 200
        scores, labels = self._identical_distributions(n)
        grouped, split = [], []
        for seed in range(5):
            g = cross_validated_lrs(scores, labels, folds=10, seed=seed, groups=self._pairs(n))
            s = cross_validated_lrs(scores, labels, folds=10, seed=seed)
            grouped.append(cllr(g[labels == 1], g[labels == 0]))
            split.append(cllr(s[labels == 1], s[labels == 0]))
        assert np.allclose(grouped, 1.0, atol=1e-9)
        assert np.mean(split) > 1.0, "the unbiased scheme is supposed to be the grouped one"
        assert np.mean(split) - 1.0 > 1e-4, (
            "the bias this guards against has vanished; if that is a genuine "
            "improvement, update the docstring on cross_validated_lrs too"
        )

    def test_the_null_covers_the_observed_value_on_null_data(self):
        n = 200
        scores, labels = self._identical_distributions(n)
        lrs = cross_validated_lrs(scores, labels, folds=10, groups=self._pairs(n))
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
        rng = np.random.default_rng(2)
        labels = np.array([0] * 300 + [1] * 300)
        scores = rng.normal(loc=2.5 * labels, scale=1.0)
        lrs = cross_validated_lrs(scores, labels, folds=10)
        observed = cllr(lrs[labels == 1], lrs[labels == 0])
        null = cllr_null(scores, labels, permutations=50, folds=10)
        assert observed < 0.6
        assert (null <= observed).mean() == 0.0

    def test_it_is_reproducible(self):
        scores, labels = self._identical_distributions(n=100)
        a = cllr_null(scores, labels, permutations=20, folds=5, seed=1)
        b = cllr_null(scores, labels, permutations=20, folds=5, seed=1)
        assert np.array_equal(a, b)


class TestTheScaleOfTheScores:
    """C3: a false negative made of arithmetic rather than of evidence."""

    @staticmethod
    def _shifted(offset: float, seed: int = 11):
        rng = np.random.default_rng(seed)
        labels = np.array([0] * 200 + [1] * 200)
        return rng.normal(loc=labels, scale=1.0) + offset, labels

    @pytest.mark.parametrize("offset", [0.0, 1e3, 1e5, 1e8])
    def test_a_large_offset_does_not_destroy_the_fit(self, offset):
        """Adding a constant to every score cannot change the evidence.

        Before standardisation, an offset of 1e5 saturated the objective, the
        gradient underflowed, BFGS stopped at the starting point and reported
        success, and a genuine one sigma separation came back as Cllr exactly
        1.000.
        """
        scores, labels = self._shifted(offset)
        lrs = cross_validated_lrs(scores, labels, folds=10)
        value = cllr(lrs[labels == 1], lrs[labels == 0])
        assert value < 0.95, f"offset {offset:g} reported Cllr {value}"

    def test_the_answer_is_invariant_to_offset_and_scale(self):
        base, labels = self._shifted(0.0)
        a = cllr(*(lambda l: (l[labels == 1], l[labels == 0]))(
            cross_validated_lrs(base, labels, folds=10)))
        moved = base * 1000.0 + 5e6
        b = cllr(*(lambda l: (l[labels == 1], l[labels == 0]))(
            cross_validated_lrs(moved, labels, folds=10)))
        assert a == pytest.approx(b, abs=1e-6)

    def test_a_constant_score_is_handled_rather_than_crashing(self):
        """Nothing to calibrate against, so the honest answer is LR = 1."""
        labels = np.array([0] * 50 + [1] * 50)
        scores = np.full(100, 7.0)
        cal = LogisticCalibrator().fit(scores, labels)
        assert cal.coef_ == 0.0
        assert cal.transform(scores, 1.0) == pytest.approx(np.ones(100))

    def test_non_finite_scores_are_refused(self):
        labels = np.array([0] * 10 + [1] * 10)
        scores = np.concatenate([np.zeros(10), np.full(10, np.nan)])
        with pytest.raises(ValueError, match="finite"):
            LogisticCalibrator().fit(scores, labels)


class TestPairedNull:
    """M3: the null has to respect the design the analysis constructs."""

    @staticmethod
    def _paired_null_data(n: int = 150, seed: int = 0):
        """Identical scores on both sides, paired by cover."""
        rng = np.random.default_rng(seed)
        per_cover = rng.normal(size=n)
        scores = np.concatenate([per_cover, per_cover])
        labels = np.concatenate([np.ones(n, int), np.zeros(n, int)])
        pairs = np.concatenate([np.arange(n), np.arange(n)])
        return scores, labels, pairs

    @staticmethod
    def _unpaired_null_data(n: int = 150, seed: int = 0):
        """Null, but not degenerate: two independent draws from one distribution.

        `_paired_null_data` makes the two sides byte identical, which is the
        structural arm and is a special case: a within pair flip there leaves
        the data unchanged, so the null is a point mass. For testing that the
        null *covers* a null arm, the arm has to have something to permute.
        """
        rng = np.random.default_rng(seed)
        scores = np.concatenate([rng.normal(size=n), rng.normal(size=n)])
        labels = np.concatenate([np.ones(n, int), np.zeros(n, int)])
        pairs = np.concatenate([np.arange(n), np.arange(n)])
        return scores, labels, pairs

    def test_the_paired_null_covers_a_provably_null_arm(self):
        scores, labels, pairs = self._unpaired_null_data()
        mean, _, _ = observed_cllr(scores, labels, folds=10, seeds=10, groups=pairs)
        null = cllr_null(scores, labels, permutations=40, folds=10, pairs=pairs)
        p = (1 + (null <= mean).sum()) / (1 + len(null))
        assert 0.05 < p < 0.95, f"a provably null arm was called a finding at p={p}"

    def test_on_byte_identical_sides_the_paired_null_is_a_point_mass(self):
        """The structural arm's degenerate case, pinned rather than papered over.

        When both sides carry the same scores, flipping a label within a pair
        changes nothing, so every permutation reproduces the observed value
        exactly and p is 1.0 by construction. That arm can demonstrate the
        fold bias; it cannot test whether the null is correctly centred,
        because it has no randomness for the null to explore.
        """
        scores, labels, pairs = self._paired_null_data()
        mean, sd, _ = observed_cllr(scores, labels, folds=10, seeds=10, groups=pairs)
        null = cllr_null(scores, labels, permutations=40, folds=10, pairs=pairs)
        assert mean == pytest.approx(1.0, abs=1e-9)
        assert sd == pytest.approx(0.0, abs=1e-12)
        assert np.allclose(null, 1.0, atol=1e-9)

    def test_pairing_changes_the_null(self):
        """If it did not, the argument for it would be decoration."""
        scores, labels, pairs = self._paired_null_data()
        free = cllr_null(scores, labels, permutations=40, folds=10, seed=1)
        paired = cllr_null(scores, labels, permutations=40, folds=10, seed=1, pairs=pairs)
        assert paired.std() < free.std()

    def test_mismatched_pair_ids_are_refused(self):
        scores, labels, _ = self._paired_null_data(n=20)
        with pytest.raises(ValueError, match="pair ids"):
            cllr_null(scores, labels, permutations=2, folds=5, pairs=np.arange(3))

    def test_a_real_signal_still_clears_the_paired_null(self):
        rng = np.random.default_rng(5)
        n = 150
        cover = rng.normal(size=n)
        scores = np.concatenate([cover + 2.5, cover])
        labels = np.concatenate([np.ones(n, int), np.zeros(n, int)])
        pairs = np.concatenate([np.arange(n), np.arange(n)])
        mean, _, _ = observed_cllr(scores, labels, folds=10, seeds=5)
        null = cllr_null(scores, labels, permutations=20, folds=10, pairs=pairs)
        assert (null <= mean).sum() == 0


class TestObservedCllrReportsItsSpread:
    def test_the_spread_is_not_negligible(self):
        """M3: one cross validation is a draw, not a measurement.

        On a null arm the fold seed moved Cllr over a range of 0.0079, wider
        than the bias the whole null apparatus exists to correct. Printing a
        single draw to three decimals presented that as a measurement.
        """
        rng = np.random.default_rng(0)
        per_cover = rng.normal(size=150)
        scores = np.concatenate([per_cover, per_cover])
        labels = np.concatenate([np.ones(150, int), np.zeros(150, int)])
        mean, sd, draws = observed_cllr(scores, labels, folds=10, seeds=20)
        assert len(draws) == 20
        assert sd > 0, "the fold seed must move the answer, or this is not needed"
        assert draws.min() < mean < draws.max()

    def test_it_is_reproducible(self):
        rng = np.random.default_rng(2)
        labels = np.array([0] * 100 + [1] * 100)
        scores = rng.normal(loc=labels)
        a, _, _ = observed_cllr(scores, labels, folds=5, seeds=5)
        b, _, _ = observed_cllr(scores, labels, folds=5, seeds=5)
        assert a == b


class TestTheDecompositionIsOneEstimator:
    """M6: the three printed columns must be arithmetically consistent.

    Averaging Cllr over 20 fold seeds and taking Cllr_min from one seed gives
    two individually correct numbers whose difference is a quantity nothing
    computed. A reader subtracts the columns and gets a calibration loss that
    mixes estimators.
    """

    def test_the_columns_subtract(self):
        rng = np.random.default_rng(0)
        labels = np.array([0] * 150 + [1] * 150)
        scores = rng.normal(loc=1.2 * labels)
        d, _ = observed_decomposition(scores, labels, folds=10, seeds=8)
        assert d.cllr - d.cllr_min == pytest.approx(d.cllr_cal, abs=1e-12)

    def test_it_agrees_with_one_seed_when_there_is_only_one(self):
        rng = np.random.default_rng(1)
        labels = np.array([0] * 120 + [1] * 120)
        scores = rng.normal(loc=0.8 * labels)
        d, sd = observed_decomposition(scores, labels, folds=10, seeds=1)
        single = decompose(cross_validated_lrs(scores, labels, folds=10, seed=0), labels)
        assert d.cllr == pytest.approx(single.cllr)
        assert d.cllr_min == pytest.approx(single.cllr_min)
        assert sd == pytest.approx(0.0)

    def test_the_total_matches_observed_cllr_over_the_same_seeds(self):
        """The two entry points must not disagree about the same quantity."""
        rng = np.random.default_rng(4)
        labels = np.array([0] * 100 + [1] * 100)
        scores = rng.normal(loc=1.0 * labels)
        d, sd = observed_decomposition(scores, labels, folds=10, seeds=6)
        mean, spread, _ = observed_cllr(scores, labels, folds=10, seeds=6)
        assert d.cllr == pytest.approx(mean)
        assert sd == pytest.approx(spread)

    def test_the_loss_stays_non_negative_on_an_inverted_detector(self):
        """The floor must not be beaten, averaged or not."""
        rng = np.random.default_rng(7)
        labels = np.array([0] * 200 + [1] * 200)
        scores = rng.normal(loc=-2.0 * labels)
        d, _ = observed_decomposition(scores, labels, folds=10, seeds=5)
        assert d.cllr_cal >= -1e-9
        assert d.cllr_min < 1.0

    def test_the_counts_are_carried_through(self):
        labels = np.array([0] * 60 + [1] * 40)
        rng = np.random.default_rng(9)
        scores = rng.normal(loc=labels)
        d, _ = observed_decomposition(scores, labels, folds=5, seeds=3)
        assert (d.n_payload, d.n_clean) == (40, 60)
        assert d.bound_cost == pytest.approx(bound_cost(100.0))


class TestInputValidation:
    def test_labels_outside_zero_and_one_are_named(self):
        with pytest.raises(ValueError, match="labels must be 0 or 1"):
            decompose(np.ones(4), np.array([0, 2, 0, 2]))

    def test_pav_accepts_plain_lists(self):
        assert pav([1.0, 2.0, 3.0, 4.0], [0, 0, 1, 1]) == pytest.approx([0.0, 0.0, 1.0, 1.0])

    def test_pav_refuses_mismatched_lengths(self):
        with pytest.raises(ValueError, match="against"):
            pav([1.0, 2.0, 3.0], [0, 1])
