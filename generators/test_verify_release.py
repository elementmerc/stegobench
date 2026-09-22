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

import contextlib
import hashlib
import io
import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import verify_release as vr  # noqa: E402
from verify_release import (  # noqa: E402
    Report, check_covers, check_licences, check_packed, check_packed_arms,
    check_pool,
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
                        "last_tier_order": len(payloads) - 1}]}), encoding="utf-8")

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


class PackedArmTests(unittest.TestCase):
    """The same risk, on the half of the release nothing used to open.

    A FRESH arm pack is sound by construction: `pack_arms` hashes every file as
    it reads it and leaves out anything that disagrees. What that guarantee
    cannot see is an arm pack left in place by a later rebuild. Its shard
    digests are right, its index agrees with itself, its counts come out, and
    it holds the previous corpus.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.build(packed=b"stego v2", built=b"stego v2")

    def build(self, packed: bytes, built: bytes) -> None:
        """One arm, one sample. The pack holds `packed`; the arm holds `built`."""
        import tarfile

        group = self.root / "arms" / "adaptive"
        (group / "wow" / "0200").mkdir(parents=True, exist_ok=True)
        (group / "clean_grey").mkdir(parents=True, exist_ok=True)
        (group / "wow" / "0200" / "00001.png").write_bytes(built)
        (group / "clean_grey" / "00001.png").write_bytes(b"clean")
        (group / "manifest.jsonl").write_text(json.dumps({
            "arm": "wow/0200", "tool": "wow", "rate": 0.2,
            "clean": "clean_grey/00001.png",
            "clean_sha256": hashlib.sha256(b"clean").hexdigest(),
            "stego": "wow/0200/00001.png",
            "stego_sha256": hashlib.sha256(built).hexdigest(),
            "source_png": "08848.png", "source_sha256": "a" * 64,
        }) + "\n", encoding="utf-8")

        tier = self.root / "release" / "nano-arms"
        tier.mkdir(parents=True, exist_ok=True)
        shard = tier / "pentimento-nano-wow-0200-00000.tar"
        with tarfile.open(shard, "w", format=tarfile.PAX_FORMAT) as tar:
            for name, blob in (
                ("000000.png", packed),
                ("000000.json", json.dumps({
                    "arm": "wow/0200", "stego": "wow/0200/00001.png",
                    "stego_sha256": hashlib.sha256(packed).hexdigest(),
                    "sha256": hashlib.sha256(packed).hexdigest(),
                }, sort_keys=True).encode()),
            ):
                info = tarfile.TarInfo(name)
                info.size = len(blob)
                tar.addfile(info, io.BytesIO(blob))

        blob = shard.read_bytes()
        # The arm is named the way `arm_key` names it, tool and rate, because
        # that is what the packer writes into the index.
        (tier / "pentimento-nano-arms-index.json").write_text(json.dumps({
            "tier": "nano",
            "arms": [{"arm": "wow-0200", "samples": 1, "shards": [{
                "shard": shard.name, "samples": 1, "bytes": len(blob),
                # Self-consistent, exactly as a stale pack's index is.
                "sha256": hashlib.sha256(blob).hexdigest()}]}],
        }), encoding="utf-8")

    def check(self) -> Report:
        r = Report()
        check_packed_arms(self.root / "release", self.root / "arms", 10 ** 9, r)
        return r

    def test_a_pack_matching_the_arms_passes(self):
        r = self.check()
        self.assertTrue(r.ok, r.failures)
        self.assertIn("packed-arms", r.notes)

    def test_a_pack_left_behind_by_a_rebuild_is_caught(self):
        self.build(packed=b"stego v1", built=b"stego v2")
        r = self.check()
        self.assertTrue(any("before the arms changed" in m
                            for m in r.failures["packed-arms"]), r.failures)

    def test_an_arm_that_is_no_longer_built_is_caught(self):
        """A pack can also outlive the arm entirely, when a rebuild drops a
        rate or renames a tool. Nothing in the pack notices its own removal."""
        (self.root / "arms" / "adaptive" / "manifest.jsonl").write_text(
            "", encoding="utf-8")
        r = self.check()
        self.assertIn("packed-arms", r.failures)

    def test_a_release_with_no_packed_arms_is_caught_rather_than_skipped(self):
        r = Report()
        check_packed_arms(self.root / "nothing", self.root / "arms", 10, r)
        self.assertIn("packed-arms", r.failures)


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
            "".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")

    def run_main(self, *extra: str) -> int:
        return vr.main(["--covers", str(self.covers), "--expect", "20", *extra])

    def test_a_check_that_did_not_run_is_named_rather_than_omitted(self):
        """The fault `nothing_checked` prevents, one level up.

        A check skipped because its argument was absent used to leave no line
        at all, so a run over covers alone printed "every checked invariant
        holds" and the reader had to reconstruct which of the eight had
        actually happened. `packed` is the one that proves the point: it is
        the check that found two stale covers inside an otherwise clean pack.
        """
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(self.run_main(), 0)
        text = out.getvalue()
        for check in ("packed", "pool", "pairs", "stale", "provenance"):
            self.assertIn(f"{check:9} NOT RUN", text.replace("  ----  ", ""),
                          f"{check} did not run and was not named: {text}")
        self.assertIn("did NOT run", text)
        self.assertNotIn("every checked invariant holds", text)

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
                in (self.covers / "manifest.jsonl").read_text(encoding="utf-8").splitlines()
                if l.strip()]
        rows[2]["tier_order"] = 99          # covers
        rows[3]["licence"] = "CC BY-SA 4.0"  # licences
        (self.covers / "manifest.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")
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
            "".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")

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


class DctProvenanceTests(unittest.TestCase):
    """The DCT arms were exempt from the provenance check, and it showed as a
    smaller number rather than as a warning.

    They are built from the clean JPEG pool, so they carry `source_jpeg` and no
    `source_png`, and the check skipped any row whose direct field was absent.
    80,000 rows went unexamined while the summary read clean.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.arms = pathlib.Path(self.tmp.name) / "arms"
        (self.arms / "jpeg-tools").mkdir(parents=True)
        (self.arms / "jpeg-tools" / "manifest.jsonl").write_text(json.dumps({
            "clean": "clean/00001.jpg", "source_png": "00001.png",
            "source_sha256": "digest1", "stego": "s/00001.jpg"}) + "\n", encoding="utf-8")
        self.covers = [cover(n, sha256=f"digest{n}") for n in range(3)]
        (self.arms / "adaptive").mkdir()

    def write(self, rows):
        (self.arms / "adaptive" / "manifest.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")

    def test_a_dct_row_is_resolved_through_the_pool_and_passes(self):
        self.write([{"stego": "a", "source_jpeg": "clean_uerd/00001.jpg",
                     "source_sha256": "digest1"}])
        r = Report()
        vr.check_provenance(self.arms, self.covers, r)
        self.assertTrue(r.ok, r.failures)

    def test_an_unstamped_dct_row_is_no_longer_exempt(self):
        self.write([{"stego": "a", "source_jpeg": "clean_uerd/00001.jpg"}])
        r = Report()
        vr.check_provenance(self.arms, self.covers, r)
        self.assertTrue(any("no source_sha256" in m
                            for m in r.failures["provenance"]), r.failures)

    def test_a_row_naming_no_cover_at_all_is_reported_not_skipped(self):
        self.write([{"stego": "a"}])
        r = Report()
        vr.check_provenance(self.arms, self.covers, r)
        self.assertTrue(any("could not look" in m
                            for m in r.failures["provenance"]), r.failures)

    def test_the_note_says_how_many_rows_there_were_in_total(self):
        self.write([{"stego": "a", "source_jpeg": "clean_uerd/00001.jpg",
                     "source_sha256": "digest1"}])
        r = Report()
        vr.check_provenance(self.arms, self.covers, r)
        self.assertIn("of 2 rows", r.notes["provenance"])


