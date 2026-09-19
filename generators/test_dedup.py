#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the deduplication store.

Run them with the standard library alone, because the build box's virtual environment
that runs the corpus build has no pytest in it and a test suite that only runs
on the laptop is a test suite that stops running:

    python3 -m unittest discover -s generators -p 'test_*.py'

The load bearing test here is `test_banded_index_finds_every_true_match`. The
whole store rests on a pigeonhole argument, that two hashes within 7 bits must
agree exactly on at least one of 8 bands, and if that argument is wrong the
store quietly stops finding duplicates rather than failing. So it is checked
against brute force rather than trusted.
"""
from __future__ import annotations

import pathlib
import random
import sqlite3
import tempfile
import unittest

import numpy as np
from PIL import Image

import dedup
from dedup import (
    BAND_COUNT,
    MAX_SUPPORTED_DISTANCE,
    DedupError,
    DedupStore,
    Fingerprint,
    fingerprint_bytes,
    fingerprint_file,
    hamming,
)


def photo(seed: int, size: int = 128) -> Image.Image:
    """A synthetic picture with real low frequency structure.

    Uniform noise is the one input a perceptual hash is entitled to get wrong,
    since it has no coarse shape to read, so the test images are built from a
    few smooth gradients with noise on top: the shape of a photograph rather
    than the shape of a random number generator.
    """
    rng = np.random.default_rng(seed)
    y, x = np.mgrid[0:size, 0:size] / size
    field = np.zeros((size, size))
    for _ in range(4):
        fx, fy = rng.uniform(0.5, 3, 2)
        phase = rng.uniform(0, 6.28)
        field += np.sin(2 * np.pi * (fx * x + fy * y) + phase)
    field = (field - field.min()) / (np.ptp(field) + 1e-9)
    noise = rng.normal(0, 0.02, (size, size))
    arr = np.clip((field + noise) * 255, 0, 255).astype(np.uint8)
    return Image.fromarray(np.dstack([arr, arr, arr]), mode="RGB")


def png_bytes(img: Image.Image) -> bytes:
    import io
    buf = io.BytesIO()
    img.save(buf, format="PNG")
    return buf.getvalue()


class HashTests(unittest.TestCase):
    def test_hashes_are_deterministic_and_64_bit(self):
        img = photo(1)
        for fn in (dedup.dhash, dedup.phash):
            a, b = fn(img), fn(img)
            self.assertEqual(a, b, f"{fn.__name__} is not deterministic")
            self.assertGreaterEqual(a, 0)
            self.assertLess(a, 1 << 64)

    def test_unrelated_pictures_hash_far_apart(self):
        a, b = photo(1), photo(2)
        self.assertGreater(hamming(dedup.dhash(a), dedup.dhash(b)),
                           MAX_SUPPORTED_DISTANCE)
        self.assertGreater(hamming(dedup.phash(a), dedup.phash(b)),
                           MAX_SUPPORTED_DISTANCE)

    def test_brightness_shift_stays_near(self):
        """A resave at a different exposure is the same picture and must collide."""
        img = photo(3)
        brighter = Image.fromarray(
            np.clip(np.asarray(img, dtype=np.int16) + 18, 0, 255).astype(np.uint8),
            mode="RGB",
        )
        self.assertLessEqual(hamming(dedup.phash(img), dedup.phash(brighter)),
                             dedup.DEFAULT_PHASH_MAX)

    def test_rescale_stays_near(self):
        img = photo(4)
        smaller = img.resize((96, 96), Image.LANCZOS)
        self.assertLessEqual(hamming(dedup.dhash(img), dedup.dhash(smaller)),
                             dedup.DEFAULT_DHASH_MAX)

    def test_dc_bit_is_pinned(self):
        """Bit 63 of pHash carries no information, by construction."""
        for seed in range(6):
            self.assertEqual(dedup.phash(photo(seed)) >> 63, 0)


class FingerprintTests(unittest.TestCase):
    def test_fingerprint_bytes_round_trip(self):
        raw = png_bytes(photo(5))
        fp = fingerprint_bytes(raw)
        self.assertEqual(len(fp.sha256), 64)
        self.assertEqual((fp.width, fp.height), (128, 128))
        self.assertEqual(fp, fingerprint_bytes(raw))

    def test_empty_input_is_refused(self):
        with self.assertRaises(DedupError):
            fingerprint_bytes(b"")

    def test_undecodable_input_is_refused_with_a_reason(self):
        with self.assertRaises(DedupError) as cm:
            fingerprint_bytes(b"this is not a picture at all")
        self.assertIn("could not decode", str(cm.exception))

    def test_missing_file_names_itself(self):
        with self.assertRaises(DedupError) as cm:
            fingerprint_file(pathlib.Path("/nonexistent/image.png"))
        self.assertIn("image.png", str(cm.exception))

    def test_tiny_image_is_refused(self):
        with self.assertRaises(DedupError) as cm:
            fingerprint_bytes(png_bytes(photo(6).resize((4, 4))))
        self.assertIn("too small", str(cm.exception))

    def test_bands_reconstruct_the_hash(self):
        fp = Fingerprint("a" * 64, 0xDEADBEEFCAFEF00D, 0x0123456789ABCDEF, 10, 10)
        for kind, value in (("d", fp.dhash), ("p", fp.phash)):
            rebuilt = 0
            for i in range(BAND_COUNT):
                rebuilt |= fp.band(kind, i) << (i * dedup.BAND_BITS)
            self.assertEqual(rebuilt, value)


class StoreTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        # Registered here rather than in tearDown, because unittest runs
        # tearDown BEFORE the cleanups, so a tearDown that removed the
        # directory would do it while the store still held the database open.
        # Windows refuses to delete an open file and this failed there while
        # passing on Linux and macOS, which is what the three runners are for.
        self.addCleanup(self.tmp.cleanup)
        self.db = pathlib.Path(self.tmp.name) / "store.sqlite3"

    def store(self, **kw) -> DedupStore:
        s = DedupStore(self.db, **kw)
        self.addCleanup(s.close)
        return s

    def test_first_image_is_accepted(self):
        s = self.store()
        d = s.offer(fingerprint_bytes(png_bytes(photo(7))), "commons", "a.png")
        self.assertTrue(d.accepted)
        self.assertIsNone(d.match)
        self.assertEqual(s.count(), 1)

    def test_identical_bytes_are_an_exact_duplicate(self):
        s = self.store()
        raw = png_bytes(photo(8))
        s.offer(fingerprint_bytes(raw), "commons", "a.png")
        d = s.offer(fingerprint_bytes(raw), "unsplash", "b.png")
        self.assertFalse(d.accepted)
        self.assertEqual(d.match.kind, "exact")
        self.assertEqual(d.match.source, "commons")
        self.assertEqual(s.count(), 1)

    def test_the_same_photo_from_two_services_is_caught(self):
        """Different bytes, same picture: the case sha256 cannot see."""
        s = self.store()
        img = photo(9)
        s.offer(fingerprint_bytes(png_bytes(img)), "commons", "orig.png")
        reposted = Image.fromarray(
            np.clip(np.asarray(img, dtype=np.int16) + 12, 0, 255).astype(np.uint8),
            mode="RGB",
        )
        fp = fingerprint_bytes(png_bytes(reposted))
        d = s.offer(fp, "unsplash", "repost.png")
        self.assertFalse(d.accepted, "a near duplicate was admitted")
        self.assertIn(d.match.kind, ("dhash", "phash"))
        self.assertLessEqual(d.match.distance, MAX_SUPPORTED_DISTANCE)

    def test_unrelated_pictures_all_get_in(self):
        s = self.store()
        for seed in range(25):
            fp = fingerprint_bytes(png_bytes(photo(100 + seed)))
            self.assertTrue(
                s.offer(fp, "commons", f"{seed}.png").accepted,
                f"seed {seed} was wrongly rejected as a duplicate",
            )
        self.assertEqual(s.count(), 25)
        self.assertEqual(s.rejection_count(), 0)

    def test_rejection_is_recorded_with_its_evidence(self):
        s = self.store()
        raw = png_bytes(photo(10))
        s.offer(fingerprint_bytes(raw), "commons", "a.png")
        s.offer(fingerprint_bytes(raw), "pexels", "b.png")
        rows = list(s.rejections())
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["source"], "pexels")
        self.assertEqual(rows[0]["ref"], "b.png")
        self.assertEqual(rows[0]["kind"], "exact")
        self.assertEqual(rows[0]["matched"], fingerprint_bytes(raw).sha256)
        self.assertTrue(rows[0]["rejected_at"].endswith("+00:00"))

    def test_offering_twice_does_not_duplicate_the_row(self):
        s = self.store()
        fp = fingerprint_bytes(png_bytes(photo(11)))
        s.offer(fp, "commons", "a.png")
        s.offer(fp, "commons", "a.png")
        s.offer(fp, "commons", "a.png")
        self.assertEqual(s.count(), 1)
        self.assertEqual(s.rejection_count(), 2)

    def test_the_store_survives_being_closed(self):
        fp = fingerprint_bytes(png_bytes(photo(12)))
        first = DedupStore(self.db)
        first.offer(fp, "commons", "a.png")
        first.close()
        second = self.store()
        self.assertEqual(second.count(), 1)
        self.assertFalse(second.offer(fp, "commons", "again.png").accepted)

    def test_source_and_ref_are_required(self):
        s = self.store()
        fp = fingerprint_bytes(png_bytes(photo(13)))
        for source, ref in (("", "a.png"), ("commons", "")):
            with self.assertRaises(DedupError):
                s.offer(fp, source, ref)

    def test_threshold_beyond_the_pigeonhole_guarantee_is_refused(self):
        with self.assertRaises(DedupError) as cm:
            DedupStore(self.db, dhash_max=MAX_SUPPORTED_DISTANCE + 1)
        self.assertIn("miss duplicates", str(cm.exception))
        with self.assertRaises(DedupError):
            DedupStore(self.db, phash_max=-1)

    def test_a_store_from_another_hash_layout_is_refused(self):
        self.store().close()
        db = sqlite3.connect(self.db)
        db.execute("UPDATE meta SET value = '4' WHERE key = 'band_count'")
        db.commit()
        db.close()
        with self.assertRaises(DedupError) as cm:
            DedupStore(self.db)
        self.assertIn("not comparable", str(cm.exception))

    def test_top_bit_hashes_survive_the_database(self):
        """SQLite integers are signed; our hashes are not. The top bit must live."""
        s = self.store()
        fp = Fingerprint("f" * 64, (1 << 64) - 1, (1 << 64) - 1, 512, 512)
        self.assertTrue(s.offer(fp, "synthetic", "max.png").accepted)
        found = s.find_match(fp)
        self.assertIsNotNone(found)
        self.assertEqual(found.kind, "exact")
        near = Fingerprint("e" * 64, (1 << 64) - 2, (1 << 64) - 2, 512, 512)
        self.assertEqual(s.find_match(near).kind, "dhash")

    def test_stats_report_what_is_held(self):
        s = self.store()
        s.offer(fingerprint_bytes(png_bytes(photo(14))), "commons", "a.png")
        s.offer(fingerprint_bytes(png_bytes(photo(14))), "unsplash", "b.png")
        s.offer(fingerprint_bytes(png_bytes(photo(15))), "commons", "c.png")
        st = s.stats()
        self.assertEqual(st["accepted"], 2)
        self.assertEqual(st["rejected"], 1)
        self.assertEqual(st["accepted_by_source"], {"commons": 2})
        self.assertEqual(st["rejected_by_reason"], {"exact": 1})


class ThinPictureTests(unittest.TestCase):
    """A picture with almost no structure must not be convicted on one hash.

    Reproduces the measured case: on 2026-09-16 an ISS Earth view and an
    unrelated Commons photograph, textures 1.00 and 1.43, came out 6 bits apart
    on dHash while pHash put them 28 apart. Six is exactly the default
    threshold, so without corroboration that pair is a false rejection.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.store = DedupStore(pathlib.Path(self.tmp.name) / "thin.sqlite3")
        self.addCleanup(self.store.close)

    def test_one_hash_is_not_enough_for_a_thin_picture(self):
        held = Fingerprint("a" * 64, 0x0000000000000000, 0x0F0F0F0F0F0F0F0F,
                           512, 512, texture=1.00)
        self.store.offer(held, "commons", "iss.png")
        # dHash 6 bits away, pHash nothing like it: the measured false pair.
        candidate = Fingerprint("b" * 64, 0b111111, 0xF0F0F0F0F0F0F0F0,
                                512, 512, texture=1.43)
        self.assertEqual(hamming(held.dhash, candidate.dhash), 6)
        self.assertGreater(hamming(held.phash, candidate.phash),
                           dedup.DEFAULT_PHASH_MAX)
        self.assertIsNone(
            self.store.find_match(candidate),
            "a thin picture was refused on one hash alone",
        )

    def test_both_hashes_agreeing_still_convicts_a_thin_picture(self):
        held = Fingerprint("c" * 64, 0x00FF00FF00FF00FF, 0x0F0F0F0F0F0F0F0F,
                           512, 512, texture=1.00)
        self.store.offer(held, "commons", "a.png")
        candidate = Fingerprint("d" * 64, 0x00FF00FF00FF00FE,
                                0x0F0F0F0F0F0F0F0E, 512, 512, texture=1.20)
        match = self.store.find_match(candidate)
        self.assertIsNotNone(match, "two agreeing hashes must still convict")

    def test_a_textured_picture_is_still_convicted_on_one_hash(self):
        """The rule must not weaken dedup on ordinary photographs."""
        held = Fingerprint("e" * 64, 0x0000000000000000, 0x0F0F0F0F0F0F0F0F,
                           512, 512, texture=20.0)
        self.store.offer(held, "commons", "a.png")
        candidate = Fingerprint("f" * 64, 0b111111, 0xF0F0F0F0F0F0F0F0,
                                512, 512, texture=25.0)
        match = self.store.find_match(candidate)
        self.assertIsNotNone(match)
        self.assertEqual(match.kind, "dhash")
        self.assertEqual(match.distance, 6)

    def test_one_thin_side_is_enough_to_require_corroboration(self):
        """Either image being thin makes the single-hash evidence unsafe."""
        held = Fingerprint("0" * 64, 0x0000000000000000, 0x0F0F0F0F0F0F0F0F,
                           512, 512, texture=30.0)
        self.store.offer(held, "commons", "a.png")
        thin = Fingerprint("1" * 64, 0b111111, 0xF0F0F0F0F0F0F0F0,
                           512, 512, texture=0.5)
        self.assertIsNone(self.store.find_match(thin))

    def test_fingerprinting_an_image_records_its_texture(self):
        fp = fingerprint_bytes(png_bytes(photo(42)))
        self.assertGreater(fp.texture, 0.0)
        flat = np.full((64, 64, 3), 120, dtype=np.uint8)
        self.assertEqual(
            fingerprint_bytes(png_bytes(Image.fromarray(flat, "RGB"))).texture,
            0.0,
        )

    def test_texture_survives_the_database(self):
        fp = fingerprint_bytes(png_bytes(photo(43)))
        self.store.offer(fp, "commons", "a.png")
        row = self.store.db.execute(
            "SELECT texture FROM images WHERE sha256 = ?", (fp.sha256,)
        ).fetchone()
        self.assertAlmostEqual(row["texture"], fp.texture, places=6)


