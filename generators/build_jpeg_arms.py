#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""The JPEG arms: the gap round 2 named and could not fill.

WHY THIS ROUND EXISTS
---------------------
An earlier evaluation round closed with three limitations written
down, and this closes the one that matters most:

    "No JPEG-domain hiding. Everything here is spatial. The JPEG-DCT blind
     spot is untested against either tool."

It matters because JPEG is what real imagery is. the detector under test accepts JPEG
uploads, and a detector that has only ever been measured on PNG has been
measured on the minority case. It also matters for us: Stegcore's own JPEG-DCT
blind spot is a documented frontier, so this arm measures both tools on ground
neither has been tested on.

PAIRING, WHICH IS THE WHOLE EXPERIMENT
--------------------------------------
Every number a detector produces is a difference between a cover and its stego
twin. If the two halves differ in any way other than the payload, the detector
is measuring that difference instead, and the result is worthless. The classic
version of this mistake is comparing a PNG cover against a JPEG stego and
discovering, with great excitement, that you can detect JPEG compression.

So the cover written here is the JPEG, not the PNG it came from:

    cover.png  ->  cover.jpg at a fixed quality   <- the CLEAN half
    cover.jpg  ->  steghide  ->  stego.jpg        <- the STEGO half

Both halves are JPEGs, both went through the same encoder at the same setting,
and the only difference between them is the embedded payload. The PNG is the
common ancestor of both and appears in neither arm.

WHY THESE TOOLS
---------------
`steghide` and `outguess` are the two JPEG-native tools that people actually
run, and they fail differently, which is the point of having both. Steghide
swaps coefficient pairs so the histogram barely moves. Outguess embeds and then
deliberately corrects the histogram it disturbed, which is what defeats a plain
chi-squared test and costs it capacity it never advertises.

THE STRUCTURAL ARM IS THE CONTROL THAT ANSWERS A LIVE QUESTION
--------------------------------------------------------------
Round 2's headline finding was that the detector under test returned byte-identical scores
on 120 of 120 pairs where 4 kB had been appended after a PNG's end marker: it
decodes the image and never looks at the container. The JPEG structural arm
appends after the JPEG end-of-image marker instead. If the scores are identical
again, the finding generalises across formats and the recommendation gets
simpler, not more complicated.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import pathlib
import random
import sys

from tiers import TierError, covers_in_tier_order, tier_name
import time

from PIL import Image

from embedders import EmbedError
from tools import OutguessEmbedder, SteghideEmbedder

# Appended after the end-of-image marker. Chosen to be obviously non-image and
# fixed in size, so the arm varies in exactly one thing.
TRAILER = b"PENTIMENTO-STRUCTURAL-ARM-" + bytes(range(256)) * 16

#: Payload as a fraction of what the tool says the cover can hold. Rates are
#: relative because steghide's capacity depends on the picture's content, so a
#: fixed byte count would be a different rate on every cover.
DEFAULT_RATES = (0.5, 0.2, 0.05)


def jpeg_of(png: pathlib.Path, dest: pathlib.Path, quality: int) -> None:
    """The clean half: the cover as a JPEG, which is what both halves will be."""
    with Image.open(png) as img:
        img.load()
        bare = Image.frombytes(img.mode, img.size, img.tobytes()).convert("RGB")
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_suffix(".jpg.part")
    bare.save(part, format="JPEG", quality=quality, subsampling=0, optimize=False)
    part.replace(dest)


