#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Score SEVERAL images with Aletheia in one process, keyed by path.

    aletheia_many.py spa|rs <image> [<image> ...]

WHY THIS EXISTS BESIDE `aletheia_one.py`
----------------------------------------
Starting a container and importing numpy, imageio and aletheialib measured
about 0.62 seconds per image on the build box. For the SPA estimator, whose
actual work is 0.67 seconds, that is roughly half of every invocation spent
getting ready to work. At the Core tier, which is hundreds of thousands of
images, the overhead alone is days.

So this imports once and then answers about many images. It calls exactly the
same functions `aletheia_one.py` calls, in the same way, with the same
channel rule and the same treatment of negative estimates, so a batched run and
an unbatched run of the same corpus produce the same numbers. If the two ever
disagree the batched one is wrong, and `test_aletheia_adapters.py` holds them to
it, including the case where an image part way through a batch fails.

THE OUTPUT IS KEYED, AND THAT IS NOT A STYLE CHOICE
---------------------------------------------------
One line per image, `<path>\\t<estimate>`, where `<path>` is the path exactly
as it was given on the command line.

The obvious alternative is one number per line in the order the files were
given. It breaks the first time an image cannot be read: every later answer
shifts up a line and is recorded against the wrong image, the numbers all look
plausible, and the run reports success. A silently misattributed score is worse
than a loud failure, because the failure gets fixed and the misattribution gets
published.

A FAILED IMAGE DOES NOT STOP THE BATCH
--------------------------------------
Anything that goes wrong with one image is reported on stderr naming that
image, and no line is printed for it on stdout. The host then records that one
image as unanswered and keeps every other answer in the batch, because losing
63 good measurements to one unreadable file would be a worse trade than the
run taking slightly longer.

The exception is a failed IMPORT, which is not about any one image: it means
this container cannot do the work at all, so it exits non-zero immediately
rather than printing nothing for every file in turn and looking like a clean
sweep. That exact failure once made a 2,000 image run exit zero and produce
nothing.
"""
from __future__ import annotations

import sys


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print(__doc__.strip().splitlines()[2].strip(), file=sys.stderr)
        return 2
    method, paths = argv[1], argv[2:]

    try:
        import numpy as np
        from imageio import imread
        from aletheialib import attacks
    except Exception as e:  # noqa: BLE001 - surfaced, not swallowed
        print(f"aletheia is not importable in this container: {e}", file=sys.stderr)
        return 3

    fn = {"spa": attacks.spa_image, "rs": attacks.rs_image}.get(method)
    if fn is None:
        print(f"unknown method {method!r}; expected spa or rs", file=sys.stderr)
        return 2

    answered = 0
    for path in paths:
        try:
            img = imread(path)
            # The same channel rule as the single image adapter: Aletheia's own
            # CLI flags an image when ANY channel crosses its threshold, so the
            # maximum across channels is the statistic that matches its shipped
            # behaviour. The mean would report a weaker detector than Aletheia
            # is.
            if img.ndim == 2:
                value = float(fn(img, None))
            else:
                channels = min(3, img.shape[2])
                value = max(float(fn(img, c)) for c in range(channels))
        except Exception as e:  # noqa: BLE001 - names the image it was about
            print(f"{path}: {type(e).__name__}: {e}", file=sys.stderr)
            continue

        if not np.isfinite(value):
            print(f"{path}: estimate is not finite", file=sys.stderr)
            continue

        # Flushed per image. A batch that is killed for running over its
        # deadline has still delivered the answers it finished, and the host
        # reads what arrived rather than losing the lot to a buffer.
        print(f"{path}\t{value:.10f}", flush=True)
        answered += 1

    # Nothing answered at all is a fact about the container or the batch rather
    # than about any one image, so it exits non-zero and says so. Exiting zero
    # here is how silence becomes a measurement.
    if answered == 0:
        print(
            f"none of the {len(paths)} image(s) in this batch could be scored",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
