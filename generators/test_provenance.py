#!/usr/bin/env python3
"""Tests for the acquisition facts a cover carries.

    python3 -m unittest discover -s generators -p 'test_*.py'

The quality estimator is checked by round trip: encode at a known quality with
Pillow, read the table back, and see whether the estimate returns the quality we
asked for. That is the only honest way to test it, because the estimator's whole
job is to invert what an encoder did.
"""
from __future__ import annotations

import io
import unittest

import numpy as np
from PIL import Image

import provenance
from provenance import (
    crop_box,
    estimate_quality,
    iso_band,
    jpeg_profile,
    pristine,
    table_for_quality,
)


def photo(seed: int = 1, size: int = 256) -> Image.Image:
    rng = np.random.default_rng(seed)
    y, x = np.mgrid[0:size, 0:size] / size
    f = sum(np.sin(2 * np.pi * (rng.uniform(.5, 3) * x + rng.uniform(.5, 3) * y))
            for _ in range(4))
    f = (f - f.min()) / (np.ptp(f) + 1e-9)
    a = np.clip((f + rng.normal(0, .03, (size, size))) * 255, 0, 255).astype(np.uint8)
    return Image.fromarray(np.dstack([a] * 3), "RGB")


def as_jpeg(img: Image.Image, quality: int, **kw) -> Image.Image:
    buf = io.BytesIO()
    img.save(buf, format="JPEG", quality=quality, **kw)
    buf.seek(0)
    out = Image.open(buf)
    out.load()
    return out


class QualityEstimateTests(unittest.TestCase):
    def test_a_known_quality_round_trips(self):
        """Encode at Q, read the table back, and recover Q."""
        img = photo()
        for quality in (50, 60, 70, 75, 80, 85, 90, 95):
            got = jpeg_profile(as_jpeg(img, quality))["estimated_quality"]
            self.assertIsNotNone(got)
            self.assertLessEqual(
                abs(got - quality), 2,
                f"encoded at {quality}, estimated {got}",
            )

    def test_low_qualities_round_trip_too(self):
        img = photo()
        for quality in (20, 30, 40):
            got = jpeg_profile(as_jpeg(img, quality))["estimated_quality"]
            self.assertLessEqual(abs(got - quality), 4, f"{quality} -> {got}")

    def test_the_generated_table_matches_the_standard_at_quality_50(self):
        """At quality 50 the scale factor is 1, so the table is the IJG base."""
        self.assertEqual(table_for_quality(50), provenance.IJG_LUMINANCE)

    def test_higher_quality_means_smaller_divisors(self):
        self.assertLess(sum(table_for_quality(95)), sum(table_for_quality(50)))
        self.assertLess(sum(table_for_quality(50)), sum(table_for_quality(10)))

    def test_a_short_table_is_refused_rather_than_guessed(self):
        self.assertIsNone(estimate_quality([16] * 10))

    def test_quality_is_clamped_to_the_legal_range(self):
        self.assertEqual(provenance.scale_for_quality(0),
                         provenance.scale_for_quality(1))
        self.assertEqual(provenance.scale_for_quality(200),
                         provenance.scale_for_quality(100))


class JpegProfileTests(unittest.TestCase):
    def test_a_png_has_no_compression_history(self):
        self.assertEqual(jpeg_profile(photo()), {"compressed": False})

    def test_a_jpeg_reports_its_tables(self):
        p = jpeg_profile(as_jpeg(photo(), 85))
        self.assertTrue(p["compressed"])
        self.assertGreaterEqual(p["quant_tables"], 1)
        self.assertEqual(len(p["quant_digest"]), 16)

    def test_the_digest_groups_by_encoder_settings(self):
        """Same settings, same digest. Different settings, different digest."""
        a = jpeg_profile(as_jpeg(photo(1), 85))["quant_digest"]
        b = jpeg_profile(as_jpeg(photo(2), 85))["quant_digest"]
        c = jpeg_profile(as_jpeg(photo(1), 60))["quant_digest"]
        self.assertEqual(a, b, "two images at one quality share a table")
        self.assertNotEqual(a, c, "different quality must not share a digest")

    def test_progressive_is_noticed(self):
        self.assertTrue(
            jpeg_profile(as_jpeg(photo(), 85, progressive=True))["progressive"]
        )

    def test_subsampling_is_reported_as_a_label(self):
        p = jpeg_profile(as_jpeg(photo(), 85, subsampling=2))
        self.assertIn(p["subsampling"], ("4:2:0", "4:2:2", "4:4:4", None))


