#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the Commons fetcher's pure parts.

    python3 -m unittest discover -s generators -p 'test_*.py'

Nothing here touches the network. The fetch loop itself is exercised by running
it against the live API, which is what the measurements in `docs/design/pentimento.md`
came from; what is tested here is the logic that decides what to keep, because
that is what changes the corpus.
"""
from __future__ import annotations

import contextlib
import io
import itertools
import pathlib
import random
import shutil
import tempfile
import unittest
import unittest.mock

from PIL import Image

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


class ExtMetadataShapeTests(unittest.TestCase):
    """The API does not promise one shape, and an overnight fetch died proving it.

    A 10,000 cover run stopped on its eighth cover with
    `'list' object has no attribute 'get'`: for some files Commons sends
    `extmetadata` as an empty list rather than an object. Every shape the API
    has been seen to send is handled here, at the boundary, once.
    """

    def test_the_ordinary_object_shape_reads(self):
        ii = {"extmetadata": {"LicenseShortName": {"value": "CC BY 4.0"}}}
        em = fetch_commons.extmetadata_of(ii)
        self.assertEqual(fetch_commons.extmeta_value(em, "LicenseShortName"),
                         "CC BY 4.0")

    def test_a_list_does_not_raise(self):
        """The exact crash, as a test."""
        for shape in ([], [{"value": "x"}], "", 0):
            em = fetch_commons.extmetadata_of({"extmetadata": shape})
            self.assertEqual(em, {}, f"shape {shape!r} was not neutralised")
            self.assertEqual(fetch_commons.extmeta_value(em, "LicenseShortName"), "")

    def test_a_missing_block_reads_as_empty(self):
        self.assertEqual(fetch_commons.extmetadata_of({}), {})

    def test_a_present_key_with_an_odd_value_still_reads(self):
        em = {"Artist": "a plain string", "Credit": None, "UsageTerms": 42}
        self.assertEqual(fetch_commons.extmeta_value(em, "Artist"), "a plain string")
        self.assertEqual(fetch_commons.extmeta_value(em, "Credit"), "")
        self.assertEqual(fetch_commons.extmeta_value(em, "UsageTerms"), "42")

    def test_an_absent_key_reads_as_empty(self):
        self.assertEqual(fetch_commons.extmeta_value({}, "LicenseShortName"), "")

    def test_metadata_in_the_wrong_shape_does_not_raise(self):
        """The same hazard on the EXIF side, which reads a list of objects."""
        for shape in ({}, "", None, [None, "x", 3]):
            self.assertEqual(exif_of({"metadata": shape}), {})


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


class ShareQuotaTests(unittest.TestCase):
    """Balancing an axis whose values are not equally available.

    Base ISO frames outnumber high ISO ones heavily in any photo collection, so
    a hard cap either never fires or stalls the fetch waiting for grain the
    source cannot supply. A share quota bounds the proportion instead.
    """

    def test_nothing_is_refused_below_the_floor(self):
        q = fetch_commons.ShareQuota("iso band", 0.5, floor=10)
        for _ in range(9):
            self.assertIsNone(q.refusal("base"))
            q.record("base")
        self.assertIsNone(q.refusal("base"), "still under the floor")

    def test_the_share_binds_once_the_floor_is_passed(self):
        q = fetch_commons.ShareQuota("iso band", 0.5, floor=10)
        for _ in range(10):
            q.record("base")
        self.assertIsNotNone(q.refusal("base"))
        self.assertIsNone(q.refusal("high"), "a starved band must stay open")

    def test_a_balanced_corpus_lets_everything_through(self):
        q = fetch_commons.ShareQuota("iso band", 0.5, floor=10)
        for i in range(40):
            band = ("base", "low", "high", "extreme")[i % 4]
            self.assertIsNone(q.refusal(band), f"refused {band} at {i}")
            q.record(band)

    def test_the_quota_converges_on_the_share(self):
        """Offer nothing but one band and it settles at the cap, not above it."""
        q = fetch_commons.ShareQuota("iso band", 0.5, floor=10)
        for _ in range(200):
            if q.refusal("base") is None:
                q.record("base")
            else:
                q.record("high")
        self.assertLessEqual(q.counts["base"] / q.total, 0.55)
        self.assertGreater(q.counts["base"] / q.total, 0.4)

    def test_an_unknown_band_is_never_refused(self):
        """A frame that never recorded its ISO is not evidence of imbalance."""
        q = fetch_commons.ShareQuota("iso band", 0.1, floor=1)
        for _ in range(50):
            q.record(None)
        self.assertIsNone(q.refusal(None))

    def test_resuming_restores_the_tally(self):
        q = fetch_commons.ShareQuota("iso band", 0.5, floor=10)
        q.resume_from(["base"] * 20)
        self.assertEqual(q.total, 20)
        self.assertIsNotNone(q.refusal("base"))

    def test_an_impossible_share_is_refused(self):
        for share in (0, -0.5, 1.5):
            with self.assertRaises(ValueError):
                fetch_commons.ShareQuota("iso band", share)
        fetch_commons.ShareQuota("iso band", 1.0)  # a share of everything is legal

    def test_the_refusal_says_which_axis_and_what_it_holds(self):
        q = fetch_commons.ShareQuota("iso band", 0.5, floor=2)
        q.record("base"); q.record("base")
        message = q.refusal("base")
        self.assertIn("iso band", message)
        self.assertIn("base", message)
        self.assertIn("50%", message)


class TiffIsAcceptedTests(unittest.TestCase):
    def test_tiff_passes_the_filter(self):
        """TIFF is where the never-compressed originals are."""
        self.assertTrue(suitable(info(mime="image/tiff"), 300, 6000, 512))

    def test_still_nothing_else(self):
        for mime in ("image/gif", "image/svg+xml", "application/pdf"):
            self.assertFalse(suitable(info(mime=mime), 300, 6000, 512))


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


class HeartbeatTests(unittest.TestCase):
    """A run measured in hours must not be indistinguishable from a wedged one.

    The beat fires on elapsed time rather than on progress, so a stretch where
    every candidate is refused still says so. That is the case that matters:
    the fetcher refuses roughly four candidates for every cover it keeps, and
    the old beat only ran after a cover was written.
    """

    def fetch(self, pages, count=2):
        """Run the fetcher against a fabricated Commons, with the clock moved
        on a minute between candidates so the beat is reached."""
        work = pathlib.Path(tempfile.mkdtemp(prefix="pentimento-test-"))
        self.addCleanup(shutil.rmtree, work, ignore_errors=True)
        # Textured rather than flat: the cover suitability gate refuses a crop
        # with no gradients, so a plain rectangle never reaches the write path.
        noise = random.Random(7)
        pixels = bytes(noise.randrange(40, 216) for _ in range(600 * 600 * 3))
        image = io.BytesIO()
        Image.frombytes("RGB", (600, 600), pixels).save(image, format="PNG")

        ticks = itertools.count(0, 60)
        out, err = io.StringIO(), io.StringIO()
        with unittest.mock.patch.object(fetch_commons, "random_candidates",
                                        return_value=iter(pages)), \
             unittest.mock.patch.object(fetch_commons, "fetch_bytes",
                                        return_value=image.getvalue()), \
             unittest.mock.patch.object(fetch_commons.time, "monotonic",
                                        side_effect=lambda: next(ticks)), \
             unittest.mock.patch.object(fetch_commons.time, "sleep",
                                        lambda _s: None), \
             contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = fetch_commons.main(
                ["--out", str(work), "--count", str(count), "--strategy",
                 "random", "--allow-scans"])
        return code, out.getvalue()

    @staticmethod
    def page(pageid, licence="CC0 1.0"):
        return ({"pageid": pageid, "title": f"File:{pageid}.png"},
                {"url": f"https://example.invalid/{pageid}.png",
                 "descriptionurl": f"https://example.invalid/{pageid}",
                 "mime": "image/png", "size": 1024 * 1024,
                 "width": 600, "height": 600, "sha1": f"{pageid:040x}",
                 "user": f"Uploader{pageid}",
                 "extmetadata": {"LicenseShortName": {"value": licence}},
                 "metadata": [{"name": "Make", "value": "Canon"}]})

    def beats(self, text):
        return [l for l in text.splitlines() if "candidates," in l]

    def test_the_run_says_what_it_is_doing_before_it_starts(self):
        _, out = self.fetch([self.page(1), self.page(2)])
        self.assertIn("fetching 2 covers into", out)

    def test_a_beat_names_progress_and_a_rate(self):
        _, out = self.fetch([self.page(i) for i in range(1, 6)], count=4)
        beats = self.beats(out)
        self.assertTrue(beats, out)
        self.assertRegex(beats[0], r"^  \d+/4 covers, \d+ candidates, "
                                   r"\d+\.\d/min$")

    def test_a_stretch_where_nothing_is_kept_still_beats(self):
        """Every one of these is refused on licence, so nothing is written."""
        refused = [self.page(i, licence="CC BY-SA 4.0") for i in range(1, 8)]
        _, out = self.fetch(refused, count=2)
        beats = self.beats(out)
        self.assertTrue(beats, "a run that keeps nothing went silent")
        self.assertIn("0/2 covers", beats[0])

    def test_nothing_a_terminal_would_have_to_interpret_is_printed(self):
        _, out = self.fetch([self.page(i) for i in range(1, 6)], count=4)
        for line in self.beats(out):
            self.assertNotIn("\r", line)
            self.assertNotIn("\x1b", line)


if __name__ == "__main__":
    unittest.main(verbosity=2)
