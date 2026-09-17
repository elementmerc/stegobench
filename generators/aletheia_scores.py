#!/usr/bin/env python3
"""Run inside the Aletheia container and emit the numbers its CLI throws away.

WHY THIS EXISTS
---------------
Aletheia's structural detectors estimate a payload: SPA and RS both return an
embedding rate, a continuous number. The command line compares that number to a
threshold and prints "No hidden data found", discarding it.

A verdict cannot produce an ROC curve. Round 2 of this evaluation found exactly
that failure in our own harness, where thresholding a score before recording it
turned an arm the engine had called correctly into a recorded 0% detection. The
number has to survive to the scoring step or the comparison is not a comparison.

So this imports the same functions the CLI calls, `aletheialib.attacks.spa_image`
and `rs_image`, and writes the estimates out per image as JSON. Nothing is
reimplemented: the detector is Aletheia's, unmodified, and this only stops the
answer being rounded to a boolean on the way out.

A COLOUR IMAGE GIVES THREE ANSWERS AND WE KEEP THE LARGEST
-----------------------------------------------------------
Both detectors work one channel at a time. Aletheia's own CLI flags an image
when ANY channel crosses the threshold, so the maximum across channels is the
statistic that matches its shipped behaviour, and using the mean instead would
report a weaker detector than Aletheia actually is. Greyscale images have one
channel and the question does not arise.

Usage, from outside the container:

    docker run --rm --network=none \\
      -v "$CORPUS:/images:ro" -v "$PWD:/driver:ro" \\
      --entrypoint python3 stegobench/aletheia:pinned \\
      /driver/aletheia_scores.py /images
"""
from __future__ import annotations

import argparse
import json
import pathlib
import sys
import time

VALID = {".png", ".jpg", ".jpeg", ".bmp", ".tif", ".tiff"}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("images", help="a directory of images, or one image")
    ap.add_argument("--out", default="-", help="JSONL destination, - for stdout")
    ap.add_argument("--aletheia", default="/opt/aletheia",
                    help="where the checkout lives inside the container")
    args = ap.parse_args(argv)

    # The CLI gets this for free by being run as /opt/aletheia/aletheia.py, which
    # puts its own directory on the path. A driver invoked from anywhere else
    # does not, and the failure is a bare ModuleNotFoundError that looks like a
    # broken image rather than a missing path entry.
    if args.aletheia not in sys.path:
        sys.path.insert(0, args.aletheia)

    import numpy as np
    from imageio.v2 import imread
    import aletheialib.attacks as attacks

    root = pathlib.Path(args.images)
    if root.is_dir():
        files = sorted(p for p in root.rglob("*") if p.suffix.lower() in VALID)
    elif root.is_file():
        files = [root]
    else:
        print(f"no such path: {root}", file=sys.stderr)
        return 2
    if not files:
        print(f"no images under {root}", file=sys.stderr)
        return 2

    sink = sys.stdout if args.out == "-" else open(args.out, "w")
    failures = 0
    last = time.monotonic()
    try:
        for n, path in enumerate(files, 1):
            record: dict = {"file": str(path.relative_to(root) if root.is_dir() else path.name)}
            try:
                img = imread(path)
                # A palette or alpha channel would change the channel count under
                # the detector and silently alter what is being measured, so the
                # shape is recorded rather than assumed.
                channels = 1 if img.ndim == 2 else img.shape[2]
                record["channels"] = int(channels)
                for name, fn in (("spa", attacks.spa_image), ("rs", attacks.rs_image)):
                    if img.ndim == 2:
                        value = float(fn(img, None))
                    else:
                        value = max(float(fn(img, c)) for c in range(min(3, channels)))
                    # Both estimators can return a small negative rate on a clean
                    # image, which is meaningful: it is the estimator's noise
                    # about zero. Clamping it here would throw away exactly the
                    # spread the false-positive rate is measured from.
                    record[name] = value if np.isfinite(value) else None
            except Exception as e:  # noqa: BLE001 - one image must not end the run
                failures += 1
                record["error"] = f"{type(e).__name__}: {e}"
            sink.write(json.dumps(record) + "\n")
            sink.flush()
            if time.monotonic() - last >= 30:
                print(f"  ... {n}/{len(files)} scored, {failures} failed", file=sys.stderr)
                last = time.monotonic()
    finally:
        if sink is not sys.stdout:
            sink.close()

    print(f"scored {len(files)} images, {failures} failed", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
