#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the registry vendoring step.

The copy exists so a crate published on its own carries a working registry.
A copy is a second source of truth, so the tests that matter here are the ones
about drift: that `--check` notices every way the two can disagree, and that a
deleted file cannot survive in the copy.
"""

import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import vendor_registry as vr  # noqa: E402


class Tree:
    def __init__(self, files):
        self.dir = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.dir.name)
        self.source = self.root / "registry"
        self.dest = self.root / "vendored"
        for name, text in files.items():
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")

    def run(self, *extra):
        return vr.main(["--source", str(self.source), "--dest", str(self.dest), *extra])

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.dir.cleanup()
        return False


ENTRY = 'name = "zsteg"\n'


class VendorTests(unittest.TestCase):
    def test_a_copy_is_written_and_then_matches(self):
        with Tree({"detectors/zsteg.toml": ENTRY, "corpora/x.toml": "a = 1\n"}) as t:
            self.assertEqual(t.run(), 0)
            self.assertEqual(t.run("--check"), 0)
            self.assertTrue((t.dest / "detectors" / "zsteg.toml").is_file())

    def test_a_changed_file_is_caught(self):
        with Tree({"detectors/zsteg.toml": ENTRY}) as t:
            t.run()
            (t.source / "detectors" / "zsteg.toml").write_text("name = \"other\"\n")
            self.assertEqual(t.run("--check"), 1)

    def test_a_new_registry_file_missing_from_the_copy_is_caught(self):
        with Tree({"detectors/zsteg.toml": ENTRY}) as t:
            t.run()
            (t.source / "detectors" / "new.toml").write_text(ENTRY)
            self.assertEqual(t.run("--check"), 1)

    def test_a_file_deleted_from_the_registry_cannot_survive_in_the_copy(self):
        # The copy is written fresh rather than merged, so a retired entry
        # cannot go on being published after it was removed.
        with Tree({"detectors/zsteg.toml": ENTRY, "detectors/old.toml": ENTRY}) as t:
            t.run()
            (t.source / "detectors" / "old.toml").unlink()
            self.assertEqual(t.run("--check"), 1)
            t.run()
            self.assertFalse((t.dest / "detectors" / "old.toml").exists())
            self.assertEqual(t.run("--check"), 0)

    def test_only_toml_is_copied(self):
        # A stray file in the registry tree must not ride along into a
        # published crate.
        with Tree({"detectors/zsteg.toml": ENTRY, "notes.md": "scratch\n"}) as t:
            t.run()
            self.assertFalse((t.dest / "notes.md").exists())

    def test_an_empty_registry_is_refused_rather_than_vendored(self):
        # Vendoring nothing produces exactly the silent defect this script
        # exists to close: a published crate whose built-in registry is empty.
        with Tree({}) as t:
            self.assertEqual(t.run(), 2)

    def test_a_missing_registry_is_a_usage_error(self):
        with Tree({"detectors/zsteg.toml": ENTRY}) as t:
            self.assertEqual(
                vr.main(["--source", str(t.root / "nope"), "--dest", str(t.dest)]), 2)

    def test_an_oversized_registry_is_refused_with_the_cause_named(self):
        with Tree({"detectors/big.toml": "x" * (vr.MAX_TOTAL_BYTES + 1)}) as t:
            self.assertEqual(t.run(), 2)

    def test_checking_before_anything_is_vendored_fails_rather_than_passing(self):
        with Tree({"detectors/zsteg.toml": ENTRY}) as t:
            self.assertEqual(t.run("--check"), 1)


class ShippedRegistryTests(unittest.TestCase):
    def test_the_real_registry_is_small_enough_to_vendor(self):
        source = vr.files(vr.SOURCE)
        self.assertGreater(len(source), 10, "the registry looks empty")
        total = sum(p.stat().st_size for p in source.values())
        self.assertLess(total, vr.MAX_TOTAL_BYTES)

    def test_the_vendored_copy_matches_when_it_exists(self):
        # Skipped rather than failed while the copy is a release-time step and
        # not a committed directory. When it becomes committed, this starts
        # holding the two together on every run.
        if not vr.VENDORED.is_dir():
            self.skipTest("no vendored copy yet; it is written at release time")
        self.assertEqual(
            vr.differences(vr.files(vr.SOURCE), vr.files(vr.VENDORED)), [])


if __name__ == "__main__":
    unittest.main()