def append_after_eoi(source: pathlib.Path, dest: pathlib.Path) -> int:
    """Attach bytes after the JPEG end-of-image marker, leaving pixels alone."""
    raw = source.read_bytes()
    if not raw.endswith(b"\xff\xd9"):
        raise EmbedError(f"{source.name} does not end with a JPEG EOI marker")
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_suffix(".jpg.part")
    part.write_bytes(raw + TRAILER)
    part.replace(dest)
    return len(TRAILER)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--covers", required=True, help="the Pentimento cover directory")
    ap.add_argument("--out", required=True)
    ap.add_argument("--count", type=int, default=200,
                    help="covers per arm, taken in tier order: 200 is Nano, "
                         "1000 Lite, 10000 Core")
    ap.add_argument("--manifest", default=None,
                    help="default: manifest.jsonl beside the covers")
    ap.add_argument("--quality", type=int, default=95,
                    help="JPEG quality for BOTH halves of every pair")
    ap.add_argument("--seed", type=int, default=20260917)
    ap.add_argument("--rates", default=",".join(str(r) for r in DEFAULT_RATES),
                    help="payload sizes as a fraction of the tool's reported capacity")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    covers_dir = pathlib.Path(args.covers)
    out = pathlib.Path(args.out)
    rates = [float(r) for r in args.rates.split(",") if r.strip()]

    # Covers are taken in tier order, NOT sampled.
    #
    # A seeded sample is reproducible, which is why the previous version looked
    # correct, and it is still wrong: sample(pool, 200) and sample(pool, 1000)
    # from one seed do not nest, so Nano would have contained covers absent from
    # Lite. distribution.md promises a tier is a prefix of one ordering, and a
    # prefix is the only thing that makes the promise true.
    manifest = pathlib.Path(args.manifest) if args.manifest else covers_dir / "manifest.jsonl"
    try:
        chosen = covers_in_tier_order(manifest, covers_dir, args.count)
    except TierError as e:
        print(f"cannot select a tier: {e}", file=sys.stderr)
        return 1
    print(f"{len(chosen)} covers, tier order 0..{len(chosen) - 1} "
          f"[{tier_name(len(chosen))}], from {manifest}")

    # Cover SELECTION is no longer random, but the payloads still are, and they
    # still have to be reproducible. Seeded here rather than where the covers
    # used to be chosen, because the two jobs are separate and conflating them
    # is what let one edit silently remove the other.
    rng = random.Random(args.seed)

    # Outguess re-encodes; it must do so at the quality both halves share.
    embedders = [SteghideEmbedder(), OutguessEmbedder(quality=args.quality)]
    for e in embedders:
        if not e.available():
            print(f"{e.id} is not available here; the arm would be a gap in the "
                  "result rather than a warning to scroll past", file=sys.stderr)
            return 2

    manifest = out / "manifest.jsonl"
    out.mkdir(parents=True, exist_ok=True)
    done = set()
    if manifest.exists():
        done = {json.loads(l)["stego"] for l in manifest.read_text().splitlines()
                if l.strip()}
        print(f"resuming: {len(done)} pairs already built")

    counts = {"clean": 0, "pairs": 0, "skipped": 0, "failed": 0}
    last_beat = time.monotonic()

    with manifest.open("a") as mf:
        for index, png in enumerate(chosen):
            stem = f"{index:05d}"
            clean = out / "clean" / f"{stem}.jpg"
            if not clean.is_file():
                jpeg_of(png, clean, args.quality)
                counts["clean"] += 1

            # Capacity is one subprocess per tool per cover, so it is measured
            # once here rather than once per rate. Outguess in particular costs
            # a container start, and probing it three times per cover tripled
            # the build for no new information.
            room_for: dict[str, int] = {}
            for embedder in embedders:
                try:
                    room_for[embedder.id] = embedder.capacity(clean)
                except EmbedError as e:
                    print(f"  {embedder.id}/{stem}: {e}", file=sys.stderr)

            jobs: list[tuple[str, object, float]] = [
                (e.id, e, rate) for e in embedders for rate in rates
            ]
            for tool_id, embedder, rate in jobs:
                arm = f"{tool_id}/{int(rate * 1000):04d}"
                stego = out / arm / f"{stem}.jpg"
                key = str(stego.relative_to(out))
                if key in done or stego.is_file():
                    continue
                room = room_for.get(tool_id)
                if room is None:
                    counts["failed"] += 1
                    continue
                # A tool cannot hide more in a file than the file contains, so a
                # reported capacity above the cover's own size is a misparsed
                # number rather than a generous tool. Caught here because the
                # first run at scale died on one: the payload built from it was
                # large enough to exhaust memory, which is a loud failure by
                # luck rather than by design.
                ceiling = clean.stat().st_size
                if room > ceiling:
                    counts["failed"] += 1
                    print(f"  {arm}/{stem}: {tool_id} reported {room} bytes of "
                          f"capacity in a {ceiling} byte file, which cannot be "
                          "right; skipping rather than trusting it",
                          file=sys.stderr)
                    continue
                try:
                    payload_bytes = max(16, int(room * rate))
                    if payload_bytes > room:
                        counts["skipped"] += 1
                        continue
                    payload = rng.randbytes(payload_bytes)
                    result = embedder.embed(clean, payload, stego)
                except EmbedError as e:
                    counts["failed"] += 1
                    print(f"  {arm}/{stem}: {e}", file=sys.stderr)
                    continue
                mf.write(json.dumps({
                    "arm": arm, "tool": tool_id, "rate": rate,
                    "clean": str(clean.relative_to(out)), "stego": key,
                    "source_png": png.name,
                    "capacity_bytes": room, "payload_bytes": result.payload_bytes,
                    "jpeg_quality": args.quality,
                    "clean_sha256": hashlib.sha256(clean.read_bytes()).hexdigest(),
                    "stego_sha256": hashlib.sha256(stego.read_bytes()).hexdigest(),
                    "detail": result.detail,
                }) + "\n")
                mf.flush()
                counts["pairs"] += 1

            # The structural arm: same pixels, extra bytes after the end marker.
            structural = out / "structural/0000" / f"{stem}.jpg"
            if not structural.is_file():
                try:
                    added = append_after_eoi(clean, structural)
                    mf.write(json.dumps({
                        "arm": "structural/0000", "tool": "append_after_eoi",
                        "rate": 0.0,
                        "clean": str(clean.relative_to(out)),
                        "stego": str(structural.relative_to(out)),
                        "source_png": png.name,
                        "payload_bytes": added, "jpeg_quality": args.quality,
                        "clean_sha256": hashlib.sha256(clean.read_bytes()).hexdigest(),
                        "stego_sha256": hashlib.sha256(
                            structural.read_bytes()).hexdigest(),
                        "detail": {"pixels_identical": True},
                    }) + "\n")
                    mf.flush()
                    counts["pairs"] += 1
                except EmbedError as e:
                    counts["failed"] += 1
                    print(f"  structural/{stem}: {e}", file=sys.stderr)

            if time.monotonic() - last_beat >= 60:
                print(f"  ... {index + 1}/{len(chosen)} covers, {counts}")
                last_beat = time.monotonic()

    print(f"built {counts['pairs']} pairs over {counts['clean']} clean covers "
          f"into {out}")
    print(f"  {counts}")
    return 0 if counts["pairs"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
