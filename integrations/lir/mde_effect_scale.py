#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Translate a minimum detectable effect in Cllr into one in AUC.

WHY THIS IS NEEDED
------------------
`minimum_detectable_effect.py` answers the question the paper asks, in the
units the paper reports: the smallest Cllr improvement a cell could have
flagged. On the JPEG table those come out around 0.0007 to 0.0010, which looks
so small that a reader could conclude the design would catch anything at all.

It would not, and the reason is the shape of Cllr near the uninformative point.
Cllr is flat to second order in discrimination around AUC 0.5: a detector has
to separate the two classes appreciably before Cllr moves at all. A Cllr
budget of 0.001 therefore buys a much larger AUC shift than the decimal places
suggest, and quoting the Cllr figure without this conversion invites exactly
the over-reading the MDE exists to prevent.

HOW THE MAP IS MEASURED
-----------------------
Not derived. The same pipeline is run on synthetic scores with a known
separation: clean drawn from N(0, 1), payload from N(d, 1), at the sample size
of the real cell, and the whole cross validated calibration, bound and fold
seed averaging applied unchanged. The drop is measured against the same
pipeline at d = 0 on the same seeds, which is the reference the paper's
permutation null estimates.

The Gaussian equal variance model is an assumption and the only one here. Real
detector scores are neither Gaussian nor equal variance, so the AUC figure this
produces is an order of magnitude statement about how much discrimination one
thousandth of a Cllr corresponds to, not a conversion factor to apply to a
particular cell. It is reported with that caveat attached.

Run as::

    python mde_effect_scale.py --n 197 --mde 0.00085 --out mde-effect-scale.txt
"""
from __future__ import annotations

import argparse
import concurrent.futures as futures
import os

import numpy as np

from analyse_panel import roc_auc
from likelihood_ratio import observed_decomposition


def one_point(job):
    """Mean Cllr and AUC at one separation, over several data draws."""
    n, d, folds, bound, seeds, reps, base_seed = job
    cllrs = np.empty(reps)
    aucs = np.empty(reps)
    labels = np.concatenate([np.ones(n, int), np.zeros(n, int)])
    for r in range(reps):
        rng = np.random.default_rng((base_seed, r))
        stego = rng.normal(d, 1.0, n)
        clean = rng.normal(0.0, 1.0, n)
        scores = np.concatenate([stego, clean])
        dec, _ = observed_decomposition(
            scores, labels, folds=folds, bound=bound, seeds=seeds
        )
        cllrs[r] = dec.cllr
        aucs[r] = roc_auc(stego, clean)
    return d, float(cllrs.mean()), float(cllrs.std()), float(aucs.mean())


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--n", type=int, default=197, help="cases per side, as in the cell")
    ap.add_argument(
        "--mde",
        type=float,
        action="append",
        default=None,
        help="a Cllr minimum detectable effect to convert; repeatable",
    )
    ap.add_argument("--folds", type=int, default=10)
    ap.add_argument("--bound", type=float, default=100.0)
    ap.add_argument("--seeds", type=int, default=20, help="fold seeds, as in the paper")
    ap.add_argument("--reps", type=int, default=12, help="independent data draws per point")
    ap.add_argument("--base-seed", type=int, default=0)
    ap.add_argument(
        "--separations",
        type=float,
        nargs="*",
        default=[0.0, 0.02, 0.04, 0.06, 0.08, 0.10, 0.14, 0.18, 0.25, 0.35, 0.50, 0.75, 1.0],
    )
    ap.add_argument("--workers", type=int, default=max(1, (os.cpu_count() or 2) - 1))
    ap.add_argument("--out", type=str, default=None)
    args = ap.parse_args(argv)
    if not args.mde:
        args.mde = [0.00066, 0.00085, 0.00099]

    jobs = [
        (args.n, d, args.folds, args.bound, args.seeds, args.reps, args.base_seed)
        for d in args.separations
    ]
    rows = []
    with futures.ProcessPoolExecutor(max_workers=args.workers) as pool:
        for res in pool.map(one_point, jobs):
            rows.append(res)
            print(f"  d={res[0]:.3f} AUC={res[3]:.4f} Cllr={res[1]:.5f}", flush=True)
    rows.sort(key=lambda r: r[0])

    baseline = rows[0][1]
    lines = [
        f"Cllr against known separation, {args.n} cases per side",
        f"settings: {args.folds} folds, bound {args.bound:g}, {args.seeds} fold seeds, "
        f"{args.reps} data draws per point, base seed {args.base_seed}",
        "Gaussian equal variance scores. This is a scale statement, not a per cell",
        "conversion: real detector scores are neither Gaussian nor equal variance.",
        "",
        f"{'separation d':>13}{'AUC':>9}{'Cllr':>10}{'sd':>9}{'drop vs d=0':>13}",
        "-" * 54,
    ]
    for d, c, sd, auc in rows:
        lines.append(f"{d:>13.3f}{auc:>9.4f}{c:>10.5f}{sd:>9.5f}{baseline - c:>13.5f}")

    lines += ["", f"reference at d = 0: Cllr {baseline:.5f}", ""]
    drops = np.array([baseline - c for _, c, _, _ in rows])
    ds = np.array([d for d, _, _, _ in rows])
    aucs = np.array([auc for _, _, _, auc in rows])
    for mde in sorted(args.mde):
        # The drop is monotone in d over this grid, so a straight interpolation
        # on the measured points is enough; the grid is the measurement and the
        # interpolation only reads between two of its rows.
        if drops.max() < mde:
            lines.append(f"MDE {mde:.5f} Cllr: beyond the grid, widen --separations")
            continue
        d_star = float(np.interp(mde, drops, ds))
        auc_star = float(np.interp(mde, drops, aucs))
        lines.append(
            f"MDE {mde:.5f} Cllr corresponds to separation d = {d_star:.3f}, "
            f"AUC = {auc_star:.4f}"
        )
    text = "\n".join(lines) + "\n"
    print(text)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            fh.write(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
