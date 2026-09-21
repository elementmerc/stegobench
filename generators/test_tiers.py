#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for tier selection, and really for one property: tiers nest.

    python3 -m unittest discover -s generators -p 'test_*.py'

The property is that Nano is a prefix of Lite is a prefix of Core. If it ever
fails, nothing downstream notices: every file is valid, every digest matches,
and the damage appears as inflated accuracy in somebody else's paper after they
trained on Lite and evaluated on Core. So it is asserted directly rather than
inferred from the code reading correctly.
"""
from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from tiers import TierError, covers_in_tier_order, tier_cover_names, tier_name  # noqa: E402


class TierNamingTests(unittest.TestCase):
    def test_the_published_sizes_have_published_names(self):
        self.assertEqual(tier_name(200), "Nano")
        self.assertEqual(tier_name(1000), "Lite")
        self.assertEqual(tier_name(10000), "Core")

    def test_an_unpublished_size_describes_itself(self):
        self.assertEqual(tier_name(37), "custom (37)")


class CoverNameTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = pathlib.Path(self.tmp.name)
        self.manifest = self.dir / "manifest.jsonl"

    def write(self, rows: list[dict]) -> pathlib.Path:
        self.manifest.write_text(
            "".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")
        return self.manifest

    def shuffled_corpus(self, n: int) -> pathlib.Path:
        # Deliberately NOT in tier_order on disk. The order of lines in the
        # manifest must not decide the tier, or the tier depends on how the
        # file was last written.
        rows = [{"file": f"{i:05d}.png", "tier_order": i} for i in range(n)]
        rows = rows[n // 2:] + rows[: n // 2]
        return self.write(rows)

    def test_nano_is_a_prefix_of_lite_is_a_prefix_of_core(self):
        manifest = self.shuffled_corpus(1000)
        nano = tier_cover_names(manifest, 20)
        lite = tier_cover_names(manifest, 100)
        core = tier_cover_names(manifest, 1000)
        self.assertTrue(nano < lite < core)

    def test_the_selection_ignores_the_order_lines_appear_in(self):
        rows = [{"file": f"{i:05d}.png", "tier_order": i} for i in range(10)]
        forwards = tier_cover_names(self.write(rows), 3)
        backwards = tier_cover_names(self.write(list(reversed(rows))), 3)
        self.assertEqual(forwards, backwards)
        self.assertEqual(forwards, {"00000.png", "00001.png", "00002.png"})

    def test_a_row_without_a_tier_order_is_refused(self):
        # Assigning one here would put a cover at a position that changes on
        # the next run, which silently breaks nesting between releases.
        with self.assertRaises(TierError) as cm:
            tier_cover_names(self.write([{"file": "00000.png"}]), 1)
        self.assertIn("tier_order", str(cm.exception))

    def test_a_gap_in_the_ordering_is_refused(self):
        rows = [{"file": "a.png", "tier_order": 0}, {"file": "b.png", "tier_order": 2}]
        with self.assertRaises(TierError) as cm:
            tier_cover_names(self.write(rows), 2)
        self.assertIn("dense", str(cm.exception))

    def test_a_duplicated_position_is_refused(self):
        # Two manifests concatenated. Both rows claim position 0, so a prefix
        # is not well defined and the count would silently be short.
        rows = [{"file": "a.png", "tier_order": 0}, {"file": "b.png", "tier_order": 0}]
        with self.assertRaises(TierError):
            tier_cover_names(self.write(rows), 2)

    def test_a_tier_larger_than_the_corpus_is_refused(self):
        with self.assertRaises(TierError) as cm:
            tier_cover_names(self.shuffled_corpus(10), 50)
        self.assertIn("not a prefix of anything", str(cm.exception))

    def test_a_missing_manifest_is_refused(self):
        with self.assertRaises(TierError):
            tier_cover_names(self.dir / "nope.jsonl", 1)

    def test_blank_lines_are_tolerated(self):
        self.manifest.write_text(
            json.dumps({"file": "a.png", "tier_order": 0}) + "\n\n"
            + json.dumps({"file": "b.png", "tier_order": 1}) + "\n", encoding="utf-8")
        self.assertEqual(tier_cover_names(self.manifest, 1), {"a.png"})

    def test_it_agrees_with_the_cover_selector_it_mirrors(self):
        """The two selectors must not drift apart.

        `covers_in_tier_order` picks the cover FILES for the cover tier;
        `tier_cover_names` picks the same covers by NAME so the arm packer can
        filter against them. If they ever disagreed, a Nano arm would hold
        stego images whose covers are not in the Nano cover tier.
        """
        manifest = self.shuffled_corpus(20)
        for name in (f"{i:05d}.png" for i in range(20)):
            (self.dir / name).write_bytes(b"x")
        paths = covers_in_tier_order(manifest, self.dir, 7)
        self.assertEqual({p.name for p in paths}, tier_cover_names(manifest, 7))


if __name__ == "__main__":
    unittest.main()
