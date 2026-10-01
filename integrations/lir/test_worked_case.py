#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the reporting sentence, which had no tests and carried a defect.

The defect worth guarding: a likelihood ratio clipped to the reporting bound
was given the verbal band ABOVE the one the data supports, because a bound of
100 is the first value of the next band. Every censored case in a laboratory
collapsed onto a scale boundary and fell on the stronger side of it, so a
ratio nobody was willing to put above 100 was reported as "moderately strong
support". Setting the bound to 99.9 would have changed the same exhibit's
wording, which is how the tie was found.

These tests pin both halves: the scale itself, so the fix did not quietly
weaken it, and the one-sided reading for a censored value.
"""
from __future__ import annotations

import math

import pytest

from worked_case import BANDS, sentence, verbal


@pytest.mark.parametrize(
    "lr,expected",
    [
        (1.0, "does not support either proposition"),
        (1.9, "does not support either proposition"),
        (2.0, "weak support"),
        (9.99, "weak support"),
        (10.0, "moderate support"),
        (99.9, "moderate support"),
        (100.0, "moderately strong support"),
        (999.0, "moderately strong support"),
        (1000.0, "strong support"),
        (1e9, "strong support"),
    ],
)
def test_the_uncensored_scale_is_the_enfsi_scale(lr, expected):
    assert verbal(lr) == expected


@pytest.mark.parametrize("lr,expected", [(2.0, "weak support"), (0.5, "weak support")])
def test_a_ratio_below_one_takes_the_band_of_its_reciprocal(lr, expected):
    assert verbal(lr) == expected


def test_a_censored_ratio_takes_the_band_it_is_the_top_of():
    """The defect, in one assertion.

    At the bound the true ratio is only known to be at least 100. The band
    below is the one the sample supports, and naive banding returns the band
    above.
    """
    assert verbal(100.0, censored=True) == "moderate support"
    assert verbal(100.0, censored=False) == "moderately strong support"


def test_censoring_is_symmetric_about_one():
    assert verbal(0.01, censored=True) == "moderate support"


def test_censoring_only_moves_a_value_sitting_exactly_on_a_boundary():
    """A censored value away from a boundary must band as it otherwise would.

    Otherwise the flag would quietly downgrade every clipped case rather than
    only the ones where the tie actually bites.
    """
    for lr in (3.0, 47.0, 150.0, 5000.0):
        assert verbal(lr, censored=True) == verbal(lr, censored=False)


def test_a_censored_sentence_reads_one_sided():
    first, second = sentence(100.0, "RS", censored=True)
    assert "at least 100 times more probable" in first
    assert "at least moderate support" in second
    assert "moderately strong" not in second


def test_an_uncensored_sentence_does_not_hedge():
    first, second = sentence(150.0, "RS")
    assert "at least" not in first
    assert "at least" not in second
    assert "moderately strong support" in second


def test_a_ratio_at_one_supports_neither_proposition_in_either_direction():
    for censored in (True, False):
        _, second = sentence(1.0, "SPA", censored=censored)
        assert second == "The evidence does not support either proposition."


def test_the_bands_are_ascending_and_closed_at_infinity():
    """`verbal` walks BANDS in order and raises if it falls off the end."""
    limits = [top for top, _ in BANDS]
    assert limits == sorted(limits)
    assert limits[-1] == math.inf
