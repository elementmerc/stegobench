#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the Kaggle staging set.

    python3 -m unittest discover -s tools/release -p 'test_*.py'

What is worth testing here is what goes public and what does not. Kaggle
versions a whole directory with no exclude flag, so a file that reaches the
staging directory reaches the world, and a file that quietly does not reach
it is missing from a corpus people are told is complete. Both failures have
happened on this project, in that order, so both have tests.
"""
from __future__ import annotations

import contextlib
import io
import json
import os
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import kaggle_stage  # noqa: E402


def packed_release(root: pathlib.Path, *, tier: str = "core", shards: int = 3,
                   extras: tuple[str, ...] | None = None,
                   arms: bool = False) -> pathlib.Path:
    """A packed release holding a published set, both control files and junk."""
    root.mkdir(parents=True, exist_ok=True)
    names = [f"pentimento-{tier}-{n:05d}.tar" for n in range(shards)]
    for name in names:
        (root / name).write_bytes(b"tar bytes for " + name.encode())

    index: dict = {"tier": tier,
                   "shards": [{"shard": n, "sha256": "0" * 64} for n in names]}
    if arms:
        arm_shard = f"pentimento-{tier}-arm-00000.tar"
        (root / arm_shard).write_bytes(b"arm shard")
        index["arms"] = [{"arm": "lsb",
                          "shards": [{"shard": arm_shard, "sha256": "0" * 64}]}]
    (root / f"pentimento-{tier}-index.json").write_text(
        json.dumps(index), encoding="utf-8")

    wanted = kaggle_stage.PUBLISHED_EXTRAS if extras is None else extras
    for name in wanted:
        (root / name).write_text(f"{name} body\n", encoding="utf-8")

    (root / "dataset-metadata.json").write_text(
        json.dumps({"id": "elementmerc/pentimento-core"}), encoding="utf-8")
    (root / "ia-metadata.json").write_text(
        json.dumps({"x-archive-meta-title": "Pentimento"}), encoding="utf-8")
    return root


class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.packed = packed_release(self.root / "core")
        self.staging = self.root / "stage"


class TestPlan(Fixture):
    def test_the_staged_set_is_shards_index_and_published_extras(self):
        prepared = kaggle_stage.plan(self.packed)
        self.assertEqual(len(prepared.shards), 3)
        self.assertEqual(prepared.indexes, ("pentimento-core-index.json",))
        self.assertEqual(set(prepared.extras), set(kaggle_stage.PUBLISHED_EXTRAS))
        for name in kaggle_stage.PUBLISHED_EXTRAS:
            self.assertIn(name, prepared.staged)
        self.assertIn("pentimento-core-00000.tar", prepared.staged)

    def test_arm_shards_named_by_the_index_are_staged_too(self):
        packed = packed_release(self.root / "arms", arms=True)
        prepared = kaggle_stage.plan(packed)
        self.assertIn("pentimento-core-arm-00000.tar", prepared.shards)

    def test_the_archive_control_file_is_never_staged(self):
        prepared = kaggle_stage.plan(self.packed)
        self.assertNotIn("ia-metadata.json", prepared.staged)
        self.assertIn("ia-metadata.json", prepared.excluded)

    def test_the_kaggle_control_file_is_staged_but_is_not_published(self):
        """Kaggle reads it out of the directory and skips uploading it."""
        prepared = kaggle_stage.plan(self.packed)
        self.assertIn("dataset-metadata.json", prepared.staged)
        self.assertNotIn("dataset-metadata.json", prepared.published)

    def test_a_missing_published_file_is_refused(self):
        (self.packed / "DATASHEET.md").unlink()
        with self.assertRaises(kaggle_stage.StagingRefused) as caught:
            kaggle_stage.plan(self.packed)
        self.assertIn("DATASHEET.md", str(caught.exception))

    def test_a_missing_shard_is_refused(self):
        (self.packed / "pentimento-core-00001.tar").unlink()
        with self.assertRaises(kaggle_stage.StagingRefused) as caught:
            kaggle_stage.plan(self.packed)
        self.assertIn("pentimento-core-00001.tar", str(caught.exception))

    def test_a_missing_arms_checksum_is_allowed_and_reported(self):
        """A covers-only release writes no arms checksum, and that is fine."""
        (self.packed / "SHA256SUMS-arms").unlink()
        prepared = kaggle_stage.plan(self.packed)
        self.assertEqual(prepared.absent_optional, ("SHA256SUMS-arms",))
        self.assertIn("SHA256SUMS-arms", kaggle_stage.describe(prepared))

    def test_a_missing_kaggle_control_file_is_refused(self):
        (self.packed / "dataset-metadata.json").unlink()
        with self.assertRaises(kaggle_stage.StagingRefused) as caught:
            kaggle_stage.plan(self.packed)
        self.assertIn("dataset-metadata.json", str(caught.exception))

    def test_a_release_with_no_index_is_refused(self):
        (self.packed / "pentimento-core-index.json").unlink()
        with self.assertRaises(kaggle_stage.StagingRefused):
            kaggle_stage.plan(self.packed)

    def test_an_unreadable_index_is_refused(self):
        (self.packed / "pentimento-core-index.json").write_text(
            "{not json", encoding="utf-8")
        with self.assertRaises(kaggle_stage.StagingRefused):
            kaggle_stage.plan(self.packed)

    def test_an_index_naming_a_path_rather_than_a_file_is_refused(self):
        (self.packed / "pentimento-core-index.json").write_text(json.dumps(
            {"shards": [{"shard": "../../escape.tar"}]}), encoding="utf-8")
        with self.assertRaises(kaggle_stage.StagingRefused) as caught:
            kaggle_stage.plan(self.packed)
        self.assertIn("escape.tar", str(caught.exception))

    def test_an_index_with_no_shard_key_is_refused(self):
        (self.packed / "pentimento-core-index.json").write_text(json.dumps(
            {"shards": [{"name": "pentimento-core-00000.tar"}]}), encoding="utf-8")
        with self.assertRaises(kaggle_stage.StagingRefused):
            kaggle_stage.plan(self.packed)

    def test_a_missing_release_directory_is_refused(self):
        with self.assertRaises(kaggle_stage.StagingRefused):
            kaggle_stage.plan(self.root / "nowhere")


class TestReport(Fixture):
    def test_every_left_behind_file_is_named_not_merely_counted(self):
        (self.packed / "scratch.txt").write_text("notes", encoding="utf-8")
        (self.packed / "workdir").mkdir()
        prepared = kaggle_stage.plan(self.packed)
        text = kaggle_stage.describe(prepared)
        self.assertIn("scratch.txt", text)
        self.assertIn("workdir/", text)
        self.assertIn("ia-metadata.json", text)

    def test_the_report_says_whether_files_were_linked_or_copied(self):
        prepared = kaggle_stage.plan(self.packed)
        how = kaggle_stage.stage(self.packed, self.staging, prepared)
        text = kaggle_stage.describe(prepared, how)
        self.assertRegex(text, r"\d+ file\(s\) hard linked, \d+ copied")


class TestStage(Fixture):
    def test_staging_puts_exactly_the_planned_set_on_disk(self):
        prepared = kaggle_stage.plan(self.packed)
        kaggle_stage.stage(self.packed, self.staging, prepared)
        self.assertEqual(sorted(p.name for p in self.staging.iterdir()),
                         sorted(prepared.staged))
        self.assertFalse((self.staging / "ia-metadata.json").exists())

    def test_staging_hard_links_rather_than_copying_on_one_filesystem(self):
        how = kaggle_stage.stage(self.packed, self.staging)
        self.assertEqual(set(how.values()), {"link"})
        shard = "pentimento-core-00000.tar"
        self.assertEqual((self.staging / shard).stat().st_ino,
                         (self.packed / shard).stat().st_ino)

    def test_staging_falls_back_to_copying_when_linking_fails(self):
        def refuse(*_args, **_kwargs):
            raise OSError(18, "Invalid cross-device link")

        real_link, os.link = os.link, refuse
        self.addCleanup(lambda: setattr(os, "link", real_link))
        how = kaggle_stage.stage(self.packed, self.staging)
        self.assertEqual(set(how.values()), {"copy"})
        shard = "pentimento-core-00000.tar"
        self.assertEqual((self.staging / shard).read_bytes(),
                         (self.packed / shard).read_bytes())

    def test_staging_twice_leaves_the_same_directory(self):
        first = kaggle_stage.stage(self.packed, self.staging)
        before = sorted(p.name for p in self.staging.iterdir())
        second = kaggle_stage.stage(self.packed, self.staging)
        self.assertEqual(first, second)
        self.assertEqual(before, sorted(p.name for p in self.staging.iterdir()))

    def test_a_leftover_from_an_earlier_run_is_removed_rather_than_published(self):
        self.staging.mkdir()
        (self.staging / "ia-metadata.json").write_text("stale", encoding="utf-8")
        (self.staging / "old-dir").mkdir()
        kaggle_stage.stage(self.packed, self.staging)
        self.assertFalse((self.staging / "ia-metadata.json").exists())
        self.assertFalse((self.staging / "old-dir").exists())

    def test_nothing_is_staged_when_the_set_is_incomplete(self):
        (self.packed / "README.md").unlink()
        with self.assertRaises(kaggle_stage.StagingRefused):
            kaggle_stage.stage(self.packed, self.staging)
        self.assertFalse(self.staging.exists())


class TestCommandLine(Fixture):
    def test_the_dry_run_reports_both_sets_and_stages_nothing(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = kaggle_stage.main(["--packed", str(self.packed), "--dry-run"])
        text = out.getvalue()
        self.assertEqual(code, 0)
        self.assertIn("README.md", text)
        self.assertIn("ia-metadata.json", text)
        self.assertFalse(self.staging.exists())

    def test_a_refusal_exits_non_zero(self):
        (self.packed / "CITATION.cff").unlink()
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            code = kaggle_stage.main(
                ["--packed", str(self.packed), "--stage", str(self.staging)])
        self.assertEqual(code, 1)
        self.assertIn("CITATION.cff", err.getvalue())

    def test_staging_without_a_destination_is_rejected(self):
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                kaggle_stage.main(["--packed", str(self.packed)])


if __name__ == "__main__":
    unittest.main()
