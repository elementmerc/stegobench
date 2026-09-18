#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""The adaptive arms: the hard case, in both domains.

WHY THIS IS THE ROUND THAT MATTERS
----------------------------------
Round 2 named three gaps. Round 3 closed the JPEG one. This closes the last and
the most serious:

    "No adaptive embedding. HUGO, WOW, S-UNIWARD and the rest are the hard
     cases and are absent."

Everything measured so far, by us or by anyone, has been an embedder that
decides *where* to write without looking at the picture. Adaptive schemes
compute a per-element cost of changing each position and concentrate the payload
where a change is cheapest to hide: texture, edges, noise. They are what the
field has spent fifteen years on and what an adversary who reads papers would
use. Measuring only the easy family reports an upper bound on an adversary
rather than an estimate of one.

TWO DOMAINS, BECAUSE THE ANSWER DIFFERS
---------------------------------------
**Spatial:** HUGO, WOW, S-UNIWARD, HILL, MiPOD, writing plus or minus 1 into
greyscale pixels. This is the family every published benchmark reports.

**JPEG:** J-UNIWARD and UERD, writing into quantised DCT coefficients. This is
the one that matters commercially, because round 3 established that both
subjects are at chance on JPEG steghide, and these are stronger than steghide.

PAIRING, AND THE TRAP THAT ALMOST GOT US TWICE
----------------------------------------------
Round 3's outguess arms were void because the stego half was re-encoded at a
different quality from the clean half, so both detectors read the compression
difference rather than the payload.

The JPEG arms here could fail the same way for a subtler reason. The cover JPEG
was written by Pillow; the stego JPEG is written by `jpeglib` after its DCT
coefficients are modified. Two encoders, two sets of habits, one confound.

So **the clean half is also written by jpeglib**, read and rewritten with no
modification at all. Both halves then come off the same writer and differ only
in the coefficients the scheme changed. The Pillow original is the common
ancestor of both and appears in neither.

The spatial arms carry their own greyscale cover for the same reason: these
schemes are defined on one channel, and the pair must be matched within the arm
even though the arm's covers are not byte-identical to the JPEG arms' covers.

