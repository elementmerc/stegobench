#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""How small an effect each published cell could have detected.

WHY THIS FILE EXISTS
--------------------
`analyse_panel.py` and `positive_control.py` report a permutation p per cell
and throw the null distribution away. Most of the JPEG cells come back with a
high p, and a high p is not evidence that nothing is there. The defensible
claim in the other direction is a **minimum detectable effect**: the smallest
Cllr improvement that this design, at this sample size and this permutation
count, would have flagged at p <= 0.05.

The null already contains that number and no new framework is needed to get
it. The decision rule in both scripts is

    p = (1 + #{null <= observed}) / (1 + permutations)

so p <= alpha requires #{null <= observed} <= floor(alpha * (1 + N)) - 1,
which means the observed Cllr must land strictly below the (k+1)-th smallest
value of the null, where k is that count. That order statistic IS the critical
value. Expressed as a distance from the centre of the null it is the minimum
detectable effect, in the Cllr units the paper already reports.

WHAT THIS IS NOT
----------------
It is not a power analysis. A power analysis needs a stated alternative
hypothesis, a distribution for the statistic under it, and gives power at a
given effect size. A permutation null gives only the null, so the honest claim
is one about the rejection boundary: an effect smaller than the MDE could not
have been flagged, and an effect larger than it would have been flagged in the
draw the null represents. It carries no 80% power guarantee, because nothing
here models the sampling variability of the observed statistic under a real
effect.

Two tails are reported, deliberately. The paper's own Limitations section
records that a faint signal can make the calibrator fit a small non zero
coefficient and so push Cllr *up*, which means a genuine effect does not
always move the statistic down. `mde_down` is the boundary for the improving
direction the verdicts test. `mde_up` is the boundary of the "worse than its
null" verdict at p > 0.95. Anything between the two boundaries is invisible to
this design in either direction.

Run as::

    python minimum_detectable_effect.py jpeg path/to/panel.jsonl \\
        --permutations 100 --out mde-jpeg.txt
    python minimum_detectable_effect.py control path/to/scores.jsonl \\
        --permutations 50 --out mde-control.txt

The settings default to the ones the published tables were produced with, so a
default run is comparable to them cell for cell. The recomputed p is printed
next to the published decision rule as a self check: it must reproduce the p in
the corresponding table, and the script says so per cell.
"""
from __future__ import annotations

import argparse
import collections
import concurrent.futures as futures
import json
import math
import os
import pathlib
import sys

import numpy as np

from analyse_panel import arm_of, cover_id, roc_auc
from analyse_panel import load as load_panel
from likelihood_ratio import cllr_null, observed_decomposition

JPEG_DETECTORS = ("aletheia_spa", "aletheia_rs", "stegexpose")
CONTROL_DETECTORS = ("spa", "rs", "ws")


def critical_index(permutations: int, alpha: float) -> int:
    """Index into the sorted null of the value the observation must beat.

    The observed statistic reaches p <= alpha exactly when the number of null
    draws at or below it is small enough, and the largest such count is
    ``floor(alpha * (1 + N)) - 1``. The observation must therefore sit strictly
    below the null value at that index (zero based), which is the (count + 1)-th
    smallest draw.
    """
    k_max = math.floor(alpha * (1 + permutations)) - 1
    if k_max < 0:
        raise ValueError(
            f"{permutations} permutations cannot reach p <= {alpha:g}; the floor is "
            f"{1 / (1 + permutations):.4f}"
        )
    return k_max


def upper_critical_index(permutations: int, alpha: float) -> int:
    """Index into the sorted null for the 'worse than its null' boundary.

    The mirror of `critical_index`: p >= 1 - alpha requires the count at or
    below the observation to be at least ``ceil((1 - alpha) * (1 + N)) - 1``,
    and the observation must reach the null value at that index.
    """
    k_min = math.ceil((1 - alpha) * (1 + permutations)) - 1
    return min(k_min, permutations - 1)


def summarise(name_arm, name_det, n_pairs, auc, decomposition, sd, null, alpha):
    null = np.sort(np.asarray(null, dtype=float))
    obs = decomposition.cllr
    centre_mean = float(null.mean())
    centre_median = float(np.median(null))
    lo_idx = critical_index(len(null), alpha)
    hi_idx = upper_critical_index(len(null), alpha)
    crit_low = float(null[lo_idx])
    crit_high = float(null[hi_idx])
    p_value = float((1 + (null <= obs).sum()) / (1 + len(null)))
    return {
        "arm": name_arm,
        "detector": name_det,
        "n_pairs": int(n_pairs),
        "auc": float(auc),
        "cllr": float(obs),
        "cllr_sd": float(sd),
        "cllr_min": float(decomposition.cllr_min),
        "cllr_cal": float(decomposition.cllr_cal),
        "p": p_value,
        "permutations": int(len(null)),
        "null_mean": centre_mean,
        "null_median": centre_median,
        "null_sd": float(null.std()),
        "null_min": float(null[0]),
        "null_max": float(null[-1]),
        "crit_low": crit_low,
        "crit_high": crit_high,
        "mde_down": centre_mean - crit_low,
        "mde_up": crit_high - centre_mean,
        "observed_effect": centre_mean - obs,
        "shortfall_ratio": (centre_mean - obs) / (centre_mean - crit_low),
        "null": null.tolist(),
    }


def run_cell(job):
    """One cell, start to finish. Deterministic and independent of the others."""
    scores = np.asarray(job["scores"], dtype=float)
    labels = np.asarray(job["labels"], dtype=int)
    pairs = np.asarray(job["pairs"], dtype=int) if job["pairs"] is not None else None
    d, sd = observed_decomposition(
        scores, labels, folds=job["folds"], bound=job["bound"], seeds=job["seeds"]
    )
    null = cllr_null(
        scores,
        labels,
        permutations=job["permutations"],
        folds=job["folds"],
        bound=job["bound"],
        seed=job["seed"],
        pairs=pairs,
        seeds=job["seeds"],
    )
    return summarise(
        job["arm"], job["detector"], job["n_pairs"], job["auc"], d, sd, null, job["alpha"]
    )


def jpeg_jobs(path: pathlib.Path, args):
    records = load_panel(path)
    by_arm: dict[str, dict[str, dict]] = collections.defaultdict(dict)
    for name, row in records.items():
        by_arm[arm_of(name)][cover_id(name)] = row
    if "clean" not in by_arm:
        raise SystemExit("no clean arm in this corpus, so nothing can be paired")
    clean = by_arm.pop("clean")

    jobs = []
    for arm in sorted(by_arm):
        stego = by_arm[arm]
        paired = sorted(set(stego) & set(clean))
        for det in JPEG_DETECTORS:
            ids = [c for c in paired if det in stego[c] and det in clean[c]]
            if len(ids) < args.folds:
                print(f"skipping {arm}/{det}: only {len(ids)} scored", file=sys.stderr)
                continue
            s_stego = np.array([float(stego[c][det]) for c in ids])
            s_clean = np.array([float(clean[c][det]) for c in ids])
            jobs.append(
                {
                    "arm": arm,
                    "detector": det,
                    "n_pairs": len(ids),
                    "auc": roc_auc(s_stego, s_clean),
                    "scores": np.concatenate([s_stego, s_clean]).tolist(),
                    "labels": np.concatenate(
                        [np.ones(len(ids), int), np.zeros(len(ids), int)]
                    ).tolist(),
                    "pairs": np.concatenate([np.arange(len(ids)), np.arange(len(ids))]).tolist(),
                    "folds": args.folds,
                    "bound": args.bound,
                    "seeds": args.seeds,
                    "seed": args.seed,
                    "permutations": args.permutations,
                    "alpha": args.alpha,
                }
            )
    return jobs, f"{len(clean)} covers in the clean arm, within pair null"


def control_jobs(path: pathlib.Path, args):
    rows = [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    clean = [r for r in rows if r["label"] == "clean"]
    arms: dict[tuple, list] = collections.defaultdict(list)
    for r in rows:
        if r["label"] == "stego":
            arms[(r["tool"], r["variant"])].append(r)

    jobs = []
    for key in sorted(arms):
        stego = arms[key]
        # Identical subsampling to `positive_control.py`: one draw per arm,
        # shared across the three detectors, from a generator seeded per arm.
        rng = np.random.default_rng(args.seed)
        n = min(len(stego), len(clean))
        stego_idx = rng.choice(len(stego), n, replace=False)
        clean_idx = rng.choice(len(clean), n, replace=False)
        picked_stego = [stego[i] for i in stego_idx]
        picked_clean = [clean[i] for i in clean_idx]
        for det in CONTROL_DETECTORS:
            s = np.array([r[det] for r in picked_stego], dtype=float)
            c = np.array([r[det] for r in picked_clean], dtype=float)
            jobs.append(
                {
                    "arm": f"{key[0]}/{key[1]}",
                    "detector": det,
                    "n_pairs": n,
                    "auc": roc_auc(s, c),
                    "scores": np.concatenate([s, c]).tolist(),
                    "labels": np.concatenate([np.ones(n, int), np.zeros(n, int)]).tolist(),
                    # Not cover paired, so a free permutation is the right null,
                    # exactly as in `positive_control.py`.
                    "pairs": None,
                    "folds": args.folds,
                    "bound": args.bound,
                    "seeds": args.seeds,
                    "seed": args.seed,
                    "permutations": args.permutations,
                    "alpha": args.alpha,
                }
            )
    return jobs, f"{len(clean)} clean pictures, free null"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("table", choices=("jpeg", "control"))
    ap.add_argument("scores", type=pathlib.Path)
    ap.add_argument("--permutations", type=int, default=None)
    ap.add_argument("--folds", type=int, default=10)
    ap.add_argument("--seeds", type=int, default=20)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--bound", type=float, default=100.0)
    ap.add_argument("--alpha", type=float, default=0.05)
    ap.add_argument("--workers", type=int, default=max(1, (os.cpu_count() or 2) - 1))
    ap.add_argument("--out", type=pathlib.Path, default=None, help="text table to write")
    ap.add_argument("--json", type=pathlib.Path, default=None, help="full nulls, for reuse")
    args = ap.parse_args(argv)

    if args.permutations is None:
        args.permutations = 100 if args.table == "jpeg" else 50
    if not args.scores.exists():
        print(f"no scores at {args.scores}", file=sys.stderr)
        return 2

    builder = jpeg_jobs if args.table == "jpeg" else control_jobs
    jobs, provenance = builder(args.scores, args)
    print(f"{len(jobs)} cells, {args.workers} workers, {provenance}", flush=True)

    results = [None] * len(jobs)
    with futures.ProcessPoolExecutor(max_workers=args.workers) as pool:
        pending = {pool.submit(run_cell, job): i for i, job in enumerate(jobs)}
        done = 0
        for fut in futures.as_completed(pending):
            i = pending[fut]
            results[i] = fut.result()
            done += 1
            print(f"  {done}/{len(jobs)} {results[i]['arm']}/{results[i]['detector']}", flush=True)

    lines = render(results, args, provenance)
    text = "\n".join(lines) + "\n"
    print(text)
    if args.out:
        args.out.write_text(text, encoding="utf-8")
        print(f"written to {args.out}", file=sys.stderr)
    if args.json:
        args.json.write_text(json.dumps(results, indent=1), encoding="utf-8")
        print(f"nulls written to {args.json}", file=sys.stderr)
    return 0


def render(results, args, provenance):
    lines = [
        f"minimum detectable effect: {args.table} table, {args.scores.name}",
        provenance,
        f"settings: {args.folds} folds, bound {args.bound:g}, {args.seeds} fold seeds, "
        f"{args.permutations} permutations, seed {args.seed}, alpha {args.alpha:g}",
        f"the smallest p this many permutations can report is "
        f"{1 / (1 + args.permutations):.4f}",
        "",
        "MDE(down) is the null mean minus the critical order statistic: the smallest",
        "Cllr improvement on the null centre that would have reached p <= alpha.",
        "MDE(up) is the mirror at p >= 1 - alpha, the 'worse than its null' boundary.",
        "obs eff is the null mean minus the observed Cllr, so it is positive when the",
        "observation went the way a real signal would take it. ratio is obs eff over",
        "MDE(down): at or above 1.0 the cell is flagged, below it the cell is not.",
        "",
    ]
    header = (
        f"{'arm':<16}{'det':<14}{'n':>6}{'AUC':>7}{'Cllr':>9}{'nullmean':>10}"
        f"{'crit':>10}{'MDE(dn)':>10}{'MDE(up)':>10}{'obs eff':>10}{'ratio':>8}{'p':>7}"
    )
    lines += [header, "-" * len(header)]
    for r in results:
        lines.append(
            f"{r['arm']:<16}{r['detector']:<14}{r['n_pairs']:>6}{r['auc']:>7.3f}"
            f"{r['cllr']:>9.5f}{r['null_mean']:>10.5f}{r['crit_low']:>10.5f}"
            f"{r['mde_down']:>10.5f}{r['mde_up']:>10.5f}{r['observed_effect']:>10.5f}"
            f"{r['shortfall_ratio']:>8.2f}{r['p']:>7.3f}"
        )
    mdes = np.array([r["mde_down"] for r in results])
    lines += [
        "",
        f"MDE(down) across {len(results)} cells: min {mdes.min():.5f}, "
        f"median {np.median(mdes):.5f}, max {mdes.max():.5f}",
    ]
    lines += ["", "LaTeX rows (arm, detector, Cllr, MDE down, obs effect, ratio, p):"]
    for r in results:
        lines.append(
            f"{r['arm'].replace('_', ' ')} & {r['detector'].replace('_', ' ')} & "
            f"{r['cllr']:.3f} & {r['mde_down']:.4f} & {r['observed_effect']:+.4f} & "
            f"{r['shortfall_ratio']:.2f} & {r['p']:.3f} \\\\"
        )
    return lines


if __name__ == "__main__":
    raise SystemExit(main())
