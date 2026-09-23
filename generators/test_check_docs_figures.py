#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the published-figure check.

    python3 -m unittest discover -s generators -p 'test_*.py'

The test that matters is the one where the docs are stale, because that is the
state the corpus was actually in: every figure correct when written, and wrong
by the time it was read.
"""
from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import check_docs_figures as cdf  # noqa: E402
from check_docs_figures import FigureError, check, derive  # noqa: E402


def arm(name: str, samples: int) -> dict:
    return {"arm": name, "samples": samples, "shards": [],
            "rows_in_manifest": samples, "container_mismatches": [],
            "digest_mismatches": [], "mispaired": [], "missing": [],
            "unlicensed": []}


class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = pathlib.Path(self.tmp.name)
        self.docs = root / "docs"
        (self.docs / "guide").mkdir(parents=True)
        self.release = root / "release"
        (self.release / "core").mkdir(parents=True)
        (self.release / "core-arms").mkdir(parents=True)
        self.covers = root / "commons"
        self.covers.mkdir()

        arms = [arm("wow-0200", 100), arm("outguess-0050", 80),
                arm("outguess-0200", 80), arm("steghide-0050", 100),
                arm("clean-grey", 50)]
        (self.release / "core-arms" / "pentimento-core-arms-index.json").write_text(
            json.dumps({"arms": arms, "tier": "Core"}))
        (self.release / "core" / "pentimento-core-index.json").write_text(
            json.dumps({"samples": 200, "tier": "Core"}))

        rows = [{"file": f"{n:05d}.png", "attribution_required": n < 110}
                for n in range(200)]
        (self.covers / "manifest.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in rows))

    def write(self, name: str, text: str) -> None:
        (self.docs / name).write_text(text)

    def run_check(self):
        return check(self.docs, derive(self.release, self.covers))


class DeriveTests(Fixture):
    def test_the_figures_come_from_the_packed_index(self):
        f = derive(self.release, self.covers)
        self.assertEqual(f["stego pairs"], 360)
        self.assertEqual(f["stego arms"], 4)
        self.assertEqual(f["clean arms"], 1)
        self.assertEqual(f["samples per outguess arm"], 80)
        self.assertEqual(f["steghide and outguess samples"], 260)
        self.assertEqual(f["covers requiring attribution"], 110)
        self.assertEqual(f["attribution percent"], 55)

    def test_outguess_arms_that_disagree_refuse_rather_than_pick_one(self):
        """The docs quote ONE number for every outguess arm. If that stops
        being true the page needs rewording, not a number chosen by luck."""
        p = self.release / "core-arms" / "pentimento-core-arms-index.json"
        d = json.loads(p.read_text())
        d["arms"][1]["samples"] = 79
        p.write_text(json.dumps(d))
        with self.assertRaises(FigureError) as cm:
            derive(self.release, self.covers)
        self.assertIn("no longer agree", str(cm.exception))

    def test_a_third_stego_arm_size_refuses_rather_than_passing(self):
        """The defect that shipped, as a test.

        The docs say outguess is "the one short arm". In the released corpus
        twenty-one arms - every spatial adaptive one, plus clean-grey - sat at
        9,882 against everything else's 10,000, so the sentence was wrong and
        no per-arm count was. A reader comparing `wow-0200` with
        `juniward-0200` had 118 covers on one side and not the other.

        Nothing could have caught it: every number in the index was correct,
        and the claim was about the SHAPE of the set rather than any one
        value.
        """
        p = self.release / "core-arms" / "pentimento-core-arms-index.json"
        d = json.loads(p.read_text())
        # wow-0200 drops below the other full arms: a third stego size.
        d["arms"][0]["samples"] = 98
        p.write_text(json.dumps(d))
        with self.assertRaises(FigureError) as cm:
            derive(self.release, self.covers)
        self.assertIn("3 different sample counts", str(cm.exception))

    def test_a_clean_arm_of_its_own_size_is_not_an_error(self):
        """There is one clean arm per distinct clean image set rather than per
        rate, so its count answers a different question and is allowed to
        differ. The fixture's clean-grey is already a third size overall; only
        the STEGO arms are held to two."""
        figures = derive(self.release, self.covers)
        self.assertEqual(figures["clean arms"], 1)


class CheckTests(Fixture):
    def test_docs_that_match_pass(self):
        self.write("index.md", "The corpus holds 360 stego pairs and 55% "
                               "of 110 covers need a credit line.")
        problems, _ = self.run_check()
        self.assertEqual(problems, [])

    def test_the_stale_figure_is_caught(self):
        """The real failure: a number that was right before the rebuild."""
        self.write("index.md", "The corpus holds 344,348 stego pairs.")
        self.write("guide/get-it.md", "The Core tier is 344,348 stego pairs.")
        figures = dict(derive(self.release, self.covers))
        figures["stego pairs"] = 344_000          # near enough to be a candidate
        problems, _ = check(self.docs, figures)
        self.assertTrue(any("stego pairs" in p for p in problems), problems)
        self.assertTrue(any("get-it.md" in p for p in problems), problems)

    def test_a_stale_percentage_is_caught(self):
        self.write("index.md", "54% of the covers require attribution.")
        problems, _ = self.run_check()
        self.assertTrue(any("attribution percent" in p for p in problems),
                        problems)

    def test_a_figure_the_docs_never_state_is_reported_not_passed(self):
        self.write("index.md", "A corpus of photographs.")
        problems, notes = self.run_check()
        self.assertEqual(problems, [])
        self.assertTrue(any("never state it" in n for n in notes), notes)

    def test_docs_with_no_pages_is_a_failure_rather_than_a_pass(self):
        """A check that examined nothing must not report clean."""
        with self.assertRaises(FigureError) as cm:
            check(self.docs, derive(self.release, self.covers))
        self.assertIn("examined nothing", str(cm.exception))

    def test_node_modules_is_not_treated_as_documentation(self):
        (self.docs / "node_modules" / "pkg").mkdir(parents=True)
        (self.docs / "node_modules" / "pkg" / "readme.md").write_text(
            "changelog for 344,348 downloads")
        self.write("index.md", "The corpus holds 360 stego pairs, 55%, 110.")
        problems, _ = self.run_check()
        self.assertEqual(problems, [])


class AmbiguityTests(Fixture):
    """A pattern loose enough to match anything verifies nothing.

    The first version searched for the noun alone, so "360 stego pairs" was
    read as an arm count of 360 and reported as a stale figure. A check that
    cries wolf gets switched off, which leaves the docs unchecked by a
    different route.
    """

    def test_the_pair_count_is_not_mistaken_for_an_arm_count(self):
        self.write("index.md", "The corpus holds 360 stego pairs.")
        problems, _ = self.run_check()
        self.assertFalse([p for p in problems if "stego arms" in p], problems)

    def test_a_bare_digit_is_not_accepted_as_stating_the_arm_count(self):
        """Otherwise any page containing the digit 4 reports the arms as
        verified, which is the worst outcome: a clean line proving nothing."""
        self.write("index.md", "Version 4 of the pipeline, 1 clean run.")
        _, notes = self.run_check()
        self.assertTrue(any("stego arms" in n and "never state it" in n
                            for n in notes), notes)

    def test_a_stale_arms_line_is_caught(self):
        self.write("index.md", "Arms: 34 stego, plus 5 clean.")
        problems, _ = self.run_check()
        self.assertTrue(any("stego arms" in p for p in problems), problems)

    def test_the_correct_arms_line_passes(self):
        self.write("index.md", "Arms: 4 stego, plus 1 clean.")
        problems, notes = self.run_check()
        self.assertEqual([p for p in problems if "stego arms" in p], [])
        self.assertTrue(any("stego arms" in n and "as shipped" in n
                            for n in notes), notes)


class FalsePositiveTests(Fixture):
    """Both of these fired against the real docs on the first run.

    Neither was a stale figure. A payload rate of "50% of capacity" was read as
    an attribution percentage, and a train/test split of 8,032 covers was read
    as an outguess sample count, because both were the right kind of number
    within the right magnitude. A candidate now has to appear on a line that is
    talking about the figure.
    """

    def test_a_payload_rate_is_not_an_attribution_percentage(self):
        self.write("index.md", "55% of covers require a credit line.")
        self.write("guide/whats-in-it.md",
                   "| End-user tools | steghide, outguess | 5%, 20%, "
                   "50% of capacity | JPEG |")
        problems, _ = self.run_check()
        self.assertEqual([p for p in problems if "attribution percent" in p],
                         [], problems)

    def test_a_train_test_split_is_not_an_outguess_count(self):
        self.write("guide/using-it.md",
                   "| What it is | A fixed train / test label, 79 and 21 "
                   "covers | A recipe |")
        self.write("index.md", "outguess fills 80 of them.")
        problems, _ = self.run_check()
        self.assertEqual(
            [p for p in problems if "outguess arm" in p], [], problems)

    def test_a_genuinely_stale_outguess_count_still_fails(self):
        """The scoping must not be so tight that nothing is checked."""
        self.write("index.md", "Samples per outguess arm: 79, not 200.")
        problems, _ = self.run_check()
        self.assertTrue(any("outguess arm" in p for p in problems), problems)


    def test_a_generated_credit_list_is_not_scanned(self):
        """Found against the shipped release: ATTRIBUTION.md carries a
        photograph of a bus numbered 10040, which read as a cover count."""
        self.write("index.md", "200 covers, 55%, 110 need a credit line.")
        (self.docs / "ATTRIBUTION.md").write_text(
            "- `09772.png` \"File:Bus 199 on route U1\", by Someone, CC BY 2.0")
        problems, _ = self.run_check()
        self.assertEqual([p for p in problems if "covers" in p], [], problems)


    def test_a_private_working_note_is_not_scanned(self):
        """`private/` is gitignored fleet-wide and never published, so a stale
        figure in a working note is not a claim to any reader."""
        self.write("index.md", "200 covers, 55%, 110 need a credit line.")
        (self.docs / "private").mkdir()
        (self.docs / "private" / "notes.md").write_text("we had 344,348 pairs")
        problems, _ = self.run_check()
        self.assertEqual(problems, [], problems)


class EndToEndTests(Fixture):
    def test_main_returns_zero_when_the_docs_agree(self):
        self.write("index.md", "360 stego pairs, 110 covers, 55%.")
        self.assertEqual(cdf.main([
            "--docs", str(self.docs), "--release", str(self.release),
            "--covers", str(self.covers)]), 0)

    def test_main_returns_one_when_they_do_not(self):
        self.write("index.md", "54% of covers need attribution.")
        self.assertEqual(cdf.main([
            "--docs", str(self.docs), "--release", str(self.release),
            "--covers", str(self.covers)]), 1)


if __name__ == "__main__":
    unittest.main()
