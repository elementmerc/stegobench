#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the real-tool embedders.

    python3 -m unittest discover -s generators -p 'test_*.py'

Two layers, and the split matters.

The **contract** tests run everywhere and need no tool installed. They check the
shared refusals, the capacity arithmetic and the parsers, which is most of what
actually breaks: a tool that is absent fails loudly, and a tool that is present
fails in ways only the tool can show you.

The **live** tests run a real embedding through each tool that is available on
this machine and skip the rest with a message naming what is missing. They are
the only tests that can catch a wrong command-line flag, so a skipped one is a
gap rather than a pass. `python3 test_tools.py --report` prints which ran.
"""
from __future__ import annotations

import pathlib
import sys
import tempfile
import unittest

import numpy as np
from PIL import Image

import tools
from embedders import EmbedError, Embedder
from tools import _parse_capacity, build


def photo(seed: int = 1, size: int = 512) -> Image.Image:
    """A cover with real structure, since flat images are refused upstream."""
    rng = np.random.default_rng(seed)
    y, x = np.mgrid[0:size, 0:size] / size
    f = sum(np.sin(2 * np.pi * (rng.uniform(.5, 4) * x + rng.uniform(.5, 4) * y))
            for _ in range(5))
    f = (f - f.min()) / (np.ptp(f) + 1e-9)
    a = np.clip((f + rng.normal(0, .05, (size, size))) * 255, 0, 255).astype(np.uint8)
    return Image.fromarray(np.dstack([a] * 3), "RGB")


class CapacityParserTests(unittest.TestCase):
    """Steghide reports capacity as prose, and the corpus depends on reading it."""

    def test_the_shapes_steghide_actually_prints(self):
        self.assertEqual(_parse_capacity("907.0 Byte"), 907)
        self.assertEqual(_parse_capacity("12.3 KB"), int(12.3 * 1024))
        self.assertEqual(_parse_capacity("1.0 MB"), 1024 ** 2)

    def test_a_bare_number_is_bytes(self):
        self.assertEqual(_parse_capacity("512"), 512)

    def test_nonsense_is_refused_rather_than_guessed(self):
        for text in ("", "lots", "KB", "?? KB"):
            with self.assertRaises(EmbedError, msg=text):
                _parse_capacity(text)

    def test_an_unknown_unit_is_refused(self):
        with self.assertRaises(EmbedError):
            _parse_capacity("4.0 GB")


class ContractTests(unittest.TestCase):
    """What every embedder promises, checked without running any of them."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.png = self.root / "cover.png"
        photo().save(self.png)
        self.jpg = self.root / "cover.jpg"
        photo(2).save(self.jpg, quality=92)

    def test_every_embedder_declares_an_id_and_formats(self):
        seen = set()
        for e in build():
            self.assertTrue(e.id, f"{type(e).__name__} has no id")
            self.assertNotIn(e.id, seen, f"duplicate id {e.id}")
            seen.add(e.id)
            self.assertTrue(e.formats, f"{e.id} accepts no formats")
            for fmt in e.formats:
                self.assertTrue(fmt.startswith("."), f"{e.id}: {fmt}")

    def test_the_tier_one_and_two_set_is_complete(self):
        self.assertEqual(
            {e.id for e in build()},
            {"steghide", "outguess", "openstego", "stegcore",
             "hstego", "stego_lsb", "stegano", "stegosuite"},
        )

    def test_no_detector_is_in_the_embedder_set(self):
        """Aletheia and friends analyse; they do not embed."""
        ids = {e.id for e in build()}
        for detector in ("aletheia", "zsteg", "stegexpose", "stegoveritas"):
            self.assertNotIn(detector, ids)

    def test_accepts_matches_the_declared_formats(self):
        for e in build():
            self.assertEqual(e.accepts(self.png), ".png" in e.formats, e.id)
            self.assertEqual(e.accepts(self.jpg), ".jpg" in e.formats, e.id)
            self.assertFalse(e.accepts(pathlib.Path("x.tiff")), e.id)

    def test_capacity_is_positive_and_below_the_cover(self):
        """A capacity above the file size would mean the arithmetic is wrong."""
        limit = self.png.stat().st_size * 4
        for e in build():
            if ".png" not in e.formats or isinstance(e, tools.SteghideEmbedder):
                continue  # steghide asks the tool rather than computing
            room = e.capacity(self.png)
            self.assertGreater(room, 0, e.id)
            self.assertLess(room, limit, f"{e.id} claims implausible capacity")

    def test_an_empty_payload_is_refused(self):
        for e in build():
            if not e.available():
                continue
            with self.assertRaises(EmbedError, msg=e.id):
                e.embed(self.png, b"", self.root / "out.png")

    def test_a_missing_cover_is_refused(self):
        for e in build():
            if not e.available():
                continue
            with self.assertRaises(EmbedError, msg=e.id):
                e.embed(self.root / "nope.png", b"hi", self.root / "out.png")

    def test_the_wrong_format_is_refused_by_name(self):
        for e in build():
            if not e.available() or ".png" in e.formats:
                continue
            with self.assertRaises(EmbedError) as cm:
                e.embed(self.png, b"hi", self.root / "out")
            self.assertIn(e.id, str(cm.exception))

    def test_an_oversized_payload_is_refused_before_it_corrupts_anything(self):
        """Silent truncation is the failure this prevents."""
        for e in build():
            if not e.available() or ".png" not in e.formats:
                continue
            too_big = b"x" * (e.capacity(self.png) + 4096)
            with self.assertRaises(EmbedError) as cm:
                e.embed(self.png, too_big, self.root / "out.png")
            self.assertIn("holds", str(cm.exception))

    def test_an_unavailable_tool_says_so_rather_than_failing_obscurely(self):
        missing = tools.StegcoreEmbedder(binary="/nonexistent/stegcore")
        self.assertFalse(missing.available())
        with self.assertRaises(EmbedError) as cm:
            missing.embed(self.png, b"hi", self.root / "out.png")
        self.assertIn("not available", str(cm.exception))


