#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Select covers in tier order, so that a smaller tier is a prefix of a larger one.

WHY THIS EXISTS
---------------
`distribution.md` says, in bold, that a tier is a prefix of one deterministic
ordering and not a fresh sample. Both arm builders were doing the opposite:

    chosen = rng.sample(pool, min(args.count, len(pool)))

A seeded random sample is reproducible, which is why it looked fine, and it is
still the wrong thing. `sample(pool, 200)` and `sample(pool, 1000)` from the same
seed do not nest: the 200 are not the first 200 of the 1,000. So Nano would have
contained covers absent from Lite, and the guarantee the tier design exists to
provide would have been false in the only way that matters.

The failure this prevents is not theoretical and it is not ours alone. Somebody
trains on Lite, evaluates on Core, and unknowingly tests on images they trained
on, because the tiers overlap in a way nothing documents. Every number they
publish is then inflated by an amount nobody can recover afterwards.

    ordering:  [ 0 1 2 3 4 5 6 7 8 9 ... ]
    Nano       └─────┘
    Lite       └───────────┘
    Core       └──────────────────────┘

    A prefix is nested by construction. A sample is not nested at all.

WHAT THE ORDER IS
-----------------
`tier_order`, assigned once by `pentimento manifest-repair` over the covers
present and appended to as the corpus grows. It is not recomputed and not
derived from a hash of the filename, because either would reshuffle every
position each time a cover arrived, and a prefix that stops being a prefix
between releases is worse than having no tiers at all.
"""
from __future__ import annotations

import json
import pathlib


class TierError(RuntimeError):
    """Raised when the manifest cannot support a tier selection."""


def covers_in_tier_order(manifest: pathlib.Path, covers_dir: pathlib.Path,
                         count: int, suffix: str = ".png") -> list[pathlib.Path]:
    """The first `count` covers by `tier_order`, as paths that exist on disk.

    Raises rather than falling back to an arbitrary order. A silent fallback here
    produces a corpus whose tiers do not nest, and nothing downstream would
    detect that: every file is valid, every digest matches, and the defect only
    surfaces as inflated accuracy in somebody else's paper.
    """
    if not manifest.is_file():
        raise TierError(
            f"no manifest at {manifest}. Tier order lives in the manifest, so "
            "without it a tier cannot be selected. Run `pentimento "
            "manifest-repair <manifest>` first."
        )

    rows = []
    for line in manifest.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        if "tier_order" not in row:
            raise TierError(
                f"{manifest} has rows without a tier_order field. Run "
                f"`pentimento manifest-repair {manifest}` to assign it, then "
                "run this command again."
            )
        rows.append(row)

    rows.sort(key=lambda r: r["tier_order"])
    orders = [r["tier_order"] for r in rows]
    if orders != list(range(len(orders))):
        raise TierError(
            "tier_order is not a dense 0..n-1 range, so a prefix is not "
            "well defined. This usually means two manifests were concatenated."
        )
    if count > len(rows):
        raise TierError(
            f"asked for {count} covers, manifest has {len(rows)}. A tier larger "
            "than the corpus is not a prefix of anything."
        )

    chosen: list[pathlib.Path] = []
    for row in rows[:count]:
        path = covers_dir / row["file"]
        if path.suffix != suffix:
            path = path.with_suffix(suffix)
        if not path.is_file():
            raise TierError(
                f"manifest row {row['tier_order']} names {path.name}, which is "
                "not on disk. Selecting round it would silently shift every "
                "later tier boundary."
            )
        chosen.append(path)
    return chosen


def tier_cover_names(manifest: pathlib.Path, count: int) -> set[str]:
    """The filenames of the first `count` covers by `tier_order`.

    The same prefix rule as `covers_in_tier_order`, without requiring the cover
    files themselves. Packing an ARM needs to know which covers are in the tier
    so it can keep the matching stego rows, and the covers it is selecting
    against may not be on the machine doing the packing.

    Returned as a set because the caller tests millions of rows against it.
    """
    rows = []
    if not manifest.is_file():
        raise TierError(
            f"no manifest at {manifest}. Tier order lives in the manifest, so "
            "without it a tier cannot be selected."
        )
    for line in manifest.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        if "tier_order" not in row:
            raise TierError(
                f"{manifest} has rows without a tier_order field. Run "
                f"`pentimento manifest-repair {manifest}` to assign it, then "
                "run this command again."
            )
        rows.append(row)

    rows.sort(key=lambda r: r["tier_order"])
    orders = [r["tier_order"] for r in rows]
    if orders != list(range(len(orders))):
        raise TierError(
            "tier_order is not a dense 0..n-1 range, so a prefix is not "
            "well defined. This usually means two manifests were concatenated."
        )
    if count > len(rows):
        raise TierError(
            f"asked for {count} covers, manifest has {len(rows)}. A tier larger "
            "than the corpus is not a prefix of anything."
        )
    return {r["file"] for r in rows[:count]}


def tier_name(count: int) -> str:
    """The published name for a tier size, or a description of an odd one.

    For PROSE. A filename wants [`tier_slug`], because this one can contain a
    space and brackets.
    """
    return {200: "Nano", 1000: "Lite", 10000: "Core", 100000: "Full"}.get(
        count, f"custom ({count})")


def tier_slug(count: int) -> str:
    """The same name, in a form safe to put in a filename.

    `tier_name` was being lowercased and dropped straight into shard names, so
    an odd count produced `pentimento-custom (8)-wow-0400-00000.tar`: a space
    and two brackets in a filename that goes to three public archives in an
    unattended upload. Those break naive shell loops, URL-encode differently
    on each of the three, and produce a public page that is wrong in a way
    this project cannot withdraw.

    Kept as a separate function rather than by sanitising `tier_name`, because
    the human-readable form is worth having in the terminal and in the index.
    """
    return {200: "nano", 1000: "lite", 10000: "core", 100000: "full"}.get(
        count, f"custom-{count}")


def slug_of_tier_name(name: str) -> str:
    """A filename-safe slug from a tier's published NAME.

    For the publish path, which reads the name back out of a packed index
    rather than knowing the count. `"Core"` gives `core` exactly as the old
    `.lower()` did, so no published identifier moves; `"custom (8)"` gives
    `custom-8` instead of keeping the space and brackets.

    That matters here more than anywhere else in the toolkit: these strings
    become an Internet Archive identifier, a Kaggle slug and a HuggingFace
    repository name, uploaded unattended. An Archive item is close to
    immutable once created, so a name that is wrong is wrong permanently.
    """
    safe = "".join(c if c.isalnum() else "-" for c in name.lower())
    while "--" in safe:
        safe = safe.replace("--", "-")
    return safe.strip("-")
