#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Power of the paper's decision rule, measured on the paper's own covers.

WHY THIS FILE EXISTS
--------------------
`minimum_detectable_effect.py` locates the rejection boundary and says, in its
own docstring, exactly what it is not:

    A power analysis needs a stated alternative hypothesis, a distribution for
    the statistic under it, and gives power at a given effect size. A
    permutation null gives only the null ... It carries no 80% power guarantee,
    because nothing here models the sampling variability of the observed
    statistic under a real effect.

This file supplies the missing piece: the sampling variability of the statistic
under a real effect, and therefore power at a stated effect size.

WHY NOT A GAUSSIAN SIMULATION
-----------------------------
The first version of this script drew synthetic Gaussian scores, as
`mde_effect_scale.py` does, and its own transfer check refused the result. The
model null came out at a critical distance of 0.0023 of Cllr against 0.0007 to
0.0010 measured on the real cells, nearly three times too wide, and the reason
is structural rather than a bad parameter choice.

The paper's null permutes labels **within matched pairs**, holding the observed
scores fixed. It is a null conditional on the covers. Drawing a fresh synthetic
dataset per replicate is an unconditional null, and it carries the whole of the
cover to cover variation that the pairing was designed to remove. The two are
not the same reference, so a boundary taken from one and a statistic taken from
the other cannot be compared, and a power number built that way would be
meaningless in a way nothing downstream would announce.

WHAT THIS DOES INSTEAD
----------------------
It keeps the real covers and injects a known effect into them.

    base cell          a real cell measured to sit at chance, so the injected
                       shift is the whole of the effect and not an addition to
                       an unknown one
    alternative        the observed pairs with the stego member shifted by
                       `d` pooled standard deviations of that cell's scores
    resampling         a paired bootstrap over covers, which is what supplies
                       the sampling variability of the statistic
    null               the paper's own within pair permutation, recomputed for
                       every replicate, because each bootstrap resample has its
                       own null
    decision rule      the paper's own `p = (1 + #{null <= obs}) / (1 + N)`
                       at the paper's own alpha
    power at d         the fraction of replicates reaching p <= alpha

Nothing here assumes a score distribution. The one assumption is that a payload
acts on the score as an additive shift in the score's own units, which is
weaker than the Gaussian equal variance model `mde_effect_scale.py` needs and
is stated in the output.

`d` is reported alongside the AUC it produces on these scores, because AUC is
what transfers between score distributions and `d` is not.

COST
----
Each replicate runs the pipeline once and its null `--permutations` times, so
the work is `reps * (1 + permutations)` pipeline runs per separation. At the
paper's settings one run is about 0.4 s, so a replicate is about 40 s. Size the
grid accordingly and run it detached.

Run as::

    python power_curve.py panel-with-tail-covers.jsonl \\
        --arm steghide/0050 --detector aletheia_spa --out power-steghide-spa.txt
