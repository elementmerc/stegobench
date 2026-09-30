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


if __name__ == "__main__":
    unittest.main()
