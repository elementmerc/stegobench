#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the rules that decide which covers get dropped.

    python3 -m unittest discover -s generators -p 'test_*.py'

These rules delete work. A first version of them selected 1,177 covers when the
right answer was 109, and dropping 12% of a corpus is not a safe default merely
because it errs towards caution: it throws away images somebody licensed to us,
it changes what the corpus measures, and it would have been done on a rule
nobody had checked. Both over-strict rules are pinned below.
"""
from __future__ import annotations

import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from select_unpublishable import classify, is_us_only, matches  # noqa: E402


def row(**kw) -> dict:
    base = {
        "file": "00000.png",
        "title": "File:Example.jpg",
        "licence": "CC BY 4.0",
        "artist": "A Photographer",
        "attribution_required": True,
        "pd_grounds": None,
    }
    base.update(kw)
    return base


class MatchesTests(unittest.TestCase):
    def test_a_version_difference_is_not_a_mismatch(self):
        self.assertTrue(matches("Cc-by-3.0", "CC BY 3.0"))
        self.assertTrue(matches("Cc-by-4.0", "CC BY 4.0"))

    def test_share_alike_does_not_satisfy_a_plain_cc_by_record(self):
        # The two carry different obligations. Accepting one for the other is
        # how a corpus publishes copyleft material under a permissive licence.
        self.assertFalse(matches("Cc-by-sa-3.0", "CC BY 3.0"))

    def test_the_public_domain_dedications_that_wear_a_cc_prefix(self):
        # `Cc-pd` and `Cc-zero` are public domain, and reading the prefix alone
        # files them as attribution licences.
        self.assertTrue(matches("Cc-pd", "Public domain"))
        self.assertTrue(matches("Cc-zero", "CC0"))
        self.assertTrue(matches("PD-USGov-NASA", "Public domain"))


class UsOnlyTests(unittest.TestCase):
    def test_a_us_term_expiring_says_nothing_about_the_source_country(self):
        self.assertTrue(is_us_only(["PD-US-expired"]))
        self.assertTrue(is_us_only(["PD-1996-text"]))

    def test_a_federal_work_is_not_jurisdiction_limited(self):
        """576 covers turned on this one line.

        A work of the US federal government is uncopyrighted by statute rather
        than by a term running out, and is treated as free worldwide. Reading
        "USGov" as "United States only" dropped every NASA photograph in the
        corpus.
        """
        for ground in ("PD-USGov", "PD-USGov-NASA", "PD-USGov-Military-Navy",
                       "PD-USGov-POTUS", "PD-USGov-NPS"):
            self.assertFalse(is_us_only([ground]), ground)

    def test_a_source_country_ground_alongside_settles_it(self):
        self.assertFalse(is_us_only(["PD-US-expired", "PD-old-70"]))
        self.assertFalse(is_us_only(["PD-US", "PD-self"]))

    def test_no_grounds_is_not_a_us_only_finding(self):
        # It is the `ungrounded` group, which is a different problem.
        self.assertFalse(is_us_only([]))


class ClassifyTests(unittest.TestCase):
    def test_a_clean_row_is_publishable(self):
        self.assertIsNone(classify(row(), {"File:Example.jpg": ["Cc-by-4.0"]}))

    def test_a_dual_licensed_file_recorded_at_its_permissive_option(self):
        """522 covers turned on this one.

        Commons files are very often offered under several licences at once,
        `Cc-by-4.0` beside `GFDL` being the commonest. Taking the permissive
        one is the point of a multi-licence offer.
        """
        page = {"File:Example.jpg": ["Cc-by-4.0", "GFDL"]}
        self.assertIsNone(classify(row(licence="CC BY 4.0"), page))

        page = {"File:Example.jpg": ["Cc-by-2.5", "Cc-by-sa-3.0-migrated", "GFDL"]}
        self.assertIsNone(classify(row(licence="CC BY 2.5"), page))

    def test_share_alike_and_nothing_else_is_refused(self):
        page = {"File:Example.jpg": ["Cc-by-sa-4.0"]}
        self.assertEqual(classify(row(licence="Public domain", pd_grounds=["PD-self"]),
                                  page), "share-alike")

    def test_a_grant_that_is_not_the_one_recorded_is_refused(self):
        page = {"File:Example.jpg": ["Cc-by-4.0"]}
        self.assertEqual(classify(row(licence="Public domain", pd_grounds=["PD-self"]),
                                  page), "misrecorded")

    def test_attribution_required_with_no_author_is_refused(self):
        self.assertEqual(
            classify(row(artist="", licence="CC BY 4.0"),
                     {"File:Example.jpg": ["Cc-by-4.0"]}),
            "unattributable")

    def test_no_author_is_fine_when_no_attribution_is_required(self):
        self.assertIsNone(classify(
            row(artist="", licence="CC0", attribution_required=False),
            {"File:Example.jpg": ["Cc-zero"]}))

    def test_public_domain_with_no_ground_is_refused(self):
        self.assertEqual(
            classify(row(licence="Public domain", attribution_required=False,
                         pd_grounds=[]), {"File:Example.jpg": ["PD-Layout"]}),
            "ungrounded")

    def test_an_unaudited_row_is_left_alone(self):
        # `pd_grounds` absent means nobody looked, which is not a finding.
        self.assertIsNone(classify(
            row(licence="Public domain", attribution_required=False,
                pd_grounds=None), {"File:Example.jpg": ["PD-self"]}))

    def test_a_page_we_could_not_read_is_not_condemned(self):
        # No templates recovered means no evidence, and no evidence is not
        # evidence of a problem.
        self.assertIsNone(classify(row(), {}))


if __name__ == "__main__":
    unittest.main()
