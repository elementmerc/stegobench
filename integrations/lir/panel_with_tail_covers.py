#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Fold late Aletheia scores for the last three covers into a copy of the panel.

WHY A NEW FILE RATHER THAN AN EDIT
----------------------------------
The published panel is 197 Aletheia-scored covers out of 200, because the
Aletheia pass ended three files short. The three are now scored, with the same
pinned container image the original pass used, and they reproduce the original
numbers byte for byte on two covers held back as a check. Merging them in place
would change the input every published number traces to, so the merge writes a
NEW corpus file and the original is left alone. The published table stays
reproducible from the published corpus; the closed caveat is a second run on a
second file.

Run as::

    python panel_with_tail_covers.py panel.jsonl tail-covers-aletheia.jsonl \\
        --prefix clean/ --out panel-with-tail-covers.jsonl

The added scores are appended as complete records, so `analyse_panel.load`'s
most-complete-record rule picks them up without any change to that rule.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import sys


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("panel", type=pathlib.Path)
    ap.add_argument("added", type=pathlib.Path, help="JSONL from aletheia_scores.py")
    ap.add_argument(
        "--prefix",
        default="clean/",
        help="arm directory the added files belong to, as the panel names it",
    )
    ap.add_argument("--out", type=pathlib.Path, required=True)
    args = ap.parse_args(argv)

    if args.out.resolve() == args.panel.resolve():
        print("refusing to overwrite the published corpus in place", file=sys.stderr)
        return 2

    best: dict[str, dict] = {}
    order: list[str] = []
    for line in args.panel.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        name = row["file"]
        if name not in best:
            order.append(name)
        if name not in best or len(row) > len(best[name]):
            best[name] = row

    added, skipped = 0, 0
    for line in args.added.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        if "file" not in row or "spa" not in row or "rs" not in row:
            skipped += 1
            continue
        name = args.prefix + pathlib.PurePosixPath(row["file"]).name
        target = best.get(name)
        if target is None:
            print(f"{name} is not in the panel; not inventing a record", file=sys.stderr)
            skipped += 1
            continue
        if "aletheia_spa" in target and "aletheia_rs" in target:
            # Already scored. Disagreement here would mean the container is not
            # the one the published pass used, which is worth saying loudly
            # rather than silently preferring one of the two numbers.
            for field, key in (("aletheia_spa", "spa"), ("aletheia_rs", "rs")):
                if target[field] != row[key]:
                    print(
                        f"{name} {field}: published {target[field]!r} against "
                        f"rescored {row[key]!r}; the two runs do not agree",
                        file=sys.stderr,
                    )
            skipped += 1
            continue
        target["aletheia_spa"] = row["spa"]
        target["aletheia_rs"] = row["rs"]
        added += 1

    with args.out.open("w", encoding="utf-8") as fh:
        for name in order:
            fh.write(json.dumps(best[name]) + "\n")
    print(f"{len(order)} records written to {args.out}; {added} filled in, {skipped} left alone")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
