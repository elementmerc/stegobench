#!/usr/bin/env python3
"""Tests for the Commons fetcher's pure parts.

    python3 -m unittest discover -s generators -p 'test_*.py'

Nothing here touches the network. The fetch loop itself is exercised by running
it against the live API, which is what the measurements in `docs/pentimento.md`
came from; what is tested here is the logic that decides what to keep, because
that is what changes the corpus.
"""
from __future__ import annotations

import unittest

import fetch_commons
from fetch_commons import DiversityCaps, exif_of, strip_html, suitable


def info(**kw) -> dict:
    """An imageinfo record with plausible defaults."""
    base = {"mime": "image/jpeg", "size": 2_000_000, "width": 4000, "height": 3000}
    base.update(kw)
    return base


class SuitabilityTests(unittest.TestCase):
    def test_a_normal_photograph_passes(self):
        self.assertTrue(suitable(info(), 300, 6000, 512))

    def test_wrong_media_type_is_refused(self):
        for mime in ("image/svg+xml", "application/pdf", "image/gif", "video/webm"):
            self.assertFalse(suitable(info(mime=mime), 300, 6000, 512), mime)

    def test_an_empty_record_is_refused(self):
        self.assertFalse(suitable({}, 300, 6000, 512))

    def test_files_outside_the_size_window_are_refused(self):
        self.assertFalse(suitable(info(size=50_000), 300, 6000, 512),
                         "a tiny file is usually a graphic, not a photograph")
        self.assertFalse(suitable(info(size=80_000_000), 300, 6000, 512),
                         "80 MB to keep a quarter of a megapixel is rude")

    def test_an_image_too_small_to_crop_is_refused(self):
        self.assertFalse(suitable(info(width=400, height=3000), 300, 6000, 512))
        self.assertFalse(suitable(info(width=4000, height=300), 300, 6000, 512))
        self.assertTrue(suitable(info(width=512, height=512), 300, 6000, 512))

    def test_missing_dimensions_are_refused_rather_than_assumed(self):
        self.assertFalse(suitable({"mime": "image/jpeg", "size": 2_000_000},
                                  300, 6000, 512))


class ExifTests(unittest.TestCase):
    def test_kept_fields_are_flattened(self):
        ii = {"metadata": [
            {"name": "Make", "value": "NIKON CORPORATION"},
            {"name": "Model", "value": "NIKON D4"},
            {"name": "ISOSpeedRatings", "value": 400},
            {"name": "Software", "value": "Adobe Photoshop"},
        ]}
        got = exif_of(ii)
        self.assertEqual(got["Make"], "NIKON CORPORATION")
        self.assertEqual(got["ISOSpeedRatings"], "400")
        self.assertNotIn("Software", got, "only the declared fields are kept")

    def test_absent_and_empty_metadata_give_an_empty_dict(self):
        self.assertEqual(exif_of({}), {})
        self.assertEqual(exif_of({"metadata": None}), {})
        self.assertEqual(exif_of({"metadata": [{"name": "Make", "value": ""}]}), {})


class StripHtmlTests(unittest.TestCase):
    def test_tags_are_removed_and_text_kept(self):
        self.assertEqual(
            strip_html('<a href="/wiki/User:Someone">Someone</a>'), "Someone"
        )

    def test_whitespace_is_collapsed(self):
        self.assertEqual(strip_html("  a   \n b  "), "a b")

    def test_empty_input_is_safe(self):
        self.assertEqual(strip_html(""), "")
        self.assertEqual(strip_html(None), "")


