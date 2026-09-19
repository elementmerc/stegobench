#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""The FLD ensemble classifier, which is what turns rich-model features into a detector.

WHY THIS EXISTS RATHER THAN A CALL INTO ALETHEIA
------------------------------------------------
Aletheia can do this, through its `e4s` command, and this does not use it. Its
implementation shells out to Octave, downloads the Octave code from a remote
host at first run, and prints a licence it expects a human to accept at a
prompt. The detector containers here run with no network and no terminal, and
a benchmark that needs an interactive download is not reproducible by the
person receiving it.

So the features come from Aletheia, which is the part that matters and the part
that is hard, and the classifier is here. That split is honest: SRM is a
published feature set with a reference implementation used here unmodified,
and the FLD ensemble is a published algorithm short enough to write down.

WHAT IT IS, IN PLAIN TERMS
--------------------------
A rich-model feature vector has about 34,000 numbers in it and a training set
has a couple of thousand images. Every classifier that tries to weigh all 34,000
at once fails, because you cannot estimate that many relationships from that few
examples: the maths needs to invert a matrix that is singular by construction.

The trick is to stop trying. Train a few hundred weak classifiers, give each one
a small random handful of the features and a random sample of the images, and
let them vote. Each is too simple to overfit, they are wrong in uncorrelated
ways, and the vote is far better than any of them.

    34,000 features                one learner sees ~1,600 of them
    ┌────────────────────┐         ┌───┐ ┌───┐ ┌───┐        ┌───┐
    │████████████████████│   ───>  │ ▓ │ │ ▓ │ │ ▓ │  ...   │ ▓ │   L learners
    └────────────────────┘         └─┬─┘ └─┬─┘ └─┬─┘        └─┬─┘
                                     └─────┴──┬──┴────────────┘
                                          majority vote

Each weak learner is a Fisher Linear Discriminant: the one direction along which
the two classes' means are furthest apart relative to their spread. One line,
one threshold.

Kodovsky, Fridrich and Holub, "Ensemble Classifiers for Steganalysis of Digital
Media", IEEE TIFS 2012. The subspace dimension is chosen here the way that paper
chooses it, by minimising out-of-bag error rather than by guessing.

WHY THE OUT-OF-BAG SEARCH MATTERS AND IS NOT OPTIONAL
-----------------------------------------------------
Each learner is trained on a bootstrap sample, so roughly a third of the images
were not used by any given learner. Those are free test data, already paid for.
Scoring each image using only the learners that never saw it gives an honest
error estimate without touching the held-out set, which is what lets the subspace
size be tuned without spending the test data to do it.

