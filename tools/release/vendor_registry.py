#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Copy the tool registry into the CLI crate so a published crate carries one.

WHY THIS EXISTS

`crates/stegobench-cli/build.rs` compiles the registry into the binary as the
last-resort fallback, so an installed binary works with no files beside it. It
reads `../../plugins/registry`, which is outside the package root, and Cargo
packages only what is inside. So `cargo publish` of the CLI crate on its own
produces a binary with an empty built-in registry: `cargo install
stegobench-cli` would give somebody a tool that knows about no tools.

The fix the operator chose: vendor a copy into the crate at release time and
list it in the package's `include`. This writes that copy.

WHY A COPY IS DANGEROUS, AND WHAT STOPS IT ROTTING

A second copy of anything is a second source of truth, and this repository's
own one-home rule exists because copies drift. So `--check` compares the two
and fails when they differ, and it runs in CI. The vendored copy is generated,
never edited: if the two disagree, `plugins/registry` is right by definition
and this script rewrites the copy.
"""

from __future__ import annotations

import argparse
import filecmp
import hashlib
import pathlib
import shutil
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
SOURCE = REPO / "plugins" / "registry"
VENDORED = REPO / "crates" / "stegobench-cli" / "registry"

#: A registry file is TOML and small. Anything else under the source tree is
#: not part of the registry and is not copied, so a stray file cannot ride
#: along into a published crate.
PATTERN = "*.toml"

#: Refused above this. The whole registry is a few tens of kilobytes; a crate
#: is not a place to put a corpus, and crates.io refuses a large package
#: anyway, so catching it here names the cause instead of the symptom.
MAX_TOTAL_BYTES = 2 * 1024 * 1024


def files(root: pathlib.Path) -> dict[str, pathlib.Path]:
    """Every registry file, keyed by its path relative to the registry root."""
    return {
        str(p.relative_to(root)): p
        for p in sorted(root.rglob(PATTERN))
        if p.is_file()
    }


def digest(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def differences(source: dict, vendored: dict) -> list[str]:
    problems = []
    for name in sorted(set(source) | set(vendored)):
        if name not in vendored:
            problems.append(f"{name}: in the registry, missing from the copy")
        elif name not in source:
            problems.append(f"{name}: in the copy, not in the registry")
        elif not filecmp.cmp(source[name], vendored[name], shallow=False):
            problems.append(f"{name}: the copy differs from the registry")
    return problems


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--check", action="store_true",
                    help="compare rather than write; exit 1 on any difference")
    ap.add_argument("--source", type=pathlib.Path, default=SOURCE)
    ap.add_argument("--dest", type=pathlib.Path, default=VENDORED)
    args = ap.parse_args(argv)

    if not args.source.is_dir():
        print(f"no registry at {args.source}", file=sys.stderr)
        return 2

    source = files(args.source)
    if not source:
        # A registry with no files would vendor an empty directory and produce
        # exactly the silent defect this script exists to close.
        print(f"{args.source} holds no {PATTERN} files, so there is nothing to "
              f"vendor and a published crate would carry an empty registry",
              file=sys.stderr)
        return 2

    total = sum(p.stat().st_size for p in source.values())
    if total > MAX_TOTAL_BYTES:
        print(f"the registry is {total:,} bytes, over the {MAX_TOTAL_BYTES:,} "
              f"ceiling for what belongs inside a published crate",
              file=sys.stderr)
        return 2

    vendored = files(args.dest) if args.dest.is_dir() else {}

    if args.check:
        problems = differences(source, vendored)
        if problems:
            print(f"the vendored registry at {args.dest} does not match "
                  f"{args.source}:", file=sys.stderr)
            for p in problems:
                print(f"  {p}", file=sys.stderr)
            print("\nThe registry is the original and the copy is generated. "
                  "Run this script without --check.", file=sys.stderr)
            return 1
        print(f"the vendored copy matches: {len(source)} file(s), "
              f"{total:,} bytes")
        return 0

    # Written fresh rather than merged, so a file deleted from the registry
    # cannot survive in the copy.
    if args.dest.exists():
        shutil.rmtree(args.dest)
    for name, path in source.items():
        target = args.dest / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)

    written = files(args.dest)
    remaining = differences(source, written)
    if remaining:
        # Verified rather than assumed: the copy is about to be published and
        # a failed copy that reported success is the whole failure class here.
        print("the copy did not come out right:", file=sys.stderr)
        for p in remaining:
            print(f"  {p}", file=sys.stderr)
        return 1

    print(f"vendored {len(written)} file(s), {total:,} bytes, into {args.dest}")
    print(f"registry digest: "
          f"{hashlib.sha256(''.join(sorted(digest(p) for p in written.values())).encode()).hexdigest()[:16]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
