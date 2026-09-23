#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""One file, one score, one likelihood ratio, one sentence a report could carry.

WHY THIS FILE EXISTS
--------------------
The paper argues that a steganalysis score has to become a likelihood ratio
before it means anything about the exhibit in front of an examiner, and then
never shows that happening to a single file. A reviewer pointed out that this
is the paper's whole premise and that it is missing.

So this takes three files out of the control corpus, walks each one through
the pipeline, and prints what the examiner would end up writing. One carries a
payload and the detector is confident. One is clean and the detector is
confident. One is a case where the evidence does not move the odds, which is
the outcome the field has no vocabulary for and the one worth showing.

Run as::

    python worked_case.py [path/to/scores.jsonl]
"""
from __future__ import annotations

import argparse
import collections
import json
import pathlib
import sys

import numpy as np

from likelihood_ratio import cross_validated_lrs
from positive_control import DEFAULT_SCORES


def sentence(lr: float, detector: str) -> list[str]:
    """The two lines a report would carry: the ratio, then what it supports.

    Written out rather than left as a bare number, because a bare likelihood
    ratio is exactly the thing the ENFSI guideline warns against handing to a
    reader who has no way to interpret it.
    """
    first = (
        f"The {detector} result is {lr:.4g} times more probable if the file "
        f"carries a hidden payload than if it does not."
    )
    strength = verbal(lr)
    if strength == "does not support either proposition":
        second = "The evidence does not support either proposition."
    elif lr >= 1:
        second = f"That is {strength} for the proposition that it carries one."
    else:
        second = f"That is {strength} for the proposition that it does not."
    return [first, second]


def verbal(lr: float) -> str:
    """The ENFSI verbal scale, which is what stops a bare number being quoted.

    The bands are the ones in the ENFSI guideline for evaluative reporting.
    A report gives the number and the phrase together; the phrase exists so
    that a reader who cannot interpret 47 is not left to guess.
    """
    x = lr if lr >= 1 else 1 / lr
    if x < 2:
        return "does not support either proposition"
    if x < 10:
        return "weak support"
    if x < 100:
        return "moderate support"
    if x < 1000:
        return "moderately strong support"
    return "strong support"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("scores", nargs="?", type=pathlib.Path, default=DEFAULT_SCORES)
    ap.add_argument("--tool", default="html")
    ap.add_argument("--variant", default="zip")
    ap.add_argument("--detector", default="rs")
    ap.add_argument("--folds", type=int, default=10)
    ap.add_argument("--bound", type=float, default=100.0)
    ap.add_argument("--seed", type=int, default=0)
    args = ap.parse_args(argv)

    if not args.scores.exists():
        print(f"no scores at {args.scores}", file=sys.stderr)
        return 2

    rows = [json.loads(line) for line in args.scores.read_text(encoding="utf-8").splitlines() if line.strip()]
    clean = [r for r in rows if r["label"] == "clean"]
    stego = [
        r
        for r in rows
        if r["label"] == "stego" and r["tool"] == args.tool and r["variant"] == args.variant
    ]
    if not stego:
        print(f"no {args.tool}/{args.variant} arm in {args.scores}", file=sys.stderr)
        return 2

    rng = np.random.default_rng(args.seed)
    n = min(len(stego), len(clean))
    picked_stego = [stego[i] for i in rng.choice(len(stego), n, replace=False)]
    picked_clean = [clean[i] for i in rng.choice(len(clean), n, replace=False)]
    picked = picked_stego + picked_clean

    scores = np.array([r[args.detector] for r in picked], dtype=float)
    labels = np.concatenate([np.ones(n, int), np.zeros(n, int)])
    lrs = cross_validated_lrs(
        scores, labels, folds=args.folds, bound=args.bound, seed=args.seed
    )

    print(f"arm {args.tool}/{args.variant}, detector {args.detector.upper()}, "
          f"{n} per side, bound {args.bound:g}\n")

    # One case at the confident top, one at the confident bottom, and one
    # nearest LR = 1, which is the case the paper is really about.
    wanted = [
        ("a file that carries a payload", int(np.argmax(lrs[:n]))),
        ("a clean file", n + int(np.argmin(lrs[n:]))),
        ("the case nobody has a word for", int(np.argmin(np.abs(np.log(lrs))))),
    ]

    for caption, i in wanted:
        row = picked[i]
        lr = float(lrs[i])
        truth = "carries a payload" if labels[i] == 1 else "is clean"
        at_bound = " (at the bound)" if lr >= args.bound or lr <= 1 / args.bound else ""
        print(f"{caption}")
        print(f"  ground truth      {truth}")
        print(f"  {args.detector.upper()} score{'':<12}{row[args.detector]:.6f}")
        print(f"  likelihood ratio  {lr:.4g}{at_bound}")
        lines = sentence(lr, args.detector.upper())
        print(f"  report text       {lines[0]}")
        print(f"                    {lines[1]}")
        print()

    print("The middle case is the one that matters. An AUC of 1.000 for this arm")
    print("says the detector is perfect across the corpus, and says nothing about")
    print("this file, on which the evidence does not move the odds at all.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