"""
from __future__ import annotations

import argparse
import concurrent.futures as futures
import math
import os
import pathlib

import numpy as np

from analyse_panel import DETECTORS, arm_of, cover_id, load, roc_auc
from likelihood_ratio import cllr_null, observed_decomposition


def replicate(job):
    """One bootstrap replicate at separation d: returns (d, p, auc, cllr)."""
    (s_clean, s_stego, d, scale, folds, bound, seeds, permutations, base_seed,
     rep) = job
    n = len(s_clean)
    rng = np.random.default_rng((base_seed, rep, int(round(d * 1e6))))
    # Paired bootstrap: a cover is resampled as a unit, so the pairing the null
    # relies on survives the resample.
    idx = rng.integers(0, n, n)
    clean_b = s_clean[idx]
    stego_b = s_stego[idx] + d * scale

    scores = np.concatenate([stego_b, clean_b])
    labels = np.concatenate([np.ones(n, int), np.zeros(n, int)])
    pairs = np.concatenate([np.arange(n), np.arange(n)])

    dec, _ = observed_decomposition(
        scores, labels, folds=folds, bound=bound, seeds=seeds
    )
    null = cllr_null(
        scores, labels, permutations=permutations, folds=folds, bound=bound,
        seed=base_seed + rep, pairs=pairs, seeds=seeds,
    )
    p = float((1 + (null <= dec.cllr).sum()) / (1 + len(null)))
    return d, p, float(roc_auc(stego_b, clean_b)), float(dec.cllr)


def wilson(k: int, n: int, z: float = 1.96) -> tuple[float, float]:
    """Wilson score interval, which behaves at a proportion of 0 or 1."""
    if n == 0:
        return (0.0, 1.0)
    p = k / n
    denom = 1.0 + z * z / n
    centre = (p + z * z / (2 * n)) / denom
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / denom
    return (max(0.0, centre - half), min(1.0, centre + half))


def crossing(xs, powers, level):
    """First x at which measured power reaches `level`, linearly interpolated.

    None when the grid never reaches it, which is a result and not a failure.
    """
    for i in range(1, len(xs)):
        if powers[i - 1] < level <= powers[i]:
            lo, hi = powers[i - 1], powers[i]
            if hi == lo:
                return xs[i]
            return xs[i - 1] + (level - lo) / (hi - lo) * (xs[i] - xs[i - 1])
    return None


def extract_cell(path: pathlib.Path, arm_name: str, det: str):
    """The paired clean and stego score vectors for one cell of the panel."""
    records = load(path)
    by_arm: dict[str, dict[str, dict]] = {}
    for name, rec in records.items():
        by_arm.setdefault(arm_of(name), {})[cover_id(name)] = rec
    if "clean" not in by_arm:
        raise SystemExit("no clean arm in this corpus, so nothing can be paired")
    clean = by_arm["clean"]
    if arm_name not in by_arm:
        raise SystemExit(
            f"arm {arm_name!r} not in corpus; have {sorted(by_arm)}"
        )
    stego = by_arm[arm_name]
    ids = [
        c for c in sorted(set(stego) & set(clean))
        if det in stego[c] and det in clean[c]
    ]
    if not ids:
        raise SystemExit(f"no paired cases for {arm_name}/{det}")
    return (
        np.array([float(clean[c][det]) for c in ids]),
        np.array([float(stego[c][det]) for c in ids]),
    )


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("panel", type=pathlib.Path)
    ap.add_argument("--arm", default="steghide/0050")
    ap.add_argument("--detector", default="aletheia_spa", choices=list(DETECTORS))
    ap.add_argument("--alpha", type=float, default=0.05)
    ap.add_argument("--permutations", type=int, default=100)
    ap.add_argument("--folds", type=int, default=10)
    ap.add_argument("--bound", type=float, default=100.0)
    ap.add_argument("--seeds", type=int, default=20, help="fold seeds, as in the paper")
    ap.add_argument("--reps", type=int, default=100, help="bootstrap replicates per d")
    ap.add_argument(
        "--separations",
        type=float,
        nargs="*",
        default=[0.0, 0.05, 0.10, 0.15, 0.20, 0.30, 0.45],
    )
    ap.add_argument("--base-seed", type=int, default=0)
    ap.add_argument("--workers", type=int, default=max(1, (os.cpu_count() or 2) - 1))
    ap.add_argument("--out", type=str, default=None)
    args = ap.parse_args(argv)

    s_clean, s_stego = extract_cell(args.panel, args.arm, args.detector)
    n = len(s_clean)
    scale = float(np.concatenate([s_clean, s_stego]).std())
    observed_auc = roc_auc(s_stego, s_clean)

    jobs = [
        (s_clean, s_stego, d, scale, args.folds, args.bound, args.seeds,
         args.permutations, args.base_seed, r)
        for d in args.separations
        for r in range(args.reps)
    ]
    total = len(jobs)
    print(
        f"{total} replicates x {1 + args.permutations} pipeline runs "
        f"on {args.workers} workers",
        flush=True,
    )

    by_d: dict[float, list[tuple[float, float, float]]] = {}
    done = 0
    with futures.ProcessPoolExecutor(max_workers=args.workers) as pool:
        for d, p, auc, cllr in pool.map(replicate, jobs, chunksize=1):
            by_d.setdefault(d, []).append((p, auc, cllr))
            done += 1
            if done % 10 == 0:
                print(f"  {done}/{total}", flush=True)

    lines = [
        f"power of the design on real covers: {args.arm} / {args.detector}",
        f"{n} paired covers, observed AUC {observed_auc:.4f} before any injection",
        f"settings: {args.folds} folds, bound {args.bound:g}, {args.seeds} fold "
        f"seeds, {args.permutations} within pair permutations, alpha "
        f"{args.alpha:g}, base seed {args.base_seed}",
        f"{args.reps} paired bootstrap replicates per separation",
        "",
        "d is a shift of the stego member in pooled score standard deviations "
        f"({scale:.4g} in this cell's units).",
        "No score distribution is assumed; the effect is assumed additive.",
        "",
        f"{'d':>7}{'AUC':>9}{'mean Cllr':>11}{'median p':>10}"
        f"{'power':>8}{'95% CI':>16}",
        "-" * 61,
    ]
    xs, powers, aucs = [], [], []
    for d in args.separations:
        rows = by_d[d]
        ps = np.array([r[0] for r in rows])
        au = np.array([r[1] for r in rows])
        cl = np.array([r[2] for r in rows])
        hits = int((ps <= args.alpha).sum())
        pw = hits / len(ps)
        lo, hi = wilson(hits, len(ps))
        xs.append(d)
        powers.append(pw)
        aucs.append(float(au.mean()))
        lines.append(
            f"{d:>7.2f}{au.mean():>9.4f}{cl.mean():>11.5f}"
            f"{np.median(ps):>10.3f}{pw:>8.3f}{f'[{lo:.3f}, {hi:.3f}]':>16}"
        )

    lines.append("")
    # At d = 0 the rule should reject at its own alpha and no more. That is the
    # check that the whole apparatus is calibrated, and it costs one grid point.
    if 0.0 in by_d:
        z = powers[xs.index(0.0)]
        lo, hi = wilson(int(z * args.reps), args.reps)
        verdict = "as it should" if lo <= args.alpha <= hi else "NOT at its nominal rate"
        lines.append(
            f"size check at d = 0: rejects {z:.3f} of the time, nominal "
            f"{args.alpha:g}, 95% CI [{lo:.3f}, {hi:.3f}] -- {verdict}"
        )
        lines.append("")

    for level in (0.50, 0.80, 0.95):
        d_star = crossing(xs, powers, level)
        if d_star is None:
            best = max(powers)
            lines.append(
                f"power {level:.0%}: not reached on this grid; highest measured "
                f"{best:.3f} at d = {xs[powers.index(best)]:.2f}"
            )
        else:
            lines.append(
                f"power {level:.0%} at d = {d_star:.3f}, "
                f"AUC = {float(np.interp(d_star, xs, aucs)):.4f}"
            )

    text = "\n".join(lines) + "\n"
    print(text)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            fh.write(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
