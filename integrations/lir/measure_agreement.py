#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Measure the agreement with `lir`, rather than quoting the test tolerances.

WHY THIS FILE EXISTS
--------------------
The paper quoted agreement of 1e-12 on Cllr and 2e-3 on the floor. Those are
the tolerances `test_against_lir.py` asserts, which is a statement about how
loosely the tests were written, not about how closely the two implementations
agree. A reviewer noticed. The real figures are much better and saying so
costs nothing.

WHAT THE COMPARISON DOES AND DOES NOT ESTABLISH
------------------------------------------------
Both implementations are fed **the same likelihood ratios**, produced by this
pipeline. What is being compared is therefore the metric, not the calibration
that produced its input: it establishes that Cllr and its decomposition are
computed correctly, and it establishes nothing at all about whether the
cross validated logistic calibration upstream of them is right. That is worth
having, and it is worth not overstating.

Run as::

    python measure_agreement.py
"""
from __future__ import annotations

import numpy as np

from likelihood_ratio import cllr, cross_validated_lrs, decompose, pav

#: Large enough that the clip never binds, so what is compared is the isotonic
#: fit rather than the bounding policy, which the two libraries differ on by
#: design.
UNBOUNDED = 1e11


def sample(separation: float, n: int = 400, seed: int = 0):
    rng = np.random.default_rng(seed)
    labels = np.array([0] * (n // 2) + [1] * (n // 2))
    scores = rng.normal(loc=separation * labels, scale=1.0)
    return scores, labels


def main() -> int:
    import lir.metrics as lir_metrics
    from lir.algorithms.isotonic_regression import IsotonicCalibrator
    from lir.data.models import LLRData

    try:
        import importlib.metadata as md

        version = md.version("lir")
    except Exception:  # pragma: no cover - only if the metadata is missing
        version = "unknown"
    print(f"lir {version}\n")

    def as_llr_data(lrs, labels):
        with np.errstate(divide="ignore"):
            llrs = np.log10(np.asarray(lrs, dtype=float))
        return LLRData(features=llrs.reshape(-1, 1), hypothesis=np.asarray(labels, int))

    separations = [-4.0, -3.0, -2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0, 3.0, 4.0]

    rel_cllr, abs_min, abs_cal, abs_iso = [], [], [], []
    for sep in separations:
        for seed in range(10):
            scores, labels = sample(sep, seed=seed)

            lrs = cross_validated_lrs(scores, labels, folds=10)
            ours = cllr(lrs[labels == 1], lrs[labels == 0])
            theirs = lir_metrics.cllr(as_llr_data(lrs, labels))
            rel_cllr.append(abs(ours - theirs) / abs(theirs))

            un = cross_validated_lrs(scores, labels, folds=10, bound=UNBOUNDED)
            d = decompose(un, labels, bound=UNBOUNDED)
            abs_min.append(abs(d.cllr_min - lir_metrics.cllr_min(as_llr_data(un, labels))))
            abs_cal.append(abs(d.cllr_cal - lir_metrics.cllr_cal(as_llr_data(un, labels))))

            # The isotonic fit itself, on balanced data where the two
            # parameterisations coincide.
            ours_post = pav(scores, labels)
            prior_odds = (labels == 1).sum() / (labels == 0).sum()
            eps = 1e-12
            ours_lr = (ours_post + eps) / (1 - ours_post + eps) / prior_odds
            theirs_llr = IsotonicCalibrator().fit_apply(
                LLRData(features=scores.reshape(-1, 1), hypothesis=labels)
            ).llrs
            theirs_lr = 10.0**theirs_llr
            finite = np.isfinite(theirs_lr) & (theirs_lr > 0) & (ours_lr < 1e10)
            if finite.any():
                abs_iso.append(
                    float(
                        np.max(np.abs(np.log10(ours_lr[finite]) - np.log10(theirs_lr[finite])))
                    )
                )

    cases = len(rel_cllr)
    print(f"{cases} cases: {len(separations)} separations from -4 to +4 sd, 10 seeds each")
    print("both implementations fed the same likelihood ratios from this pipeline\n")
    print(f"{'quantity':<28}{'worst':>12}{'median':>12}")
    print("-" * 52)
    print(f"{'Cllr, relative':<28}{max(rel_cllr):>12.2e}{np.median(rel_cllr):>12.2e}")
    print(f"{'Cllr_min, absolute':<28}{max(abs_min):>12.2e}{np.median(abs_min):>12.2e}")
    print(f"{'Cllr_cal, absolute':<28}{max(abs_cal):>12.2e}{np.median(abs_cal):>12.2e}")
    print(
        f"{'isotonic fit, log10 LR':<28}{max(abs_iso):>12.2e}{np.median(abs_iso):>12.2e}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
