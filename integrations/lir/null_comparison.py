#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""The three nulls side by side, on the arm whose answer is known exactly.

WHY THIS FILE EXISTS
--------------------
Section 4.3 of the paper compares a free permutation null against a within
pair one, and the table it prints was generated before the estimator matching
fix. Under the corrected null both bands are narrower, so the numbers had to
be measured again rather than carried forward.

It also measures the thing the fix was about, which the paper asserted and
never showed: what happens to the null when each draw is a single cross
validation and the observed statistic is a mean over twenty.

Run with the optional lir environment::

    python null_comparison.py <panel.jsonl>
"""
from __future__ import annotations

import argparse
import collections
import pathlib

import numpy as np

from analyse_panel import DETECTORS, arm_of, cover_id, load
from likelihood_ratio import cllr_null, observed_cllr


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("panel", type=pathlib.Path)
    ap.add_argument("--arm", default="structural/0000")
    ap.add_argument("--detector", default="aletheia_spa")
    ap.add_argument("--folds", type=int, default=10)
    ap.add_argument("--seeds", type=int, default=20)
    ap.add_argument("--permutations", type=int, default=100)
    args = ap.parse_args(argv)

    records = load(args.panel)
    by: dict[str, dict] = collections.defaultdict(dict)
    for name, row in records.items():
        by[arm_of(name)][cover_id(name)] = row
    clean = by.pop("clean")
    stego = by[args.arm]
    det = args.detector
    ids = [c for c in sorted(set(stego) & set(clean)) if det in stego[c] and det in clean[c]]
    s = np.array([float(stego[c][det]) for c in ids])
    c = np.array([float(clean[c][det]) for c in ids])
    print(f"{args.arm}, {det}, {len(ids)} covers per side")
    print(f"maximum absolute difference between the two sides: {np.abs(s - c).max():g}")

    scores = np.concatenate([s, c])
    labels = np.concatenate([np.ones(len(ids), int), np.zeros(len(ids), int)])
    pairs = np.concatenate([np.arange(len(ids)), np.arange(len(ids))])

    observed, sd, draws = observed_cllr(scores, labels, folds=args.folds, seeds=args.seeds)
    print(f"\nobserved, mean over {args.seeds} fold seeds: {observed:.5f} (sd {sd:.5f})")
    print(f"a single draw ranges {draws.min():.5f} to {draws.max():.5f} over those seeds")

    def report(name, null):
        lo, hi = np.quantile(null, 0.05), np.quantile(null, 0.95)
        p = float((1 + (null <= observed).sum()) / (1 + len(null)))
        print(
            f"{name:<34}{null.mean():>10.5f}  [{lo:.5f}, {hi:.5f}]"
            f"  width {hi - lo:.5f}  p {p:.3f}"
        )
        return hi - lo

    print(f"\n{'null':<34}{'mean':>10}  {'5-95%':<22}{'':<14}")
    w_free = report("free, matched estimator", cllr_null(
        scores, labels, permutations=args.permutations, folds=args.folds, seeds=args.seeds))
    w_pair = report("within pair, matched estimator", cllr_null(
        scores, labels, permutations=args.permutations, folds=args.folds,
        pairs=pairs, seeds=args.seeds))
    w_bad = report("within pair, single seed draws", cllr_null(
        scores, labels, permutations=args.permutations, folds=args.folds,
        pairs=pairs, seeds=1))

    print(f"\nthe unmatched null is {w_bad / w_pair:.1f} times as wide as the matched one")
    print(f"the free null is {w_free / w_pair:.1f} times as wide as the paired one")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
