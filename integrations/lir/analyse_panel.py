#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Report a scored corpus as likelihood ratios beside the usual numbers.

Run as::

    python analyse_panel.py path/to/panel.jsonl

WHAT THIS IS FOR
----------------
The same arms, the same scores, reported twice: once in the language
steganalysis papers use (AUC), and once in the language a forensic report uses
(Cllr and its decomposition). Putting them in one table is the point, because
the cases where they disagree are the interesting ones and nobody looks for
them while the two live in separate literatures.

THE TWO TRAPS THIS FILE IS BUILT TO AVOID
-------------------------------------------
**An arm is scored only against the covers it was actually run on.** outguess
covers 160 of the 200 pictures. The 40 it never touched are not a control for
it, and pooling them into the clean side moved its AUC by five points in the
direction of a more dramatic finding.

**The panel writes a placeholder record before it scores**, so a file appears
twice: once empty, once filled. Reading the first occurrence gives an empty
set and the last gives a partial one, and neither announces itself. The record
kept is the one with the most fields.
"""
from __future__ import annotations

import argparse
import collections
import json
import pathlib
import sys
import time

import numpy as np

from likelihood_ratio import (
    bound_cost,
    cllr_null,
    observed_decomposition,
)

DETECTORS = ("aletheia_spa", "aletheia_rs", "stegexpose")


def load(path: pathlib.Path) -> dict[str, dict]:
    """One record per file, keeping the most complete."""
    best: dict[str, dict] = {}
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        name = row["file"]
        if name not in best or len(row) > len(best[name]):
            best[name] = row
    return best


def cover_id(name: str) -> str:
    """The cover a file came from, whatever depth it sits at.

    `outguess/0200/00042.jpg` and `clean/00042.jpg` are the same cover, and
    that correspondence is what makes the pairing possible.
    """
    return pathlib.PurePosixPath(name).name


def arm_of(name: str) -> str:
    parts = pathlib.PurePosixPath(name).parts
    return "/".join(parts[:-1]) or "clean"


def roc_auc(payload: np.ndarray, clean: np.ndarray) -> float:
    """Tie aware AUC, by rank sum with average ranks for ties."""
    combined = np.concatenate([payload, clean])
    order = combined.argsort(kind="mergesort")
    ranks = np.empty(len(combined), dtype=float)
    ranks[order] = np.arange(1, len(combined) + 1)
    # Average the ranks within each run of equal values.
    values = combined[order]
    start = 0
    for i in range(1, len(values) + 1):
        if i == len(values) or values[i] != values[start]:
            ranks[order[start:i]] = ranks[order[start:i]].mean()
            start = i
    n_p = len(payload)
    return float((ranks[:n_p].sum() - n_p * (n_p + 1) / 2) / (n_p * len(clean)))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("panel", type=pathlib.Path)
    ap.add_argument("--folds", type=int, default=10)
    ap.add_argument("--bound", type=float, default=100.0)
    ap.add_argument(
        "--permutations",
        type=int,
        default=200,
        help="size of the permutation null the result has to beat",
    )
    ap.add_argument("--seed", type=int, default=0, help="seed for the null and the reported fit")
    ap.add_argument(
        "--seeds",
        type=int,
        default=20,
        help="fold seeds to average the observed Cllr over",
    )
    ap.add_argument(
        "--useful",
        type=float,
        default=0.95,
        help="Cllr a result must beat to be called informative rather than merely detectable",
    )
    args = ap.parse_args(argv)

    records = load(args.panel)
    by_arm: dict[str, dict[str, dict]] = collections.defaultdict(dict)
    for name, row in records.items():
        by_arm[arm_of(name)][cover_id(name)] = row

    if "clean" not in by_arm:
        print("no clean arm in this corpus, so nothing can be paired", file=sys.stderr)
        return 2
    clean = by_arm.pop("clean")

    print(f"corpus: {args.panel}")
    print(f"clean arm: {len(clean)} covers\n")

    header = (
        f"{'arm':<20}{'detector':<14}{'paired':>7}"
        f"{'AUC':>7}{'Cllr (sd)':>17}{'Cllr_min':>9}{'Cllr_cal':>9}{'p':>7}  verdict"
    )
    print(header)
    print("-" * len(header))

    started = time.monotonic()
    for arm in sorted(by_arm):
        stego = by_arm[arm]
        # Only the covers this arm was actually run on.
        paired = sorted(set(stego) & set(clean))

        for det in DETECTORS:
            ids = [c for c in paired if det in stego[c] and det in clean[c]]
            if len(ids) < args.folds:
                print(f"{arm:<20}{det:<14}{len(ids):>7}  too few scored to calibrate")
                continue

            s_stego = np.array([float(stego[c][det]) for c in ids])
            s_clean = np.array([float(clean[c][det]) for c in ids])
            scores = np.concatenate([s_stego, s_clean])
            labels = np.concatenate([np.ones(len(ids), int), np.zeros(len(ids), int)])
            # A stego picture and the cover it came from are one unit, and the
            # null is that the label within that unit is arbitrary.
            pairs = np.concatenate([np.arange(len(ids)), np.arange(len(ids))])

            auc = roc_auc(s_stego, s_clean)
            # Averaged over fold seeds: a single cross validation is a draw
            # whose spread exceeds the effect being measured. Every column of
            # the decomposition comes from the same draws, so the printed
            # Cllr minus the printed Cllr_min is the printed Cllr_cal.
            d, sd_cllr = observed_decomposition(
                scores, labels, folds=args.folds, bound=args.bound, seeds=args.seeds
            )
            mean_cllr = d.cllr

            # The reference is what this same pipeline yields when the labels
            # are meaningless, not the textbook 1.0. See `cllr_null`.
            null = cllr_null(
                scores,
                labels,
                permutations=args.permutations,
                folds=args.folds,
                bound=args.bound,
                seed=args.seed,
                pairs=pairs,
            )
            # (1 + count) / (1 + n): a permutation p of exactly zero claims more
            # than the number of permutations can support.
            p_value = float((1 + (null <= mean_cllr).sum()) / (1 + len(null)))

            if p_value < 0.05 and mean_cllr < args.useful:
                verdict = "informative"
            elif p_value < 0.05:
                # Distinguishable from the null and still worth nothing to an
                # examiner. Saying "informative" here would be this module
                # committing the overstatement it exists to prevent.
                verdict = "detectable, not useful"
            elif p_value > 0.95:
                verdict = "worse than its null"
            else:
                verdict = "no evidential value"

            print(
                f"{arm:<20}{det:<14}{len(ids):>7}"
                f"{auc:>7.3f}{f'{mean_cllr:.3f} ({sd_cllr:.3f})':>17}"
                f"{d.cllr_min:>9.3f}{d.cllr_cal:>9.3f}{p_value:>7.3f}  {verdict}"
            )
            if time.monotonic() - started > 30:
                print(f"  ... {time.monotonic() - started:.0f}s elapsed", flush=True)
                started = time.monotonic()

    print()
    print(
        f"settings: {args.folds} folds, bound {args.bound:g}, {args.seeds} fold seeds, "
        f"{args.permutations} within-pair permutations"
    )
    print(
        f"the smallest p this many permutations can report is "
        f"{1 / (1 + args.permutations):.4f}"
    )
    print("Cllr = 1.000 is the textbook cost of answering 'this tells you nothing'.")
    print("The reference used here is a within-pair permutation null, which sits slightly")
    print("above 1.000 because cross validated calibration of noise is not free.")
    print(f"Cllr is the mean over {args.seeds} fold seeds; the bracket is its standard")
    print("deviation, because one cross validation is a draw rather than a measurement.")
    print(f"A flawless system bounded at {args.bound:g} would still score {bound_cost(args.bound):.4f}.")
    print(f"'informative' additionally requires Cllr below {args.useful:g}.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
