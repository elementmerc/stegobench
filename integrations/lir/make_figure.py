#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Draw the figure for the likelihood ratio paper.

Two panels, because the paper makes two claims here and the third, the
positive control, is a 45 row table in the paper and does not need a picture
of the same numbers.

(a) The permutation null for one cell, with the observed Cllr on it. This is
    the method: it shows why the textbook reference of 1.0 is not the right
    comparison for a cross validated pipeline, because the null itself sits
    above it.

(b) **Every** JPEG cell, all three detectors, against its own null band. This
    is the result. An earlier version of this figure drew one detector and
    captioned it as every arm, and titled itself "every arm sits inside its
    null", which stopped being true once the null was corrected.

ERROR BARS
----------
The observed statistic is a mean over `seeds` fold seeds and the null band is
a band of quantities estimated the same way, so the bar that belongs next to
it is the standard error of that mean, sd / sqrt(seeds), not the spread of a
single draw. The earlier figure drew the latter and so overstated the
uncertainty on every point by a factor of sqrt(20).

Needs matplotlib, which the rest of this directory deliberately does not.
Run it with the optional lir environment.
"""
from __future__ import annotations

import argparse
import collections
import pathlib
import pickle
import sys

import numpy as np

from analyse_panel import DETECTORS, arm_of, cover_id, load
from likelihood_ratio import cllr_null, observed_cllr

#: How the arm directory names are written in the paper's tables.
PRETTY = {"0050": "0.05", "0200": "0.20", "0500": "0.50"}

SHORT = {"aletheia_spa": "SPA", "aletheia_rs": "RS", "stegexpose": "StegExpose"}


def pretty_arm(arm: str) -> str:
    parts = arm.split("/")
    if len(parts) == 2 and parts[1] in PRETTY:
        return f"{parts[0]}/{PRETTY[parts[1]]}"
    return parts[0]


def gather(panel: pathlib.Path, folds: int, seeds: int, perms: int):
    """Every arm against every detector, which is what the caption claims."""
    records = load(panel)
    by_arm: dict[str, dict] = collections.defaultdict(dict)
    for name, row in records.items():
        by_arm[arm_of(name)][cover_id(name)] = row
    clean = by_arm.pop("clean")

    out = []
    for arm in sorted(by_arm):
        stego = by_arm[arm]
        paired = sorted(set(stego) & set(clean))
        for det in DETECTORS:
            ids = [c for c in paired if det in stego[c] and det in clean[c]]
            if len(ids) < folds:
                continue
            s = np.array([float(stego[c][det]) for c in ids])
            c = np.array([float(clean[c][det]) for c in ids])
            scores = np.concatenate([s, c])
            labels = np.concatenate([np.ones(len(ids), int), np.zeros(len(ids), int)])
            pairs = np.concatenate([np.arange(len(ids)), np.arange(len(ids))])

            mean, sd, _ = observed_cllr(
                scores, labels, folds=folds, seeds=seeds, groups=pairs
            )
            null = cllr_null(
                scores, labels, permutations=perms, folds=folds, pairs=pairs, seeds=seeds
            )
            p = float((1 + (null <= mean).sum()) / (1 + len(null)))
            out.append((pretty_arm(arm), SHORT[det], mean, sd / np.sqrt(seeds), null, p))
            print(f"  {pretty_arm(arm):16} {SHORT[det]:11} Cllr {mean:.4f}  p {p:.3f}", flush=True)
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("panel", type=pathlib.Path)
    ap.add_argument("--folds", type=int, default=10)
    ap.add_argument("--seeds", type=int, default=20)
    ap.add_argument("--permutations", type=int, default=100)
    ap.add_argument("--out", type=pathlib.Path, default=pathlib.Path("figure1.pdf"))
    ap.add_argument(
        "--cache",
        type=pathlib.Path,
        help="reuse the computed cells from here, or write them if absent. "
        "The permutation run takes twenty minutes and the layout takes twenty seconds",
    )
    args = ap.parse_args(argv)

    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    if args.cache and args.cache.exists():
        print(f"reusing cells from {args.cache}", flush=True)
        cells = pickle.loads(args.cache.read_bytes())
    else:
        print("computing cells ...", flush=True)
        cells = gather(args.panel, args.folds, args.seeds, args.permutations)
        if args.cache:
            args.cache.write_bytes(pickle.dumps(cells))
    if not cells:
        print("no cells", file=sys.stderr)
        return 2

    fig, (ax1, ax2) = plt.subplots(
        1, 2, figsize=(7.2, 3.15), gridspec_kw={"width_ratios": [1.0, 1.2]}
    )

    # (a) The known-truth arm under both fold schemes. Its two sides carry
    # byte-identical scores, so the true Cllr is exactly 1.000 and there is no
    # argument about it. Grouping folds by cover lands on it exactly;
    # splitting pairs across folds does not, and the gap is what a draft of
    # this paper mistook for a property of the estimator.
    records = load(args.panel)
    by_arm: dict[str, dict] = collections.defaultdict(dict)
    for name, row in records.items():
        by_arm[arm_of(name)][cover_id(name)] = row
    clean_rows = by_arm["clean"]
    st_rows = next(by_arm[a] for a in by_arm if a.startswith("structural"))
    ids = sorted(set(st_rows) & set(clean_rows))
    det_key = DETECTORS[0]
    s = np.array([float(st_rows[c][det_key]) for c in ids])
    c = np.array([float(clean_rows[c][det_key]) for c in ids])
    scores = np.concatenate([s, c])
    labels = np.concatenate([np.ones(len(ids), int), np.zeros(len(ids), int)])
    pairs = np.concatenate([np.arange(len(ids)), np.arange(len(ids))])
    _, _, grouped = observed_cllr(
        scores, labels, folds=args.folds, seeds=args.seeds, groups=pairs
    )
    _, _, split = observed_cllr(scores, labels, folds=args.folds, seeds=args.seeds)

    ax1.hist(split, bins=14, color="0.80", edgecolor="0.45", linewidth=0.5,
             label="pairs split across folds")
    ax1.axvline(float(np.mean(grouped)), color="black", linewidth=1.8,
                label="folds grouped by cover")
    ax1.axvline(1.0, color="0.25", linestyle=":", linewidth=1.2,
                label="true value, 1.000")
    ax1.set_xlabel(r"$C_{llr}$")
    ax1.set_ylabel("fold seeds")
    ax1.set_title(f"(a) a known-truth arm, {SHORT[det_key]}", fontsize=9)
    ax1.legend(fontsize=6.5, frameon=False, loc="upper right")
    ax1.tick_params(labelsize=8)

    # (b) Every cell against its own null band.
    ys = np.arange(len(cells))
    for y, (arm, det, mean, sem, null, p) in enumerate(cells):
        lo, hi = np.quantile(null, 0.05), np.quantile(null, 0.95)
        ax2.plot([lo, hi], [y, y], color="0.78", linewidth=3.6, solid_capstyle="butt")
        # A cell that separates from its null is drawn open, so the reader can
        # count them without reading the table.
        filled = p > 0.05
        ax2.errorbar(
            mean,
            y,
            xerr=sem,
            fmt="o",
            color="black",
            markerfacecolor="black" if filled else "white",
            markersize=3.4,
            elinewidth=0.9,
            capsize=1.6,
            zorder=3,
        )
    ax2.axvline(1.0, color="0.25", linestyle=":", linewidth=1.0)
    ax2.set_yticks(ys)
    ax2.set_yticklabels([f"{a} {d}" for a, d, *_ in cells], fontsize=5.8)
    ax2.set_xlabel(r"$C_{llr}$")
    ax2.set_title(
        f"(b) all {len(cells)} cells; open marker is $p \\leq 0.05$", fontsize=9
    )
    ax2.tick_params(axis="x", labelsize=8)
    ax2.invert_yaxis()
    ax2.margins(y=0.01)

    fig.tight_layout()
    fig.savefig(args.out, bbox_inches="tight")
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
