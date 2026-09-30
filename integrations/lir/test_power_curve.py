#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the pure helpers in `power_curve`.

The measurement itself needs a corpus and roughly forty minutes, so it is not
exercised here. These two functions are what the reported numbers are read
through: `wilson` produces every interval in the power column and `crossing`
produces every "power N% at" line. A wrong answer from either misreports a
result without failing anything.
"""
from __future__ import annotations

import math

import pytest

from power_curve import crossing, wilson


class TestWilson:
    def test_zero_successes_excludes_nothing_below_zero(self):
        lo, hi = wilson(0, 80)
        assert lo == 0.0
        assert 0.0 < hi < 0.1

    def test_all_successes_is_capped_at_one(self):
        lo, hi = wilson(80, 80)
        assert hi == 1.0
        assert 0.9 < lo < 1.0

    def test_interval_brackets_the_point_estimate(self):
        for k in (1, 7, 33, 64, 79):
            lo, hi = wilson(k, 80)
            assert lo <= k / 80 <= hi

    def test_the_published_size_check_reproduces(self):
        # 2 of 80 is the 0.025 the n = 200 run reports, and the interval quoted
        # in the paper is [0.007, 0.087]. If this drifts, the paper is wrong.
        lo, hi = wilson(2, 80)
        assert round(lo, 3) == 0.007
        assert round(hi, 3) == 0.087

    def test_the_published_eighty_percent_point_reproduces(self):
        lo, hi = wilson(64, 80)
        assert round(lo, 3) == 0.700
        assert round(hi, 3) == 0.873

    def test_wider_at_the_same_proportion_with_less_data(self):
        narrow = wilson(40, 80)
        wide = wilson(5, 10)
        assert (wide[1] - wide[0]) > (narrow[1] - narrow[0])

    def test_no_data_is_the_whole_interval_rather_than_a_division_by_zero(self):
        assert wilson(0, 0) == (0.0, 1.0)

    def test_interval_is_finite_everywhere_on_a_small_n(self):
        for k in range(0, 6):
            lo, hi = wilson(k, 5)
            assert math.isfinite(lo) and math.isfinite(hi)
            assert 0.0 <= lo <= hi <= 1.0


class TestCrossing:
    XS = [0.0, 0.03, 0.06, 0.09, 0.12]
    PW = [0.025, 0.037, 0.412, 0.800, 1.000]

    def test_exact_grid_hit_returns_that_point(self):
        # 0.800 sits exactly on the grid, so the 80% crossing must be d = 0.09
        # and not an interpolation through it.
        assert crossing(self.XS, self.PW, 0.80) == pytest.approx(0.09)

    def test_interpolates_between_two_points(self):
        d = crossing(self.XS, self.PW, 0.50)
        assert 0.06 < d < 0.09
        assert d == pytest.approx(0.06 + (0.50 - 0.412) / (0.800 - 0.412) * 0.03)

    def test_unreached_level_returns_none_rather_than_the_last_point(self):
        # The n = 160 run never reaches 95%, and reporting the top of the grid
        # as though it were the crossing is exactly the overstatement this
        # returns None to avoid.
        assert crossing([0.0, 0.06], [0.28, 0.83], 0.95) is None

    def test_level_already_met_at_the_first_point_is_not_a_crossing(self):
        # Nothing crossed: the loop starts at index 1 by design, so a level the
        # first point already satisfies is not reported as attained partway.
        assert crossing([0.0, 0.1], [0.99, 1.0], 0.50) is None

    def test_flat_segment_does_not_divide_by_zero(self):
        assert crossing([0.0, 0.1, 0.2], [0.4, 0.4, 0.9], 0.4) is None
        assert crossing([0.0, 0.1], [0.2, 0.5], 0.5) == pytest.approx(0.1)

    def test_non_monotone_input_takes_the_first_upward_crossing(self):
        # The refused n = 160 curve is non-monotone. Whatever is reported from
        # such a curve, it must be deterministic and it must be the first
        # crossing rather than the last.
        xs = [0.0, 0.06, 0.12, 0.18]
        pw = [0.287, 0.062, 0.200, 0.838]
        assert crossing(xs, pw, 0.50) == pytest.approx(
            0.12 + (0.50 - 0.200) / (0.838 - 0.200) * 0.06
        )

    def test_single_point_cannot_cross(self):
        assert crossing([0.0], [1.0], 0.5) is None
