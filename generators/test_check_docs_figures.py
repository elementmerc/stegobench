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
import unittest.mock

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


class RepoFixture(Fixture):
    """The harness repository beside the corpus site.

    The corpus site was checked from the first day and the repository that
    ships the checker was not, so its README carried 344,348 pairs against a
    real 344,357 for days: the one document out of reach kept the one number
    the checker's own header names as the wrong one.

    The manifest is widened to 2,000 covers here so a derived figure lands in
    the range a year occupies. Without one, nothing in the fixture could tell
    a count from a date, which is a collision the real prose produced twice.
    """

    def setUp(self):
        super().setUp()
        self.repo = pathlib.Path(self.tmp.name) / "harness"
        (self.repo / "docs" / "design").mkdir(parents=True)
        # The corpus site still has to hold a page, or the check refuses
        # before it reaches the repository at all.
        self.write("index.md", "A corpus of photographs.")

        rows = [{"file": f"{n:05d}.png", "tier_order": n,
                 "attribution_required": n < 1090,
                 "split": "train" if n < 1600 else "test",
                 "licence": "Public domain" if n < 1922 else "CC0"}
                for n in range(2000)]
        (self.covers / "manifest.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in rows))
        (self.release / "core" / "pentimento-core-index.json").write_text(
            json.dumps({"samples": 2000, "tier": "Core"}))

    def repo_write(self, name: str, text: str) -> None:
        p = self.repo / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)

    def run_check(self):
        return check(self.docs, derive(self.release, self.covers), self.repo)


class RepoTests(RepoFixture):
    def test_a_stale_pair_count_in_the_harness_readme_fails_and_names_it(self):
        """The defect that shipped. A count alone is unactionable when the fix
        is editing a document, so the finding has to carry the file, the
        figure, the value found and the value expected."""
        self.repo_write("README.md", "The first corpus tier is complete: "
                                     "4 stego arms and 1 clean one, 352 pairs.")
        problems, _ = self.run_check()
        self.assertEqual(len(problems), 1, problems)
        for part in ("README.md", "stego pairs", "352", "360"):
            self.assertIn(part, problems[0])

    def test_a_correct_harness_readme_passes_and_says_it_was_compared(self):
        self.repo_write("README.md", "The first corpus tier is complete: "
                                     "4 stego arms and 1 clean one, 360 pairs.")
        problems, notes = self.run_check()
        self.assertEqual(problems, [])
        self.assertTrue(any("stego pairs" in n and "as shipped" in n
                            for n in notes), notes)

    def test_a_figure_wrapped_onto_the_next_line_is_still_checked(self):
        """Exactly how the real README carries it: the number ends one line at
        79 columns and the noun begins the next. Line by line the figure had
        no noun beside it and the noun no number, so the headline count of the
        whole corpus reported as never stated while the page stated it."""
        self.repo_write("README.md",
                        "The first corpus tier is complete: 4 stego arms and\n"
                        "1 clean one, 352\n"
                        "pairs, built in a single run with every arm\n"
                        "resumable.\n")
        problems, _ = self.run_check()
        self.assertTrue(any("stego pairs" in p and "352" in p
                            for p in problems), problems)

    def test_llms_txt_is_read_like_any_other_page(self):
        """A model reads it to decide how to call the tool, so a stale figure
        there reaches every agent before it reaches any human."""
        self.repo_write("README.md", "A harness for steganalysis.")
        self.repo_write("llms.txt", "The corpus holds 352 stego pairs.")
        problems, _ = self.run_check()
        self.assertTrue(any("llms.txt" in p for p in problems), problems)

    def test_a_stale_arms_sentence_in_the_readme_is_caught(self):
        """The repository writes the arm counts as a sentence and the corpus
        site as a table cell. Accepting only the cell reported a figure the
        README states plainly as never stated."""
        self.repo_write("README.md", "Complete: 3 stego arms and 2 clean ones.")
        problems, _ = self.run_check()
        self.assertTrue(any("stego arms" in p for p in problems), problems)

    def test_the_correct_arms_sentence_in_the_readme_passes(self):
        self.repo_write("README.md", "Complete: 4 stego arms and 1 clean one.")
        problems, notes = self.run_check()
        self.assertEqual([p for p in problems if "stego arms" in p], [])
        self.assertTrue(any("stego arms" in n and "as shipped" in n
                            for n in notes), notes)

    def test_a_harness_page_stating_no_figures_is_reported_not_passed(self):
        """A checker that reports clean because it found nothing to check is
        the failure this whole file exists to prevent."""
        self.repo_write("README.md", "A harness for steganalysis.")
        problems, notes = self.run_check()
        self.assertEqual(problems, [])
        self.assertTrue(any("stego pairs" in n and "never state it" in n
                            for n in notes), notes)

    def test_a_private_working_note_in_the_harness_is_not_read(self):
        self.repo_write("README.md", "The corpus holds 360 pairs.")
        self.repo_write("docs/private/notes.md", "we had 352 stego pairs")
        problems, _ = self.run_check()
        self.assertEqual(problems, [], problems)

    def test_node_modules_in_the_harness_docs_is_not_read(self):
        self.repo_write("README.md", "The corpus holds 360 pairs.")
        self.repo_write("docs/node_modules/pkg/readme.md",
                        "changelog for 352 stego pairs")
        problems, _ = self.run_check()
        self.assertEqual(problems, [], problems)

    def test_a_repository_with_no_published_prose_is_a_failure(self):
        """Pointed at the wrong directory it must refuse, not report clean on
        the strength of having read nothing."""
        empty = pathlib.Path(self.tmp.name) / "empty"
        empty.mkdir()
        with self.assertRaises(FigureError) as cm:
            check(self.docs, derive(self.release, self.covers), empty)
        self.assertIn("no published prose", str(cm.exception))

    def test_a_docs_tree_inside_the_repository_is_not_reported_twice(self):
        """Otherwise one stale number in one file reads as two defects."""
        self.repo_write("README.md", "A harness for steganalysis.")
        self.repo_write("docs/guide.md", "The corpus holds 352 stego pairs.")
        problems, _ = check(self.repo / "docs",
                            derive(self.release, self.covers), self.repo)
        self.assertEqual(len(problems), 1, problems)


