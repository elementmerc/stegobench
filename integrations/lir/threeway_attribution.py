#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Split the conflated bound-attribution figure into its two honest halves.

WHAT WAS WRONG WITH THE ONE NUMBER
----------------------------------
`bound_attribution.py` compares a floor fitted to the RAW detector scores and
left unclipped (call it N) against a floor fitted to the REPORTED ratios and
clipped to the declared bound (call it B). The movement from N to B is two
fixes at once:

  N -> A   move the floor onto the reported ratios, so the floor is monotone in
           what the system actually said. This is the textbook definition being
           applied correctly, not a consequence of bounding, and it is what
           stops the loss going negative on an inverted detector.
  A -> B   clip the floor to the declared bound. This is the bound's own cost,
           and it is the only part of the movement the bound is responsible for.

Charging the whole movement to the bound overstates the bound's role wherever
the raw-score floor was the larger error, and it is the reason the published
range runs to -159%: on an inverted detector the raw-score floor is not a floor
at all, so the denominator is meaningless.

This recomputes the attribution as three columns, all expressed as a share of
the apparent calibration loss the naive decomposition reports:

  S1 = (cal_N - cal_A) / cal_N   the definition fix
  S2 = (cal_A - cal_B) / cal_N   the bound's own cost
  S3 = (cal_N - cal_B) / cal_N   the total, which is the published figure

S1 + S2 = S3 exactly, by construction. A fourth column carries the prior
study's normalisation, the clip's share of the movement itself,
(cal_A - cal_B) / (cal_N - cal_B), so the two can be read against each other.

The corpus is licence restricted and is not shipped; see `positive_control.py`.

Run as::

    python threeway_attribution.py [path/to/scores.jsonl] --out threeway-attr.txt
"""
from __future__ import annotations

import argparse
import collections
import json
import pathlib
import platform
import shlex
import sys

import numpy as np

from analyse_panel import roc_auc
from bounded_floor_properties import measure
from likelihood_ratio import cross_validated_lrs
from positive_control import DEFAULT_SCORES, DETECTORS

BANDS = (
    ("AUC >= 0.99", lambda a: a >= 0.99),
    ("0.9 <= AUC < 0.99", lambda a: 0.9 <= a < 0.99),
    ("0.5 <= AUC < 0.9", lambda a: 0.5 <= a < 0.9),
    ("AUC < 0.5 (inverted)", lambda a: a < 0.5),
)


def cells(path: pathlib.Path, folds: int, bound: float, seeds: int,
          subsample_seed: int) -> list[dict]:
    """Every (arm, detector) cell of the control, with all four floors.

    The picture subsample is drawn once per arm from a fixed seed, exactly as
    `bound_attribution.py` and `positive_control.py` draw it, so the cells are
    the same cells the published figure was computed on. Only the fold seed
    varies, and the reported numbers are means over it.
    """
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

    out: list[dict] = []
    for key in sorted(arms):
        stego = arms[key]
        rng = np.random.default_rng(subsample_seed)
        n = min(len(stego), len(clean))
        picked_stego = [stego[i] for i in rng.choice(len(stego), n, replace=False)]
        picked_clean = [clean[i] for i in rng.choice(len(clean), n, replace=False)]

        for det in DETECTORS:
            s = np.array([r[det] for r in picked_stego], dtype=float)
            c = np.array([r[det] for r in picked_clean], dtype=float)
            scores = np.concatenate([s, c])
            labels = np.concatenate([np.ones(n, int), np.zeros(n, int)])

            per_seed = []
            for seed in range(seeds):
                unbounded = cross_validated_lrs(
                    scores, labels, folds=folds, bound=np.inf, seed=seed
                )
                per_seed.append(
                    measure(scores, unbounded, labels, 1.0 / bound, bound)
                )
            mean = {k: float(np.mean([r[k] for r in per_seed])) for k in per_seed[0]}
            mean.update(
                arm=f"{key[0]}/{key[1]}",
                detector=det,
                n_pairs=n,
                auc=roc_auc(s, c),
                seed0={k: per_seed[0][k] for k in per_seed[0]},
            )
            out.append(mean)
    return out


def shares(row: dict) -> tuple[float, float, float, float]:
    """S1, S2, S3 and the clip's share of the movement, for one cell."""
    cal_n, cal_a, cal_b = row["cal_n"], row["cal_a"], row["cal_b"]
    if cal_n > 1e-9:
        s1 = (cal_n - cal_a) / cal_n
        s2 = (cal_a - cal_b) / cal_n
        s3 = (cal_n - cal_b) / cal_n
    else:
        s1 = s2 = s3 = float("nan")
    move = cal_n - cal_b
    clip = (cal_a - cal_b) / move if abs(move) > 1e-9 else float("nan")
    return s1, s2, s3, clip


