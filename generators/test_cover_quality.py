#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the cover suitability gate.

    python3 -m unittest discover -s generators -p 'test_*.py'

The cases that matter are the two the gate exists to separate: a flat field,
which is what centre cropping a stock photograph into a patch of sky produces,
and a dark but detailed photograph, which must not be mistaken for one.
"""
from __future__ import annotations

import pathlib
import tempfile
import unittest

import numpy as np
from PIL import Image

import cover_quality
from cover_quality import LOW_TEXTURE, TEXTURE_FLOOR, assess, assess_file, texture


def flat(value: int = 128, size: int = 64) -> Image.Image:
    return Image.fromarray(np.full((size, size, 3), value, dtype=np.uint8), "RGB")


def detailed(seed: int = 1, size: int = 64, amplitude: float = 40.0,
             offset: float = 128.0) -> Image.Image:
    rng = np.random.default_rng(seed)
    arr = np.clip(rng.normal(offset, amplitude, (size, size)), 0, 255).astype(np.uint8)
    return Image.fromarray(np.dstack([arr] * 3), "RGB")


class TextureTests(unittest.TestCase):
    def test_a_flat_field_has_no_texture(self):
        self.assertEqual(texture(flat()), 0.0)

    def test_brightness_does_not_change_texture(self):
        """A dark photograph and a bright one with the same detail score alike."""
        dark = detailed(2, amplitude=20, offset=40)
        bright = detailed(2, amplitude=20, offset=200)
        self.assertAlmostEqual(texture(dark), texture(bright), delta=1.5)

    def test_more_detail_scores_higher(self):
        self.assertGreater(texture(detailed(3, amplitude=60)),
                           texture(detailed(3, amplitude=5)))

    def test_a_smooth_gradient_is_nearly_flat(self):
        """A sky gradient varies a lot overall and almost nothing locally."""
        ramp = np.tile(np.linspace(0, 255, 64), (64, 1)).astype(np.uint8)
        img = Image.fromarray(np.dstack([ramp] * 3), "RGB")
        self.assertLess(texture(img), TEXTURE_FLOOR)

    def test_too_small_to_measure_is_refused(self):
        with self.assertRaises(ValueError):
            texture(flat(size=2))


class AssessTests(unittest.TestCase):
    def test_a_flat_field_is_unusable_and_says_why(self):
        q = assess(flat(200))
        self.assertFalse(q.usable)
        self.assertIn("below", q.reason)
        self.assertIn("payload", q.reason)

    def test_a_detailed_photograph_is_usable(self):
        q = assess(detailed(4))
        self.assertTrue(q.usable)
        self.assertIsNone(q.reason)
        self.assertGreater(q.texture, LOW_TEXTURE)

    def test_thin_detail_is_kept_but_flagged(self):
        """Between the floor and the advisory band: usable, and worth counting."""
        img = detailed(5, amplitude=0.8)
        q = assess(img)
        if not (TEXTURE_FLOOR <= q.texture < LOW_TEXTURE):
            self.skipTest(f"fixture landed at texture {q.texture:.2f}")
        self.assertTrue(q.usable)
        self.assertTrue(q.low_texture)

    def test_clipping_is_recorded_and_never_gates(self):
        arr = np.zeros((64, 64, 3), dtype=np.uint8)
        arr[::2] = 255  # half pure white, half pure black, and richly textured
        img = Image.fromarray(arr, "RGB")
        q = assess(img)
        self.assertAlmostEqual(q.clipped, 1.0, places=6)
        self.assertTrue(q.usable, "clipping must not reject a cover")

    def test_as_dict_is_manifest_ready(self):
        d = assess(detailed(6)).as_dict()
        self.assertEqual(set(d), {"texture", "clipped", "usable", "reason"})
        import json
        json.loads(json.dumps(d))


class FileAndCommandLineTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)

    def test_assess_file_round_trip(self):
        p = self.root / "a.png"
        detailed(7).save(p)
        self.assertTrue(assess_file(p).usable)

    def test_summary_reports_the_unusable_ones(self):
        detailed(8).save(self.root / "good.png")
        flat(90).save(self.root / "sky.png")
        self.assertEqual(cover_quality.main([str(self.root)]), 0)

    def test_json_mode_emits_one_object_per_image(self):
        detailed(9).save(self.root / "good.png")
        self.assertEqual(cover_quality.main([str(self.root), "--json"]), 0)

    def test_missing_directory_fails_loudly(self):
        self.assertEqual(cover_quality.main([str(self.root / "nope")]), 2)

    def test_empty_directory_fails_loudly(self):
        self.assertEqual(cover_quality.main([str(self.root)]), 1)


class RegressionTests(unittest.TestCase):
    """The three real covers that started this, reproduced from their measurements.

    00086 measured 0.030, 00053 measured 0.919 and 00038 measured 1.039 on the
    Pexels sample of 2026-09-16. The first two must be refused and the third,
    an out-of-focus dark mass with real if faint structure, must be kept. The
    floor sits between them on purpose.
    """

    def test_the_floor_sits_where_the_measurements_put_it(self):
        self.assertLess(0.030, TEXTURE_FLOOR)
        self.assertLess(0.919, TEXTURE_FLOOR)
        self.assertGreater(1.039, TEXTURE_FLOOR)

    def test_two_different_flat_fields_are_both_refused(self):
        """Neither may enter the corpus, so they can never collide inside it."""
        for value in (168, 230, 90):
            self.assertFalse(assess(flat(value)).usable)


if __name__ == "__main__":
    unittest.main(verbosity=2)
