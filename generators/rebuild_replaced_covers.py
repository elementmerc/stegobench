#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Invalidate the arm files derived from covers that were replaced.

WHY INVALIDATE RATHER THAN REBUILD
----------------------------------
`build_adaptive_arms.py` is idempotent by output existence: it skips any pair
whose stego file is already on disk or already named in the arm manifest. That
makes "rebuild these 121 covers" the same job as "delete what they produced and
run the builder again", which is worth preferring over a second, parallel code
path that rebuilds a subset. The builder that produced the other 9,879 covers is
the builder that produces these, with the same seeds and the same skips.

So this tool deletes and unrecords. It builds nothing.

WHAT A REPLACED COVER LEAVES BEHIND
-----------------------------------
A cover at `tier_order` N is `00N.png` in the cover directory, and everything
downstream carries the same stem::

    arms/<root>/jpeg-tools/clean/NNNNN.jpg          the JPEG cover
    arms/<root>/jpeg-tools/<tool>/NNNNN.jpg         the tool arms
    arms/<root>/adaptive/clean_grey/NNNNN.png       the spatial cover
    arms/<root>/adaptive/clean_jpeg_pass1/NNNNN.jpg the coefficient source
    arms/<root>/adaptive/clean_jpeg/NNNNN.jpg       the shipping clean half
    arms/<root>/adaptive/<scheme>/<rate>/NNNNN.*    every stego half

Rather than enumerate that layout, which drifts as arms are added, this walks
the arm root and matches on the stem. A file named for a replaced cover is a
file derived from a cover that no longer exists.

THE ORDERING TRAP
-----------------
`build_adaptive_arms.py` selects its JPEG covers positionally::

    jpeg_pool = sorted(pathlib.Path(args.jpeg_covers).glob("*.jpg"))
    jpeg_pool = jpeg_pool[: len(chosen)]

Index *i* is the *i*-th name in sorted order, which equals `{i:05d}.jpg` only
while the pool is complete. Delete one and every cover after it shifts down by
one: pair 5,000 is then built from cover 5,001's coefficients, every digest
matches, every count is right, and the arm is silently wrong for half the
corpus.

So the JPEG cover pool must be rebuilt and verified complete BEFORE any adaptive
arm is rebuilt. `--check-pool` asserts that invariant and this tool prints the
ordering it requires.

Usage::

    python rebuild_replaced_covers.py --arms ~/pentimento/arms/core \\
        --swaps ~/pentimento/covers/commons/backfill-round1.json \\
        --swaps ~/pentimento/covers/commons/backfill.json \\
        --covers ~/pentimento/covers/commons --dry-run