def summarise(label: str, sub: list[dict], key) -> str:
    v = np.array([key(r) for r in sub], dtype=float)
    v = v[np.isfinite(v)]
    if not len(v):
        return f"  {label:<24}{'n/a':>8}"
    return (f"  {label:<24}{len(v):>5}{np.median(v) * 100:>12.1f}%"
            f"{v.min() * 100:>12.1f}%{v.max() * 100:>12.1f}%"
            f"{np.percentile(v, 5) * 100:>10.1f}%{np.percentile(v, 95) * 100:>10.1f}%")


def block(rows: list[dict], title: str) -> list[str]:
    lines = [title, "-" * len(title)]
    w = lines.append
    for name, key, note in (
        ("S1  definition fix   (cal_N -> cal_A) / cal_N", lambda r: shares(r)[0], ""),
        ("S2  bound's own cost (cal_A -> cal_B) / cal_N", lambda r: shares(r)[1], ""),
        ("S3  total, published (cal_N -> cal_B) / cal_N", lambda r: shares(r)[2], ""),
        ("clip share of the movement (cal_A-cal_B)/(cal_N-cal_B)",
         lambda r: shares(r)[3], ""),
    ):
        w(f"  {name}{note}")
        w(f"  {'band':<24}{'n':>5}{'median':>13}{'min':>13}{'max':>13}"
          f"{'p5':>11}{'p95':>11}")
        w(summarise("all cells", rows, key))
        for band, keep in BANDS:
            sub = [r for r in rows if keep(r["auc"])]
            if sub:
                w(summarise(band, sub, key))
        w("")
    return lines


def per_cell(rows: list[dict]) -> list[str]:
    lines = ["PER CELL", "--------"]
    w = lines.append
    w(f"  {'arm':<16}{'det':<5}{'n':>6}{'AUC':>7}{'Cllr':>8}{'cal_N':>9}"
      f"{'cal_A':>8}{'cal_B':>8}{'S1':>8}{'S2':>8}{'S3':>8}{'clip':>8}")
    for r in sorted(rows, key=lambda r: -r["auc"]):
        s1, s2, s3, clip = shares(r)
        w(f"  {r['arm']:<16}{r['detector']:<5}{r['n_pairs']:>6}{r['auc']:>7.3f}"
          f"{r['cllr']:>8.3f}{r['cal_n']:>+9.3f}{r['cal_a']:>8.3f}{r['cal_b']:>8.3f}"
          f"{s1 * 100:>7.1f}%{s2 * 100:>7.1f}%{s3 * 100:>7.1f}%{clip * 100:>7.1f}%")
    w("")
    return lines


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("scores", nargs="?", type=pathlib.Path, default=None)
    ap.add_argument("--folds", type=int, default=10)
    ap.add_argument("--bound", type=float, default=100.0)
    ap.add_argument("--seeds", type=int, default=20)
    ap.add_argument("--subsample-seed", type=int, default=0)
    ap.add_argument("--out", type=pathlib.Path)
    args = ap.parse_args(argv)

    path = args.scores or DEFAULT_SCORES
    if not path.exists():
        print(f"no scores at {path}", file=sys.stderr)
        print("This corpus is licence restricted and is not shipped.", file=sys.stderr)
        print(
            "Point at your own copy: pass it as the first argument, or set "
            "STEGOBENCH_CONTROL_SCORES.",
            file=sys.stderr,
        )
        return 2

    rows = cells(path, args.folds, args.bound, args.seeds, args.subsample_seed)

    report = [
        f"command : {shlex.join([pathlib.Path(sys.argv[0]).name] + sys.argv[1:])}",
        f"host    : {platform.node()}, python {platform.python_version()}, "
        f"numpy {np.__version__}",
        f"corpus  : {path}",
        f"cells   : {len(rows)}; bound {args.bound:g}; folds {args.folds}; "
        f"fold seeds {args.seeds}; picture subsample seed {args.subsample_seed}",
        "",
        "SPLIT BOUND ATTRIBUTION ON THE SPATIAL CONTROL",
        "=" * 45,
        "",
    ]
    report += block(rows, f"MEAN OVER {args.seeds} FOLD SEEDS")

    seed0 = []
    for r in rows:
        copy = dict(r["seed0"])
        copy.update(arm=r["arm"], detector=r["detector"], n_pairs=r["n_pairs"],
                    auc=r["auc"])
        seed0.append(copy)
    report += block(
        seed0,
        "FOLD SEED 0 ONLY, which is what bound_attribution.py published",
    )
    report += per_cell(rows)

    text = "\n".join(report) + "\n"
    if args.out:
        args.out.write_text(text, encoding="utf-8")
        print(f"wrote {args.out}")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