class RedundancyIsNotDuplicationTests(unittest.TestCase):
    """The store's documented limit, asserted so it cannot be quietly forgotten.

    If someone later raises the thresholds hoping to catch bulk-uploader
    redundancy, this test is where they find out why that cannot work.
    """

    def test_chance_collisions_explode_with_the_radius(self):
        """Why the threshold is not simply raised to reach redundant images."""
        import math
        space = 2 ** 64

        def per_candidate(radius: int, held: int) -> float:
            return held * sum(math.comb(64, k) for k in range(radius + 1)) / space

        # At the working threshold nothing collides by accident, even at a
        # corpus far larger than anything planned.
        self.assertLess(per_candidate(MAX_SUPPORTED_DISTANCE, 25_000_000), 0.01)
        # At the radius that would be needed to reach for redundancy, a large
        # corpus would reject almost everything for no reason at all.
        self.assertGreater(per_candidate(15, 25_000_000), 100)

    def test_no_reachable_threshold_separates_redundant_photographs(self):
        """The measured ISS distances, asserted against the supported ceiling.

        Recorded 2026-09-16: the *closest* pair of ISS Earth views sat 24 bits
        apart on both hashes, while unrelated Commons photographs sat 24 apart
        too. The sets do not separate, and the closest redundant pair is more
        than three times the furthest distance this index can serve.
        """
        closest_redundant_pair = 24
        self.assertGreater(closest_redundant_pair, MAX_SUPPORTED_DISTANCE * 3)