class PristineTests(unittest.TestCase):
    def test_a_png_original_is_pristine(self):
        self.assertTrue(pristine("image/png", {"compressed": False}))
        self.assertTrue(pristine("image/tiff", {"compressed": False}))

    def test_a_jpeg_is_never_pristine(self):
        self.assertFalse(pristine("image/jpeg", {"compressed": True}))
        self.assertFalse(pristine("image/jpeg", {"compressed": False}),
                         "the mime type alone disqualifies it")

    def test_a_png_carrying_quantisation_tables_is_not_pristine(self):
        self.assertFalse(pristine("image/png", {"compressed": True}))

    def test_an_unknown_type_is_not_assumed_pristine(self):
        self.assertFalse(pristine(None, {"compressed": False}))
        self.assertFalse(pristine("image/webp", {"compressed": False}))


class IsoBandTests(unittest.TestCase):
    def test_bands_split_by_noise_regime(self):
        self.assertEqual(iso_band({"ISOSpeedRatings": "100"}), "base")
        self.assertEqual(iso_band({"ISOSpeedRatings": "400"}), "low")
        self.assertEqual(iso_band({"ISOSpeedRatings": "1600"}), "high")
        self.assertEqual(iso_band({"ISOSpeedRatings": "12800"}), "extreme")

    def test_the_boundaries_land_where_documented(self):
        self.assertEqual(iso_band({"ISOSpeedRatings": "200"}), "low")
        self.assertEqual(iso_band({"ISOSpeedRatings": "199"}), "base")

    def test_a_frame_that_never_said_gives_none(self):
        for value in ({}, {"ISOSpeedRatings": ""}, {"ISOSpeedRatings": None}):
            self.assertIsNone(iso_band(value))

    def test_the_shapes_commons_actually_sends_are_parsed(self):
        """The API returns these as strings, sometimes bracketed or with units."""
        self.assertEqual(iso_band({"ISOSpeedRatings": "[400]"}), "low")
        self.assertEqual(iso_band({"ISOSpeedRatings": "400 "}), "low")
        self.assertEqual(iso_band({"ISOSpeedRatings": "1600, 1600"}), "high")

    def test_nonsense_gives_none_rather_than_raising(self):
        self.assertIsNone(iso_band({"ISOSpeedRatings": "auto"}))


class CropBoxTests(unittest.TestCase):
    def test_the_box_is_the_requested_size_and_inside_the_image(self):
        for seed in range(30):
            left, top, right, bottom = crop_box(4000, 3000, 512, f"{seed}")
            self.assertEqual((right - left, bottom - top), (512, 512))
            self.assertGreaterEqual(left, 0)
            self.assertGreaterEqual(top, 0)
            self.assertLessEqual(right, 4000)
            self.assertLessEqual(bottom, 3000)

    def test_the_same_seed_gives_the_same_box(self):
        self.assertEqual(crop_box(4000, 3000, 512, "page-17"),
                         crop_box(4000, 3000, 512, "page-17"))

    def test_different_seeds_move_the_box(self):
        boxes = {crop_box(4000, 3000, 512, f"page-{i}") for i in range(20)}
        self.assertGreater(len(boxes), 15, "the position is barely varying")

    def test_an_exact_fit_has_only_one_position(self):
        self.assertEqual(crop_box(512, 512, 512, "anything"), (0, 0, 512, 512))

    def test_too_small_is_refused(self):
        with self.assertRaises(ValueError):
            crop_box(400, 3000, 512, "s")

    def test_positions_spread_across_the_frame(self):
        """A centre crop over-samples subjects; this must not quietly recentre."""
        lefts = [crop_box(4000, 3000, 512, f"p{i}")[0] for i in range(200)]
        self.assertLess(min(lefts), 700, "nothing near the left edge")
        self.assertGreater(max(lefts), 2800, "nothing near the right edge")


if __name__ == "__main__":
    unittest.main(verbosity=2)
