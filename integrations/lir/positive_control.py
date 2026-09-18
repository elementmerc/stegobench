#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Show the pipeline answering 'informative' on data that carries a signal.

WHY THIS FILE EXISTS
--------------------
Run over the round3-q95 JPEG arms, `analyse_panel.py` returns "no evidential
value" for every single arm and every single detector. That is the correct
answer, because those detectors are blind in the JPEG DCT domain. It is also
exactly what a broken pipeline that can only ever say "nothing" would print.

So the pipeline is pointed at real data where the answer is known to be the
other one. Without this, the JPEG table is not a finding, it is an untested
control.

The data is the spatial LSB corpus from Stegcore's threshold calibration:
8,000 clean pictures and 36,000 stego across five payload types and three
encodings, scored by Stegcore's own SPA, RS and WS. It is not redistributable
(see `docs/cover-source-licensing.md`), so this script reads it from disk
rather than shipping it.

Usage::

    python positive_control.py [path/to/scores.jsonl]
"""
from __future__ import annotations

import argparse
import collections
import json
import pathlib
import sys

import numpy as np

from analyse_panel import roc_auc
from likelihood_ratio import cllr_null, cross_validated_lrs, decompose

DEFAULT_SCORES = pathlib.Path(
    "/home/mercury/the-factory/Stegcore/private/calibration/scores-2026-05-22.jsonl"
)
DETECTORS = ("spa", "rs", "ws")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("scores", nargs="?", type=pathlib.Path, default=DEFAULT_SCORES)
    ap.add_argument("--permutations", type=int, default=50)
    ap.add_argument("--folds", type=int, default=10)
    ap.add_argument("--seed", type=int, default=0)
    args = ap.parse_args(argv)

    if not args.scores.exists():
        print(f"no scores at {args.scores}", file=sys.stderr)
        print("This corpus is licence restricted and is not shipped.", file=sys.stderr)
        return 2

    rows = [json.loads(line) for line in args.scores.read_text().splitlines() if line.strip()]
    clean = [r for r in rows if r["label"] == "clean"]
    arms: dict[tuple, list] = collections.defaultdict(list)
    for r in rows:
        if r["label"] == "stego":
            arms[(r["tool"], r["variant"])].append(r)

    print(f"positive control: {args.scores.name}, {len(clean)} clean pictures\n")
    header = (
        f"{'arm':<14}{'det':<6}{'n':>7}{'AUC':>8}{'Cllr':>8}"
        f"{'Cllr_min':>10}{'Cllr_cal':>10}{'p':>7}  verdict"
    )
    print(header)
    print("-" * len(header))

    rng = np.random.default_rng(args.seed)
    for key in sorted(arms):
        stego = arms[key]
        for det in DETECTORS:
            s = np.array([r[det] for r in stego], dtype=float)
            c = np.array([r[det] for r in clean], dtype=float)
            # Match the two sides, so the calibrator is not handed a base rate
            # that has nothing to do with any case it would be used on.
            n = min(len(s), len(c))
            s = rng.choice(s, n, replace=False)
            c = rng.choice(c, n, replace=False)

            scores = np.concatenate([s, c])
            labels = np.concatenate([np.ones(n, int), np.zeros(n, int)])
            lrs = cross_validated_lrs(scores, labels, folds=args.folds)
            d = decompose(lrs, labels, scores)
            null = cllr_null(scores, labels, permutations=args.permutations, folds=args.folds)
            p = float((null <= d.cllr).mean())
            verdict = (
                "informative"
                if p < 0.05
                else ("worse than chance" if p > 0.95 else "no evidential value")
            )
            print(
                f"{key[0] + '/' + key[1]:<14}{det:<6}{2 * n:>7}{roc_auc(s, c):>8.3f}"
                f"{d.cllr:>8.3f}{d.cllr_min:>10.3f}{d.cllr_cal:>10.3f}{p:>7.3f}  {verdict}"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
