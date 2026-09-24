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
        # The SOURCE is a .tar and the STAGED name is not, because Kaggle
        # unpacks .tar and nothing else.
        self.assertIn("pentimento-core-00000.tar", prepared.shards)
        self.assertIn("pentimento-core-00000.tar.bin", prepared.staged)
        self.assertNotIn("pentimento-core-00000.tar", prepared.staged)

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
        # The checksum file is rewritten rather than linked, because its
        # contents name the files a reader checks.
        self.assertEqual(set(how.values()), {"link", "rewritten"})
        shard = "pentimento-core-00000.tar"
        self.assertEqual((self.staging / (shard + ".bin")).stat().st_ino,
                         (self.packed / shard).stat().st_ino)

    def test_staging_falls_back_to_copying_when_linking_fails(self):
        def refuse(*_args, **_kwargs):
            raise OSError(18, "Invalid cross-device link")

        real_link, os.link = os.link, refuse
        self.addCleanup(lambda: setattr(os, "link", real_link))
        how = kaggle_stage.stage(self.packed, self.staging)
        self.assertEqual(set(how.values()), {"copy", "rewritten"})
        shard = "pentimento-core-00000.tar"
        self.assertEqual((self.staging / (shard + ".bin")).read_bytes(),
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


class TheShardsKaggleWouldUnpack(unittest.TestCase):
    """Kaggle extracts `.tar` and nothing else, so the shards land renamed.

    The failure being prevented is not hypothetical and not small. Ten shards
    became 20,014 loose files, Kaggle's own file listing then returned HTTP 500
    partway through enumerating them, and the Data Card stopped rendering
    entirely: a visitor was told the corpus was inaccessible while every byte
    of it was fine. It was repaired by hand, and the toolchain did not know,
    so the next sanctioned publish would have undone the repair silently.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.packed = packed_release(self.root / "core")
        self.staging = self.root / "stage"

    def checksums(self, body: str) -> None:
        (self.packed / "SHA256SUMS-covers").write_text(body, encoding="utf-8")

    def test_a_shard_lands_under_a_name_kaggle_leaves_alone(self):
        kaggle_stage.stage(self.packed, self.staging)
        self.assertTrue((self.staging / "pentimento-core-00000.tar.bin").is_file())
        self.assertFalse((self.staging / "pentimento-core-00000.tar").exists())

    def test_nothing_but_a_tar_is_renamed(self):
        # Renaming anything else would be cost with no purchase, and a reader
        # has to recognise what they downloaded.
        kaggle_stage.stage(self.packed, self.staging)
        for name in ("README.md", "CITATION.cff", "pentimento-core-index.json",
                     "dataset-metadata.json"):
            self.assertTrue((self.staging / name).is_file(), name)

    def test_the_checksum_file_names_the_shards_as_they_arrive(self):
        # The defect this closes: the shipped file named containers that were
        # not on that mirror, so `sha256sum -c`, which the README gives as the
        # FIRST thing to run, reported every shard missing on a download that
        # was completely intact.
        self.checksums("a" * 64 + "  pentimento-core-00000.tar\n"
                       + "b" * 64 + "  README.md\n")
        kaggle_stage.stage(self.packed, self.staging)
        body = (self.staging / "SHA256SUMS-covers").read_text(encoding="utf-8")
        self.assertIn("a" * 64 + "  pentimento-core-00000.tar.bin", body)
        self.assertNotIn("  pentimento-core-00000.tar\n", body)
        self.assertIn("b" * 64 + "  README.md", body)

    def test_the_digests_are_not_touched_by_the_rename(self):
        # The bytes of a shard are the same whatever it is called, so a
        # rewritten line is the same claim said in a different name. A rename
        # that altered a digest would be silent corruption of the one file
        # whose job is detecting corruption.
        self.checksums("c" * 64 + "  pentimento-core-00000.tar\n")
        kaggle_stage.stage(self.packed, self.staging)
        body = (self.staging / "SHA256SUMS-covers").read_text(encoding="utf-8")
        self.assertEqual(body.split("  ")[0], "c" * 64)

    def test_a_line_naming_something_this_mirror_does_not_carry_is_left_alone(self):
        # The arms checksum file names shards a covers-only publish never
        # sends. Inventing a .tar.bin for one of those would assert something
        # about a file nobody can download here.
        self.checksums("d" * 64 + "  pentimento-core-arms-00000.tar\n")
        kaggle_stage.stage(self.packed, self.staging)
        body = (self.staging / "SHA256SUMS-covers").read_text(encoding="utf-8")
        self.assertIn("  pentimento-core-arms-00000.tar\n", body)
        self.assertNotIn(".tar.bin", body)

    def test_the_report_says_the_shards_were_renamed(self):
        # An operator who never sees the rename happen cannot notice it
        # stopping, and stopping is what broke the dataset last time.
        prepared = kaggle_stage.plan(self.packed)
        text = kaggle_stage.describe(prepared)
        self.assertIn("renamed to *.tar.bin", text)
        self.assertIn("unpacks .tar", text)
        # A bare `assertIn(".tar.bin")` passed while the line actually read
        # `*.tar.tar.bin`, because the suffix constant means the whole
        # extension in one module and only the added part in the other.
        self.assertNotIn(".tar.tar", text)

    def test_a_staged_shard_still_opens_as_a_tar(self):
        # The rename is safe only because `tarfile` sniffs content rather than
        # trusting the extension. If that stopped being true, every reader
        # following the published instructions would be stuck, so it is
        # asserted rather than assumed.
        import tarfile

        real = self.packed / "pentimento-core-00000.tar"
        with tarfile.open(real, "w") as tar:
            payload = b"\x89PNG\r\n\x1a\n" + bytes(16)
            info = tarfile.TarInfo("00000.png")
            info.size = len(payload)
            tar.addfile(info, io.BytesIO(payload))

        kaggle_stage.stage(self.packed, self.staging)
        landed = self.staging / "pentimento-core-00000.tar.bin"
        with tarfile.open(landed) as tar:
            self.assertEqual([m.name for m in tar], ["00000.png"])

    def test_the_shard_the_notebook_opens_is_a_file_the_staging_carries(self):
        # The two are generated by different programs and published together,
        # so nothing but a test holds them to the same name. The live notebook
        # pointed at a directory that no longer existed, and every cell after
        # the first failed for anyone who ran it.
        import kaggle_notebook

        # The shared fixture writes placeholder bodies, and the notebook reads
        # these two for real figures, so they are written properly here.
        (self.packed / "licence-summary.json").write_text(json.dumps(
            {"total": 10, "attribution_required": 5,
             "attribution_required_pct": 50.0}), encoding="utf-8")
        index = json.loads((self.packed / "pentimento-core-index.json")
                           .read_text(encoding="utf-8"))
        index["samples"] = 10
        (self.packed / "pentimento-core-index.json").write_text(
            json.dumps(index), encoding="utf-8")

        notebook = kaggle_notebook.build(self.packed)
        source = "\n".join("".join(c["source"]) for c in notebook["cells"])
        staged = set(kaggle_stage.plan(self.packed).staged)
        named = [n for n in staged if n.startswith("pentimento-core-000")]
        self.assertTrue(named, "the staging carried no shard to name")
        self.assertTrue(
            any(n in source for n in named),
            f"the notebook names none of the staged shards {sorted(named)}")


if __name__ == "__main__":
    unittest.main()
