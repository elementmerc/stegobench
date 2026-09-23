#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the collection licence check.

    python3 -m unittest discover -s generators -p 'test_*.py'

Until 2026-09-21 the collection licence was a constant, and the run printed
"strictest present, not loosest" while nothing had read a licence. A share-alike
cover reaching the corpus would have been published under a licence that did not
cover it, and the sentence claiming otherwise would have printed identically.
That is the shape these tests exist to keep out: a statement that reads as a
measurement.
"""
from __future__ import annotations

import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from publish_tier import obligation_class, strictest_obligation  # noqa: E402


class ObligationClassTests(unittest.TestCase):
    def test_public_domain_and_cc0_impose_nothing(self):
        for licence in ("CC0", "Public domain", "No rights reserved"):
            self.assertEqual(obligation_class(licence), "none", licence)

    def test_every_cc_by_version_is_attribution(self):
        for v in ("1.0", "2.0", "2.5", "3.0", "4.0"):
            self.assertEqual(obligation_class(f"CC BY {v}"), "attribution")

    def test_share_alike_is_not_covered(self):
        for licence in ("CC BY-SA 3.0", "CC BY-SA 4.0", "GFDL"):
            self.assertEqual(obligation_class(licence), "uncovered", licence)

    def test_non_commercial_is_not_covered(self):
        for licence in ("CC BY-NC 4.0", "CC BY-NC-ND 4.0"):
            self.assertEqual(obligation_class(licence), "uncovered", licence)

    def test_an_unrecognised_licence_is_uncovered_rather_than_assumed(self):
        """Guessing is worst exactly where the licence is unfamiliar."""
        self.assertEqual(obligation_class("Some Bespoke Terms v2"), "uncovered")
        self.assertEqual(obligation_class(""), "uncovered")


class StrictestTests(unittest.TestCase):
    def test_the_corpus_as_it_stands_is_covered(self):
        strictest, uncovered = strictest_obligation(
            ["CC0", "CC BY 2.0", "Public domain", "CC BY 4.0", "CC BY 3.0",
             "CC BY 2.5", "CC BY 1.0"])
        self.assertEqual(strictest, "attribution")
        self.assertEqual(uncovered, [])

    def test_an_all_public_domain_set_needs_no_attribution(self):
        strictest, uncovered = strictest_obligation(["CC0", "Public domain"])
        self.assertEqual(strictest, "none")
        self.assertEqual(uncovered, [])

    def test_one_share_alike_cover_is_reported(self):
        """The defect this guards, at the smallest scale that produces it."""
        strictest, uncovered = strictest_obligation(
            ["CC0", "CC BY 4.0", "CC BY-SA 3.0"])
        self.assertEqual(uncovered, ["CC BY-SA 3.0"])

    def test_every_uncovered_value_is_named_not_just_the_first(self):
        _, uncovered = strictest_obligation(
            ["CC BY 4.0", "CC BY-SA 3.0", "CC BY-NC 4.0", "GFDL"])
        self.assertEqual(len(uncovered), 3)

    def test_duplicates_are_reported_once(self):
        _, uncovered = strictest_obligation(["CC BY-SA 3.0"] * 5)
        self.assertEqual(uncovered, ["CC BY-SA 3.0"])


if __name__ == "__main__":
    unittest.main()
