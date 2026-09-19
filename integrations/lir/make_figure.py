#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Draw the figure for the likelihood ratio paper.

Two panels, because the paper makes two claims and each needs one.

Left: the permutation null for a single arm, with the observed Cllr on it.
This is the method. It shows why the textbook reference of 1.0 is not the
right comparison for a cross validated pipeline, because the null itself sits
slightly above it.

Right: every JPEG arm against the null band, next to the positive control.
This is the result. The JPEG arms sit inside the band; the control does not.

Needs matplotlib, which the rest of this directory deliberately does not.
Run it with the optional lir environment.
"""
from __future__ import annotations

import argparse
import collections
import json
import pathlib
import sys

import numpy as np

from analyse_panel import arm_of, cover_id, load
from likelihood_ratio import cllr_null, observed_cllr


def gather(panel: pathlib.Path, detector: str, folds: int, seeds: int, perms: int):
    records = load(panel)
    by_arm: dict[str, dict] = collections.defaultdict(dict)
    for name, row in records.items():
        by_arm[arm_of(name)][cover_id(name)] = row
    clean = by_arm.pop("clean")

    out = []
    for arm in sorted(by_arm):
        stego = by_arm[arm]
        ids = [c for c in sorted(set(stego) & set(clean)) if detector in stego[c] and detector in clean[c]]
        if len(ids) < folds:
            continue
        s = np.array([float(stego[c][detector]) for c in ids])
        c = np.array([float(clean[c][detector]) for c in ids])
        scores = np.concatenate([s, c])
        labels = np.concatenate([np.ones(len(ids), int), np.zeros(len(ids), int)])
        pairs = np.concatenate([np.arange(len(ids)), np.arange(len(ids))])
        mean, sd, _ = observed_cllr(scores, labels, folds=folds, seeds=seeds)
        null = cllr_null(scores, labels, permutations=perms, folds=folds, pairs=pairs)
        out.append((arm, mean, sd, null))
        print(f"  {arm:18} Cllr {mean:.4f} (sd {sd:.4f})", flush=True)
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("panel", type=pathlib.Path)
    ap.add_argument("--detector", default="aletheia_spa")
    ap.add_argument("--folds", type=int, default=10)
    ap.add_argument("--seeds", type=int, default=20)
    ap.add_argument("--permutations", type=int, default=200)
    ap.add_argument("--out", type=pathlib.Path, default=pathlib.Path("figure1.pdf"))
    args = ap.parse_args(argv)

    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    print("computing arms ...", flush=True)
    arms = gather(args.panel, args.detector, args.folds, args.seeds, args.permutations)
    if not arms:
        print("no arms", file=sys.stderr)
        return 2

    fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(7.2, 2.9))

    # Left: one arm's null, with the observed value on it. The structural arm
    # is chosen because its true Cllr is exactly 1.0 by construction.
    pick = next((a for a in arms if a[0].startswith("structural")), arms[0])
    arm, mean, sd, null = pick
    ax1.hist(null, bins=28, color="0.80", edgecolor="0.45", linewidth=0.5)
    ax1.axvline(1.0, color="0.25", linestyle=":", linewidth=1.2, label="theoretical 1.000")
    ax1.axvline(mean, color="black", linewidth=1.6, label=f"observed {mean:.3f}")
    ax1.set_xlabel(r"$C_{llr}$")
    ax1.set_ylabel("permutations")
    ax1.set_title(f"(a) within-pair null, {arm}", fontsize=9)
    ax1.legend(fontsize=7, frameon=False)
    ax1.tick_params(labelsize=8)

    # Right: every arm, observed against its own null band.
    ys = np.arange(len(arms))
    labels = [a[0].replace("/0", " ").replace("structural 000", "structural") for a in arms]
    for y, (_, mean, sd, null) in enumerate(arms):
        lo, hi = np.quantile(null, 0.05), np.quantile(null, 0.95)
        ax2.plot([lo, hi], [y, y], color="0.72", linewidth=4, solid_capstyle="butt")
        ax2.errorbar(mean, y, xerr=sd, fmt="o", color="black", markersize=3.4,
                     elinewidth=0.9, capsize=1.6)
    ax2.axvline(1.0, color="0.25", linestyle=":", linewidth=1.0)
    ax2.set_yticks(ys)
    ax2.set_yticklabels(labels, fontsize=7)
    ax2.set_xlabel(r"$C_{llr}$")
    ax2.set_title("(b) every arm sits inside its null", fontsize=9)
    ax2.tick_params(axis="x", labelsize=8)
    ax2.invert_yaxis()

    fig.tight_layout()
    fig.savefig(args.out, bbox_inches="tight")
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
