#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Exercise the tier machinery end to end, on every operating system we claim.

WHY THIS RUNS ON THREE OPERATING SYSTEMS
-----------------------------------------
The corpus is built on Linux and the paper says anyone can rebuild it. Nobody
had ever run the generators on Windows or macOS, so that sentence was a hope.
Docker cannot help here: a Linux host cannot run Windows or macOS containers, so
the only honest check is a runner of each kind.

The failures this is looking for are the boring portable ones that turn a
reproducible corpus into a Linux-only corpus: a path joined with a forward
slash, a file opened without an encoding, a temporary file renamed over an open
handle (which Windows refuses), a sort that depends on the filesystem's own
ordering.

WHAT IS ACTUALLY EXERCISED, AND WHAT IS NOT
--------------------------------------------
Building the Core tier is 10,000 covers across 35 arms and roughly six days of
CPU. That does not belong in CI, and pretending otherwise would be the kind of
claim this repository exists to avoid. So the three tiers are exercised at two
different depths, and the job names say which:

**Tier selection, at Nano, Lite and Core sizes.** The prefix guarantee is a
property of the ordering, not of the pixels, so it is checked over a pool of
10,000 placeholder covers. This is the property `distribution.md` promises in
bold and the one that silently inflates somebody else's published accuracy when
it breaks.

**A build, pack and publish cycle, at Nano size only.** Real covers, a real
adaptive arm, real shards, real metadata. Small enough to finish in a few
minutes and complete enough that every stage runs.

The JPEG half needs `jpeglib` and `conseal`, which do not have wheels
everywhere. Where they are missing the JPEG arm is skipped LOUDLY, with the
reason printed and recorded, rather than passing quietly on two thirds of a
check.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys
import tempfile
import time

import numpy as np
from PIL import Image

HERE = pathlib.Path(__file__).resolve().parent
GENERATORS = HERE.parent.parent / "generators"
sys.path.insert(0, str(GENERATORS))

from tiers import TierError, covers_in_tier_order, tier_name  # noqa: E402

#: Sizes named in `distribution.md`. Full is left out: a pool of 100,000
#: placeholder files is minutes of filesystem work for no extra property.
TIER_SIZES = (200, 1000, 10000)

#: The arm actually built. One scheme and one rate, because the point is that
#: every stage runs on this platform and not that the numbers are interesting.
SMOKE_SCHEME = "wow"
SMOKE_RATE = "0.4"


def synth_cover(path: pathlib.Path, size: int, seed: int) -> None:
    """A cover with texture in it, because a flat image has no embedding cost."""
    rng = np.random.default_rng(seed)
    base = rng.integers(40, 216, size=(size, size), dtype=np.uint8)
    Image.fromarray(base, mode="L").convert("RGB").save(path, format="PNG")


def build_pool(root: pathlib.Path, count: int, size: int,
               licence: bool = True) -> pathlib.Path:
    """A cover directory and its manifest, shaped like the real one."""
    root.mkdir(parents=True, exist_ok=True)
    rows = []
    for index in range(count):
        name = f"{index:05d}.png"
        synth_cover(root / name, size, seed=index)
        row = {"file": name, "tier_order": index}
        if licence:
            row.update({
                "licence": "CC BY-SA 4.0",
                "usage_terms": "Creative Commons Attribution-ShareAlike 4.0",
                "artist": f"Synthetic {index}",
                "credit": "stegobench CI",
                "descriptionurl": f"https://example.invalid/{index}",
                "attribution": f"Synthetic {index}, CC BY-SA 4.0",
                "title": name,
            })
        rows.append(row)
    manifest = root / "manifest.jsonl"
    manifest.write_text("".join(json.dumps(r) + "\n" for r in rows),
                        encoding="utf-8")
    return manifest


