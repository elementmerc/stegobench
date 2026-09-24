#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the Kaggle starter notebook generator.

    python3 -m unittest discover -s tools/release -p 'test_*.py'

The notebook is prose with a run button, and it quotes the corpus back at the
reader. So the thing worth testing is that every figure in it came from the
packed release rather than from a constant somebody typed: a notebook carrying
"5,429 covers" over a corpus of 5,453 is the exact failure this generator
exists to prevent, and it would look completely fine.

The second thing worth testing is that the notebook is a notebook. A malformed
`.ipynb` is rejected by Kaggle at push time, which is after the release has
gone out, and the error it gives back is not helpful.
"""
from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import kaggle_notebook  # noqa: E402


def packed_release(root: pathlib.Path, *, tier: str = "Core", covers: int = 10000,
                   shards: int = 10, required: int = 5453,
                   pct: float = 54.5,
                   slug: str = "elementmerc/pentimento-core") -> pathlib.Path:
    """A packed release holding only what the generator reads."""
    root.mkdir(parents=True, exist_ok=True)
    (root / "licence-summary.json").write_text(json.dumps({
        "total": covers,
        "attribution_required": required,
        "attribution_required_pct": pct,
    }), encoding="utf-8")
    (root / f"pentimento-{tier.lower()}-index.json").write_text(json.dumps({
        "tier": tier,
        "samples": covers,
        # `shard`, which is the key pack_tier.py actually writes. The notebook
        # only counts these, so a wrong key would pass here and mislead the
        # next person who copied this fixture for something that reads it.
        "shards": [{"shard": f"pentimento-{tier.lower()}-{n:05d}.tar"}
                   for n in range(shards)],
    }), encoding="utf-8")
    (root / "dataset-metadata.json").write_text(json.dumps({
        "id": slug, "title": "Pentimento Core",
    }), encoding="utf-8")
    return root


def sources(notebook: dict) -> str:
    """Every cell's text, joined, for asking what the notebook says."""
    return "\n".join("".join(cell["source"]) for cell in notebook["cells"])


class LineSplittingTests(unittest.TestCase):
    """A notebook source is a list of lines, each keeping its newline.

    Getting this wrong produces a file that opens and renders as one
    unbroken line, which no test of the JSON structure would catch.
    """

    def test_a_single_line_keeps_no_newline(self):
        self.assertEqual(kaggle_notebook._lines(["one"]), ["one"])

    def test_every_line_but_the_last_keeps_its_newline(self):
        self.assertEqual(kaggle_notebook._lines(["one", "two", "three"]),
                         ["one\n", "two\n", "three"])

    def test_a_blank_line_survives(self):
        self.assertEqual(kaggle_notebook._lines(["one", "", "two"]),
                         ["one\n", "\n", "two"])

    def test_the_text_round_trips(self):
        body = "a\n\nb\nc"
        self.assertEqual("".join(kaggle_notebook._lines(body.split("\n"))), body)


class CellTests(unittest.TestCase):
    def test_every_cell_gets_an_id(self):
        # nbformat warns about a missing id today and says it becomes an
        # error, so a notebook without them would stop running on a future
        # Kaggle image with nothing in this repository having changed.
        first = kaggle_notebook.code("x = 1")
        second = kaggle_notebook.text("hello")
        self.assertTrue(first["id"])
        self.assertTrue(second["id"])
        self.assertNotEqual(first["id"], second["id"])

    def test_a_code_cell_carries_the_fields_nbformat_requires(self):
        cell = kaggle_notebook.code("x = 1")
        self.assertEqual(cell["cell_type"], "code")
        self.assertEqual(cell["outputs"], [])
        self.assertIsNone(cell["execution_count"])


