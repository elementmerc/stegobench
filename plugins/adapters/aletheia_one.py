#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Score one image with Aletheia, and print the number rather than a verdict.

WHY THIS EXISTS
---------------
Aletheia's command line computes an estimate, compares it to a threshold, and
prints a sentence. The estimate is what a comparison needs: a verdict cannot
produce an ROC curve, so driving Aletheia through its CLI throws away the
statistic the whole evaluation is made of. Worse, the threshold is Aletheia's
choice of operating point, not ours, and an arm the estimator called correctly
can be recorded as a 0% detection because the sentence said "no".

So this calls the same functions the CLI calls, `aletheialib.attacks.spa_image`
and `rs_image`, unmodified, and prints the estimate. Nothing is reimplemented.
The detector is entirely Aletheia's; this only stops the answer being rounded
to a boolean on the way out.

A COLOUR IMAGE GIVES THREE ANSWERS AND WE KEEP THE LARGEST
-----------------------------------------------------------
Both estimators work one channel at a time. Aletheia's own CLI flags an image
when ANY channel crosses the threshold, so the maximum across channels is the
statistic that matches its shipped behaviour. Using the mean would report a
weaker detector than Aletheia actually is, which would be unfair to it.

NEGATIVE ESTIMATES ARE KEPT
---------------------------
Both estimators return a small negative rate on some clean images. That is
meaningful: it is the estimator's noise about zero, and it is exactly the
spread a false-positive rate is measured from. Clamping it to zero would throw
that away and make every clean image look identical.

Usage:
    aletheia_one.py <image> spa|rs
"""
from __future__ import annotations

import sys


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(__doc__.strip().splitlines()[-1], file=sys.stderr)
        return 2
    path, method = argv[1], argv[2]

    try:
        import numpy as np
        from imageio import imread
        from aletheialib import attacks
    except Exception as e:  # noqa: BLE001 - surfaced, not swallowed
        # Loudly, because the alternative is what actually happened once: a
        # missing support package made a 2,000 image run exit zero and produce
        # nothing at all.
        print(f"aletheia is not importable in this container: {e}", file=sys.stderr)
        return 3

    fn = {"spa": attacks.spa_image, "rs": attacks.rs_image}.get(method)
    if fn is None:
        print(f"unknown method {method!r}; expected spa or rs", file=sys.stderr)
        return 2

    try:
        img = imread(path)
        if img.ndim == 2:
            value = float(fn(img, None))
        else:
            channels = min(3, img.shape[2])
            value = max(float(fn(img, c)) for c in range(channels))
    except Exception as e:  # noqa: BLE001 - one image, and it says which
        print(f"{type(e).__name__}: {e}", file=sys.stderr)
        return 1

    if not np.isfinite(value):
        print("estimate is not finite", file=sys.stderr)
        return 1

    print(f"{value:.10f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