class DiversityCapTests(unittest.TestCase):
    """The mechanism that handles redundancy, which deduplication cannot.

    Reproduces the case that prompted it: a 20 cover run returned four frames of
    `ISS0xx-E-xxxxx - View of Earth`, every one a genuinely different picture
    from one camera and one uploader.
    """

    def test_an_uploader_is_capped(self):
        caps = DiversityCaps(per_uploader=2, per_camera=0)
        for _ in range(2):
            self.assertIsNone(caps.refusal("NASA-bot", None))
            caps.record("NASA-bot", None)
        self.assertIn("NASA-bot", caps.refusal("NASA-bot", None))
        self.assertIsNone(caps.refusal("someone-else", None),
                          "one uploader hitting the cap must not block others")

    def test_a_camera_is_capped(self):
        caps = DiversityCaps(per_uploader=0, per_camera=1)
        caps.record(None, "NIKON CORPORATION NIKON D4")
        self.assertIn("NIKON D4", caps.refusal(None, "NIKON CORPORATION NIKON D4"))
        self.assertIsNone(caps.refusal(None, "Apple iPhone 13"))

    def test_zero_disables_a_cap(self):
        caps = DiversityCaps(per_uploader=0, per_camera=0)
        for _ in range(500):
            caps.record("NASA-bot", "NIKON D4")
        self.assertIsNone(caps.refusal("NASA-bot", "NIKON D4"))

    def test_unknown_uploader_or_camera_never_blocks(self):
        """A missing field is not evidence of concentration."""
        caps = DiversityCaps(per_uploader=1, per_camera=1)
        caps.record(None, None)
        caps.record(None, None)
        self.assertIsNone(caps.refusal(None, None))

    def test_the_camera_key_joins_make_and_model(self):
        self.assertEqual(
            DiversityCaps.camera_key({"Make": "Apple", "Model": "iPhone 13"}),
            "Apple iPhone 13",
        )
        self.assertEqual(DiversityCaps.camera_key({"Make": "Apple"}), "Apple")
        self.assertIsNone(DiversityCaps.camera_key({}))

    def test_two_makes_sharing_a_model_name_stay_distinct(self):
        a = DiversityCaps.camera_key({"Make": "Canon", "Model": "EOS 5D"})
        b = DiversityCaps.camera_key({"Make": "Nikon", "Model": "EOS 5D"})
        self.assertNotEqual(a, b)

    def test_resuming_restores_the_tally(self):
        """An interrupted run must not reset the counters and double every cap."""
        rows = [
            {"uploader": "NASA-bot", "exif": {"Make": "NIKON", "Model": "D4"}},
            {"uploader": "NASA-bot", "exif": {"Make": "NIKON", "Model": "D4"}},
            {"uploader": "someone", "exif": {"Make": "Apple", "Model": "iPhone"}},
        ]
        caps = DiversityCaps(per_uploader=2, per_camera=2)
        caps.resume_from(rows)
        self.assertIsNotNone(caps.refusal("NASA-bot", "NIKON D4"))
        self.assertIsNone(caps.refusal("someone", "Apple iPhone"))

    def test_resuming_tolerates_rows_written_before_these_fields_existed(self):
        caps = DiversityCaps(per_uploader=1, per_camera=1)
        caps.resume_from([{"file": "00000.png"}, {"exif": None}])
        self.assertIsNone(caps.refusal("anyone", "any camera"))

    def test_the_iss_case_is_actually_stopped(self):
        caps = DiversityCaps(per_uploader=40, per_camera=3)
        admitted = 0
        for _ in range(100):
            if caps.refusal("NASA-importer", "NIKON CORPORATION NIKON D4") is None:
                caps.record("NASA-importer", "NIKON CORPORATION NIKON D4")
                admitted += 1
        self.assertEqual(admitted, 3, "the camera cap, the tighter of the two, binds")


class PermissiveSetTests(unittest.TestCase):
    def test_share_alike_is_not_permissive(self):
        """Ruled 2026-09-16: permissive only, so share-alike must stay out."""
        for lic in ("cc by-sa 4.0", "cc by-sa 3.0", "cc by-sa 2.0", "gfdl"):
            self.assertNotIn(lic, fetch_commons.PERMISSIVE)

    def test_the_expected_permissive_licences_are_present(self):
        for lic in ("cc0", "public domain", "cc by 2.0", "cc by 4.0"):
            self.assertIn(lic, fetch_commons.PERMISSIVE)

    def test_non_commercial_and_no_derivatives_are_absent(self):
        for lic in fetch_commons.PERMISSIVE:
            self.assertNotIn("-nc", lic)
            self.assertNotIn("-nd", lic)


if __name__ == "__main__":
    unittest.main(verbosity=2)
