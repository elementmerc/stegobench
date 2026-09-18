#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Run the classifier against a problem with no signal in it, and check it fails.

WHY
---
The rich-model baseline came back at AUC 0.93 on an arm where two shipped
detectors sit at 0.53. That is the result the report's conclusion turns on, so
it has to survive being doubted.

It also arrived with something odd attached: an out-of-bag error of 0.40, which
is close to chance, sitting beside a test accuracy of 0.82. Out-of-bag is
normally the pessimistic estimate but not by thirty points, and a test score far
better than the internal estimate is the classic shape of leakage.

So this takes the CLEAN features only, the ones with no payload anywhere in
them, splits them into two arbitrary groups, and runs the identical training and
scoring path. There is nothing to find, so anything above chance is the pipeline
inventing signal, and the 0.93 would be worthless.

    clean covers ──> half labelled "0"  ┐
                 ──> half labelled "1"  ┘──> train ──> score held-out half

A second control shuffles the real labels instead, which catches a different
mistake: features and labels drifting out of alignment during the staging and
concatenation steps.

If both land at chance, the only remaining explanation for 0.93 is that the
detector genuinely separates the classes.
"""
from __future__ import annotations

import argparse
import pathlib
import sys

import numpy as np

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from fld_ensemble import FldEnsemble  # noqa: E402
from score_arms import roc_auc  # noqa: E402


def load(work: pathlib.Path, name: str) -> np.ndarray:
    """Load one staged set, concatenating its shards in shard order."""
    parts = sorted(work.glob(f"{name}__s*.fea"))
    if not parts:
        single = work / f"{name}.fea"
        if single.is_file():
            parts = [single]
    if not parts:
        raise FileNotFoundError(f"no feature files for {name} under {work}")
    blocks = []
    for p in parts:
        a = np.loadtxt(p, dtype=np.float64)
        blocks.append(a.reshape(1, -1) if a.ndim == 1 else a)
    return np.vstack(blocks)


def evaluate(x: np.ndarray, y: np.ndarray, seed: int, learners: int,
             test_frac: float = 0.3) -> dict:
    rng = np.random.default_rng(seed)
    order = rng.permutation(len(x))
    cut = int(round(len(x) * (1 - test_frac)))
    tr, te = order[:cut], order[cut:]
    clf = FldEnsemble(n_estimators=learners, seed=seed).fit(x[tr], y[tr])
    scores = clf.decision_function(x[te]).tolist()
    labels = [bool(v) for v in y[te]]
    return {
        "auc": roc_auc(scores, labels),
        "accuracy": clf.score(x[te], y[te]),
        "oob_error": clf.oob_error_,
        "d_sub": clf.d_sub_,
        "n_train": len(tr),
        "n_test": len(te),
    }


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--work", required=True, help="the baseline run's work directory")
    ap.add_argument("--learners", type=int, default=100)
    ap.add_argument("--seed", type=int, default=20260917)
    ap.add_argument("--repeats", type=int, default=3,
                    help="different arbitrary splits; one could be lucky")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    feats = pathlib.Path(args.work) / "features"

    clean = np.vstack([load(feats, "train_clean"), load(feats, "test_clean")])
    stego = np.vstack([load(feats, "train_stego"), load(feats, "test_stego")])
    print(f"clean {clean.shape}, stego {stego.shape}")

    print("\nCONTROL 1: clean against clean, split arbitrarily")
    print("nothing distinguishes these groups, so anything above chance is invented")
    for r in range(args.repeats):
        rng = np.random.default_rng(args.seed + r)
        idx = rng.permutation(len(clean))
        half = len(clean) // 2
        x = clean[np.concatenate([idx[:half], idx[half:half * 2]])]
        y = np.hstack([np.zeros(half), np.ones(half)])
        res = evaluate(x, y, args.seed + r, args.learners)
        print(f"  run {r}: AUC {res['auc']:.4f}  accuracy {res['accuracy']:.4f}  "
              f"oob {res['oob_error']:.4f}  d_sub {res['d_sub']}")

    print("\nCONTROL 2: the real features, labels shuffled")
    print("catches features and labels drifting apart during staging")
    x_all = np.vstack([clean, stego])
    y_true = np.hstack([np.zeros(len(clean)), np.ones(len(stego))])
    for r in range(args.repeats):
        rng = np.random.default_rng(args.seed + 100 + r)
        res = evaluate(x_all, rng.permutation(y_true), args.seed + 100 + r, args.learners)
        print(f"  run {r}: AUC {res['auc']:.4f}  accuracy {res['accuracy']:.4f}  "
              f"oob {res['oob_error']:.4f}  d_sub {res['d_sub']}")

    print("\nREFERENCE: the real problem, same code path")
    res = evaluate(x_all, y_true, args.seed, args.learners)
    print(f"  AUC {res['auc']:.4f}  accuracy {res['accuracy']:.4f}  "
          f"oob {res['oob_error']:.4f}  d_sub {res['d_sub']}")
    print(f"  ({res['n_train']} train, {res['n_test']} test)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
