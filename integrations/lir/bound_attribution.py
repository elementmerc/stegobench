#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""How much of an apparent calibration loss was really the bound?

WHY THIS FILE EXISTS
--------------------
The paper claimed that "68 to 71%" of apparent calibration loss was the bound
leaking into the floor, and that "37 to 50%" of total Cllr was the arithmetic
cost of the clip. Both were true of the arms they were measured on and both
were quoted as though they described the corpus. A reviewer reconstructed the
other 39 cells and got a far wider spread, including negative values.

So this recomputes both quantities on **every** cell and prints the
distribution, which is the only version of the claim that survives contact
with the whole corpus.

THE TWO DECOMPOSITIONS BEING COMPARED
---------------------------------------
naive     isotonic floor fitted to the RAW detector scores, left unbounded,
          while the reported ratios are clipped. This is what a reader
          following the textbook definition writes.
corrected floor fitted to the REPORTED ratios and clipped to the same bound.
          See `decompose`.

Run as::

    python bound_attribution.py [path/to/scores.jsonl]
"""
from __future__ import annotations

import argparse
import collections
import json
import pathlib
import sys

import numpy as np

from analyse_panel import roc_auc
from likelihood_ratio import cllr, cross_validated_lrs, decompose, pav
from positive_control import DEFAULT_SCORES, DETECTORS


def naive_decomposition(scores, lrs, labels, ) -> tuple[float, float, float]:
    """Cllr, floor and loss the way the textbook definition reads.

    PAV on the raw scores, no bound on the result. The reported ratios are
    still the bounded ones, because that half was never in question.
    """
    labels = np.asarray(labels, dtype=int)
    payload = labels == 1
    total = cllr(lrs[payload], lrs[~payload])

    posterior = pav(scores, labels)
    prior_odds = payload.sum() / (~payload).sum()
    eps = 1e-12
    pav_lrs = (posterior + eps) / (1.0 - posterior + eps) / prior_odds
    floor = cllr(pav_lrs[payload], pav_lrs[~payload])
    return total, floor, total - floor


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("scores", nargs="?", type=pathlib.Path, default=DEFAULT_SCORES)
    ap.add_argument("--folds", type=int, default=10)
    ap.add_argument("--bound", type=float, default=100.0)
    ap.add_argument("--seed", type=int, default=0)
    args = ap.parse_args(argv)

    if not args.scores.exists():
        print(f"no scores at {args.scores}", file=sys.stderr)
        return 2

    rows = [json.loads(line) for line in args.scores.read_text(encoding="utf-8").splitlines() if line.strip()]
    clean = [r for r in rows if r["label"] == "clean"]
    arms: dict[tuple, list] = collections.defaultdict(list)
    for r in rows:
        if r["label"] == "stego":
            arms[(r["tool"], r["variant"])].append(r)

    from likelihood_ratio import bound_cost

    clip = bound_cost(args.bound)
    print(f"bound {args.bound:g}, whose own cost is {clip:.4f}\n")
    header = (
        f"{'arm':<14}{'det':<5}{'AUC':>7}{'Cllr':>8}"
        f"{'cal_naive':>11}{'cal_corr':>10}{'bound share':>13}{'clip/Cllr':>11}"
    )
    print(header)
    print("-" * len(header))

    shares, clips, informative_clips, aucs = [], [], [], []
    for key in sorted(arms):
        stego = arms[key]
        rng = np.random.default_rng(args.seed)
        n = min(len(stego), len(clean))
        picked_stego = [stego[i] for i in rng.choice(len(stego), n, replace=False)]
        picked_clean = [clean[i] for i in rng.choice(len(clean), n, replace=False)]

        for det in DETECTORS:
            s = np.array([r[det] for r in picked_stego], dtype=float)
            c = np.array([r[det] for r in picked_clean], dtype=float)
            scores = np.concatenate([s, c])
            labels = np.concatenate([np.ones(n, int), np.zeros(n, int)])

            lrs = cross_validated_lrs(
                scores, labels, folds=args.folds, bound=args.bound, seed=args.seed
            )
            d = decompose(lrs, labels, bound=args.bound)
            _, _, cal_naive = naive_decomposition(scores, lrs, labels)

            # The fraction of the apparent loss that the bound was responsible
            # for. Undefined when the naive loss is at or below zero, which
            # happens on an inverted detector: the system beats its own
            # unbounded floor, which is the second failure in Section 4.1.
            share = (cal_naive - d.cllr_cal) / cal_naive if cal_naive > 1e-9 else float("nan")
            clip_share = clip / d.cllr

            aucs.append(roc_auc(s, c))
            shares.append(share)
            clips.append(clip_share)
            if d.cllr < 0.95:
                informative_clips.append(clip_share)

            print(
                f"{key[0] + '/' + key[1]:<14}{det:<5}{roc_auc(s, c):>7.3f}{d.cllr:>8.3f}"
                f"{cal_naive:>11.3f}{d.cllr_cal:>10.3f}"
                f"{share * 100:>12.1f}%{clip_share * 100:>10.1f}%",
                flush=True,
            )

    shares = np.array(shares)
    finite = shares[np.isfinite(shares)]
    clips = np.array(clips)
    inf_clips = np.array(informative_clips)

    print()
    print(f"cells: {len(shares)}; bound share defined on {len(finite)} of them")
    print(
        f"bound share of apparent calibration loss, all cells: "
        f"median {np.median(finite) * 100:.1f}%, "
        f"range {finite.min() * 100:.1f}% to {finite.max() * 100:.1f}%"
    )
    # Where the detector is at chance the naive loss is itself near zero, so
    # the share is a ratio of two small numbers and carries no information.
    # The cells that matter are the ones an examiner would report.
    works = np.array(aucs) > 0.99
    sub = shares[works & np.isfinite(shares)]
    print(
        f"the same, restricted to the {works.sum()} cells with AUC above 0.99: "
        f"median {np.median(sub) * 100:.1f}%, "
        f"range {sub.min() * 100:.1f}% to {sub.max() * 100:.1f}%"
    )
    print(
        f"clip as a share of total Cllr, all cells: median {np.median(clips) * 100:.1f}%, "
        f"range {clips.min() * 100:.1f}% to {clips.max() * 100:.1f}%"
    )
    print(
        f"clip as a share of total Cllr, the {len(inf_clips)} cells with Cllr below 0.95: "
        f"median {np.median(inf_clips) * 100:.1f}%, "
        f"range {inf_clips.min() * 100:.1f}% to {inf_clips.max() * 100:.1f}%"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