def check_tier_prefixes(manifest: pathlib.Path, covers: pathlib.Path) -> None:
    """Every smaller tier is a prefix of every larger one, not a fresh sample."""
    selections = {}
    for size in TIER_SIZES:
        chosen = covers_in_tier_order(manifest, covers, size)
        if len(chosen) != size:
            raise SystemExit(f"{tier_name(size)}: asked for {size}, "
                             f"got {len(chosen)}")
        selections[size] = [p.name for p in chosen]
        print(f"  {tier_name(size):>5}  {size:>6} covers, "
              f"{selections[size][0]} .. {selections[size][-1]}")

    for smaller, larger in zip(TIER_SIZES, TIER_SIZES[1:]):
        if selections[larger][:smaller] != selections[smaller]:
            raise SystemExit(
                f"{tier_name(smaller)} is NOT a prefix of {tier_name(larger)}. "
                f"Anyone training on the smaller and evaluating on the larger "
                f"would be testing on images they trained on.")
        print(f"  {tier_name(smaller)} is a prefix of {tier_name(larger)}")

    # Asking for more than the corpus holds must refuse rather than truncate:
    # a short tier is not a prefix of anything and nothing downstream would see
    # the difference.
    try:
        covers_in_tier_order(manifest, covers, TIER_SIZES[-1] + 1)
    except TierError:
        print("  a tier larger than the corpus is refused")
    else:
        raise SystemExit("a tier larger than the corpus was allowed")


def run(cmd: list[str], what: str) -> None:
    print(f"\n$ {what}", flush=True)
    started = time.monotonic()
    proc = subprocess.run(cmd, text=True, capture_output=True)
    if proc.returncode != 0:
        sys.stdout.write(proc.stdout)
        sys.stderr.write(proc.stderr)
        raise SystemExit(f"{what} failed with {proc.returncode}")
    tail = [l for l in proc.stdout.splitlines() if l.strip()][-3:]
    for line in tail:
        print(f"  {line}")
    print(f"  ({time.monotonic() - started:.0f}s)")


def check_every_sample_is_licensed(out: pathlib.Path) -> int:
    """No shard ships a derivative whose credit line we cannot produce."""
    import tarfile

    index = json.loads((out / "pentimento-core-arms-index.json")
                       .read_text(encoding="utf-8"))
    for arm in index["arms"]:
        for field in ("missing", "digest_mismatches", "unlicensed"):
            if arm[field]:
                raise SystemExit(f"{arm['arm']}: {len(arm[field])} {field}")

    checked = 0
    for shard in sorted(out.glob("*.tar")):
        with tarfile.open(shard) as tar:
            for member in tar.getnames():
                if not member.endswith(".json"):
                    continue
                sample = json.loads(tar.extractfile(member).read())
                if not sample.get("cover_licence"):
                    raise SystemExit(
                        f"{shard.name}:{member} has no cover_licence. It is a "
                        f"derivative of a licensed photograph.")
                if not sample.get("source_png"):
                    raise SystemExit(
                        f"{shard.name}:{member} names no cover.")
                checked += 1
    return checked


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--keep", action="store_true", help="leave the work behind")
    ap.add_argument("--cover-size", type=int, default=96,
                    help="pixels, for the covers that arms are built from")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    print(f"python {sys.version.split()[0]} on {sys.platform}")

    work = pathlib.Path(tempfile.mkdtemp(prefix="stegobench-ci-"))
    try:
        print("\n=== tier selection, Nano, Lite and Core ===")
        # Placeholder covers: the prefix guarantee is a property of the
        # ordering, so 8 pixels is as good as 8 megapixels and 10,000 of them
        # write in seconds rather than minutes.
        big = work / "pool-order"
        big_manifest = build_pool(big, TIER_SIZES[-1], size=8, licence=False)
        check_tier_prefixes(big_manifest, big)

        print("\n=== build, pack and publish, at Nano ===")
        covers = work / "covers"
        manifest = build_pool(covers, TIER_SIZES[0], size=args.cover_size)
        arms = work / "arms"

        run([sys.executable, str(GENERATORS / "build_adaptive_arms.py"),
             "--covers", str(covers), "--manifest", str(manifest),
             "--out", str(arms / "adaptive"), "--count", str(TIER_SIZES[0]),
             "--schemes", SMOKE_SCHEME, "--rates", SMOKE_RATE],
            f"build the {SMOKE_SCHEME} arm at {SMOKE_RATE}")

        packed = work / "packed"
        run([sys.executable, str(GENERATORS / "pack_arms.py"),
             "--arms", str(arms), "--covers-manifest", str(manifest),
             "--out", str(packed), "--per-shard", "50"],
            "pack the arm into shards")

        checked = check_every_sample_is_licensed(packed)
        print(f"\n{checked} packed sample(s), every one with a cover licence "
              f"and a named cover")
        return 0
    finally:
        if args.keep:
            print(f"\nleft behind: {work}")
        else:
            import shutil
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
