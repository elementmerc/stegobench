# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the corpus reader and the AUC.

These had no tests at all, including `load()`, whose own docstring flags the
placeholder-record trap as the thing most likely to go wrong silently.
"""
from __future__ import annotations

import json

import numpy as np
import pytest

from analyse_panel import ALPHA, arm_of, cover_id, load, main, roc_auc, verdict_for


def write(tmp_path, rows):
    path = tmp_path / "panel.jsonl"
    path.write_text("".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")
    return path


class TestLoadKeepsTheMostCompleteRecord:
    def test_the_placeholder_is_discarded_whichever_order_it_arrives(self, tmp_path):
        """The panel writes an empty record before it scores.

        Reading the first occurrence gives an empty set and the last gives a
        partial one. Neither announces itself, which is what makes this worth
        a test rather than a comment.
        """
        for rows in (
            [{"file": "clean/a.jpg"}, {"file": "clean/a.jpg", "spa": 0.4}],
            [{"file": "clean/a.jpg", "spa": 0.4}, {"file": "clean/a.jpg"}],
        ):
            got = load(write(tmp_path, rows))
            assert got["clean/a.jpg"] == {"file": "clean/a.jpg", "spa": 0.4}

    def test_blank_lines_are_skipped(self, tmp_path):
        path = tmp_path / "p.jsonl"
        path.write_text('{"file": "a.jpg", "spa": 1}\n\n   \n{"file": "b.jpg", "spa": 2}\n', encoding="utf-8")
        assert set(load(path)) == {"a.jpg", "b.jpg"}

    def test_an_empty_file_loads_to_nothing(self, tmp_path):
        path = tmp_path / "p.jsonl"
        path.write_text("", encoding="utf-8")
        assert load(path) == {}


class TestPathHelpers:
    @pytest.mark.parametrize(
        "name,arm,cover",
        [
            ("clean/00042.jpg", "clean", "00042.jpg"),
            ("outguess/0200/00042.jpg", "outguess/0200", "00042.jpg"),
            ("bare.jpg", "clean", "bare.jpg"),
        ],
    )
    def test_arm_and_cover(self, name, arm, cover):
        assert arm_of(name) == arm
        assert cover_id(name) == cover

    def test_the_same_cover_is_recognised_across_arms(self):
        assert cover_id("outguess/0200/00042.jpg") == cover_id("clean/00042.jpg")


class TestRocAuc:
    @staticmethod
    def brute_force(payload, clean):
        wins = sum(
            1.0 if p > c else 0.5 if p == c else 0.0 for p in payload for c in clean
        )
        return wins / (len(payload) * len(clean))

    def test_matches_a_brute_force_reference_including_ties(self):
        rng = np.random.default_rng(0)
        for _ in range(100):
            # Small integers, so ties are common rather than rare.
            p = rng.integers(0, 5, size=rng.integers(2, 12)).astype(float)
            c = rng.integers(0, 5, size=rng.integers(2, 12)).astype(float)
            assert roc_auc(p, c) == pytest.approx(self.brute_force(p, c))

    def test_perfect_and_inverted_and_identical(self):
        assert roc_auc(np.array([3.0, 4.0]), np.array([1.0, 2.0])) == 1.0
        assert roc_auc(np.array([1.0, 2.0]), np.array([3.0, 4.0])) == 0.0
        assert roc_auc(np.array([1.0, 1.0]), np.array([1.0, 1.0])) == 0.5


class TestEndToEnd:
    def test_a_corpus_with_no_clean_arm_is_refused(self, tmp_path, capsys):
        path = write(tmp_path, [{"file": "steghide/0500/a.jpg", "aletheia_spa": 0.4}])
        assert main([str(path)]) == 2

    def test_an_arm_too_small_to_calibrate_says_so(self, tmp_path, capsys):
        rows = []
        for i in range(4):
            rows.append({"file": f"clean/{i}.jpg", "aletheia_spa": 0.01 * i})
            rows.append({"file": f"steghide/0500/{i}.jpg", "aletheia_spa": 0.5 + 0.01 * i})
        assert main([str(write(tmp_path, rows)), "--folds", "10"]) == 0
        assert "too few scored to calibrate" in capsys.readouterr().out

    def test_a_real_arm_runs_and_reports_every_detector(self, tmp_path, capsys):
        rng = np.random.default_rng(1)
        rows = []
        for i in range(40):
            base = rng.normal()
            rows.append(
                {
                    "file": f"clean/{i}.jpg",
                    "aletheia_spa": base,
                    "aletheia_rs": base,
                    "stegexpose": base,
                }
            )
            rows.append(
                {
                    "file": f"steghide/0500/{i}.jpg",
                    "aletheia_spa": base + 3.0,
                    "aletheia_rs": base + 3.0,
                    "stegexpose": base + 3.0,
                }
            )
        code = main(
            [str(write(tmp_path, rows)), "--folds", "5", "--permutations", "10", "--seeds", "3"]
        )
        assert code == 0
        out = capsys.readouterr().out
        for det in ("aletheia_spa", "aletheia_rs", "stegexpose"):
            assert det in out
        # A three sigma separation is real evidence and must not read as nothing.
        assert "informative" in out


# --------------------------------------------------------------- verdicts


@pytest.mark.parametrize(
    "p_value,cllr,expected",
    [
        (0.001, 0.5, "informative"),
        (0.049, 0.5, "informative"),
        (0.001, 0.95, "detectable, not useful"),
        (0.050, 0.5, "no evidential value"),
        (0.500, 0.5, "no evidential value"),
    ],
)
def test_the_three_verdicts(p_value, cllr, expected):
    assert verdict_for(p_value, cllr, useful=0.9) == expected


def test_a_cell_only_separates_strictly_below_alpha():
    """ALPHA itself is not significant, so the boundary cannot drift."""
    assert verdict_for(ALPHA, 0.1, useful=0.9) == "no evidential value"
    assert verdict_for(ALPHA - 1e-12, 0.1, useful=0.9) == "informative"


def test_usefulness_is_a_strict_threshold_too():
    assert verdict_for(0.01, 0.9, useful=0.9) == "detectable, not useful"
    assert verdict_for(0.01, 0.9 - 1e-12, useful=0.9) == "informative"


@pytest.mark.parametrize("p_value", [0.951, 0.96, 0.9804, 0.99, 1.0])
def test_the_upper_tail_never_earns_a_verdict_of_its_own(p_value):
    """The defect this guards, in one assertion.

    A per-cell p in the upper tail used to print "worse than its null". Over
    a 45 cell panel 2.25 cells clear 0.95 by chance; the run that prompted
    the removal carried one, fewer than chance predicts, on a cell whose AUC
    was 0.530 and so pointing the right way. Nothing above ALPHA may say more
    than "no evidential value".
    """
    assert verdict_for(p_value, 1.001, useful=0.9) == "no evidential value"


def test_no_verdict_string_claims_the_system_is_worse_than_nothing():
    seen = {
        verdict_for(p, c, useful=0.9)
        for p in (0.0, 0.01, 0.05, 0.5, 0.95, 0.99, 1.0)
        for c in (0.0, 0.5, 0.9, 1.0, 1.5)
    }
    assert seen == {"informative", "detectable, not useful", "no evidential value"}
    assert not any("worse" in v for v in seen)
