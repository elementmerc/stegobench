#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""The seam between Python and the one implementation of the metrics.

    python3 -m unittest discover -s generators -p 'test_*.py'

WHY THIS EXISTS
---------------
Every reported number in this directory used to come out of arithmetic written
here, beside a second copy of the same arithmetic in Rust. The two disagreed,
and nothing in either output said which one a reader was holding. `metrics.py`
removes the copy by calling the binary, so what has to be tested changes shape:
not "is the AUC right", which the Rust crate's own tests cover, but "does a
Python caller reach that code, and does it get a refusal rather than a number
when the numbers cannot honestly be ranked".

The tests that need a built binary skip with a message naming what to build
when there is not one. A skip here is a gap rather than a pass: there is
deliberately no Python fallback, so on a machine with no binary these numbers
cannot be produced at all, which is the intended behaviour and not something
this can assert around.
"""
from __future__ import annotations

import math
import os
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import metrics as M  # noqa: E402
from metrics import MetricsRefused, MetricsUnavailable, metrics  # noqa: E402


def binary_or_skip() -> pathlib.Path:
    try:
        path, _ = M.resolve_binary()
    except MetricsUnavailable as e:
        raise unittest.SkipTest(f"no stegobench binary: {e}") from e
    return path


def on_the_budget() -> tuple[list[float], list[bool]]:
    """The fixture the two implementations disagreed on.

    100 clean images and 100 stego. One clean image outranks everything, then
    40 stego, then the rest. At a false-alarm budget of one per cent the single
    false alarm is exactly the budget, not a hair over it, so the 40 stego
    images below it are affordable and the detection rate is 0.40.

    The implementation this replaced set its threshold at the highest clean
    score inside the budget and counted stego images scoring STRICTLY above it,
    which drops every stego image tied with that threshold. Here that is the
    whole 40, and the figure it reports is 0.00.
    """
    scores = [1.0]
    labels = [False]
    scores += [0.9] * 40
    labels += [True] * 40
    scores += [0.5] * 99
    labels += [False] * 99
    scores += [0.1] * 60
    labels += [True] * 60
    return scores, labels


def retired_quantile_tpr(pos: list[float], neg: list[float], target: float) -> float:
    """The implementation `emit_results.py` carried until this consolidation.

    Kept here, and nowhere that runs, so the difference the change makes is
    written down and checked rather than described in a commit message.
    """
    allowed = int(len(neg) * target)
    cut = sorted(neg, reverse=True)[allowed - 1] if allowed >= 1 else max(neg)
    return sum(1 for v in pos if v > cut) / len(pos)


class BinaryResolutionTests(unittest.TestCase):
    """Where it looks, in what order, and what it says when it finds nothing."""

    def test_the_order_is_environment_then_path_then_this_checkout(self):
        wheres = [where for where, _ in M._candidates()]
        self.assertEqual(
            wheres,
            [
                "STEGOBENCH_BIN",
                "on PATH",
                "this checkout, release build",
                "this checkout, debug build",
            ],
        )

    def test_a_release_build_is_preferred_over_a_debug_one(self):
        # Compared by path parts rather than by a substring, because a
        # substring with a separator in it only matches on the platforms that
        # use that separator, and this is asserting an order that holds
        # everywhere.
        paths = [p for _, p in M._candidates() if p is not None]
        release = [i for i, p in enumerate(paths) if p.parent.name == "release"]
        debug = [i for i, p in enumerate(paths) if p.parent.name == "debug"]
        self.assertTrue(release and debug)
        self.assertLess(release[0], debug[0])

    def test_the_checkout_looks_for_the_name_this_platform_actually_builds(self):
        """A Windows build is `stegobench.exe` and nothing else.

        `shutil.which` applies PATHEXT, so the PATH candidate is fine either
        way, but the two that name a path in this checkout do not go through
        it. Without the extension they look for a file a Windows build never
        writes, and report the binary as missing while it sits beside them.
        """
        expected = "stegobench.exe" if os.name == "nt" else "stegobench"
        checkout = [p for where, p in M._candidates() if where.startswith("this checkout")]
        self.assertEqual(len(checkout), 2)
        for path in checkout:
            self.assertEqual(path.name, expected)

    def test_nothing_anywhere_names_every_place_and_the_line_to_type(self):
        original = M._candidates
        M.resolve_binary.cache_clear()
        M._candidates = lambda: [("nowhere", pathlib.Path("/nonexistent/stegobench"))]
        try:
            with self.assertRaises(MetricsUnavailable) as caught:
                M.resolve_binary()
        finally:
            M._candidates = original
            M.resolve_binary.cache_clear()
        message = str(caught.exception)
        self.assertIn("/nonexistent/stegobench", message)
        self.assertIn("cargo build --release", message)
        self.assertIn("STEGOBENCH_BIN", message)
        # The absence of a fallback is part of the message, because somebody
        # reading it is about to go looking for the option to turn one on.
        self.assertIn("no Python fallback", message)


class HappyPathTests(unittest.TestCase):
    def setUp(self):
        binary_or_skip()

    def test_perfect_separation(self):
        answer = metrics([0.9, 0.8, 0.2, 0.1], [True, True, False, False],
                         budgets=[0.0, 0.5])
        self.assertEqual(answer["auc"], 1.0)
        self.assertEqual(answer["tpr_at_fpr"][0.0], 1.0)
        self.assertEqual((answer["n_clean"], answer["n_stego"]), (2, 2))

    def test_every_score_identical_is_exactly_a_half(self):
        # The average-rank treatment is what makes this 0.5 rather than
        # whatever the sort happened to do with the ties.
        answer = metrics([0.5] * 4, [True, False, True, False], budgets=[0.5])
        self.assertEqual(answer["auc"], 0.5)

    def test_the_answer_is_keyed_by_the_budget_that_was_asked_for(self):
        answer = metrics([0.9, 0.1], [True, False], budgets=[0.1, 0.5])
        self.assertEqual(sorted(answer["tpr_at_fpr"]), [0.1, 0.5])

    def test_the_default_budgets_are_the_three_a_result_document_carries(self):
        answer = metrics([0.9, 0.1], [True, False])
        self.assertEqual(sorted(answer["tpr_at_fpr"]), [0.01, 0.05, 0.10])


class RefusalTests(unittest.TestCase):
    """Every condition that must come back as a refusal rather than a figure."""

    def setUp(self):
        binary_or_skip()

    def assertRefused(self, reason, *args, **kwargs):
        with self.assertRaises(MetricsRefused) as caught:
            metrics(*args, **kwargs)
        self.assertEqual(caught.exception.reason, reason, str(caught.exception))
        return caught.exception

    def test_one_class_has_nothing_to_tell_apart(self):
        self.assertRefused("one-sided", [0.1, 0.2], [True, True])

    def test_a_length_mismatch_is_refused_rather_than_truncated(self):
        e = self.assertRefused("length-mismatch", [0.9, 0.8, 0.2], [True, False])
        self.assertIn("different record sets", str(e))

    def test_a_score_that_is_not_a_number_has_no_ranking(self):
        self.assertRefused("not-a-number",
                           [0.9, math.nan, 0.2, 0.1], [True, True, False, False])

    def test_nothing_at_all_is_not_a_detector_that_caught_nothing(self):
        self.assertRefused("empty", [], [])

    def test_a_budget_that_is_not_a_rate(self):
        for budget in (-0.1, 1.5, math.nan):
            self.assertRefused("budget-not-a-rate", [0.9, 0.1], [True, False],
                               budgets=[budget])

    def test_the_same_budget_twice(self):
        self.assertRefused("duplicate-budget", [0.9, 0.1], [True, False],
                           budgets=[0.01, 0.01])

    def test_no_budgets_at_all(self):
        self.assertRefused("no-budgets", [0.9, 0.1], [True, False], budgets=[])

    def test_an_infinite_score_is_refused_before_it_is_sent(self):
        # JSON cannot carry it, and sending it as "no answer" would turn a real
        # ranking into a missing one.
        self.assertRefused("not-representable",
                           [math.inf, 0.1], [True, False])

    def test_a_label_that_is_not_a_flag(self):
        self.assertRefused("label-not-a-flag", [0.9, 0.1], [True, "yes"])

    def test_a_refusal_carries_the_documented_exit_code(self):
        e = self.assertRefused("one-sided", [0.1, 0.2], [True, True])
        # 3, a pre-flight refusal: the tool is capable and is declining, and
        # retrying it unchanged will decline again.
        self.assertEqual(e.exit_code, 3)


class BoundaryCaseTests(unittest.TestCase):
    """The disagreement that made two implementations one."""

    def setUp(self):
        binary_or_skip()

    def test_an_operating_point_exactly_on_the_budget_is_inside_it(self):
        scores, labels = on_the_budget()
        answer = metrics(scores, labels, budgets=[0.01])
        self.assertEqual(answer["tpr_at_fpr"][0.01], 0.4)

    def test_the_retired_implementation_answered_differently_here(self):
        scores, labels = on_the_budget()
        pos = [s for s, l in zip(scores, labels) if l]
        neg = [s for s, l in zip(scores, labels) if not l]
        self.assertEqual(retired_quantile_tpr(pos, neg, 0.01), 0.0)
        self.assertEqual(metrics(scores, labels, budgets=[0.01])["tpr_at_fpr"][0.01], 0.4)


if __name__ == "__main__":
    unittest.main()