SIMULATION, STATED PLAINLY
--------------------------
`conseal` simulates embedding at the optimal coding rate: it produces the change
map a perfect coder would produce, which is what every published adaptive result
rests on and is slightly harder to detect than a real coder's output. The
manifest records this per row so nobody has to infer it. `hstego`, wired
separately, pays the real syndrome-trellis overhead and is the comparison that
measures the gap.
"""
from __future__ import annotations

import argparse
import hashlib
import os
import json
import pathlib
import random
import sys
import time

import numpy as np

from tiers import TierError, covers_in_tier_order, tier_name
from PIL import Image

try:
    import conseal as cl
except ImportError:  # pragma: no cover - the message is the whole point
    print("conseal is not installed:  pip install conseal", file=sys.stderr)
    raise SystemExit(2)

try:
    import jpeglib
except ImportError:  # pragma: no cover
    jpeglib = None

#: Spatial schemes take bits per pixel.
SPATIAL = {
    "hugo": lambda a, r, s: cl.hugo.simulate_single_channel(a, r, seed=s),
    "wow": lambda a, r, s: cl.wow.simulate_single_channel(a, r, seed=s),
    "suniward": lambda a, r, s: cl.suniward.simulate_single_channel(a, r, seed=s),
    "hill": lambda a, r, s: cl.hill.simulate_single_channel(a, r, seed=s),
    "mipod": lambda a, r, s: cl.mipod.simulate_single_channel(a, r, seed=s),
}

#: JPEG schemes take bits per non-zero AC coefficient, which is a different
#: axis from bits per pixel and must not be read as the same number.
JPEG_SCHEMES = ("juniward", "uerd")

DEFAULT_RATES = (0.4, 0.2, 0.1, 0.05)


def _scratch(dest: pathlib.Path, suffix: str) -> pathlib.Path:
    """A temp path unique to this process.

    Write-then-rename is only atomic if the temp name is not shared. Six workers
    split by scheme still race to create the same greyscale cover, and with a
    fixed `.part` name the first rename removes the file the second is about to
    rename, which fails with a bare FileNotFoundError naming a path that plainly
    exists a moment earlier. The pid makes each writer's scratch file its own;
    the rename onto the final name stays atomic and last writer wins with
    identical content.
    """
    return dest.with_suffix(f".{os.getpid()}{suffix}")


def write_png(array: np.ndarray, dest: pathlib.Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = _scratch(dest, ".png.part")
    Image.fromarray(array.astype(np.uint8), mode="L").save(part, format="PNG")
    part.replace(dest)


def jpeg_passthrough(source: pathlib.Path, dest: pathlib.Path) -> None:
    """The clean half of a JPEG arm: same coefficients, same writer as the stego.

    Read and written straight back with nothing changed, so the only difference
    from its stego twin is the coefficients the scheme touched. Skipping this and
    using the Pillow-written original would put a different encoder on each side
    of the pair, which is the confound that voided round 3's outguess arms.
    """
    dest.parent.mkdir(parents=True, exist_ok=True)
    im = jpeglib.read_dct(str(source))
    part = _scratch(dest, ".jpg.part")
    im.write_dct(str(part))
    part.replace(dest)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--covers", required=True, help="Pentimento PNG covers")
    ap.add_argument("--jpeg-covers", default=None,
                    help="the round 3 clean JPEGs, for the JPEG adaptive arms")
    ap.add_argument("--out", required=True)
    ap.add_argument("--count", type=int, default=100,
                    help="covers per arm, taken in tier order: 200 is Nano, "
                         "1000 Lite, 10000 Core")
    ap.add_argument("--manifest", default=None,
                    help="default: manifest.jsonl beside the covers")
    ap.add_argument("--size", type=int, default=512)
    ap.add_argument("--seed", type=int, default=20260917)
    ap.add_argument("--rates", default=",".join(str(r) for r in DEFAULT_RATES))
    ap.add_argument("--schemes", default=",".join(sorted(SPATIAL) + list(JPEG_SCHEMES)))
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    rates = [float(r) for r in args.rates.split(",") if r.strip()]
    schemes = [s.strip() for s in args.schemes.split(",") if s.strip()]

    # Covers are taken in tier order, NOT sampled. A seeded sample is
    # reproducible and does not nest: sample(pool, 200) is not the first 200 of
    # sample(pool, 1000), so Nano would have held covers absent from Lite and
    # the prefix guarantee in distribution.md would have been false.
    covers_dir = pathlib.Path(args.covers)
    manifest = pathlib.Path(args.manifest) if args.manifest else covers_dir / "manifest.jsonl"
    try:
        chosen = covers_in_tier_order(manifest, covers_dir, args.count)
    except TierError as e:
        print(f"cannot select a tier: {e}", file=sys.stderr)
        return 1
    print(f"{len(chosen)} covers, tier order 0..{len(chosen) - 1} "
          f"[{tier_name(len(chosen))}], from {manifest}")

    jpeg_pool: list[pathlib.Path] = []
    wants_jpeg = any(s in JPEG_SCHEMES for s in schemes)
    if wants_jpeg:
        if jpeglib is None:
            print("jpeglib is missing, so the JPEG arms cannot be built",
                  file=sys.stderr)
            return 2
        if not args.jpeg_covers:
            print("--jpeg-covers is required for the JPEG adaptive arms",
                  file=sys.stderr)
            return 2
        jpeg_pool = sorted(pathlib.Path(args.jpeg_covers).glob("*.jpg"))
        if not jpeg_pool:
            print(f"no JPEGs under {args.jpeg_covers}", file=sys.stderr)
            return 2
        jpeg_pool = jpeg_pool[: len(chosen)]
        print(f"{len(jpeg_pool)} JPEG covers for the DCT arms")

    manifest = out / "manifest.jsonl"
    done = set()
    if manifest.exists():
        done = {json.loads(l)["stego"] for l in manifest.read_text().splitlines()
                if l.strip()}
        print(f"resuming: {len(done)} pairs already built")

    counts = {"pairs": 0, "failed": 0, "skipped": 0}
    last_beat = time.monotonic()

    with manifest.open("a") as mf:
        # --- spatial arms -------------------------------------------------
        spatial_schemes = [s for s in schemes if s in SPATIAL]
        if spatial_schemes:
            grey_dir = out / "clean_grey"
            for index, png in enumerate(chosen):
                stem = f"{index:05d}"
                clean = grey_dir / f"{stem}.png"
                if not clean.is_file():
                    with Image.open(png) as img:
                        img.load()
                        arr = np.asarray(img.convert("L"), dtype=np.uint8)
                    write_png(arr, clean)
                else:
                    with Image.open(clean) as img:
                        arr = np.asarray(img, dtype=np.uint8)

                for scheme in spatial_schemes:
                    for rate in rates:
                        arm = f"{scheme}/{int(rate * 1000):04d}"
                        stego = out / arm / f"{stem}.png"
                        key = str(stego.relative_to(out))
                        if key in done or stego.is_file():
                            continue
                        try:
                            seed = (args.seed + index * 7919
                                    + int(rate * 100000)) % (2 ** 31)
                            changed_arr = SPATIAL[scheme](arr, float(rate), seed)
                            changed_arr = np.clip(changed_arr, 0, 255)
                        except Exception as e:  # noqa: BLE001 - one arm, not the run
                            counts["failed"] += 1
                            print(f"  {arm}/{stem}: {type(e).__name__}: {e}",
                                  file=sys.stderr)
                            continue
                        write_png(changed_arr, stego)
                        changed = int((changed_arr.astype(np.int16)
                                       != arr.astype(np.int16)).sum())
                        mf.write(json.dumps({
                            "arm": arm, "tool": scheme, "rate": rate,
                            "domain": "spatial", "rate_unit": "bits per pixel",
                            "clean": str(clean.relative_to(out)), "stego": key,
                            "source_png": png.name,
                            "samples_changed": changed,
                            "change_rate": round(changed / arr.size, 6),
                            "coding": "simulated at the optimal rate, not a real STC",
                            "clean_sha256": hashlib.sha256(
                                clean.read_bytes()).hexdigest(),
                            "stego_sha256": hashlib.sha256(
                                stego.read_bytes()).hexdigest(),
                        }) + "\n")
                        mf.flush()
                        counts["pairs"] += 1

                if time.monotonic() - last_beat >= 60:
                    print(f"  spatial ... {index + 1}/{len(chosen)}, {counts}")
                    last_beat = time.monotonic()

        # --- JPEG arms ----------------------------------------------------
        jpeg_schemes = [s for s in schemes if s in JPEG_SCHEMES]
        if jpeg_schemes and jpeg_pool:
            jclean_dir = out / "clean_jpeg"
            for index, src in enumerate(jpeg_pool):
                stem = f"{index:05d}"
                clean = jclean_dir / f"{stem}.jpg"
                if not clean.is_file():
                    jpeg_passthrough(src, clean)
                try:
                    im = jpeglib.read_dct(str(clean))
                    y0 = im.Y.copy()
                    qt = im.qt[0]
                except Exception as e:  # noqa: BLE001
                    counts["failed"] += 1
                    print(f"  jpeg/{stem}: unreadable: {e}", file=sys.stderr)
                    continue

                spatial_for_juniward = None
                if "juniward" in jpeg_schemes:
                    with Image.open(clean) as img:
                        img.load()
                        # J-UNIWARD asserts `len(x0.shape) == 2`: a plain
                        # greyscale plane, not a single-channel 3D array. Its
                        # cost map is computed from the decoded pixels, which is
                        # why it needs the spatial side at all where UERD works
                        # from the coefficients alone.
                        spatial_for_juniward = np.asarray(
                            img.convert("L"), dtype=np.uint8)

                for scheme in jpeg_schemes:
                    for rate in rates:
                        arm = f"{scheme}/{int(rate * 1000):04d}"
                        stego = out / arm / f"{stem}.jpg"
                        key = str(stego.relative_to(out))
                        if key in done or stego.is_file():
                            continue
                        seed = (args.seed + index * 7919
                                + int(rate * 100000)) % (2 ** 31)
                        try:
                            if scheme == "uerd":
                                y1 = cl.uerd.simulate_single_channel(
                                    y0=y0, qt=qt, alpha=float(rate), seed=seed)
                            else:
                                y1 = cl.juniward.simulate_single_channel(
                                    x0=spatial_for_juniward, y0=y0, qt=qt,
                                    alpha=float(rate), seed=seed)
                        except Exception as e:  # noqa: BLE001
                            counts["failed"] += 1
                            print(f"  {arm}/{stem}: {type(e).__name__}: {e}",
                                  file=sys.stderr)
                            continue
                        changed = int((y1 != y0).sum())
                        if changed == 0:
                            counts["skipped"] += 1
                            continue
                        stego.parent.mkdir(parents=True, exist_ok=True)
                        im.Y = y1
                        part = _scratch(stego, ".jpg.part")
                        im.write_dct(str(part))
                        part.replace(stego)
                        im.Y = y0
                        mf.write(json.dumps({
                            "arm": arm, "tool": scheme, "rate": rate,
                            "domain": "jpeg-dct",
                            "rate_unit": "bits per non-zero AC coefficient",
                            "clean": str(clean.relative_to(out)), "stego": key,
                            "source_jpeg": src.name,
                            "coefficients_changed": changed,
                            "coding": "simulated at the optimal rate, not a real STC",
                            "clean_sha256": hashlib.sha256(
                                clean.read_bytes()).hexdigest(),
                            "stego_sha256": hashlib.sha256(
                                stego.read_bytes()).hexdigest(),
                        }) + "\n")
                        mf.flush()
                        counts["pairs"] += 1

                if time.monotonic() - last_beat >= 60:
                    print(f"  jpeg ... {index + 1}/{len(jpeg_pool)}, {counts}")
                    last_beat = time.monotonic()

    print(f"built {counts['pairs']} pairs into {out}")
    print(f"  {counts}")
    return 0 if counts["pairs"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
