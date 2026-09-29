#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the JPEG tool arm builder's sharding and manifest merge.

    python3 -m unittest discover -s generators -p 'test_*.py'

The embedding itself needs steghide and outguess, so these cover the parts that
decide WHICH covers a run touches and what the merged manifest says. Those are
the parts that can be wrong without anything failing: a stride that overlaps
builds one pair twice, a stride that has a gap leaves a hole no count reveals,
and a merge that keeps arrival order makes two honest runs produce different
published artefacts.
"""
from __future__ import annotations

import contextlib
import io
import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from PIL import Image  # noqa: E402

import build_jpeg_arms as bja  # noqa: E402
from build_jpeg_arms import merge_shards, shard_manifests  # noqa: E402
from embedders import Embedder, EmbedResult  # noqa: E402


class StrideTests(unittest.TestCase):
    """The stride is `index % shards == shard`. It has to partition exactly."""

    def assigned(self, total: int, shards: int, shard: int) -> list[int]:
        return [i for i in range(total) if i % shards == shard]

    def test_every_cover_lands_in_exactly_one_shard(self):
        total, shards = 10000, 8
        seen: list[int] = []
        for s in range(shards):
            seen += self.assigned(total, shards, s)
        self.assertEqual(sorted(seen), list(range(total)))
        self.assertEqual(len(seen), len(set(seen)), "a cover was claimed twice")

    def test_the_shards_are_within_one_of_each_other(self):
        """Stride rather than block, so no shard gets all the large covers."""
        sizes = [len(self.assigned(10000, 8, s)) for s in range(8)]
        self.assertLessEqual(max(sizes) - min(sizes), 1)

    def test_one_shard_is_the_whole_corpus(self):
        self.assertEqual(self.assigned(100, 1, 0), list(range(100)))


class ShardArgumentTests(unittest.TestCase):
    def test_a_shard_outside_the_range_is_refused(self):
        code = bja.main(["--covers", "/nonexistent", "--out", "/nonexistent",
                         "--shards", "4", "--shard", "4"])
        self.assertEqual(code, 2)

    def test_a_negative_shard_is_refused(self):
        code = bja.main(["--covers", "/nonexistent", "--out", "/nonexistent",
                         "--shards", "4", "--shard", "-1"])
        self.assertEqual(code, 2)

    def test_zero_shards_is_refused(self):
        code = bja.main(["--covers", "/nonexistent", "--out", "/nonexistent",
                         "--shards", "0", "--shard", "0"])
        self.assertEqual(code, 2)


class MergeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.out = pathlib.Path(self.tmp.name)

    def shard(self, n: int, rows: list[dict]) -> None:
        (self.out / f"manifest.shard-{n:02d}.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")

    def rows(self) -> list[dict]:
        return [json.loads(l) for l
                in (self.out / "manifest.jsonl").read_text(encoding="utf-8").splitlines()
                if l.strip()]

    def test_the_shards_are_folded_into_one_manifest(self):
        self.shard(0, [{"stego": "steghide/0050/00000.jpg"}])
        self.shard(1, [{"stego": "steghide/0050/00001.jpg"}])
        self.assertEqual(merge_shards(self.out), 0)
        self.assertEqual(len(self.rows()), 2)

    def test_the_shard_files_are_removed_afterwards(self):
        self.shard(0, [{"stego": "a"}])
        merge_shards(self.out)
        self.assertEqual(shard_manifests(self.out), [])

    def test_the_merged_order_does_not_depend_on_which_shard_finished_first(self):
        """Iteration order leaking into a published artefact is what makes two
        honest runs disagree."""
        self.shard(0, [{"stego": "z"}, {"stego": "b"}])
        self.shard(1, [{"stego": "a"}, {"stego": "m"}])
        merge_shards(self.out)
        self.assertEqual([r["stego"] for r in self.rows()],
                         ["a", "b", "m", "z"])

    def test_an_existing_manifest_is_kept_rather_than_overwritten(self):
        (self.out / "manifest.jsonl").write_text(
            json.dumps({"stego": "old"}) + "\n", encoding="utf-8")
        self.shard(0, [{"stego": "new"}])
        merge_shards(self.out)
        self.assertEqual({r["stego"] for r in self.rows()}, {"old", "new"})

    def test_the_same_pair_built_identically_twice_collapses(self):
        self.shard(0, [{"stego": "a", "bytes": 1}])
        self.shard(1, [{"stego": "a", "bytes": 1}])
        self.assertEqual(merge_shards(self.out), 0)
        self.assertEqual(len(self.rows()), 1)

    def test_the_same_pair_built_DIFFERENTLY_twice_is_refused(self):
        """The stride makes this impossible, so if it happens the sharding is
        broken and keeping one of the two silently is the worst answer."""
        self.shard(0, [{"stego": "a", "bytes": 1}])
        self.shard(1, [{"stego": "a", "bytes": 2}])
        self.assertEqual(merge_shards(self.out), 1)

    def test_merging_with_no_shards_is_refused_rather_than_reporting_success(self):
        self.assertEqual(merge_shards(self.out), 1)


if __name__ == "__main__":
    unittest.main()


class StubEmbedder(Embedder):
    """The smallest thing the arm loop will accept as a tool.

    It exists so the resume rule can be tested against the REAL loop rather
    than a copy of it. Every other part of the builder runs for real: Pillow
    writes the clean JPEG, `payloads` derives the payload, `append_after_eoi`
    builds the structural arm, and the manifest is the one the builder writes.
    Only the embedding itself is stubbed, because steghide and outguess are
    containers and CI has neither.
    """

    formats = (".jpg",)
    tool_id = "stub"

    def __init__(self, quality: int | None = None):
        self.quality = quality

    @property
    def id(self) -> str:
        return self.tool_id

    def available(self) -> bool:
        return True

    def capacity(self, cover: pathlib.Path) -> int:
        # Under the cover's own size, which the loop refuses to exceed.
        return 64

    def embed(self, cover: pathlib.Path, payload: bytes,
              stego: pathlib.Path, password: str | None = None) -> EmbedResult:
        stego.parent.mkdir(parents=True, exist_ok=True)
        stego.write_bytes(cover.read_bytes() + b"STUB" + payload)
        return EmbedResult(stego, len(payload), self.id, {"stub": True})


class StubHide(StubEmbedder):
    tool_id = "stubhide"


class StubGuess(StubEmbedder):
    tool_id = "stubguess"


class ResumeTests(unittest.TestCase):
    """The resume rule, driven through the builder's own loop.

    The rule is three lines and it has been fixed once and regressed twice, so
    the thing worth testing is not the rule in isolation (`test_tools.py` does
    that) but that the loop around it still reaches the right state after an
    interruption.
    """

    ARM = "stubhide/0500"

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = pathlib.Path(self.tmp.name)
        self.covers = root / "covers"
        self.out = root / "out"
        self.covers.mkdir(parents=True)

        rows = []
        for i in range(2):
            name = f"{i:05d}.png"
            Image.new("RGB", (32, 32), (10 + i * 40, 90, 140)).save(
                self.covers / name)
            rows.append({"file": name, "tier_order": i})
        (self.covers / "manifest.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")

        for name, stub in (("SteghideEmbedder", StubHide),
                           ("OutguessEmbedder", StubGuess)):
            real = getattr(bja, name)
            setattr(bja, name, stub)
            self.addCleanup(setattr, bja, name, real)

    def run_builder(self, count: int = 1) -> int:
        # The builder's progress lines are not what is under test, and a run
        # per case would bury the suite's own output in them.
        with contextlib.redirect_stdout(io.StringIO()):
            return bja.main(["--covers", str(self.covers),
                             "--out", str(self.out), "--count", str(count),
                             "--rates", "0.5"])

    def rows(self) -> list[dict]:
        text = (self.out / "manifest.jsonl").read_text(encoding="utf-8")
        return [json.loads(l) for l in text.splitlines() if l.strip()]

    def keys(self) -> list[str]:
        return [r["stego"] for r in self.rows()]

    def stego(self, stem: str = "00000") -> pathlib.Path:
        return self.out / self.ARM / f"{stem}.jpg"

    def test_a_fresh_run_records_every_pair_it_wrote(self):
        self.assertEqual(self.run_builder(), 0)
        keys = self.keys()
        self.assertIn(f"{self.ARM}/00000.jpg", keys)
        self.assertIn("structural/0000/00000.jpg", keys)
        self.assertEqual(len(keys), len(set(keys)))
        for key in keys:
            self.assertTrue((self.out / key).is_file(), key)

    def test_the_keys_are_separated_the_same_way_on_every_platform(self):
        """A manifest key is an identifier, so it cannot be OS shaped.

        Built with the platform separator, a corpus packed on Windows records
        `arm\\0500\\00000.jpg` and one packed anywhere else records
        `arm/0500/00000.jpg`. Nothing downstream matches the two: the resume
        rule rebuilds every arm it already has, and a pair written on one
        machine never finds its twin on another.
        """
        self.assertEqual(self.run_builder(), 0)
        keys = self.keys()
        self.assertTrue(keys, "nothing was recorded, so nothing was checked")
        for key in keys:
            self.assertNotIn("\\", key, key)
            self.assertIn("/", key, key)

    def test_a_recorded_pair_is_not_rebuilt(self):
        self.run_builder()
        before = len(self.rows())
        self.stego().write_bytes(b"SENTINEL")
        self.assertEqual(self.run_builder(), 1, "nothing was left to build")
        self.assertEqual(len(self.rows()), before)
        self.assertEqual(self.stego().read_bytes(), b"SENTINEL",
                         "a recorded pair was rebuilt over")

    def test_an_image_with_no_manifest_row_is_rebuilt(self):
        """The one that cost 118 covers across twenty-one arms.

        The image is on disk, the manifest has no row for it, and the old rule
        skipped it BECAUSE it existed, so the row was never written and
        `pack_arms` never packed it.
        """
        self.run_builder()
        built = self.stego().read_bytes()
        rows = [r for r in self.rows() if r["stego"] != f"{self.ARM}/00000.jpg"]
        (self.out / "manifest.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")
        self.stego().write_bytes(b"ORPHAN")

        self.assertEqual(self.run_builder(), 0)
        self.assertIn(f"{self.ARM}/00000.jpg", self.keys())
        self.assertEqual(self.stego().read_bytes(), built,
                         "the orphan was kept instead of being rebuilt")

    def test_a_manifest_row_whose_file_is_missing_stays_recorded(self):
        """The row is the record; the packer reads rows, not directories."""
        self.run_builder()
        before = self.rows()
        self.stego().unlink()
        self.assertEqual(self.run_builder(), 1, "nothing was left to build")
        self.assertEqual(self.rows(), before, "a row was duplicated or lost")
        self.assertFalse(self.stego().exists())

    def test_a_resumed_run_finishes_a_partially_built_arm(self):
        """Cover 0 recorded, cover 1 written and orphaned by the interruption."""
        self.run_builder(count=1)
        recorded = self.rows()
        orphan = self.stego("00001")
        orphan.parent.mkdir(parents=True, exist_ok=True)
        orphan.write_bytes(b"ORPHAN")

        self.assertEqual(self.run_builder(count=2), 0)
        keys = self.keys()
        self.assertEqual(len(keys), len(set(keys)), "a pair was recorded twice")
        for row in recorded:
            self.assertIn(row, self.rows(), "an already recorded row changed")
        self.assertIn(f"{self.ARM}/00001.jpg", keys)
        self.assertNotEqual(orphan.read_bytes(), b"ORPHAN",
                            "the orphan was adopted rather than rebuilt")
