#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the cover-digest stamp.

    python3 -m unittest discover -s generators -p 'test_*.py'

The tests that matter are the refusals. Stamping a digest asserts "this arm was
built from the cover at this filename now", and writing that without checking
replaces a silent gap with a confident lie, which is strictly worse. So every
precondition gets a test that breaks it.
"""
from __future__ import annotations

import hashlib
import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import stamp_source_digests as ssd  # noqa: E402
from stamp_source_digests import (  # noqa: E402
    missing_rebuilt, rows_without_a_source, stamp, unknown_sources,
)


class PureTests(unittest.TestCase):
    def test_a_row_with_no_source_is_counted(self):
        self.assertEqual(rows_without_a_source(
            [{"source_png": "a.png"}, {}, {"source_png": ""}]), 2)

    def test_a_source_outside_the_manifest_is_named(self):
        rows = [{"source_png": "a.png"}, {"source_png": "gone.png"}]
        self.assertEqual(unknown_sources(rows, {"a.png": "d"}), ["gone.png"])

    def test_a_replaced_cover_with_no_rows_is_named(self):
        """Its rows were deleted by the invalidation, so an absence means the
        rebuild has not reached it yet."""
        rows = [{"stego": "hugo/0050/00001.png"}]
        self.assertEqual(missing_rebuilt(rows, {"00001", "00002"}), ["00002"])

    def test_stamping_writes_the_manifest_digest(self):
        rows = [{"source_png": "a.png"}, {"source_png": "b.png"}]
        self.assertEqual(stamp(rows, {"a.png": "aa", "b.png": "bb"}), 2)
        self.assertEqual(rows[0]["source_sha256"], "aa")

    def test_stamping_is_idempotent(self):
        rows = [{"source_png": "a.png"}]
        stamp(rows, {"a.png": "aa"})
        self.assertEqual(stamp(rows, {"a.png": "aa"}), 0)

    def test_a_wrong_existing_stamp_is_corrected(self):
        rows = [{"source_png": "a.png", "source_sha256": "stale"}]
        self.assertEqual(stamp(rows, {"a.png": "aa"}), 1)
        self.assertEqual(rows[0]["source_sha256"], "aa")


class EndToEndTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = pathlib.Path(self.tmp.name)
        self.covers = root / "commons"
        self.arms = root / "arms"
        self.covers.mkdir()
        (self.arms / "hugo/0050").mkdir(parents=True)
        (self.arms / "clean_grey").mkdir(parents=True)

        cover_rows, self.digests = [], {}
        for n in range(5):
            payload = f"cover {n}".encode()
            (self.covers / f"{n:05d}.png").write_bytes(payload)
            self.digests[f"{n:05d}.png"] = hashlib.sha256(payload).hexdigest()
            row = {"file": f"{n:05d}.png", "tier_order": n,
                   "sha256": self.digests[f"{n:05d}.png"]}
            if n == 2:
                row["replaces"] = {"reason": "us-only"}
            cover_rows.append(row)
        (self.covers / "manifest.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in cover_rows))

        self.rows = []
        for n in range(5):
            clean_bytes = f"clean {n}".encode()
            (self.arms / "clean_grey" / f"{n:05d}.png").write_bytes(clean_bytes)
            (self.arms / "hugo/0050" / f"{n:05d}.png").write_bytes(b"stego")
            self.rows.append({
                "arm": "hugo/0050",
                "clean": f"clean_grey/{n:05d}.png",
                "stego": f"hugo/0050/{n:05d}.png",
                "source_png": f"{n:05d}.png",
                "clean_sha256": hashlib.sha256(clean_bytes).hexdigest(),
            })
        self.manifest = self.arms / "manifest.jsonl"
        self.write(self.rows)

        self.swaps = root / "backfill.json"
        self.swaps.write_text(json.dumps(
            {"swaps": [{"position": 2, "file": "00002.png"}]}))

    def write(self, rows: list[dict]) -> None:
        self.manifest.write_text("".join(json.dumps(r) + "\n" for r in rows))

    def run_main(self, *extra: str) -> int:
        return ssd.main(["--arms", str(self.arms), "--covers", str(self.covers),
                         "--swaps", str(self.swaps), *extra])

    def read(self) -> list[dict]:
        return [json.loads(l) for l in self.manifest.read_text().splitlines()
                if l.strip()]

    def test_a_consistent_corpus_is_stamped(self):
        self.assertEqual(self.run_main(), 0)
        for row in self.read():
            self.assertEqual(row["source_sha256"],
                             self.digests[row["source_png"]])

    def test_a_dry_run_writes_nothing(self):
        before = self.manifest.read_text()
        self.assertEqual(self.run_main("--dry-run"), 0)
        self.assertEqual(before, self.manifest.read_text())

    def test_the_original_is_backed_up(self):
        self.run_main()
        self.assertTrue(
            self.manifest.with_suffix(".jsonl.pre-stamp").is_file())

    def test_a_replaced_cover_that_has_not_been_rebuilt_is_refused(self):
        """The precondition that makes the stamp true rather than a claim."""
        self.write([r for r in self.rows if "00002" not in r["stego"]])
        self.assertEqual(self.run_main(), 1)
        self.assertFalse(any("source_sha256" in r for r in self.read()))

    def test_a_clean_half_that_has_drifted_is_refused(self):
        """The row no longer describes the file it points at, so nothing it
        says about provenance can be trusted either."""
        (self.arms / "clean_grey" / "00003.png").write_bytes(b"different")
        self.assertEqual(self.run_main(), 1)

    def test_a_source_the_manifest_does_not_know_is_refused(self):
        rows = list(self.rows)
        rows[1] = dict(rows[1], source_png="99999.png")
        self.write(rows)
        self.assertEqual(self.run_main(), 1)

    def test_a_row_with_no_source_at_all_is_refused(self):
        rows = [dict(r) for r in self.rows]
        rows[1].pop("source_png")
        self.write(rows)
        self.assertEqual(self.run_main(), 1)

    def test_running_twice_changes_nothing_the_second_time(self):
        self.assertEqual(self.run_main(), 0)
        after = self.manifest.read_text()
        self.assertEqual(self.run_main(), 0)
        self.assertEqual(after, self.manifest.read_text())

    def test_a_missing_cover_manifest_exits_one(self):
        (self.covers / "manifest.jsonl").unlink()
        self.assertEqual(self.run_main(), 1)


if __name__ == "__main__":
    unittest.main()