class RepoFalsePositiveTests(RepoFixture):
    """Every one of these fired against the real repository on the first run.

    None was a stale figure, and a gate that cries wolf is one somebody
    switches off, which leaves the prose unchecked by a different route.
    """

    def test_a_year_in_prose_is_not_read_as_a_count(self):
        """Found against the shipped design notes: "a 1924 Polish physics
        textbook" in a sentence about public domain scans read as a stale
        count of covers under that licence, whose real value is 1,922."""
        self.repo_write("README.md", "A harness for steganalysis.")
        self.repo_write("docs/design/sources.md",
                        "Commons' public domain holdings are dominated by "
                        "scans: a 1924 Polish physics textbook, for one.")
        problems, _ = self.run_check()
        self.assertEqual(problems, [], problems)

    def test_an_iso_date_beside_a_licence_is_not_read_as_a_count(self):
        """Found against the shipped design notes: `Ruled 2026-09-16:
        permissive only. CC0, public domain and plain CC BY` read as 2,026
        covers under the public domain, whose real value is 1,922."""
        self.repo_write("README.md", "A harness for steganalysis.")
        self.repo_write("docs/design/sources.md",
                        "Ruled 2026-09-16: public domain and plain CC BY.")
        problems, _ = self.run_check()
        self.assertEqual(problems, [], problems)

    def test_a_number_one_sentence_away_is_not_a_candidate(self):
        """Rejoining wrapped lines made a nine-line paragraph one unit, and
        anything anywhere in it became a candidate for anything else. The unit
        is the sentence: what a human would point at when asked where a number
        is claimed."""
        self.repo_write("README.md", "A harness for steganalysis.")
        self.repo_write("docs/design/sources.md",
                        "The corpus covers every region. A survey listed "
                        "2150 plates.")
        problems, _ = self.run_check()
        self.assertEqual([p for p in problems if "covers" in p], [], problems)

    def test_a_survey_of_another_corpus_is_not_scanned(self):
        """`cover-source-licensing.md` records that a mirror advertising
        20,000 covers holds 9,975, which is a finding about somebody else's
        dataset sitting inside the band around our own 10,000."""
        self.repo_write("README.md", "A harness for steganalysis.")
        self.repo_write("docs/design/cover-source-licensing.md",
                        "That mirror actually holds 2150 covers.")
        problems, _ = self.run_check()
        self.assertEqual(problems, [], problems)