class FiguresComeFromTheReleaseTests(unittest.TestCase):
    """The whole reason this is a generator rather than a checked-in file."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)

    def test_the_cover_count_is_the_one_in_the_index(self):
        packed = packed_release(self.root / "core", covers=1234)
        text = sources(kaggle_notebook.build(packed))
        self.assertIn("1,234", text)

    def test_the_attribution_figures_are_the_ones_in_the_summary(self):
        packed = packed_release(self.root / "core", covers=1234,
                                required=567, pct=45.9)
        text = sources(kaggle_notebook.build(packed))
        self.assertIn("567 of 1,234 covers (45.9%)", text)

    def test_the_shard_count_is_the_one_in_the_index(self):
        # The count, not the sentence around it. This used to assert "3 tar
        # shards", which matched only because the prose said the shards were
        # folders, so the wording and the figure were pinned by one string and
        # correcting the wording looked like breaking the count.
        packed = packed_release(self.root / "core", shards=3)
        text = sources(kaggle_notebook.build(packed))
        self.assertIn("3 shards", text)

    def test_the_notebook_opens_the_shard_under_the_name_kaggle_carries(self):
        # It pointed at `pentimento-core-00000`, a directory that existed only
        # while Kaggle was unpacking the shards. Every cell after the first
        # failed for anybody who ran it, and the live notebook had to be
        # patched by hand.
        packed = packed_release(self.root / "core")
        text = sources(kaggle_notebook.build(packed))
        self.assertIn("pentimento-core-00000.tar.bin", text)
        self.assertNotIn("'pentimento-core-00000')", text)

    def test_the_notebook_does_not_tell_a_reader_the_shards_are_folders(self):
        # True until 2026-09-24 and false since. A starter notebook is read
        # more carefully than the card, so a stale explanation here costs more
        # than one anywhere else on the page.
        packed = packed_release(self.root / "core")
        text = sources(kaggle_notebook.build(packed))
        self.assertNotIn("are **folders**", text)
        self.assertNotIn("the container is gone", text)

    def test_a_different_tier_names_its_own_shards(self):
        # Nano and Lite ship the same structure under a different prefix, and
        # a notebook opening `pentimento-core-00000` on the Nano page would
        # fail at the first cell it asks the reader to run.
        packed = packed_release(self.root / "nano", tier="Nano", covers=200)
        text = sources(kaggle_notebook.build(packed))
        self.assertIn("pentimento-nano-00000", text)
        self.assertNotIn("pentimento-core-00000", text)

    def test_the_bossbase_caution_is_present(self):
        # It is the one claim the corpus documentation leads with everywhere
        # else, and a starter notebook is the page most likely to be read
        # instead of the card rather than beside it.
        packed = packed_release(self.root / "core")
        self.assertIn("BOSSbase", sources(kaggle_notebook.build(packed)))

    def test_the_split_warning_is_present(self):
        packed = packed_release(self.root / "core")
        text = sources(kaggle_notebook.build(packed))
        self.assertIn("source_png", text)
        self.assertIn("Split by cover", text)

    def test_a_release_missing_its_index_says_so_and_says_what_writes_it(self):
        packed = self.root / "bare"
        packed.mkdir()
        (packed / "licence-summary.json").write_text(
            json.dumps({"total": 1, "attribution_required": 0,
                        "attribution_required_pct": 0.0}), encoding="utf-8")
        with self.assertRaises(kaggle_notebook.ReleaseIncomplete) as caught:
            kaggle_notebook.build(packed)
        self.assertIn("pack_tier.py", str(caught.exception))

    def test_a_release_missing_its_licence_summary_names_the_file(self):
        packed = packed_release(self.root / "core")
        (packed / "licence-summary.json").unlink()
        with self.assertRaises(kaggle_notebook.ReleaseIncomplete) as caught:
            kaggle_notebook.build(packed)
        self.assertIn("licence-summary.json", str(caught.exception))

    def test_two_tiers_in_one_directory_are_refused(self):
        # Whichever sorted first would win silently, and the notebook would
        # quote one tier's cover count over another tier's shard names.
        packed = packed_release(self.root / "core")
        (packed / "pentimento-nano-index.json").write_text(
            json.dumps({"tier": "Nano", "samples": 200, "shards": []}),
            encoding="utf-8")
        with self.assertRaises(kaggle_notebook.ReleaseIncomplete) as caught:
            kaggle_notebook.build(packed)
        self.assertIn("2 pack indexes", str(caught.exception))


class KernelTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.out = self.root / "kernel"

    def run_main(self, packed, *extra):
        return kaggle_notebook.main(
            ["--packed", str(packed), "--out", str(self.out), *extra])

    def test_it_writes_a_notebook_that_parses_as_one(self):
        packed = packed_release(self.root / "core")
        self.assertEqual(self.run_main(packed), 0)
        notebook = json.loads(
            (self.out / "pentimento-first-look.ipynb").read_text(encoding="utf-8"))
        self.assertEqual(notebook["nbformat"], 4)
        self.assertTrue(notebook["cells"])
        for cell in notebook["cells"]:
            self.assertIn(cell["cell_type"], {"code", "markdown"})
            self.assertTrue(cell["id"])

    def test_the_kernel_attaches_to_the_dataset_in_the_card(self):
        # A notebook naming the wrong slug runs against nothing, and the
        # failure appears only when somebody opens it on Kaggle.
        packed = packed_release(self.root / "core", slug="someone/pentimento-lite")
        self.assertEqual(self.run_main(packed), 0)
        meta = json.loads((self.out / "kernel-metadata.json").read_text(encoding="utf-8"))
        self.assertEqual(meta["dataset_sources"], ["someone/pentimento-lite"])
        self.assertTrue(meta["id"].startswith("someone/"))

    def test_the_kernel_id_is_the_slug_of_its_own_title(self):
        # Kaggle slugifies the title, warns when the id disagrees, and then
        # publishes under the title's slug, which leaves the tool talking
        # about one URL and the platform serving another.
        packed = packed_release(self.root / "core")
        self.assertEqual(self.run_main(packed, "--title", "Pentimento Core: First Look!"), 0)
        meta = json.loads((self.out / "kernel-metadata.json").read_text(encoding="utf-8"))
        self.assertEqual(meta["id"], "elementmerc/pentimento-core-first-look")

    def test_the_notebook_does_not_ask_kaggle_for_the_internet(self):
        # It reads an attached dataset and nothing else. A kernel that asks
        # for network access invites the question of what it is contacting.
        packed = packed_release(self.root / "core")
        self.assertEqual(self.run_main(packed), 0)
        meta = json.loads((self.out / "kernel-metadata.json").read_text(encoding="utf-8"))
        self.assertEqual(meta["enable_internet"], "false")
        self.assertEqual(meta["enable_gpu"], "false")

    def test_a_missing_release_is_refused(self):
        self.assertEqual(self.run_main(self.root / "nothing-here"), 1)

    def test_an_incomplete_release_is_reported_rather_than_traced(self):
        packed = packed_release(self.root / "core")
        (packed / "pentimento-core-index.json").unlink()
        self.assertEqual(self.run_main(packed), 1)

    def test_a_release_with_no_card_is_refused(self):
        # The card is where the slug lives, and a notebook cannot attach to a
        # dataset it cannot name.
        packed = packed_release(self.root / "core")
        (packed / "dataset-metadata.json").unlink()
        self.assertEqual(self.run_main(packed), 1)


if __name__ == "__main__":
    unittest.main()