A fixed subspace size would be a guess, and the right value moves with the
payload and the feature set. Guessing it and then reporting the test number is
how a baseline ends up weaker than it should be, which in this evaluation would
understate the very thing being measured.
"""
from __future__ import annotations

import sys
import time

import numpy as np

#: Subspace sizes to search. The paper's own search is coarse and multiplicative
#: for the same reason: the error curve is flat near its minimum, so a fine
#: search costs time and buys nothing.
DEFAULT_SUBSPACES = (200, 400, 800, 1600, 3200)

#: Ridge added to the within-class scatter before inversion. A subspace can still
#: be rank deficient when two features are duplicates, which happens in SRM
#: because some submodels overlap by construction. Without this the inversion
#: raises on a minority of draws and the ensemble silently loses those learners.
RIDGE = 1e-10


class FldEnsemble:
    """Bagged Fisher discriminants on random feature subspaces.

    `fit` chooses the subspace size by out-of-bag error unless one is given.
    `decision_function` returns the fraction of learners voting stego, which is
    a score in [0, 1] and is what the ROC is computed from. It is deliberately
    not a probability and should not be read as one.
    """

    def __init__(self, n_estimators: int = 200, d_sub: int | None = None,
                 seed: int = 0, verbose: bool = False) -> None:
        if n_estimators < 1:
            raise ValueError("n_estimators must be at least 1")
        self.n_estimators = n_estimators
        self.d_sub = d_sub
        self.seed = seed
        self.verbose = verbose
        self._learners: list[tuple[np.ndarray, np.ndarray, float]] = []

    # ── one weak learner ──────────────────────────────────────────────
    @staticmethod
    def _fisher(xc: np.ndarray, xs: np.ndarray) -> tuple[np.ndarray, float] | None:
        """The direction separating two classes, and the midpoint threshold.

        Returns None when the subspace is degenerate beyond what the ridge can
        rescue, so the caller can drop the learner rather than propagate a NaN
        into the vote, which would be a silently wrong answer.
        """
        mu_c, mu_s = xc.mean(axis=0), xs.mean(axis=0)
        dc, ds = xc - mu_c, xs - mu_s
        within = dc.T @ dc + ds.T @ ds
        within.flat[:: within.shape[0] + 1] += RIDGE * np.trace(within) / within.shape[0]
        try:
            w = np.linalg.solve(within, mu_s - mu_c)
        except np.linalg.LinAlgError:
            return None
        if not np.all(np.isfinite(w)):
            return None
        return w, float(w @ (mu_c + mu_s) / 2.0)

    # ── fit ───────────────────────────────────────────────────────────
    def fit(self, x: np.ndarray, y: np.ndarray) -> "FldEnsemble":
        x = np.asarray(x, dtype=np.float64)
        y = np.asarray(y).astype(int).ravel()
        if x.ndim != 2:
            raise ValueError(f"features must be 2D, got shape {x.shape}")
        if len(x) != len(y):
            raise ValueError(f"{len(x)} feature rows against {len(y)} labels")
        classes = set(np.unique(y).tolist())
        if classes != {0, 1}:
            raise ValueError(f"labels must be exactly 0 and 1, found {sorted(classes)}")

        n, d = x.shape
        # A subspace wider than the bootstrap sample makes the scatter singular
        # for every learner, so the whole ensemble is empty and the failure looks
        # like a modelling result rather than a configuration mistake.
        usable = [s for s in DEFAULT_SUBSPACES if s <= min(d, n // 3)]
        if self.d_sub is not None:
            if self.d_sub > d:
                raise ValueError(f"d_sub {self.d_sub} exceeds {d} features")
            candidates = [self.d_sub]
        elif usable:
            candidates = usable
        else:
            candidates = [max(1, min(d, n // 3))]

        best = (np.inf, candidates[0])
        for d_sub in candidates:
            err = self._oob_error(x, y, d_sub)
            if self.verbose:
                print(f"  d_sub {d_sub:>5}: out-of-bag error {err:.4f}", file=sys.stderr)
            if err < best[0]:
                best = (err, d_sub)
        self.oob_error_ = best[0]
        self.d_sub_ = best[1]

        rng = np.random.default_rng(self.seed)
        self._learners = []
        idx0, idx1 = np.flatnonzero(y == 0), np.flatnonzero(y == 1)
        last = time.monotonic()
        for i in range(self.n_estimators):
            feats = rng.choice(d, size=self.d_sub_, replace=False)
            b0 = rng.choice(idx0, size=len(idx0), replace=True)
            b1 = rng.choice(idx1, size=len(idx1), replace=True)
            fit = self._fisher(x[np.ix_(b0, feats)], x[np.ix_(b1, feats)])
            if fit is not None:
                self._learners.append((feats, fit[0], fit[1]))
            if self.verbose and time.monotonic() - last >= 30:
                print(f"  ... {i + 1}/{self.n_estimators} learners", file=sys.stderr)
                last = time.monotonic()

        if not self._learners:
            raise RuntimeError(
                "every learner was degenerate, so there is no classifier. "
                f"features={d}, samples={n}, d_sub={self.d_sub_}. "
                "This usually means the feature file is constant or duplicated."
            )
        return self

    def _oob_error(self, x: np.ndarray, y: np.ndarray, d_sub: int,
                   n_probe: int = 40) -> float:
        """Error measured only on images each learner never saw."""
        rng = np.random.default_rng(self.seed + 977 * d_sub)
        n, d = x.shape
        idx0, idx1 = np.flatnonzero(y == 0), np.flatnonzero(y == 1)
        votes = np.zeros(n)
        seen = np.zeros(n)
        for _ in range(n_probe):
            feats = rng.choice(d, size=d_sub, replace=False)
            b0 = rng.choice(idx0, size=len(idx0), replace=True)
            b1 = rng.choice(idx1, size=len(idx1), replace=True)
            fit = self._fisher(x[np.ix_(b0, feats)], x[np.ix_(b1, feats)])
            if fit is None:
                continue
            w, b = fit
            oob = np.setdiff1d(np.arange(n), np.union1d(b0, b1), assume_unique=False)
            if not len(oob):
                continue
            votes[oob] += (x[np.ix_(oob, feats)] @ w > b).astype(float)
            seen[oob] += 1
        scored = seen > 0
        if not scored.any():
            return 1.0
        pred = (votes[scored] / seen[scored]) > 0.5
        return float(np.mean(pred != y[scored].astype(bool)))

    # ── use ───────────────────────────────────────────────────────────
    def decision_function(self, x: np.ndarray) -> np.ndarray:
        if not self._learners:
            raise RuntimeError("fit has not been called")
        x = np.asarray(x, dtype=np.float64)
        votes = np.zeros(len(x))
        for feats, w, b in self._learners:
            votes += (x[:, feats] @ w > b).astype(float)
        return votes / len(self._learners)

    def predict(self, x: np.ndarray) -> np.ndarray:
        return (self.decision_function(x) > 0.5).astype(int)

    def score(self, x: np.ndarray, y: np.ndarray) -> float:
        return float(np.mean(self.predict(x) == np.asarray(y).astype(int).ravel()))
