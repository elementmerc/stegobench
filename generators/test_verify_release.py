#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the pre-publication verifier.

    python3 -m unittest discover -s generators -p 'test_*.py'

A verifier that passes everything is worse than no verifier, because it
converts an unmonitored risk into a monitored one nobody re-examines. So every
test here breaks one invariant and asserts the tool says so, and each of the
breakages is one this corpus has actually had.
"""
from __future__ import annotations

import hashlib
import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import verify_release as vr  # noqa: E402
from verify_release import (  # noqa: E402
    Report, check_covers, check_licences, check_packed, check_pool,
)


def cover(n: int, **kw) -> dict:
    base = {
        "file": f"{n:05d}.png",
        "tier_order": n,
        "pageid": 1000 + n,
        "sha256": f"{n:064d}",
        "licence": "CC0",
        "artist": "A Photographer",
        "attribution_required": False,
        "attribution": f"cover {n}, CC0, via Wikimedia Commons",
    }
    base.update(kw)
    return base


class CoverTests(unittest.TestCase):
    def test_a_sound_manifest_passes(self):
        r = Report()
        check_covers([cover(n) for n in range(10)], 10, r)
        self.assertTrue(r.ok, r.failures)

    def test_the_wrong_count_is_caught(self):
        r = Report()
        check_covers([cover(n) for n in range(9)], 10, r)
        self.assertIn("covers", r.failures)

    def test_a_hole_in_tier_order_is_caught(self):
        """A tier is a prefix of this ordering, so a hole moves every boundary
        after it and no tier matches any checksum anyone recorded."""
        rows = [cover(n) for n in range(10)]
        rows[5]["tier_order"] = 99
        r = Report()
        check_covers(rows, 10, r)
        self.assertTrue(any("not dense" in m for m in r.failures["covers"]))

    def test_a_duplicate_pageid_is_caught(self):
        """The defect a second backfill run would have produced: one photograph
        at two tier positions under two filenames."""
        rows = [cover(n) for n in range(10)]
        rows[3]["pageid"] = rows[4]["pageid"]
        r = Report()
        check_covers(rows, 10, r)
        self.assertTrue(any("pageid is not unique" in m
                            for m in r.failures["covers"]))

    def test_a_duplicate_digest_is_caught(self):
        rows = [cover(n) for n in range(10)]
        rows[3]["sha256"] = rows[4]["sha256"]
        r = Report()
        check_covers(rows, 10, r)
        self.assertTrue(any("sha256 is not unique" in m
                            for m in r.failures["covers"]))


class LicenceTests(unittest.TestCase):
    def test_a_licence_outside_the_permitted_set_is_caught(self):
        rows = [cover(0), cover(1, licence="CC BY-SA 3.0")]
        r = Report()
        check_licences(rows, r)
        self.assertTrue(any("outside the permitted set" in m
                            for m in r.failures["licences"]))

    def test_an_unattributable_cover_is_caught(self):
        """Twelve of these survived a cull meant to remove exactly them,
        because two definitions of "unattributable" disagreed."""
        rows = [cover(0, attribution_required=True, artist="Unknown author")]
        r = Report()
        check_licences(rows, r)
        self.assertTrue(any("no usable author" in m
                            for m in r.failures["licences"]))

    def test_an_empty_artist_is_caught_too(self):
        rows = [cover(0, attribution_required=True, artist="")]
        r = Report()
        check_licences(rows, r)
        self.assertIn("licences", r.failures)

    def test_a_missing_credit_line_is_caught(self):
        rows = [cover(0, attribution="")]
        r = Report()
        check_licences(rows, r)
        self.assertTrue(any("no credit line" in m
                            for m in r.failures["licences"]))

    def test_a_sound_set_passes(self):
        r = Report()
        check_licences([cover(n) for n in range(5)], r)
        self.assertTrue(r.ok, r.failures)


class PackedTierTests(unittest.TestCase):
    """The real risk is a STALE pack, not a false nesting.

    Nesting holds by construction: a tier is a prefix of one `tier_order`. A
    tier packed before the covers were replaced is the thing that can be wrong
    while every self-consistent check inside it passes.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.release = pathlib.Path(self.tmp.name) / "nano"
        self.release.mkdir(parents=True)
        self.rows = []
        payloads = {}
        for n in range(5):
            payloads[n] = f"cover {n}".encode()
            self.rows.append(cover(
                n, sha256=hashlib.sha256(payloads[n]).hexdigest()))
        self.pack(payloads)

    def pack(self, payloads: dict) -> None:
        import io
        import tarfile
        tar_path = self.release / "pentimento-nano-00000.tar"
        with tarfile.open(tar_path, "w") as tar:
            for n, payload in sorted(payloads.items()):
                info = tarfile.TarInfo(f"{n:06d}.png")
                info.size = len(payload)
                tar.addfile(info, io.BytesIO(payload))
        (self.release / "pentimento-nano-index.json").write_text(json.dumps({
            "tier": "Nano", "samples": len(payloads),
            "shards": [{"shard": tar_path.name, "samples": len(payloads),
                        "first_tier_order": 0,
                        "last_tier_order": len(payloads) - 1}]}))

    def test_a_pack_matching_the_manifest_passes(self):
        r = Report()
        check_packed(self.release, self.rows, 10 ** 9, r)
        self.assertTrue(r.ok, r.failures)

    def test_a_pack_made_before_a_cover_was_replaced_is_caught(self):
        """The situation tonight: covers were replaced in place, keeping their
        filenames, so nothing in the packed tier says it went stale."""
        self.rows[2]["sha256"] = hashlib.sha256(b"the replacement").hexdigest()
        r = Report()
        check_packed(self.release, self.rows, 10 ** 9, r)
        self.assertTrue(any("packed before the covers changed" in m
                            for m in r.failures["packed"]), r.failures)

    def test_a_shard_named_in_the_index_but_absent_is_caught(self):
        (self.release / "pentimento-nano-00000.tar").unlink()
        r = Report()
        check_packed(self.release, self.rows, 10 ** 9, r)
        self.assertTrue(any("not on disk" in m for m in r.failures["packed"]))

    def test_a_release_with_no_packed_tier_is_caught_rather_than_skipped(self):
        r = Report()
        check_packed(self.release.parent / "empty", self.rows, 10, r)
        self.assertIn("packed", r.failures)


class PoolTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.pool = pathlib.Path(self.tmp.name)

    def fill(self, n: int) -> None:
        for i in range(n):
            (self.pool / f"{i:05d}.jpg").write_bytes(b"x")

    def test_a_dense_pool_passes(self):
        self.fill(5)
        r = Report()
        check_pool(self.pool, 5, r)
        self.assertTrue(r.ok, r.failures)

    def test_a_gap_is_caught(self):
        self.fill(5)
        (self.pool / "00002.jpg").unlink()
        r = Report()
        check_pool(self.pool, 5, r)
        self.assertIn("pool", r.failures)

    def test_a_missing_pool_is_caught_rather_than_skipped(self):
        r = Report()
        check_pool(self.pool / "nope", 5, r)
        self.assertIn("pool", r.failures)


class DigestTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.rows = []
        for n in range(5):
            payload = f"cover {n}".encode()
            (self.root / f"{n:05d}.png").write_bytes(payload)
            self.rows.append(cover(
                n, sha256=hashlib.sha256(payload).hexdigest()))

    def test_matching_bytes_pass(self):
        r = Report()
        vr.check_digests(self.rows, self.root, None, r)
        self.assertTrue(r.ok, r.failures)

    def test_a_changed_file_is_caught(self):
        (self.root / "00002.png").write_bytes(b"tampered")
        r = Report()
        vr.check_digests(self.rows, self.root, None, r)
        self.assertTrue(any("do not match" in m for m in r.failures["digests"]))

    def test_a_missing_file_is_caught(self):
        (self.root / "00003.png").unlink()
        r = Report()
        vr.check_digests(self.rows, self.root, None, r)
        self.assertTrue(any("not on disk" in m for m in r.failures["digests"]))


class EndToEndTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.covers = pathlib.Path(self.tmp.name) / "commons"
        self.covers.mkdir()
        rows = []
        for n in range(20):
            payload = f"cover {n}".encode()
            (self.covers / f"{n:05d}.png").write_bytes(payload)
            rows.append(cover(n, sha256=hashlib.sha256(payload).hexdigest()))
        (self.covers / "manifest.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in rows))

    def run_main(self, *extra: str) -> int:
        return vr.main(["--covers", str(self.covers), "--expect", "20", *extra])

    def test_a_sound_corpus_exits_zero(self):
        self.assertEqual(self.run_main("--full"), 0)

    def test_a_tampered_cover_exits_one(self):
        (self.covers / "00004.png").write_bytes(b"tampered")
        self.assertEqual(self.run_main("--full"), 1)

    def test_a_missing_manifest_exits_one(self):
        (self.covers / "manifest.jsonl").unlink()
        self.assertEqual(self.run_main(), 1)

    def test_all_failing_checks_are_reported_not_just_the_first(self):
        rows = [json.loads(l) for l
                in (self.covers / "manifest.jsonl").read_text().splitlines()
                if l.strip()]
        rows[2]["tier_order"] = 99          # covers
        rows[3]["licence"] = "CC BY-SA 4.0"  # licences
        (self.covers / "manifest.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in rows))
        report = Report()
        check_covers(rows, 20, report)
        check_licences(rows, report)
        self.assertEqual(set(report.failures) >= {"covers", "licences"}, True)


if __name__ == "__main__":
    unittest.main()


class ProvenanceTests(unittest.TestCase):
    """An arm that cannot name the cover it came from, by content."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.arms = pathlib.Path(self.tmp.name) / "arms"
        self.arms.mkdir(parents=True)
        self.covers = [cover(n, sha256=f"digest{n}") for n in range(3)]

    def write(self, rows):
        (self.arms / "manifest.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in rows))

    def test_stamped_rows_that_agree_pass(self):
        self.write([{"stego": "a", "source_png": "00001.png",
                     "source_sha256": "digest1"}])
        r = Report()
        vr.check_provenance(self.arms, self.covers, r)
        self.assertTrue(r.ok, r.failures)

    def test_an_unstamped_row_is_caught(self):
        self.write([{"stego": "a", "source_png": "00001.png"}])
        r = Report()
        vr.check_provenance(self.arms, self.covers, r)
        self.assertTrue(any("no source_sha256" in m
                            for m in r.failures["provenance"]))

    def test_a_row_naming_a_digest_the_manifest_disowns_is_caught(self):
        """The shape of a stale arm: the filename still resolves, the image
        behind it does not."""
        self.write([{"stego": "a", "source_png": "00001.png",
                     "source_sha256": "the-old-photograph"}])
        r = Report()
        vr.check_provenance(self.arms, self.covers, r)
        self.assertTrue(any("does not agree" in m
                            for m in r.failures["provenance"]))
