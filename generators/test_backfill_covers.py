#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the cover backfill.

    python3 -m unittest discover -s generators -p 'test_*.py'

This tool overwrites images in a corpus that took hours to fetch, and the
property it must not break is the one nothing downstream can detect: tiers nest
because `tier_order` is dense and a tier is a prefix of it. A splice that
renumbers, duplicates or leaves a hole produces a corpus where every file is
valid and every digest matches, and the damage shows up as accuracy somebody
else cannot account for.
"""
from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import backfill_covers  # noqa: E402
from backfill_covers import BackfillError, choose, splice  # noqa: E402


def cover(n: int, **kw) -> dict:
    base = {
        "file": f"{n:05d}.png",
        "tier_order": n,
        "pageid": 1000 + n,
        "title": f"File:Original {n}.jpg",
        "licence": "Public domain",
        "sha256": f"{n:064d}",
        "artist": "A Photographer",
    }
    base.update(kw)
    return base


def candidate(n: int, **kw) -> dict:
    base = {
        "file": f"{n:05d}.png",
        "tier_order": None,
        "pageid": 9000 + n,
        "title": f"File:Replacement {n}.jpg",
        "licence": "CC0",
        "sha256": f"{n + 500:064d}",
        "artist": "B Photographer",
    }
    base.update(kw)
    return base


class ChooseTests(unittest.TestCase):
    def test_an_unpublishable_candidate_is_skipped(self):
        picks = choose([candidate(0), candidate(1), candidate(2)],
                       {"00001.png"}, 2)
        self.assertEqual([p["file"] for p in picks], ["00000.png", "00002.png"])

    def test_too_few_clean_candidates_is_refused(self):
        """A replacement that fails the same test is not a fix.

        Lowering the bar here would swap one unpublishable cover for another
        and report success, and nobody would look again.
        """
        with self.assertRaises(BackfillError) as cm:
            choose([candidate(0), candidate(1)], {"00000.png"}, 2)
        self.assertIn("lowering the standard", str(cm.exception))


class SpliceTests(unittest.TestCase):
    def test_the_replacement_keeps_the_position_and_the_name(self):
        row = splice(cover(42, reason="us-only"), candidate(7))
        self.assertEqual(row["file"], "00042.png")
        self.assertEqual(row["tier_order"], 42)

    def test_everything_about_the_image_comes_from_the_candidate(self):
        row = splice(cover(42, reason="us-only"), candidate(7))
        self.assertEqual(row["pageid"], 9007)
        self.assertEqual(row["licence"], "CC0")
        self.assertEqual(row["title"], "File:Replacement 7.jpg")
        self.assertEqual(row["artist"], "B Photographer")

    def test_the_swap_is_recorded_rather_than_silent(self):
        row = splice(cover(42, reason="share-alike"), candidate(7))
        self.assertEqual(row["replaces"]["pageid"], 1042)
        self.assertEqual(row["replaces"]["reason"], "share-alike")


class EndToEndTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = pathlib.Path(self.tmp.name)
        self.covers = root / "commons"
        self.candidates = root / "backfill"
        self.covers.mkdir()
        self.candidates.mkdir()

        self.rows = [cover(n) for n in range(10)]
        for row in self.rows:
            (self.covers / row["file"]).write_bytes(b"original " + row["file"].encode())
        self.write(self.covers / "manifest.jsonl", self.rows)

        self.cands = [candidate(n) for n in range(5)]
        for c in self.cands:
            payload = b"replacement " + c["file"].encode()
            (self.candidates / c["file"]).write_bytes(payload)
            import hashlib
            c["sha256"] = hashlib.sha256(payload).hexdigest()
        self.write(self.candidates / "manifest.jsonl", self.cands)

        self.doomed = root / "unpublishable.json"
        self.doomed.write_text(json.dumps({"covers": [
            {"file": "00003.png", "reason": "us-only", "tier_order": 3},
            {"file": "00007.png", "reason": "share-alike", "tier_order": 7},
        ]}))
        self.rejects = root / "candidate-rejects.json"
        self.rejects.write_text(json.dumps({"covers": []}))

    def write(self, path: pathlib.Path, rows: list[dict]) -> None:
        path.write_text("".join(json.dumps(r) + "\n" for r in rows))

    def run_main(self, *extra: str) -> int:
        return backfill_covers.main([
            "--covers", str(self.covers),
            "--unpublishable", str(self.doomed),
            "--candidates", str(self.candidates),
            "--candidate-rejects", str(self.rejects),
            *extra,
        ])

    def manifest(self) -> list[dict]:
        return [json.loads(line) for line
                in (self.covers / "manifest.jsonl").read_text().splitlines()
                if line.strip()]

    def test_the_ordering_stays_dense_and_the_count_stays_the_same(self):
        self.assertEqual(self.run_main(), 0)
        rows = self.manifest()
        self.assertEqual(len(rows), 10)
        self.assertEqual([r["tier_order"] for r in rows], list(range(10)))

    def test_only_the_named_covers_change(self):
        self.run_main()
        rows = {r["file"]: r for r in self.manifest()}
        self.assertEqual(rows["00003.png"]["pageid"], 9000)
        self.assertEqual(rows["00007.png"]["pageid"], 9001)
        for untouched in ("00000.png", "00004.png", "00009.png"):
            self.assertEqual(rows[untouched]["pageid"],
                             1000 + int(untouched[:5]))

    def test_the_image_on_disk_is_replaced_and_matches_its_digest(self):
        import hashlib
        self.run_main()
        rows = {r["file"]: r for r in self.manifest()}
        actual = hashlib.sha256((self.covers / "00003.png").read_bytes()).hexdigest()
        self.assertEqual(actual, rows["00003.png"]["sha256"])
        self.assertIn(b"replacement", (self.covers / "00003.png").read_bytes())

    def test_a_dry_run_writes_nothing(self):
        before = (self.covers / "manifest.jsonl").read_text()
        self.assertEqual(self.run_main("--dry-run"), 0)
        self.assertEqual(before, (self.covers / "manifest.jsonl").read_text())
        self.assertIn(b"original", (self.covers / "00003.png").read_bytes())

    def test_the_swaps_are_logged(self):
        self.run_main()
        log = json.loads((self.covers / "backfill.json").read_text())
        self.assertEqual(len(log["swaps"]), 2)
        self.assertEqual(log["swaps"][0]["reason"], "us-only")

    def test_it_refuses_without_the_candidates_having_been_audited(self):
        code = backfill_covers.main([
            "--covers", str(self.covers),
            "--unpublishable", str(self.doomed),
            "--candidates", str(self.candidates),
        ])
        self.assertEqual(code, 1)

    def test_a_cover_named_for_replacement_that_does_not_exist_is_refused(self):
        self.doomed.write_text(json.dumps({"covers": [
            {"file": "99999.png", "reason": "us-only", "tier_order": 99999}]}))
        self.assertEqual(self.run_main(), 1)

    def test_not_enough_clean_candidates_is_refused_before_anything_is_written(self):
        self.rejects.write_text(json.dumps({"covers": [
            {"file": c["file"]} for c in self.cands[:4]]}))
        before = (self.covers / "manifest.jsonl").read_text()
        self.assertEqual(self.run_main(), 1)
        self.assertEqual(before, (self.covers / "manifest.jsonl").read_text())


if __name__ == "__main__":
    unittest.main()
