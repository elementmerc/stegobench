#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the manifest repair pass.

    python3 -m unittest discover -s generators -p 'test_*.py'

The thing worth guarding here is not the arithmetic. It is that this tool
rewrites, in place, the only record of every cover's licence, credit line and
train/test split, and that a mistyped `--salt` is not detectable afterwards
from the file itself. So the previous manifest has to survive the run.
"""
from __future__ import annotations

import contextlib
import io
import json
import pathlib
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import manifest_repair  # noqa: E402


def write_manifest(root: pathlib.Path, count: int = 4) -> pathlib.Path:
    rows = [{
        "file": f"{i:05d}.png",
        "licence": "CC0 1.0",
        "artist": f"Photographer {i}",
        "credit": "test",
        "title": f"{i:05d}.png",
        "commons_sha1": f"{i:040x}",
        "descriptionurl": f"https://example.invalid/{i}",
    } for i in range(count)]
    path = root / "manifest.jsonl"
    path.write_text("".join(json.dumps(r) + "\n" for r in rows),
                    encoding="utf-8")
    return path


def repair(*argv: str) -> tuple[int, str]:
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        code = manifest_repair.main(list(argv))
    return code, out.getvalue()


def rows_of(path: pathlib.Path) -> list[dict]:
    return [json.loads(l) for l in path.read_text(encoding="utf-8").splitlines()
            if l.strip()]


class BackupTests(unittest.TestCase):
    def setUp(self):
        self.work = pathlib.Path(tempfile.mkdtemp(prefix="pentimento-test-"))
        self.addCleanup(shutil.rmtree, self.work, ignore_errors=True)
        self.manifest = write_manifest(self.work)
        self.before = self.manifest.read_bytes()

    def backups(self) -> list[pathlib.Path]:
        return sorted(self.work.glob("manifest.jsonl.bak.*"))

    def test_an_in_place_run_keeps_the_manifest_it_replaced(self):
        code, out = repair(str(self.manifest))
        self.assertEqual(code, 0)
        kept = self.backups()
        self.assertEqual(len(kept), 1, out)
        self.assertEqual(kept[0].read_bytes(), self.before)

    def test_the_backup_path_is_reported_rather_than_left_to_be_found(self):
        _, out = repair(str(self.manifest))
        self.assertIn(str(self.backups()[0]), out)

    def test_the_repaired_manifest_is_still_written(self):
        repair(str(self.manifest))
        rows = rows_of(self.manifest)
        self.assertEqual([r["tier_order"] for r in rows], [0, 1, 2, 3])

    def test_no_backup_is_kept_when_the_output_is_somewhere_else(self):
        """Writing elsewhere leaves the original where it was already."""
        code, _ = repair(str(self.manifest), "--out",
                         str(self.work / "repaired.jsonl"))
        self.assertEqual(code, 0)
        self.assertEqual(self.backups(), [])
        self.assertEqual(self.manifest.read_bytes(), self.before)

    def test_a_dry_run_writes_nothing_at_all(self):
        code, out = repair(str(self.manifest), "--dry-run")
        self.assertEqual(code, 0)
        self.assertIn("nothing written", out)
        self.assertEqual(self.backups(), [])
        self.assertEqual(self.manifest.read_bytes(), self.before)

    def test_the_backup_can_be_declined(self):
        code, _ = repair(str(self.manifest), "--no-backup")
        self.assertEqual(code, 0)
        self.assertEqual(self.backups(), [])

    def test_a_second_wrong_run_cannot_overwrite_the_good_copy(self):
        """The guard is against a mistyped salt, which is often run twice."""
        repair(str(self.manifest), "--salt", "oops")
        first = self.backups()
        self.assertEqual(len(first), 1)
        self.assertEqual(first[0].read_bytes(), self.before)
        repair(str(self.manifest), "--salt", "oops-again")
        self.assertEqual(len(self.backups()), 2)
        self.assertEqual(first[0].read_bytes(), self.before)


class SaltHelpTests(unittest.TestCase):
    """The default is the PUBLISHED corpus's salt, which is right for one
    reader and exactly wrong for the other."""

    def setUp(self):
        # Defaults reach every subcommand's help through the entry point, which
        # is the documented way in and the only place the convention lives.
        import cli
        cli.with_defaults_in_help()

    def help_text(self) -> str:
        with contextlib.redirect_stdout(io.StringIO()) as out:
            with self.assertRaises(SystemExit):
                manifest_repair.main(["--help"])
        return " ".join(out.getvalue().split())

    def test_the_default_salt_is_stated(self):
        self.assertIn("pentimento-v1", self.help_text())

    def test_the_help_separates_a_new_corpus_from_an_existing_one(self):
        text = self.help_text()
        self.assertIn("NEW corpus", text)
        self.assertIn("EXISTING one", text)


class CaptureClassTests(unittest.TestCase):
    """The capture device a row claims, and the one it actually had."""

    @staticmethod
    def classify(make: str, model: str, title: str = "File:Example.jpg"):
        return manifest_repair.capture_class(
            {"exif": {"Make": make, "Model": model}, "title": title})

    def test_a_scanned_cover_is_not_recorded_as_a_camera(self):
        """Flatbed and film scanners that the observed list missed.

        Every one of these landed in `camera` because the scanner patterns were
        drawn from the 10,000 covers to hand and nothing else. An unmatched
        scanner doesn't fall into `unknown`, it falls through to the camera
        branch, so the corpus published a scanned page as sensor capture and a
        user calibrating on it inherited a domain shift with no field to see it
        in. The Epson Expression pair is the shape in miniature: the old
        pattern required an XL suffix, so the 12000XL was a scanner and the
        1680 beside it was a camera.
        """
        for make, model in (("EPSON", "Expression 1680"),
                            ("Seiko Epson Corp.", "EPSON Expression 12000XL"),
                            ("UMAX", "Astra 4000U"),
                            ("Mustek", "BearPaw 2448TA Pro"),
                            ("AGFA", "SnapScan e50"),
                            ("FUJITSU", "fi-7160"),
                            ("Nikon", "LS-2000")):
            with self.subTest(model=model):
                cls, basis = self.classify(make, model)
                self.assertEqual(cls, "scanner")
                self.assertEqual(basis, "scanner hardware in EXIF")

    def test_a_camera_is_still_recorded_as_a_camera(self):
        """The other half of the same failure, and the costlier one.

        Widening the scanner patterns is only safe if it cannot swallow a
        sensor capture. A camera wrongly filed as a scanner is worse than the
        defect it fixes, because the camera population is what the corpus is
        for and these rows would be quietly excluded from it.
        """
        for make, model in (("NIKON CORPORATION", "NIKON D750"),
                            ("Canon", "Canon EOS 5D Mark IV"),
                            ("Apple", "iPhone 13 Pro"),
                            ("Panasonic", "DMC-LS80"),
                            ("Nikon", "COOLPIX L820"),
                            ("Fujifilm", "FinePix S2000HD"),
                            ("Xiaomi", "Redmi Note 8")):
            with self.subTest(model=model):
                cls, basis = self.classify(make, model)
                self.assertEqual(cls, "camera")
                self.assertEqual(basis, "camera hardware in EXIF")

    def test_a_row_with_no_capture_hardware_is_unknown_rather_than_guessed(self):
        """An absent make is a gap, and a gap is not a camera.

        The class is published and acted on, so the honest answer where there
        is no evidence has to stay distinguishable from an answer there is
        evidence for.
        """
        cls, basis = manifest_repair.capture_class({"exif": {}, "title": "x"})
        self.assertEqual(cls, "unknown")
        self.assertEqual(basis, "no capture hardware recorded")


class AttributionTests(unittest.TestCase):
    """Whether a downstream user owes the author a credit, per licence."""

    @staticmethod
    def row(licence: str) -> dict:
        return {"licence": licence, "artist": "A. Photographer",
                "title": "File:Example.jpg",
                "descriptionurl": "https://example.invalid/1"}

    def test_a_share_alike_cover_still_requires_attribution(self):
        """CC BY-SA came out of the repair pass with the flag set to false.

        Share-alike adds an obligation on top of attribution and never removes
        it, but the set this is derived from held only the plain CC BY
        versions. The corpus is published to three public archives, so a false
        flag here is credit withheld from an author whose licence demands it,
        and it silences `select_unpublishable.py` too: that tool only checks
        whether an author was recorded on rows the flag says need one.
        """
        for licence in ("CC BY-SA 1.0", "CC BY-SA 2.0", "CC BY-SA 2.5",
                        "CC BY-SA 3.0", "CC BY-SA 4.0"):
            with self.subTest(licence=licence):
                line, required = manifest_repair.attribution_for(self.row(licence))
                self.assertTrue(required)
                self.assertIn(licence, line)

    def test_a_share_alike_credit_line_links_the_licence_it_names(self):
        """A short name is not a licence, and by-sa resolves elsewhere to by.

        The credit line asks for a link because CC BY-SA section 3(a)(1)(A)(iv)
        does. Leaving CC BY-SA out of the URL table produced a line naming a
        share-alike licence with no way to reach it, which is the failure the
        table exists to prevent.
        """
        line, _ = manifest_repair.attribution_for(self.row("CC BY-SA 3.0"))
        self.assertIn("https://creativecommons.org/licenses/by-sa/3.0/", line)

    def test_a_public_domain_cover_requires_no_attribution(self):
        """The widening must not turn every licence into an obligation.

        CC0 and public domain rows carry a credit line as a courtesy and no
        duty, and a user who cannot tell the two apart gets no value from
        either.
        """
        for licence in ("CC0", "Public domain"):
            with self.subTest(licence=licence):
                _, required = manifest_repair.attribution_for(self.row(licence))
                self.assertFalse(required)


if __name__ == "__main__":
    unittest.main()