class LiveEmbedTests(unittest.TestCase):
    """A real embedding through every tool present. Skips name what is absent."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.png = self.root / "cover.png"
        photo(7).save(self.png)
        self.jpg = self.root / "cover.jpg"
        photo(8).save(self.jpg, quality=92)

    def _cover_for(self, e: Embedder) -> pathlib.Path:
        return self.png if ".png" in e.formats else self.jpg

    def _run_one(self, e: Embedder):
        if not e.available():
            self.skipTest(f"{e.id} is not available here")
        cover = self._cover_for(e)
        payload = b"pentimento round trip " * 8
        room = e.capacity(cover)
        self.assertGreater(room, len(payload),
                           f"{e.id}: test cover too small for the test payload")
        out = self.root / f"stego_{e.id}{cover.suffix}"

        result = e.embed(cover, payload, out)

        self.assertTrue(out.is_file(), f"{e.id} reported success and wrote nothing")
        self.assertGreater(out.stat().st_size, 0, f"{e.id} wrote an empty file")
        self.assertEqual(result.tool, e.id)
        self.assertEqual(result.payload_bytes, len(payload))
        self.assertIsInstance(result.detail, dict)

        # The pair must differ, or nothing was embedded. This is the check that
        # catches a tool that exits 0 and copies its input, which several do
        # when they silently fail.
        self.assertNotEqual(out.read_bytes(), cover.read_bytes(),
                            f"{e.id} produced a byte-identical copy of the cover")

        # A spatial tool must not change the picture's size, or the pair is not
        # matched and every result drawn from it is confounded.
        if cover.suffix == ".png":
            with Image.open(cover) as a, Image.open(out) as b:
                self.assertEqual(a.size, b.size,
                                 f"{e.id} changed the image dimensions")

    def test_steghide(self):
        self._run_one(tools.SteghideEmbedder())

    def test_outguess(self):
        self._run_one(tools.OutguessEmbedder())

    def test_openstego(self):
        self._run_one(tools.OpenStegoEmbedder())

    def test_stegcore(self):
        self._run_one(tools.StegcoreEmbedder())

    def test_hstego(self):
        self._run_one(tools.HStegoEmbedder())

    def test_stego_lsb(self):
        self._run_one(tools.StegoLsbEmbedder(sys.executable))

    def test_stegano(self):
        self._run_one(tools.SteganoEmbedder(sys.executable))

    def test_stegosuite(self):
        self._run_one(tools.StegosuiteEmbedder())


if __name__ == "__main__":
    if "--report" in sys.argv:
        sys.argv.remove("--report")
        print("embedder availability on this machine:")
        for name, ok in tools.survey(sys.executable).items():
            print(f"  {'ok  ' if ok else 'MISS'} {name}")
        print()
    unittest.main(verbosity=2)
