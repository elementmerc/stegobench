#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the Croissant record check.

    python3 -m unittest discover -s generators -p 'test_*.py'

The tests that matter here are the three that reproduce defects this record
actually shipped with, because a validator written after the fact is only worth
having if it would have caught what got past us: a three-entry `@context`, a
record with no `recordSet` at all, and a cover `FileSet` with no `containedIn`
that matched 769 arm shards. Each one parses as JSON, each one survives every
other tool in this repository, and each one makes the dataset silently fail to
load for a stranger.
"""
from __future__ import annotations

import copy
import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import validate_croissant as vc  # noqa: E402
from validate_croissant import (  # noqa: E402
    CONFORMS_TO, REQUIRED_CONTEXT, problems_with_distribution,
    problems_with_record_sets, structural_problems,
)


def sound_record() -> dict:
    return {
        "@context": {t: t for t in REQUIRED_CONTEXT},
        "@type": "sc:Dataset",
        "conformsTo": CONFORMS_TO,
        "name": "pentimento-nano",
        "description": "A redistributable labelled cover corpus.",
        "license": "https://creativecommons.org/licenses/by/4.0/",
        "url": "https://example.invalid/pentimento",
        "version": "1.0.0",
        "datePublished": "2026-09-21",
        "citeAs": "@misc{pentimento}",
        "distribution": [
            {"@id": "archive", "@type": "cr:FileObject",
             "contentUrl": "https://example.invalid/nano.tar",
             "encodingFormat": "application/x-tar"},
            {"@id": "cover-images", "@type": "cr:FileSet",
             "containedIn": {"@id": "archive"},
             "includes": "*.png", "encodingFormat": "image/png"},
        ],
        "recordSet": [
            {"@id": "covers", "@type": "cr:RecordSet", "field": [
                {"@id": "covers/image", "dataType": "sc:ImageObject",
                 "source": {"fileSet": {"@id": "cover-images"}}},
            ]},
        ],
    }


class SoundRecordTests(unittest.TestCase):
    def test_a_sound_record_has_no_complaints(self):
        self.assertEqual(structural_problems(sound_record()), [])


class HistoricalDefectTests(unittest.TestCase):
    """The three that actually shipped."""

    def test_a_three_entry_context_is_caught(self):
        doc = sound_record()
        doc["@context"] = {"@vocab": "https://schema.org/", "cr": "x",
                           "sc": "https://schema.org/"}
        problems = structural_problems(doc)
        self.assertTrue(any("@context is missing" in p for p in problems),
                        f"got {problems}")

    def test_a_record_with_no_record_set_is_caught(self):
        doc = sound_record()
        doc["recordSet"] = []
        problems = structural_problems(doc)
        self.assertTrue(any("recordSet is empty" in p for p in problems),
                        f"got {problems}")

    def test_a_file_set_with_no_containedIn_is_caught(self):
        """The defect that swallowed 769 arm shards into the cover set."""
        doc = sound_record()
        del doc["distribution"][1]["containedIn"]
        problems = structural_problems(doc)
        self.assertTrue(any("containedIn" in p for p in problems),
                        f"got {problems}")


class DistributionTests(unittest.TestCase):
    def test_a_dangling_containedIn_is_caught(self):
        dist = [{"@id": "s", "@type": "cr:FileSet",
                 "containedIn": {"@id": "nowhere"}, "includes": "*.png",
                 "encodingFormat": "image/png"}]
        problems = problems_with_distribution(dist)
        self.assertTrue(any("nowhere" in p for p in problems))

    def test_a_file_set_with_no_glob_selects_nothing(self):
        dist = [{"@id": "a", "@type": "cr:FileObject", "contentUrl": "u",
                 "encodingFormat": "application/x-tar"},
                {"@id": "s", "@type": "cr:FileSet",
                 "containedIn": {"@id": "a"}, "encodingFormat": "image/png"}]
        self.assertTrue(any("includes" in p
                            for p in problems_with_distribution(dist)))

    def test_a_duplicate_id_is_caught(self):
        dist = [{"@id": "a", "@type": "cr:FileObject", "contentUrl": "u",
                 "encodingFormat": "x"},
                {"@id": "a", "@type": "cr:FileObject", "contentUrl": "u",
                 "encodingFormat": "x"}]
        self.assertTrue(any("more than once" in p
                            for p in problems_with_distribution(dist)))

    def test_a_file_object_that_cannot_be_fetched_is_caught(self):
        dist = [{"@id": "a", "@type": "cr:FileObject",
                 "encodingFormat": "x"}]
        self.assertTrue(any("cannot be fetched" in p
                            for p in problems_with_distribution(dist)))

    def test_an_empty_distribution_is_caught(self):
        self.assertTrue(problems_with_distribution([]))


class RecordSetTests(unittest.TestCase):
    def test_a_field_with_no_source_is_caught(self):
        rs = [{"@id": "covers", "field": [
            {"@id": "covers/x", "dataType": "sc:Text"}]}]
        self.assertTrue(any("no source" in p
                            for p in problems_with_record_sets(rs, set())))

    def test_a_field_with_no_dataType_is_caught(self):
        rs = [{"@id": "covers", "field": [
            {"@id": "covers/x", "source": {"fileSet": {"@id": "s"}}}]}]
        self.assertTrue(any("dataType" in p
                            for p in problems_with_record_sets(rs, {"s"})))

    def test_a_source_naming_a_missing_file_set_is_caught(self):
        rs = [{"@id": "covers", "field": [
            {"@id": "covers/x", "dataType": "sc:Text",
             "source": {"fileSet": {"@id": "gone"}}}]}]
        self.assertTrue(any("gone" in p
                            for p in problems_with_record_sets(rs, {"s"})))

    def test_a_record_set_with_no_fields_is_caught(self):
        rs = [{"@id": "covers", "field": []}]
        self.assertTrue(problems_with_record_sets(rs, set()))


class TopLevelTests(unittest.TestCase):
    def test_every_required_property_is_required(self):
        for key in vc.REQUIRED_TOP:
            doc = sound_record()
            del doc[key]
            problems = structural_problems(doc)
            self.assertTrue(any(key in p for p in problems),
                            f"removing {key!r} produced no complaint")

    def test_the_wrong_conformsTo_is_caught(self):
        doc = sound_record()
        doc["conformsTo"] = "http://mlcommons.org/croissant/0.8"
        self.assertTrue(any("conformsTo" in p
                            for p in structural_problems(doc)))

    def test_the_wrong_type_is_caught(self):
        doc = sound_record()
        doc["@type"] = "sc:CreativeWork"
        self.assertTrue(any("@type" in p for p in structural_problems(doc)))


class MainTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = pathlib.Path(self.tmp.name) / "croissant.json"

    def write(self, doc: dict) -> None:
        self.path.write_text(json.dumps(doc), encoding="utf-8")

    def test_a_sound_record_exits_zero(self):
        self.write(sound_record())
        self.assertEqual(vc.main([str(self.path)]), 0)

    def test_a_broken_record_exits_one(self):
        doc = sound_record()
        doc["recordSet"] = []
        self.write(doc)
        self.assertEqual(vc.main([str(self.path)]), 1)

    def test_unreadable_json_exits_one_rather_than_raising(self):
        self.path.write_text("{not json", encoding="utf-8")
        self.assertEqual(vc.main([str(self.path)]), 1)

    def test_require_reference_fails_when_the_validator_is_absent(self):
        """A release gate wants the reference implementation, not our reading
        of the specification."""
        self.write(sound_record())
        ran, _ = vc.reference_problems(self.path)
        expected = 0 if ran else 1
        self.assertEqual(
            vc.main([str(self.path), "--require-reference"]), expected)


if __name__ == "__main__":
    unittest.main()
