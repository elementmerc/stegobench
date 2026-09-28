#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""The two metric functions every reported number in this directory rests on.

    python3 -m unittest discover -s generators -p 'test_*.py'

WHY THIS EXISTS
---------------
`roc_auc` and `tpr_at_fpr` live in `score_arms.py` and are imported by
`panel_scores.py`, so every figure the panel prints comes out of them, and
nothing tested them. What they answer when the input is not what the caller
thinks it is matters more here than the happy path: a metric that quietly
returns a number over half the images, or over scores it could not order, is
indistinguishable in a report from one that measured what it says it did.
"""
from __future__ import annotations

import io
import math
import pathlib
import random
import sys
import unittest
from contextlib import redirect_stdout

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from score_arms import report, roc_auc, tpr_at_fpr  # noqa: E402


def naive_tpr_at_fpr(scores, labels, max_fpr):
    """The obvious quadratic shape, kept as the thing the fast one is checked
    against. It is the implementation this module shipped with."""
    positives = sum(labels)
    negatives = len(labels) - positives
    if not positives or not negatives:
        return None
    best = 0.0
    for threshold in sorted(set(scores)):
        tp = sum(1 for s, l in zip(scores, labels) if l and s >= threshold)
        fp = sum(1 for s, l in zip(scores, labels) if not l and s >= threshold)
        if fp / negatives <= max_fpr:
            best = max(best, tp / positives)
    return best


class TprAtFprTests(unittest.TestCase):
    def test_known_cases(self):
        # 2 stego (0.9, 0.4), 2 clean (0.5, 0.1). Catching the 0.4 stego means
        # first admitting the 0.5 clean, so a zero budget buys half.
        scores = [0.9, 0.4, 0.5, 0.1]
        labels = [True, True, False, False]
        self.assertEqual(tpr_at_fpr(scores, labels, 0.0), 0.5)
        self.assertEqual(tpr_at_fpr(scores, labels, 0.5), 1.0)

    def test_agrees_with_the_quadratic_version_it_replaced(self):
        rng = random.Random(20260928)
        for trial in range(200):
            n = rng.randrange(2, 40)
            # Deliberately few distinct values on some trials, so tie groups
            # are common: ties are where a sweep and a per-threshold scan are
            # easiest to make disagree.
            span = rng.choice([2, 3, 1000])
            scores = [rng.randrange(span) / span for _ in range(n)]
            labels = [rng.random() < 0.5 for _ in range(n)]
            if not any(labels) or all(labels):
                continue
            for budget in (0.0, 0.01, 0.1, 0.5, 1.0):
                self.assertEqual(
                    tpr_at_fpr(scores, labels, budget),
                    naive_tpr_at_fpr(scores, labels, budget),
                    f"trial {trial}, budget {budget}: {scores} {labels}",
                )

    def test_a_budget_that_is_not_a_rate_is_refused(self):
        scores, labels = [0.9, 0.1], [True, False]
        for budget in (-0.1, 1.5, math.nan):
            with self.assertRaises(ValueError):
                tpr_at_fpr(scores, labels, budget)

    def test_a_single_class_has_no_answer(self):
        self.assertIsNone(tpr_at_fpr([0.1, 0.2], [True, True], 0.01))


class RocAucTests(unittest.TestCase):
    def test_known_cases(self):
        self.assertEqual(roc_auc([0.1, 0.2, 0.8, 0.9], [False, False, True, True]), 1.0)
        self.assertEqual(roc_auc([0.9, 0.8, 0.2, 0.1], [False, False, True, True]), 0.0)
        # Every score identical: the average-rank treatment is what makes this
        # exactly 0.5 rather than whatever the sort happened to do.
        self.assertEqual(roc_auc([0.5] * 4, [True, False, True, False]), 0.5)

    def test_a_single_class_has_no_answer(self):
        self.assertIsNone(roc_auc([0.1, 0.2], [True, True]))


class UnrankableInputTests(unittest.TestCase):
    """Two ways the input can be something no ranking should be built on."""

    def test_a_length_mismatch_is_refused_rather_than_truncated(self):
        # `zip` would walk the shorter list and answer over a subset nobody
        # named, which in a report is indistinguishable from the real figure.
        scores = [0.9, 0.8, 0.2, 0.1]
        with self.assertRaises(ValueError):
            roc_auc(scores, [True, False, True])
        with self.assertRaises(ValueError):
            tpr_at_fpr(scores, [True, False, True], 0.01)
        with self.assertRaises(ValueError):
            roc_auc(scores[:3], [True, False, True, False])

    def test_a_score_that_is_not_a_number_has_no_ranking(self):
        # NaN compares false against everything, so it sorts wherever it
        # started and the rank sum built on it means nothing.
        scores = [0.9, math.nan, 0.2, 0.1]
        labels = [True, True, False, False]
        self.assertIsNone(roc_auc(scores, labels))
        self.assertIsNone(tpr_at_fpr(scores, labels, 0.01))


class ReportTests(unittest.TestCase):
    def test_an_arm_with_no_computable_auc_prints_a_dash_rather_than_failing(self):
        # The report formats the AUC with a width, and `format(None, ">6")`
        # raises. It reaches the printer whenever a detector's scores cannot be
        # ranked, which is at the very end of a scoring run that has already
        # cost hours.
        rows = [{"arm": "wow-0200", "clean": "clean/0.png", "stego": "wow/0.png"}]
        scored = {
            "clean/0.png": {"file": "clean/0.png", "stegashield": {"probability": math.nan}},
            "wow/0.png": {"file": "wow/0.png", "stegashield": {"probability": 0.9}},
        }
        buffer = io.StringIO()
        with redirect_stdout(buffer):
            report(rows, scored, with_stegcore=False)
        self.assertIn("wow-0200", buffer.getvalue())


if __name__ == "__main__":
    unittest.main()
