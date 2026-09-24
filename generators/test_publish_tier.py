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
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import publish_tier  # noqa: E402
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


class KaggleCardTests(unittest.TestCase):
    """The card is what a reader meets before they download anything.

    The live dataset scored 0.47 on Kaggle's own usability measure with no
    keywords, no stated update frequency and no file descriptions, which means
    it was reachable only by somebody who already knew its name.
    """

    def setUp(self):
        self.packed = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.packed)
        for name in ("README.md", "LICENCES.md",
                     "load_pentimento.py", "pentimento-core-00000.tar"):
            (self.packed / name).write_text("x", encoding="utf-8")
        (self.packed / "ATTRIBUTION.csv").write_text(
            "file,licence,licence_url,artist,title,source,attribution\n",
            encoding="utf-8")

    def test_every_described_file_actually_ships(self):
        described = {r["path"] for r in publish_tier.kaggle_resources(self.packed)}
        present = {publish_tier.kaggle_name(p.name) for p in self.packed.iterdir()}
        self.assertTrue(described <= present,
                        f"describes files that are not there: {described - present}")

    def test_the_shards_are_described_under_the_name_they_land_under(self):
        """The shards are most of the file list and all of the data.

        They are named `.tar.bin` on Kaggle, because Kaggle unpacks anything
        named `.tar`, so a resource entry naming the `.tar` describes a file
        nobody can see and leaves the actual data unexplained.
        """
        described = {r["path"]: r["description"]
                     for r in publish_tier.kaggle_resources(self.packed)}
        self.assertIn("pentimento-core-00000.tar.bin", described)
        self.assertNotIn("pentimento-core-00000.tar", described)
        self.assertFalse([p for p in described if p.endswith(".tar")])
        self.assertIn("tar", described["pentimento-core-00000.tar.bin"],
                      "a reader meeting `.tar.bin` needs to be told it is a tar")

    def test_a_shard_already_staged_for_kaggle_keeps_its_name(self):
        """This can run over the release directory or over a staged copy.

        A second `.bin` on a name that already carries one describes a file
        that is not there, which is the fault the renaming exists to avoid.
        """
        self.assertEqual(
            publish_tier.kaggle_name("pentimento-core-00000.tar.bin"),
            "pentimento-core-00000.tar.bin")
        (self.packed / "pentimento-core-00000.tar").unlink()
        (self.packed / "pentimento-core-00000.tar.bin").write_text(
            "x", encoding="utf-8")
        described = {r["path"] for r in publish_tier.kaggle_resources(self.packed)}
        self.assertIn("pentimento-core-00000.tar.bin", described)
        self.assertNotIn("pentimento-core-00000.tar.bin.bin", described)

    def test_a_file_that_stops_shipping_stops_being_described(self):
        (self.packed / "LICENCES.md").unlink()
        described = {r["path"] for r in publish_tier.kaggle_resources(self.packed)}
        self.assertNotIn("LICENCES.md", described)

    def test_every_description_says_something(self):
        for resource in publish_tier.kaggle_resources(self.packed):
            self.assertGreater(len(resource["description"]), 30, resource["path"])

    def test_the_csv_columns_are_described_from_its_own_header(self):
        schema = publish_tier.kaggle_csv_schema(self.packed / "ATTRIBUTION.csv")
        names = [f["name"] for f in schema["fields"]]
        self.assertEqual(names[0], "file")
        self.assertIn("attribution", names)
        for field in schema["fields"]:
            self.assertTrue(field["description"].strip(), field["name"])

    def test_an_undescribed_column_is_refused_rather_than_shipped(self):
        """A published column nobody explains is one a reader guesses at."""
        (self.packed / "ATTRIBUTION.csv").write_text(
            "file,mystery\n", encoding="utf-8")
        with self.assertRaises(ValueError) as caught:
            publish_tier.kaggle_csv_schema(self.packed / "ATTRIBUTION.csv")
        self.assertIn("mystery", str(caught.exception))

    def test_the_csv_resource_carries_its_schema(self):
        entry = [r for r in publish_tier.kaggle_resources(self.packed)
                 if r["path"] == "ATTRIBUTION.csv"][0]
        self.assertIn("schema", entry)

    def test_the_keywords_are_from_kaggle_vocabulary_not_our_own_words(self):
        """Kaggle rejects the WHOLE update if one keyword is off its list.

        Measured against the live API on 2026-09-24: every word that actually
        describes this corpus, including "steganalysis" and "steganography",
        is refused. A keyword added here because it reads well silently
        breaks the next metadata push.
        """
        refused = {"steganalysis", "steganography", "image forensics",
                   "cover source mismatch", "digital forensics",
                   "cybersecurity", "photography", "image processing",
                   "security"}
        self.assertFalse(refused & set(publish_tier.KAGGLE_KEYWORDS))
        self.assertGreaterEqual(len(publish_tier.KAGGLE_KEYWORDS), 5)

    def test_the_licence_is_the_form_the_update_endpoint_accepts(self):
        """Create took "CC-BY-4.0"; update answers "invalid license" to it."""
        self.assertEqual(publish_tier.KAGGLE_LICENCE,
                         "Attribution 4.0 International (CC BY 4.0)")


if __name__ == "__main__":
    unittest.main()
