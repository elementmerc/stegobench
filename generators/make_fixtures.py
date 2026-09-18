#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Build the self-test fixtures: one image that is clean, one that is not.

WHY BOTH, ALWAYS
----------------
A detector that answers "stego" to everything passes a detect-only check. One
that answers "clean" to everything passes a clear-only check. Either alone is a
control that cannot fail, and this project has produced four of those in two
days, including a rich-model extraction that ran over 2,000 images, exited zero
every time and produced nothing because a support package was missing.

So every tool is asked both questions and must get both right.

WHY THE PAYLOAD IS LOUD
-----------------------
The stego fixture carries 0.4 bits per pixel of uniform noise, which is far
more than any real adversary would use. That is the point: this is not a
measurement, it is a smoke test. A detector that cannot see a payload this
obvious is broken or misconfigured, and saying so in seconds is worth more than
being realistic. The measurement arms go down to 0.005 bpp and live elsewhere.

WHY IT IS DETERMINISTIC
-----------------------
Fixed seed, fixed dimensions, no timestamps, so two runs produce byte identical
files and the committed fixtures can be verified by anybody. A fixture that
changes between runs cannot be the reference for anything.
"""
from __future__ import annotations

import argparse
import hashlib
import pathlib
import sys

import numpy as np
from PIL import Image

#: Fixed, so the fixtures are reproducible by anyone who runs this.
SEED = 20260918
SIZE = 256


def clean_cover(rng: np.random.Generator) -> np.ndarray:
    """A synthetic photograph, textured enough to be a fair test.

    A flat or purely random image is not a fair cover: flat regions make any
    change obvious and pure noise hides everything, so either would produce a
    smoke test that says nothing about a real image. This is smooth gradients
    plus mild structure, which is closer to a photograph's statistics.
    """
    y, x = np.mgrid[0:SIZE, 0:SIZE].astype(np.float64)
    base = (
        128
        + 48 * np.sin(x / 19.0)
        + 36 * np.cos(y / 27.0)
        + 24 * np.sin((x + y) / 41.0)
    )
    grain = rng.normal(0.0, 3.0, size=(SIZE, SIZE))
    return np.clip(base + grain, 0, 255).astype(np.uint8)


def embed_lsb(cover: np.ndarray, rate_bpp: float, rng: np.random.Generator) -> np.ndarray:
    """LSB replacement at `rate_bpp`, into positions chosen without repetition.

    Replacement rather than matching, deliberately: it is the easiest thing in
    the world to detect, which is what a smoke test wants.
    """
    stego = cover.copy().reshape(-1)
    count = int(stego.size * rate_bpp)
    positions = rng.choice(stego.size, size=count, replace=False)
    bits = rng.integers(0, 2, size=count, dtype=np.uint8)
    stego[positions] = (stego[positions] & 0xFE) | bits
    changed = int(np.count_nonzero(stego != cover.reshape(-1)))
    return stego.reshape(cover.shape), changed


def write_appended(src: pathlib.Path, dst: pathlib.Path, payload: bytes) -> str:
    """A valid PNG with bytes stapled after IEND.

    WHY A THIRD FIXTURE EXISTS
    --------------------------
    One pair of fixtures cannot serve every tool, and assuming it could made
    the first self-test report zsteg as broken when zsteg was right.

    zsteg is a STRUCTURAL scanner: it reads the container and looks for
    extractable text or files, and it never decodes the picture. Our LSB
    fixture carries uniform random bits, which is exactly what it should NOT
    call a finding, so asking zsteg about it tests nothing and fails a working
    tool.

    This is the bait a structural scanner is supposed to take: the image is
    untouched, and four kilobytes of text sit past the end marker where no
    pixel-domain detector will ever look. It is also the arm StegaShield missed
    560 times out of 560.
    """
    dst.write_bytes(src.read_bytes() + payload)
    return hashlib.sha256(dst.read_bytes()).hexdigest()


def write_png(path: pathlib.Path, array: np.ndarray) -> str:
    # optimize=False keeps the encoder's behaviour stable across Pillow
    # versions; a fixture that changes when a library updates is not a fixture.
    Image.fromarray(array, mode="L").save(path, format="PNG", optimize=False)
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", default="fixtures")
    ap.add_argument("--rate", type=float, default=0.4)
    args = ap.parse_args(argv)

    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    rng = np.random.default_rng(SEED)
    cover = clean_cover(rng)
    stego, changed = embed_lsb(cover, args.rate, rng)

    if changed == 0:
        # A rate that changes nothing is a silent failure that would surface
        # later as an apparently undetectable arm.
        print("the embedding changed no samples, so the fixture is not stego",
              file=sys.stderr)
        return 1

    # Recognisable text rather than noise, because a structural scanner reports
    # what it can extract, and something a human can read in the output is the
    # difference between a passing test and a debugging session.
    appended_payload = (b"STEGOBENCH-FIXTURE-APPENDED-DATA " * 128)[:4096]

    clean_path = out / "clean.png"
    stego_path = out / f"lsb-{args.rate}bpp.png"
    clean_sha = write_png(clean_path, cover)
    stego_sha = write_png(stego_path, stego)

    expected = int(cover.size * args.rate * 0.5)
    print(f"{clean_path}  sha256 {clean_sha[:16]}")
    print(f"{stego_path}  sha256 {stego_sha[:16]}")
    print(f"{changed} samples changed, about {expected} expected "
          f"(half the payload bits already match)")
    appended_path = out / "appended.png"
    appended_sha = write_appended(clean_path, appended_path, appended_payload)
    print(f"{appended_path}  sha256 {appended_sha[:16]}  "
          f"({len(appended_payload)} bytes after IEND, pixels untouched)")

    print(f"seed {SEED}, {SIZE}x{SIZE}, re-running reproduces these exactly")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
