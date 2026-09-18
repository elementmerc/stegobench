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
import time

import numpy as np

from analyse_panel import roc_auc
from likelihood_ratio import cllr_null, cross_validated_lrs, decompose, observed_cllr

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
    ap.add_argument("--seeds", type=int, default=20, help="fold seeds to average Cllr over")
    ap.add_argument("--useful", type=float, default=0.95)
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
        f"{'arm':<14}{'det':<6}{'n':>7}{'AUC':>8}{'Cllr (sd)':>17}"
        f"{'Cllr_min':>10}{'Cllr_cal':>10}{'p':>7}  verdict"
    )
    print(header)
    print("-" * len(header))

    started = time.monotonic()
    for key in sorted(arms):
        stego = arms[key]
        # Subsample PICTURES, once per arm, and use the same ones for every
        # detector. The earlier version drew from the score *values* and drew
        # again inside the detector loop, so the three rows of a published arm
        # described three different subsets and could not be read across.
        rng = np.random.default_rng(args.seed)
        n = min(len(stego), len(clean))
        stego_idx = rng.choice(len(stego), n, replace=False)
        clean_idx = rng.choice(len(clean), n, replace=False)
        picked_stego = [stego[i] for i in stego_idx]
        picked_clean = [clean[i] for i in clean_idx]

        for det in DETECTORS:
            s = np.array([r[det] for r in picked_stego], dtype=float)
            c = np.array([r[det] for r in picked_clean], dtype=float)

            scores = np.concatenate([s, c])
            labels = np.concatenate([np.ones(n, int), np.zeros(n, int)])
            mean_cllr, sd_cllr, _ = observed_cllr(
                scores, labels, folds=args.folds, seeds=args.seeds
            )
            lrs = cross_validated_lrs(scores, labels, folds=args.folds, seed=args.seed)
            d = decompose(lrs, labels)
            # This corpus is not cover paired: the clean and stego sets are
            # different pictures, so a free permutation is the right null here
            # and a within-pair one would be a fiction.
            null = cllr_null(
                scores, labels, permutations=args.permutations, folds=args.folds, seed=args.seed
            )
            p = float((1 + (null <= mean_cllr).sum()) / (1 + len(null)))
            if p < 0.05 and mean_cllr < args.useful:
                verdict = "informative"
            elif p < 0.05:
                verdict = "detectable, not useful"
            elif p > 0.95:
                verdict = "worse than chance"
            else:
                verdict = "no evidential value"
            print(
                f"{key[0] + '/' + key[1]:<14}{det:<6}{2 * n:>7}{roc_auc(s, c):>8.3f}"
                f"{f'{mean_cllr:.3f} ({sd_cllr:.3f})':>17}{d.cllr_min:>10.3f}"
                f"{d.cllr_cal:>10.3f}{p:>7.3f}  {verdict}",
                flush=True,
            )
        if time.monotonic() - started > 30:
            print(f"  ... {time.monotonic() - started:.0f}s elapsed", flush=True)
            started = time.monotonic()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
