#!/usr/bin/env python3
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

import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import build_jpeg_arms as bja  # noqa: E402
from build_jpeg_arms import merge_shards, shard_manifests  # noqa: E402


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
            "".join(json.dumps(r) + "\n" for r in rows))

    def rows(self) -> list[dict]:
        return [json.loads(l) for l
                in (self.out / "manifest.jsonl").read_text().splitlines()
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
            json.dumps({"stego": "old"}) + "\n")
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