"""
from __future__ import annotations

import argparse
import json
import pathlib
import sys

#: Extensions an arm file can carry. Anything else under the arm root is
#: metadata rather than a derived image.
IMAGE_SUFFIXES = (".png", ".jpg", ".jpeg")

#: Manifest fields that name a file by its path within the arm root.
PATH_FIELDS = ("clean", "stego")


class RebuildError(RuntimeError):
    pass


def positions(swap_files: list[pathlib.Path]) -> set[int]:
    """Every `tier_order` named across the swap logs.

    Taken as a union because a corpus can be backfilled more than once and the
    rounds are recorded separately. A position appearing twice was replaced
    twice and still needs invalidating once.
    """
    found: set[int] = set()
    for path in swap_files:
        try:
            log = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError) as e:
            raise RebuildError(f"cannot read {path}: {e}") from e
        swaps = log.get("swaps")
        if not swaps:
            raise RebuildError(f"{path} records no swaps")
        for swap in swaps:
            if "position" not in swap:
                raise RebuildError(f"{path}: a swap has no position")
            found.add(int(swap["position"]))
    return found


def stems_of(found: set[int]) -> set[str]:
    return {f"{n:05d}" for n in found}


def confirm_replaced(covers: pathlib.Path, found: set[int]) -> list[str]:
    """Cross-check the swap logs against the live cover manifest.

    A position named by a log but carrying no `replaces` in the manifest means
    the log and the corpus disagree, and deleting arms on the strength of a
    stale log would throw away sound work. Returns the complaints rather than
    raising, so the caller can print all of them at once.
    """
    manifest = covers / "manifest.jsonl"
    if not manifest.is_file():
        raise RebuildError(f"no cover manifest at {manifest}")
    by_order = {}
    for line in manifest.read_text().splitlines():
        if line.strip():
            row = json.loads(line)
            by_order[row["tier_order"]] = row

    complaints = []
    for n in sorted(found):
        row = by_order.get(n)
        if row is None:
            complaints.append(f"tier_order {n} is not in the cover manifest")
        elif "replaces" not in row:
            complaints.append(
                f"tier_order {n} is named as replaced but the manifest row "
                f"records no `replaces`; the swap log may be stale")
    return complaints


def doomed_files(arms: pathlib.Path, stems: set[str]) -> list[pathlib.Path]:
    """Every derived image under the arm root named for a replaced cover."""
    out = []
    for path in sorted(arms.rglob("*")):
        if path.is_file() and path.suffix.lower() in IMAGE_SUFFIXES:
            if path.stem in stems:
                out.append(path)
    return out


def row_is_doomed(row: dict, stems: set[str]) -> bool:
    """Whether an arm manifest row describes a replaced cover.

    Either half naming the stem condemns the row: a pair whose clean half is
    stale is not half valid, it is void.
    """
    for field in PATH_FIELDS:
        value = row.get(field)
        if value and pathlib.PurePosixPath(value).stem in stems:
            return True
    source = row.get("source_png")
    return bool(source and pathlib.PurePosixPath(source).stem in stems)


def strip_manifest(path: pathlib.Path, stems: set[str],
                   dry_run: bool) -> tuple[int, int]:
    """Remove the doomed rows. Returns (kept, dropped)."""
    rows = [json.loads(line) for line in path.read_text().splitlines()
            if line.strip()]
    keep = [r for r in rows if not row_is_doomed(r, stems)]
    dropped = len(rows) - len(keep)
    if dropped and not dry_run:
        backup = path.with_suffix(".jsonl.pre-rebuild")
        if not backup.exists():
            backup.write_text(path.read_text())
        part = path.with_suffix(".jsonl.part")
        part.write_text("".join(json.dumps(r, sort_keys=True) + "\n"
                                for r in keep))
        part.replace(path)
    return len(keep), dropped


def check_pool(pool_dir: pathlib.Path, expected: int) -> list[str]:
    """Assert the JPEG cover pool is dense, because the builder indexes it.

    See THE ORDERING TRAP above. This is the check that turns a silent
    misalignment into a refusal.
    """
    names = sorted(p.name for p in pool_dir.glob("*.jpg"))
    complaints = []
    if len(names) != expected:
        complaints.append(
            f"{pool_dir} holds {len(names):,} JPEGs and the tier wants "
            f"{expected:,}. The builder indexes this pool positionally, so a "
            f"gap shifts every cover after it onto the wrong coefficients.")
    for i, name in enumerate(names):
        if name != f"{i:05d}.jpg":
            complaints.append(
                f"{pool_dir}: position {i} is {name}, not {i:05d}.jpg. The "
                f"pool is not dense and the arms would be mispaired.")
            break
    return complaints


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--arms", required=True, help="the arm root, e.g. arms/core")
    ap.add_argument("--swaps", action="append", required=True,
                    help="a backfill_covers.py log. Repeatable")
    ap.add_argument("--covers", default=None,
                    help="the live cover directory, to cross-check the logs")
    ap.add_argument("--check-pool", default=None,
                    help="a JPEG cover pool to verify is dense")
    ap.add_argument("--expect", type=int, default=10000,
                    help="how many covers the pool should hold")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    arms = pathlib.Path(args.arms)
    if not arms.is_dir():
        print(f"no arm root at {arms}", file=sys.stderr)
        return 1

    try:
        found = positions([pathlib.Path(p) for p in args.swaps])
    except RebuildError as e:
        print(f"cannot start: {e}", file=sys.stderr)
        return 1
    stems = stems_of(found)
    print(f"{len(found):,} replaced covers named across "
          f"{len(args.swaps)} swap log(s)")

    if args.covers:
        try:
            complaints = confirm_replaced(pathlib.Path(args.covers), found)
        except RebuildError as e:
            print(f"cannot cross-check: {e}", file=sys.stderr)
            return 1
        if complaints:
            print("\nthe swap logs disagree with the cover manifest:",
                  file=sys.stderr)
            for c in complaints[:10]:
                print(f"  {c}", file=sys.stderr)
            if len(complaints) > 10:
                print(f"  ... and {len(complaints) - 10:,} more",
                      file=sys.stderr)
            print("\nrefusing to delete arms on a log the corpus does not "
                  "confirm", file=sys.stderr)
            return 1
        print("cross-checked: every position carries a `replaces` in the "
              "cover manifest")

    if args.check_pool:
        complaints = check_pool(pathlib.Path(args.check_pool), args.expect)
        if complaints:
            print("\nthe JPEG cover pool is not safe to build from:",
                  file=sys.stderr)
            for c in complaints:
                print(f"  {c}", file=sys.stderr)
            return 1
        print(f"JPEG cover pool is dense: {args.expect:,} covers, "
              f"position i is {{i:05d}}.jpg")

    doomed = doomed_files(arms, stems)
    print(f"\n{len(doomed):,} derived image(s) to remove")
    by_arm: dict[str, int] = {}
    for path in doomed:
        key = str(path.parent.relative_to(arms))
        by_arm[key] = by_arm.get(key, 0) + 1
    for key in sorted(by_arm):
        print(f"  {key:44} {by_arm[key]:>5}")

    manifests = sorted(arms.rglob("manifest.jsonl"))
    print(f"\n{len(manifests)} arm manifest(s)")
    total_dropped = 0
    for path in manifests:
        kept, dropped = strip_manifest(path, stems, args.dry_run)
        total_dropped += dropped
        if dropped:
            print(f"  {path.relative_to(arms)}: {dropped:,} row(s) dropped, "
                  f"{kept:,} kept")

    if args.dry_run:
        print(f"\ndry run: nothing removed. {len(doomed):,} files and "
              f"{total_dropped:,} manifest rows would go.")
        return 0

    for path in doomed:
        path.unlink()
    print(f"\nremoved {len(doomed):,} file(s) and {total_dropped:,} "
          f"manifest row(s)")
    print("\nRebuild in this order, and not the other way round:\n"
          "  1. the JPEG cover pool (build_jpeg_arms.py), then verify it is "
          "dense\n"
          "  2. the tool arms\n"
          "  3. the adaptive arms (build_adaptive_arms.py), which index that "
          "pool positionally\n"
          "Then repack.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
