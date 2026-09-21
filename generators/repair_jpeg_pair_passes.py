#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Give the clean JPEG half the second writer pass its stego twin already had.

WHAT WAS WRONG
--------------
The stego half of every J-UNIWARD and UERD sample was produced by reading the
clean half and writing it back with modified coefficients. That is one more
`jpeglib` write than the clean half had been through, and `jpeglib` prepends a
JFIF APP0 segment on every write. So the two halves of every pair differed by a
marker that has nothing to do with any payload::

    clean  FFE0 FFE0       FFDB FFDB FFC0 ...
    stego  FFE0 FFE0 FFE0  FFDB FFDB FFC0 ...

Counting APP0 segments then separates stego from clean perfectly, across eight
arms and 80,000 images, without reading a single coefficient. Measured on 200
pairs in each of three arms: 200 of 200 clean at two segments, 200 of 200 stego
at three, zero pairs with matching headers.

WHY ONLY THE CLEAN HALF IS REBUILT
----------------------------------
A DCT-domain read and write leaves the coefficients untouched, which was
checked rather than assumed. So passing the existing clean half through
`jpeglib` once more changes its header to match the stego half and changes
nothing else. The stego images are already correct with respect to their
coefficients and stay exactly as they are.

That makes this a 10,000 file repair rather than an 80,000 file rebuild, and it
leaves every published stego digest untouched.

WHAT IT DOES NOT DO
-------------------
It does not repack. Run `pack_arms.py` afterwards, which now refuses a pair
whose container headers differ, so a partial repair cannot reach a shard.

Usage::

    python repair_jpeg_pair_passes.py --arms ~/pentimento/arms/core --dry-run
    python repair_jpeg_pair_passes.py --arms ~/pentimento/arms/core
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import shutil
import sys

import jpeglib
import numpy as np


def header(path: pathlib.Path) -> list[tuple[int, int]]:
    """Every marker and length before the first scan.

    The whole container preamble, not just the APP0 count, because matching on
    the thing that was wrong last time is how the next one gets through.
    """
    data = path.read_bytes()
    out: list[tuple[int, int]] = []
    i = 2
    while i < len(data) - 1:
        if data[i] != 0xFF:
            break
        marker = data[i + 1]
        if marker == 0xDA:  # start of scan; everything after is entropy coded
            break
        if marker in (0xD8, 0xD9):
            i += 2
            continue
        length = int.from_bytes(data[i + 2:i + 4], "big")
        out.append((marker, length))
        i += 2 + length
    return out


def repair(clean: pathlib.Path, working: pathlib.Path, dry_run: bool) -> bool:
    """One more pass over `clean`, keeping the one-pass copy as `working`.

    Returns whether anything was written. Refuses rather than writes if the
    round trip moved a coefficient, because that would mean the repair is not
    the no-op on content it is supposed to be.
    """
    before = jpeglib.read_dct(str(clean))
    coefficients = before.Y.copy()

    if not dry_run:
        working.parent.mkdir(parents=True, exist_ok=True)
        if not working.is_file():
            shutil.copy2(clean, working)
        part = clean.with_suffix(".jpg.repair")
        before.write_dct(str(part))
        after = jpeglib.read_dct(str(part))
        if not np.array_equal(after.Y, coefficients):
            part.unlink(missing_ok=True)
            raise RuntimeError(
                f"{clean.name}: the round trip changed coefficients, so this "
                "is not the content-preserving repair it claims to be")
        part.replace(clean)
    return True


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--arms", required=True)
    ap.add_argument("--group", default="adaptive")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--limit", type=int, default=0)
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    root = pathlib.Path(args.arms) / args.group
    clean_dir = root / "clean_jpeg"
    working_dir = root / "clean_jpeg_pass1"
    if not clean_dir.is_dir():
        print(f"no clean_jpeg under {root}", file=sys.stderr)
        return 1

    cleans = sorted(clean_dir.glob("*.jpg"))
    if args.limit:
        cleans = cleans[: args.limit]
    print(f"{len(cleans):,} clean JPEG(s) in {clean_dir}")

    # One representative stego, to state the target rather than assume it.
    sample_stego = next((root / "juniward" / "0050").glob("*.jpg"), None)
    if sample_stego is None:
        print("no juniward/0050 arm to compare against", file=sys.stderr)
        return 1
    target = header(sample_stego)
    print(f"target header, from {sample_stego.name}: "
          f"{[hex(m) for m, _ in target]}")

    done = failed = 0
    for n, clean in enumerate(cleans, 1):
        try:
            repair(clean, working_dir / clean.name, args.dry_run)
            done += 1
        except Exception as e:  # noqa: BLE001
            failed += 1
            print(f"  {clean.name}: {e}", file=sys.stderr)
        if n % 500 == 0:
            print(f"  {n:,}/{len(cleans):,}")

    print(f"\n{'would repair' if args.dry_run else 'repaired'}: {done:,}"
          + (f", failed: {failed:,}" if failed else ""))

    if not args.dry_run and not failed:
        # Check the result rather than trust the loop, on a sample large enough
        # to catch a systematic failure.
        checked = mismatched = 0
        for clean in cleans[:: max(1, len(cleans) // 200)]:
            stego = root / "juniward" / "0050" / clean.with_suffix(".jpg").name
            if not stego.is_file():
                continue
            checked += 1
            if header(clean) != header(stego):
                mismatched += 1
        print(f"verified {checked} pair(s): {checked - mismatched} matching, "
              f"{mismatched} still mismatched")
        if mismatched:
            return 1

    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
