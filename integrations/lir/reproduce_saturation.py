#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Reproduce the offset failure in Section 4.2, exactly, from a stated seed.

WHY THIS FILE EXISTS
--------------------
The paper quoted a fitted coefficient of -7.7e-07 before standardising and
1.27 after, with no recipe attached. A reviewer could not reproduce either
number and got a different failure mode from the one described. Neither of us
could tell whether the disagreement was the claim or the setup, which is the
whole problem with a number that has no script behind it.

So: one file, one seed, one optimiser, printed with everything a reader needs
to run it themselves. Whatever it prints is what goes in the paper.

THE SETUP
---------
Two Gaussians of unit variance separated by one standard deviation, whose
AUC in expectation is Phi(1/sqrt(2)) = 0.760 and which comes out at 0.779 on
this sample. Every score is then shifted by a constant. The
shift changes nothing about the problem: the separation, the AUC and the best
achievable Cllr are all identical, because a logistic fit with a free
intercept is equivariant under translation. Any difference in the answer is
arithmetic rather than evidence.

Run as::

    python reproduce_saturation.py
"""
from __future__ import annotations

import argparse

import numpy as np
from scipy.optimize import minimize

from analyse_panel import roc_auc
from likelihood_ratio import cllr, cross_validated_lrs


def fit_raw(x: np.ndarray, y: np.ndarray, analytic: bool, x0=(0.0, 0.0)):
    """The same BFGS fit as `LogisticCalibrator`, minus the standardisation.

    `analytic` selects whether the gradient is supplied in closed form or left
    to scipy's finite differences. It turns out to be the whole story: the
    failure the paper describes needs the finite difference version, because
    what breaks is the difference quotient, not the objective.
    """

    def nll(params):
        a, b = params
        z = a * x + b
        return float(np.sum(np.logaddexp(0.0, z) - y * z))

    def grad(params):
        a, b = params
        p = 1.0 / (1.0 + np.exp(-np.clip(a * x + b, -700, 700)))
        return np.array([np.sum((p - y) * x), np.sum(p - y)])

    res = minimize(nll, x0=list(x0), method="BFGS", jac=grad if analytic else None)
    return res, float(np.linalg.norm(grad(res.x))) / len(y)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--n", type=int, default=500, help="cases per class")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--separation", type=float, default=1.0)
    ap.add_argument(
        "--offsets",
        type=float,
        nargs="+",
        default=[0.0, 1e2, 1e3, 1e4, 1e5, 1e6],
    )
    args = ap.parse_args(argv)

    rng = np.random.default_rng(args.seed)
    base_payload = rng.normal(args.separation, 1.0, args.n)
    base_clean = rng.normal(0.0, 1.0, args.n)
    labels = np.concatenate([np.ones(args.n, int), np.zeros(args.n, int)])
    base = np.concatenate([base_payload, base_clean])

    print(f"n = {args.n} per class, separation {args.separation:g} sd, seed {args.seed}")
    print(f"AUC = {roc_auc(base_payload, base_clean):.4f}")
    print("optimiser: scipy BFGS, x0 = (0, 0), both gradient modes\n")

    y = labels.astype(float)
    header = (
        f"{'offset':>9} | {'finite difference gradient':^38} | "
        f"{'closed form gradient':^38}"
    )
    sub = (
        f"{'':>9} | {'coef':>12}{'success':>9}{'|grad|/n':>10}{'Cllr':>7} | "
        f"{'coef':>12}{'success':>9}{'|grad|/n':>10}{'Cllr':>7}"
    )
    print(header)
    print(sub)
    print("-" * len(sub))

    for offset in args.offsets:
        scores = base + offset
        cells = []
        for analytic in (False, True):
            res, resid = fit_raw(scores, y, analytic=analytic)
            # What that fit would have reported, with the prior odds of this
            # balanced design (1.0) divided out and the usual bound applied.
            z = res.x[0] * scores + res.x[1]
            lrs = np.clip(np.exp(np.clip(z, -700, 700)), 1 / 100.0, 100.0)
            value = cllr(lrs[labels == 1], lrs[labels == 0])
            cells.append(
                f"{res.x[0]:>12.3e}{str(bool(res.success)):>9}{resid:>10.1e}{value:>7.3f}"
            )
        print(f"{offset:>9.0e} | {cells[0]} | {cells[1]}")

    centre, scale = float(base.mean()), float(base.std())
    res_std, _ = fit_raw((base - centre) / scale, y, analytic=True)
    lrs_std = cross_validated_lrs(base + args.offsets[-1], labels, folds=10, seed=0)
    print()
    print(f"standardised coefficient: {res_std.x[0]:.4f}, the same at every offset")
    print(
        f"the pipeline's cross validated Cllr at offset {args.offsets[-1]:.0e}: "
        f"{cllr(lrs_std[labels == 1], lrs_std[labels == 0]):.4f}"
    )
    print()
    print("coef       the slope BFGS reaches on the unstandardised scores")
    print("success    scipy's own convergence flag, which is the thing not to trust")
    print("|grad|/n   the true gradient at the answer, scaled by sample size;")
    print("           `LogisticCalibrator` refuses a fit above 1e-3 on this")
    print("Cllr       what that fit would have reported")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