class BandedIndexTests(unittest.TestCase):
    """The pigeonhole claim the whole lookup rests on, checked against brute force."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.store = DedupStore(
            pathlib.Path(self.tmp.name) / "bands.sqlite3",
            dhash_max=MAX_SUPPORTED_DISTANCE,
            phash_max=MAX_SUPPORTED_DISTANCE,
        )
        self.addCleanup(self.store.close)
        self.rng = random.Random(20260916)
        self.held = []
        for i in range(400):
            h = self.rng.getrandbits(64)
            fp = Fingerprint(f"{i:064d}", h, h, 512, 512)
            self.store.offer(fp, "synthetic", f"{i}.png")
            self.held.append(h)

    def flip(self, value: int, bits: int) -> int:
        for pos in self.rng.sample(range(64), bits):
            value ^= 1 << pos
        return value

    def test_banded_index_finds_every_true_match(self):
        for distance in range(MAX_SUPPORTED_DISTANCE + 1):
            for _ in range(30):
                target = self.rng.choice(self.held)
                query = self.flip(target, distance)
                fp = Fingerprint("q" * 64, query, query, 512, 512)
                brute = min(hamming(query, h) for h in self.held)
                found = self.store.find_match(fp)
                self.assertIsNotNone(
                    found,
                    f"missed a match at Hamming {distance}: the banded index is "
                    f"not returning a superset of the true matches",
                )
                self.assertEqual(
                    found.distance, brute,
                    "the index returned a worse match than brute force found",
                )

    def test_a_far_away_hash_is_new(self):
        for _ in range(50):
            query = self.rng.getrandbits(64)
            if min(hamming(query, h) for h in self.held) <= MAX_SUPPORTED_DISTANCE:
                continue
            fp = Fingerprint("z" * 64, query, query, 512, 512)
            self.assertIsNone(self.store.find_match(fp))


class CommandLineTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.db = self.root / "store.sqlite3"
        self.images = self.root / "covers"
        self.images.mkdir()

    def test_add_then_check_then_stats(self):
        for seed in range(5):
            photo(200 + seed).save(self.images / f"{seed:03d}.png")
        # the same picture twice, under two names
        photo(200).save(self.images / "repost.png")

        self.assertEqual(
            dedup.main(["--db", str(self.db), "add", str(self.images),
                        "--source", "commons"]),
            0,
        )
        with DedupStore(self.db) as s:
            self.assertEqual(s.count(), 5)
            self.assertEqual(s.rejection_count(), 1)

        # an image already held exits 1, the shell convention for "found"
        self.assertEqual(
            dedup.main(["--db", str(self.db), "check",
                        str(self.images / "000.png")]),
            1,
        )
        fresh = self.root / "fresh.png"
        photo(999).save(fresh)
        self.assertEqual(
            dedup.main(["--db", str(self.db), "check", str(fresh)]), 0
        )
        self.assertEqual(dedup.main(["--db", str(self.db), "stats"]), 0)
        self.assertEqual(dedup.main(["--db", str(self.db), "rejections"]), 0)

    def test_add_on_an_empty_directory_fails_loudly(self):
        self.assertEqual(
            dedup.main(["--db", str(self.db), "add", str(self.images),
                        "--source", "commons"]),
            1,
        )

    def test_add_on_a_missing_directory_fails_loudly(self):
        self.assertEqual(
            dedup.main(["--db", str(self.db), "add", str(self.root / "nope"),
                        "--source", "commons"]),
            2,
        )

    def test_an_unreadable_file_does_not_stop_the_run(self):
        photo(300).save(self.images / "good.png")
        (self.images / "broken.png").write_bytes(b"not a png")
        self.assertEqual(
            dedup.main(["--db", str(self.db), "add", str(self.images),
                        "--source", "commons"]),
            0,
        )
        with DedupStore(self.db) as s:
            self.assertEqual(s.count(), 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
