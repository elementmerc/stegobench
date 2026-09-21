#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Record which cover each arm row was built from, by content rather than name.

THE GAP THIS CLOSES
-------------------
An arm row records `source_png`, the cover's filename. `backfill_covers.py`
replaces an unpublishable cover **in place**, keeping its filename and its
`tier_order`, because that is what preserves the tier prefix guarantee.

Those two facts together mean an arm manifest cannot witness its own staleness.
Before the swap, row X says `source_png: 01185.png`. After the swap, with a
completely different photograph at that name, row X still says
`source_png: 01185.png`. Every field is unchanged, every digest in the row still
matches the files the row points at, and the arm is derived from an image that no
longer exists anywhere.

Nothing downstream can tell. The only witness is the cover's own digest, and no
arm row carried one.

WHY THIS DOES NOT NEED A REBUILD
--------------------------------
Stamping a digest onto an existing row asserts "this row was built from the
cover that is at this filename now". That is a claim, and writing it blindly
would replace a gap with a lie, which is worse.

It is true only under a precondition: every row belonging to a replaced cover
has been rebuilt since the replacement. That is exactly what
`rebuild_replaced_covers.py` plus a builder run establishes, so this tool
CHECKS the precondition rather than assuming it, and refuses if it does not
hold:

    - every row names a `source_png` the cover manifest knows
    - for every replaced cover, rows exist at all (they were deleted, so their
      presence means something rebuilt them)
    - every row's clean half on disk matches the `clean_sha256` the row
      recorded, so the row describes the files that are actually there

If any of those fails, the arms and the covers disagree and no digest should be
written until they do not.

Usage::

    python stamp_source_digests.py --arms ~/pentimento/arms/core \\
        --covers ~/pentimento/covers/commons \\
        --swaps ~/pentimento/covers/commons/backfill-round1.json \\
        --swaps ~/pentimento/covers/commons/backfill.json --dry-run
"""
from __future__ import annotations

import argparse
import collections
import hashlib
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from rebuild_replaced_covers import RebuildError, positions, stems_of  # noqa: E402


class StampError(RuntimeError):
    pass


def digest(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for block in iter(lambda: fh.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def cover_digests(covers: pathlib.Path) -> dict[str, str]:
    """Filename to digest, read from the cover manifest.

    The manifest rather than the directory, because the manifest is what the
    corpus publishes and a file on disk that disagrees with it is a separate
    failure that `verify_release.py` reports.
    """
    manifest = covers / "manifest.jsonl"
    if not manifest.is_file():
        raise StampError(f"no cover manifest at {manifest}")
    out = {}
    for line in manifest.read_text().splitlines():
        if line.strip():
            row = json.loads(line)
            out[row["file"]] = row["sha256"]
    return out


def unknown_sources(rows: list[dict], known: dict[str, str]) -> list[str]:
    return sorted({r["source_png"] for r in rows
                   if r.get("source_png") and r["source_png"] not in known})


def rows_without_a_source(rows: list[dict]) -> int:
    return sum(1 for r in rows if not r.get("source_png"))


def missing_rebuilt(rows: list[dict], stems: set[str]) -> list[str]:
    """Replaced covers with no arm rows at all.

    Their rows were deleted by the invalidation, so an absence means the
    rebuild has not reached them and stamping would be premature.
    """
    present = {pathlib.PurePosixPath(r["stego"]).stem for r in rows}
    return sorted(stems - present)


def drifted(rows: list[dict], base: pathlib.Path, limit: int) -> list[str]:
    """Rows whose clean half on disk differs from the digest they recorded."""
    out = []
    for row in rows[:limit] if limit else rows:
        recorded = row.get("clean_sha256")
        if not recorded:
            continue
        path = base / row["clean"]
        if path.is_file() and digest(path) != recorded:
            out.append(row["clean"])
    return out


def stamp(rows: list[dict], known: dict[str, str]) -> int:
    """Write `source_sha256` onto every row that lacks it. Returns how many."""
    written = 0
    for row in rows:
        source = row.get("source_png")
        if not source:
            continue
        want = known[source]
        if row.get("source_sha256") != want:
            row["source_sha256"] = want
            written += 1
    return written


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--arms", required=True)
    ap.add_argument("--covers", required=True)
    ap.add_argument("--swaps", action="append", default=[],
                    help="backfill_covers.py logs, to check the rebuild landed")
    ap.add_argument("--drift-sample", type=int, default=2000,
                    help="rows per manifest to re-digest; 0 means all")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    arms = pathlib.Path(args.arms)
    covers = pathlib.Path(args.covers)

    try:
        known = cover_digests(covers)
    except StampError as e:
        print(f"cannot start: {e}", file=sys.stderr)
        return 1
    print(f"{len(known):,} covers in the manifest")

    stems: set[str] = set()
    if args.swaps:
        try:
            stems = stems_of(positions([pathlib.Path(p) for p in args.swaps]))
        except RebuildError as e:
            print(f"cannot read the swap logs: {e}", file=sys.stderr)
            return 1
        print(f"{len(stems):,} covers were replaced and must have been rebuilt")

    manifests = sorted(arms.rglob("manifest.jsonl"))
    if not manifests:
        print(f"no arm manifests under {arms}", file=sys.stderr)
        return 1

    loaded = {}
    problems: dict[str, list[str]] = collections.defaultdict(list)
    for path in manifests:
        rows = [json.loads(line) for line in path.read_text().splitlines()
                if line.strip()]
        loaded[path] = rows
        name = str(path.relative_to(arms))

        orphans = rows_without_a_source(rows)
        if orphans:
            problems[name].append(f"{orphans:,} row(s) record no source_png, so "
                                  f"nothing says which cover they came from")
        unknown = unknown_sources(rows, known)
        if unknown:
            problems[name].append(
                f"{len(unknown)} source_png value(s) are not in the cover "
                f"manifest, e.g. {unknown[:3]}")
        absent = missing_rebuilt(rows, stems) if stems else []
        if absent:
            problems[name].append(
                f"{len(absent)} replaced cover(s) have no rows here, so the "
                f"rebuild has not finished: {absent[:5]}")
        moved = drifted(rows, path.parent, args.drift_sample)
        if moved:
            problems[name].append(
                f"{len(moved)} clean half/halves differ from the digest their "
                f"row recorded, e.g. {moved[:3]}")

    if problems:
        print("\nrefusing to stamp: the arms and the covers disagree, and a "
              "digest written now would be a claim rather than a fact",
              file=sys.stderr)
        for name, messages in problems.items():
            print(f"  {name}", file=sys.stderr)
            for message in messages:
                print(f"    {message}", file=sys.stderr)
        return 1

    total = 0
    for path, rows in loaded.items():
        written = stamp(rows, known)
        total += written
        print(f"  {path.relative_to(arms)}: {written:,} of {len(rows):,} "
              f"row(s) stamped")
        if written and not args.dry_run:
            backup = path.with_suffix(".jsonl.pre-stamp")
            if not backup.exists():
                backup.write_text(path.read_text())
            part = path.with_suffix(".jsonl.part")
            part.write_text("".join(json.dumps(r, sort_keys=True) + "\n"
                                    for r in rows))
            part.replace(path)

    if args.dry_run:
        print(f"\ndry run: {total:,} row(s) would be stamped")
        return 0
    print(f"\nstamped {total:,} row(s). An arm can now be shown to derive from "
          f"a specific cover, by content rather than by name.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