class FiguresCheckTests(unittest.TestCase):
    """The published prose is part of the release, and nothing read it.

    Every other check reads the manifest. A stale `licence-summary.json` put
    "5,429 covers require attribution" into the shipped README while the
    manifest said 5,453, and all eight checks passed, because none of them was
    looking at a sentence.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = pathlib.Path(self.tmp.name)
        self.docs = root / "docs"
        self.docs.mkdir()
        self.release = root / "release"
        (self.release / "core").mkdir(parents=True)
        (self.release / "core-arms").mkdir(parents=True)
        self.covers = root / "commons"
        self.covers.mkdir()

        def a(name, n):
            return {"arm": name, "samples": n, "shards": [],
                    "rows_in_manifest": n, "container_mismatches": [],
                    "digest_mismatches": [], "mispaired": [], "missing": [],
                    "unlicensed": []}

        (self.release / "core-arms" / "pentimento-core-arms-index.json").write_text(
            json.dumps({"arms": [a("wow-0200", 100), a("outguess-0050", 80),
                                 a("steghide-0050", 100), a("clean-grey", 50)],
                        "tier": "Core"}))
        (self.release / "core" / "pentimento-core-index.json").write_text(
            json.dumps({"samples": 200, "tier": "Core"}))
        (self.covers / "manifest.jsonl").write_text("".join(
            json.dumps({"file": f"{n:05d}.png", "attribution_required": n < 110})
            + "\n" for n in range(200)))

    def test_matching_prose_passes(self):
        (self.docs / "index.md").write_text(
            "280 stego pairs, 110 covers, 55%.")
        r = Report()
        vr.check_figures(self.docs, self.release, self.covers, r)
        self.assertTrue(r.ok, r.failures)

    def test_a_stale_published_figure_fails_the_release(self):
        (self.docs / "index.md").write_text(
            "5,429 of 10,000 covers, 110 covers, 55%.")
        r = Report()
        vr.check_figures(self.docs, self.release, self.covers, r)
        self.assertIn("figures", r.failures)

    def test_a_docs_directory_with_no_pages_is_a_failure(self):
        r = Report()
        vr.check_figures(self.docs, self.release, self.covers, r)
        self.assertIn("figures", r.failures)

    def test_prose_stating_no_figure_at_all_does_not_report_clean(self):
        """Nothing compared is not the same as nothing wrong."""
        (self.docs / "index.md").write_text("A corpus of photographs.")
        r = Report()
        vr.check_figures(self.docs, self.release, self.covers, r)
        self.assertIn("figures", r.failures)
        self.assertTrue(any("could not look" in m
                            for m in r.failures["figures"]), r.failures)


class CouldNotLookTests(unittest.TestCase):
    """The fault found eight times across this fleet in one day.

    "0 pairs, containers identical" reads as a pass and means the opposite.
    **Could not look** and **looked and found nothing wrong** render as the
    same clean line, and the clean line is the one people act on. Every check
    that counts must refuse a count of zero.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.arms = pathlib.Path(self.tmp.name) / "arms"
        self.arms.mkdir(parents=True)
        (self.arms / "manifest.jsonl").write_text("", encoding="utf-8")

    def test_an_empty_arm_manifest_fails_the_pair_check(self):
        r = Report()
        vr.check_pairs(self.arms, 10, r)
        self.assertIn("pairs", r.failures)
        self.assertTrue(any("cannot look" in m for m in r.failures["pairs"]))

    def test_an_empty_arm_manifest_fails_the_stale_check(self):
        r = Report()
        vr.check_stale(self.arms, 10, r)
        self.assertIn("stale", r.failures)

    def test_an_empty_arm_manifest_fails_the_provenance_check(self):
        r = Report()
        vr.check_provenance(self.arms, [], r)
        self.assertIn("provenance", r.failures)

    def test_an_empty_row_set_fails_the_digest_check(self):
        r = Report()
        vr.check_digests([], pathlib.Path(self.tmp.name), None, r)
        self.assertIn("digests", r.failures)

    def test_a_packed_tier_whose_members_match_nothing_fails(self):
        import io
        import tarfile
        rel = pathlib.Path(self.tmp.name) / "nano"
        rel.mkdir()
        tar_path = rel / "pentimento-nano-00000.tar"
        with tarfile.open(tar_path, "w") as tar:
            info = tarfile.TarInfo("README.txt")
            info.size = 3
            tar.addfile(info, io.BytesIO(b"abc"))
        (rel / "pentimento-nano-index.json").write_text(json.dumps({
            "tier": "Nano",
            "shards": [{"shard": tar_path.name, "first_tier_order": 0,
                        "last_tier_order": 0}]}), encoding="utf-8")
        r = Report()
        vr.check_packed(rel, [cover(0)], 10 ** 9, r)
        self.assertIn("packed", r.failures)