class RepoBoundsTests(RepoFixture):
    def test_an_oversized_page_is_refused_rather_than_truncated(self):
        """A generated file must not turn the scan into an unbounded read, and
        must not be quietly half-read either."""
        self.repo_write("README.md", "A harness for steganalysis.")
        (self.repo / "docs" / "huge.md").write_text(
            "x" * (cdf.MAX_PAGE_BYTES + 1))
        with self.assertRaises(FigureError) as cm:
            self.run_check()
        self.assertIn("huge.md", str(cm.exception))
        self.assertIn("cap", str(cm.exception))

    def test_too_many_pages_is_refused_rather_than_scanned(self):
        self.repo_write("README.md", "A harness for steganalysis.")
        with unittest.mock.patch.object(cdf, "MAX_PAGES", 1):
            with self.assertRaises(FigureError) as cm:
                self.run_check()
        self.assertIn("page cap", str(cm.exception))

    def test_too_much_prose_in_total_is_refused(self):
        """The per-page and per-tree caps multiply out to gigabytes, so on
        their own they are not a bound."""
        self.repo_write("README.md", "A harness for steganalysis.")
        self.repo_write("docs/a.md", "x" * 200)
        with unittest.mock.patch.object(cdf, "MAX_TOTAL_BYTES", 100):
            with self.assertRaises(FigureError) as cm:
                self.run_check()
        self.assertIn("bytes of prose", str(cm.exception))

    def test_the_run_says_how_many_pages_it_read_and_where(self):
        """Every failure this file guards against renders identically to a
        clean run, so a reader has to be able to see that something was read
        and which trees it came from."""
        self.repo_write("README.md", "The corpus holds 360 pairs.")
        _, notes = self.run_check()
        self.assertTrue(any("page(s) read under" in n and str(self.repo) in n
                            for n in notes), notes)

    def test_a_page_that_is_not_utf8_is_refused_by_name(self):
        self.repo_write("README.md", "A harness for steganalysis.")
        (self.repo / "docs" / "broken.md").write_bytes(b"\xff\xfe pairs")
        with self.assertRaises(FigureError) as cm:
            self.run_check()
        self.assertIn("broken.md", str(cm.exception))


class UnwrapTests(unittest.TestCase):
    def test_wrapped_prose_is_rejoined(self):
        self.assertEqual(cdf.unwrap("one 344,357\npairs here"),
                         "one 344,357 pairs here")

    def test_table_rows_stay_separate(self):
        """Joining them is what makes a figure in one row read as a candidate
        for the figure in the next."""
        text = "| CC0 | 2,625 |\n| CC BY 2.0 | 2,624 |"
        self.assertEqual(cdf.unwrap(text), text)

    def test_list_items_and_headings_stay_separate(self):
        text = "## Arms\n- 35 stego\n- 4 clean"
        self.assertEqual(cdf.unwrap(text), text)

    def test_a_code_fence_is_left_alone(self):
        text = "```\na = 1\nb = 2\n```"
        self.assertEqual(cdf.unwrap(text), text)


class EndToEndTests(Fixture):
    # `--no-repo`, because the default is a real checkout of the harness whose
    # real figures have nothing to do with this fixture's 360 pairs.
    def test_main_returns_zero_when_the_docs_agree(self):
        self.write("index.md", "360 stego pairs, 110 covers, 55%.")
        self.assertEqual(cdf.main([
            "--no-repo",
            "--docs", str(self.docs), "--release", str(self.release),
            "--covers", str(self.covers)]), 0)

    def test_main_returns_one_when_they_do_not(self):
        self.write("index.md", "54% of covers need attribution.")
        self.assertEqual(cdf.main([
            "--no-repo",
            "--docs", str(self.docs), "--release", str(self.release),
            "--covers", str(self.covers)]), 1)


class RepoEndToEndTests(RepoFixture):
    def test_main_returns_one_when_the_harness_readme_is_stale(self):
        """The whole point: a figure that drifts in the repository fails the
        same way one that drifts on the corpus site does."""
        self.repo_write("README.md", "The corpus holds 352 stego pairs.")
        self.assertEqual(cdf.main([
            "--docs", str(self.docs), "--repo", str(self.repo),
            "--release", str(self.release),
            "--covers", str(self.covers)]), 1)

    def test_main_returns_zero_when_the_harness_readme_agrees(self):
        self.repo_write("README.md", "The corpus holds 360 stego pairs.")
        self.assertEqual(cdf.main([
            "--docs", str(self.docs), "--repo", str(self.repo),
            "--release", str(self.release),
            "--covers", str(self.covers)]), 0)

    def test_main_reports_the_repository_refusal_rather_than_crashing(self):
        empty = pathlib.Path(self.tmp.name) / "empty"
        empty.mkdir()
        self.assertEqual(cdf.main([
            "--docs", str(self.docs), "--repo", str(empty),
            "--release", str(self.release),
            "--covers", str(self.covers)]), 1)


if __name__ == "__main__":
    unittest.main()
