#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Whether a candidate crop is a usable steganographic cover at all.

WHERE THIS CAME FROM
--------------------
Running the deduplication store over the first 200 real covers flagged two
"duplicates" that were nothing of the kind. Looked at by eye, three of those
images were flat fields of colour: one olive, one pale grey, one blue, each with
no structure whatever. They are different pictures, and the gradient hash
collided because a picture with no gradients gives a hash made of rounding
noise.

The false match was the symptom. The disease is that a flat crop should never
have been a candidate. Centre cropping a large stock photograph to 512 pixels
lands in sky, studio backdrop or bokeh often enough to matter: 3 of 200 here,
about 1.5%.

WHY A FLAT COVER IS WORSE THAN USELESS
--------------------------------------
It distorts results in both directions at once.

Content-adaptive schemes (HUGO, WOW, S-UNIWARD, HILL, MiPOD) work by computing
what each pixel costs to change and spending the payload where changes hide:
texture, edges, noise. Handed a flat field they have nowhere cheap to go, so
they either fail outright or pile changes into a region where any detector sees
them. Measured on such covers, an adaptive scheme looks far weaker than it is.

LSB methods go the other way. On a flat field the least significant bit plane is
nearly constant, so flipping bits into it produces an anomaly a first-year
detector catches at any payload. Measured on such covers, a detector looks far
stronger than it is.

Neither number says anything about the real world, and mixing a few of these
into an arm quietly shifts its average.

THE THRESHOLD, AND WHY IT IS THIS NUMBER
----------------------------------------
Texture is the mean absolute 4-neighbour Laplacian: for every pixel, how far it
sits from the average of the four around it. It reads local structure and
ignores brightness, so a dark photograph and a bright one with the same detail
score the same.

The floor is **1.0 grey level**, and it is an argument rather than a taste.
Steganographic embedding perturbs a sample by one grey level. When the mean
local structure of an image is below one grey level, the perturbation is larger
than everything the picture itself is doing, and the cover is degenerate by
definition rather than by preference. Above that the choice becomes a matter of
degree, and degree is not something to guess at.

Measured across those 200 covers: p0 = 0.03, p1 = 0.91, p5 = 1.84, p50 = 12.5,
p100 = 108.6. The floor removes the bottom 1%.

RECORD EVERYTHING, REJECT ONLY THE DEGENERATE
---------------------------------------------
Both measurements are written to the manifest for every cover, including the
ones that pass. That is deliberate (baseline Section 3.7): a threshold recorded
alongside the data can be raised later from the manifest alone, with no
refetching, and the effect of raising it can be measured rather than argued
about. A threshold applied at fetch time and then forgotten cannot.

Saturation clipping is recorded on the same principle and deliberately NOT
gated. Clipped pixels (pure 0 or pure 255) cannot move in one of the two
directions, which matters to plus-or-minus-1 schemes. It was measured before
being trusted as a problem: across the same 200 covers the median is 0.6% and
exactly one image exceeds 20%. There is no gate here because there is nothing
yet to gate, and a threshold invented for a problem nobody has measured is a
threshold that fires on something else later.
"""
from __future__ import annotations

import argparse
import dataclasses
import json
import pathlib
import sys

import numpy as np
from PIL import Image

# Below this the one grey level an embedder moves a sample by exceeds the mean
# local structure of the whole picture. See the module docstring.
TEXTURE_FLOOR = 1.0

# Not a gate. Covers between the floor and here are usable but thin on detail,
# and a corpus whose share of them is climbing is a corpus whose cover source
# has drifted. Reported, never enforced.
LOW_TEXTURE = 4.0


@dataclasses.dataclass(frozen=True)
class Quality:
    texture: float
    clipped: float
    usable: bool
    reason: str | None = None

    @property
    def low_texture(self) -> bool:
        return self.usable and self.texture < LOW_TEXTURE

    def as_dict(self) -> dict[str, object]:
        return {
            "texture": round(self.texture, 4),
            "clipped": round(self.clipped, 6),
            "usable": self.usable,
            "reason": self.reason,
        }


def texture(img: Image.Image) -> float:
    """Mean absolute 4-neighbour Laplacian, in grey levels.

    Reads local structure and ignores overall brightness: a dark photograph and
    a bright one carrying the same detail score the same.
    """
    a = np.asarray(img.convert("L"), dtype=np.float64)
    if a.shape[0] < 3 or a.shape[1] < 3:
        raise ValueError(f"{a.shape[1]}x{a.shape[0]} is too small to measure texture")
    lap = (
        4 * a[1:-1, 1:-1]
        - a[:-2, 1:-1] - a[2:, 1:-1]
        - a[1:-1, :-2] - a[1:-1, 2:]
    )
    return float(np.abs(lap).mean())


def clipped_fraction(img: Image.Image) -> float:
    """Share of samples pinned at pure black or pure white.

    Those samples cannot move in one of the two directions, which is a real
    constraint on plus-or-minus-1 embedding. Recorded, not gated.
    """
    a = np.asarray(img.convert("RGB"))
    return float(((a == 0) | (a == 255)).mean())


def assess(img: Image.Image) -> Quality:
    t = texture(img)
    c = clipped_fraction(img)
    if t < TEXTURE_FLOOR:
        return Quality(
            t, c, False,
            f"texture {t:.3f} is below {TEXTURE_FLOOR}: the picture varies less "
            "from pixel to pixel than the one grey level an embedder moves a "
            "sample by, so it cannot hold a payload meaningfully",
        )
    return Quality(t, c, True)


def assess_file(path: pathlib.Path) -> Quality:
    with Image.open(path) as img:
        img.load()
        return assess(img)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("directory")
    ap.add_argument("--json", action="store_true",
                    help="one JSON object per image instead of a summary")
    args = ap.parse_args(argv)

    root = pathlib.Path(args.directory)
    if not root.is_dir():
        print(f"not a directory: {root}", file=sys.stderr)
        return 2
    paths = sorted(p for p in root.rglob("*")
                   if p.suffix.lower() in (".png", ".jpg", ".jpeg", ".webp", ".bmp"))
    if not paths:
        print(f"no images under {root}", file=sys.stderr)
        return 1

    scores, unusable, low = [], [], 0
    for path in paths:
        try:
            q = assess_file(path)
        except Exception as e:  # noqa: BLE001 - name the file that failed
            print(f"  could not assess {path.name}: {e}", file=sys.stderr)
            continue
        scores.append(q.texture)
        if not q.usable:
            unusable.append((path.name, q))
        elif q.low_texture:
            low += 1
        if args.json:
            print(json.dumps({"file": str(path.relative_to(root)), **q.as_dict()}))

    if args.json:
        return 0

    arr = np.array(scores)
    print(f"{len(scores)} images under {root}")
    print("  texture percentiles: " + ", ".join(
        f"p{q}={np.percentile(arr, q):.2f}" for q in (0, 1, 5, 50, 95, 100)
    ))
    print(f"  unusable (below {TEXTURE_FLOOR}): {len(unusable)}")
    for name, q in unusable:
        print(f"    {name}  texture={q.texture:.3f}")
    print(f"  low texture (below {LOW_TEXTURE}, kept): {low}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
