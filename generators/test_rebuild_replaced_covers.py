#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the replaced-cover invalidation.

    python3 -m unittest discover -s generators -p 'test_*.py'

This tool deletes images from a corpus that took twenty hours of compute to
build, so the tests care about two things above all: that it deletes exactly the
files derived from a replaced cover and nothing else, and that it refuses when
the evidence it is acting on disagrees with the corpus.
"""
from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import rebuild_replaced_covers as rrc  # noqa: E402
from rebuild_replaced_covers import (  # noqa: E402
    RebuildError, check_pool, positions, row_is_doomed, stems_of,
)


class PositionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)

    def log(self, name: str, spots: list[int]) -> pathlib.Path:
        path = self.root / name
        path.write_text(json.dumps(
            {"swaps": [{"position": n, "file": f"{n:05d}.png"} for n in spots]}))
        return path

    def test_rounds_are_unioned_not_replaced(self):
        a = self.log("a.json", [1, 2, 3])
        b = self.log("b.json", [3, 9])
        self.assertEqual(positions([a, b]), {1, 2, 3, 9})

    def test_an_empty_log_is_refused(self):
        path = self.root / "empty.json"
        path.write_text(json.dumps({"swaps": []}))
        with self.assertRaises(RebuildError):
            positions([path])

    def test_a_swap_with_no_position_is_refused(self):
        path = self.root / "bad.json"
        path.write_text(json.dumps({"swaps": [{"file": "00001.png"}]}))
        with self.assertRaises(RebuildError):
            positions([path])

    def test_stems_are_the_five_digit_names(self):
        self.assertEqual(stems_of({0, 42, 9999}), {"00000", "00042", "09999"})


class RowTests(unittest.TestCase):
    def test_a_stale_stego_half_condemns_the_row(self):
        row = {"clean": "clean_grey/00007.png", "stego": "hugo/0050/00042.png"}
        self.assertTrue(row_is_doomed(row, {"00042"}))

    def test_a_stale_clean_half_condemns_the_row_too(self):
        """A pair with one stale half is not half valid, it is void."""
        row = {"clean": "clean_grey/00042.png", "stego": "hugo/0050/00042.png"}
        self.assertTrue(row_is_doomed(row, {"00042"}))

    def test_an_untouched_row_survives(self):
        row = {"clean": "clean_grey/00007.png", "stego": "hugo/0050/00007.png"}
        self.assertFalse(row_is_doomed(row, {"00042"}))

    def test_the_source_cover_is_checked_as_well(self):
        row = {"clean": "c/00001.png", "stego": "s/00001.png",
               "source_png": "00042.png"}
        self.assertTrue(row_is_doomed(row, {"00042"}))


class PoolTests(unittest.TestCase):
    """The ordering trap: the builder indexes the JPEG pool positionally."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.pool = pathlib.Path(self.tmp.name)

    def fill(self, n: int) -> None:
        for i in range(n):
            (self.pool / f"{i:05d}.jpg").write_bytes(b"x")

    def test_a_dense_pool_passes(self):
        self.fill(5)
        self.assertEqual(check_pool(self.pool, 5), [])

    def test_a_hole_is_caught_and_the_reason_is_stated(self):
        """The defect this guards.

        Delete one JPEG and every cover after it shifts down a position, so
        pair N is built from cover N+1's coefficients. Every digest matches,
        every count is right, and half the arm is silently mispaired.
        """
        self.fill(5)
        (self.pool / "00002.jpg").unlink()
        complaints = check_pool(self.pool, 5)
        self.assertTrue(complaints)
        self.assertTrue(any("shifts" in c or "not dense" in c
                            for c in complaints))

    def test_a_pool_of_the_wrong_size_is_caught(self):
        self.fill(3)
        self.assertTrue(check_pool(self.pool, 5))


class EndToEndTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = pathlib.Path(self.tmp.name)
        self.arms = root / "arms"
        self.covers = root / "covers"
        self.covers.mkdir()

        # Two arms over five covers, of which 2 and 4 were replaced.
        rows = []
        for arm in ("hugo/0050", "wow/0100"):
            (self.arms / arm).mkdir(parents=True)
            for i in range(5):
                (self.arms / arm / f"{i:05d}.png").write_bytes(b"stego")
                rows.append({"arm": arm,
                             "clean": f"clean_grey/{i:05d}.png",
                             "stego": f"{arm}/{i:05d}.png"})
        (self.arms / "clean_grey").mkdir(parents=True)
        for i in range(5):
            (self.arms / "clean_grey" / f"{i:05d}.png").write_bytes(b"clean")
        self.manifest = self.arms / "manifest.jsonl"
        self.manifest.write_text("".join(json.dumps(r) + "\n" for r in rows))

        # A cover manifest where 2 and 4 carry `replaces`.
        cover_rows = []
        for i in range(5):
            row = {"file": f"{i:05d}.png", "tier_order": i}
            if i in (2, 4):
                row["replaces"] = {"pageid": 100 + i, "reason": "us-only"}
            cover_rows.append(row)
        (self.covers / "manifest.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in cover_rows))

        self.swaps = root / "backfill.json"
        self.swaps.write_text(json.dumps({"swaps": [
            {"position": 2, "file": "00002.png"},
            {"position": 4, "file": "00004.png"}]}))

    def run_main(self, *extra: str) -> int:
        return rrc.main(["--arms", str(self.arms),
                         "--swaps", str(self.swaps),
                         "--covers", str(self.covers), *extra])

    def test_only_the_replaced_covers_files_are_removed(self):
        self.assertEqual(self.run_main(), 0)
        for arm in ("hugo/0050", "wow/0100"):
            for i in (0, 1, 3):
                self.assertTrue((self.arms / arm / f"{i:05d}.png").is_file())
            for i in (2, 4):
                self.assertFalse((self.arms / arm / f"{i:05d}.png").exists())

    def test_the_clean_halves_go_too(self):
        self.run_main()
        self.assertFalse((self.arms / "clean_grey" / "00002.png").exists())
        self.assertTrue((self.arms / "clean_grey" / "00000.png").is_file())

    def test_the_manifest_rows_are_stripped_so_the_builder_does_not_skip(self):
        """If the rows stayed, the builder's resume set would skip the rebuild
        and report success having rebuilt nothing."""
        self.run_main()
        rows = [json.loads(l) for l in self.manifest.read_text().splitlines()
                if l.strip()]
        self.assertEqual(len(rows), 6)
        self.assertFalse(any("00002" in r["stego"] or "00004" in r["stego"]
                             for r in rows))

    def test_the_original_manifest_is_backed_up(self):
        self.run_main()
        backup = self.manifest.with_suffix(".jsonl.pre-rebuild")
        self.assertTrue(backup.is_file())
        self.assertEqual(len(backup.read_text().splitlines()), 10)

    def test_a_dry_run_changes_nothing(self):
        before = self.manifest.read_text()
        self.assertEqual(self.run_main("--dry-run"), 0)
        self.assertEqual(before, self.manifest.read_text())
        self.assertTrue((self.arms / "hugo/0050/00002.png").is_file())

    def test_running_twice_is_safe(self):
        self.assertEqual(self.run_main(), 0)
        after_first = self.manifest.read_text()
        self.assertEqual(self.run_main(), 0)
        self.assertEqual(after_first, self.manifest.read_text())

    def test_a_stale_swap_log_is_refused_before_anything_is_deleted(self):
        """A log naming a cover the manifest does not record as replaced means
        the two disagree, and acting on it throws away sound arms."""
        self.swaps.write_text(json.dumps({"swaps": [
            {"position": 1, "file": "00001.png"}]}))
        self.assertEqual(self.run_main(), 1)
        self.assertTrue((self.arms / "hugo/0050/00001.png").is_file())

    def test_a_position_outside_the_corpus_is_refused(self):
        self.swaps.write_text(json.dumps({"swaps": [
            {"position": 99, "file": "00099.png"}]}))
        self.assertEqual(self.run_main(), 1)


if __name__ == "__main__":
    unittest.main()
