#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Add the fields the manifest promised and did not carry, and fix one that lied.

WHY THIS EXISTS
---------------
A hostile review of the 10,000 cover corpus on 2026-09-18 found five things
between what the documentation claims and what the manifest holds. Four of them
are missing fields and one is a bug. None of them are visible from the images,
which is what makes them dangerous: the corpus passes its own pristine audit and
still ships provenance a downstream user cannot act on.

    attribution        54.3% of covers are CC BY and the manifest gave a user
                       no assembled credit line, no licence URL, and in 19 cases
                       no artist at all. An attribution obligation you cannot
                       discharge from the data supplied is a trap.

    licence_url        The manifest carried Commons' short name ("CC BY 2.0")
                       and nothing resolvable. A short name is not a licence.

    capture_class      255 covers are flatbed and film scanners, which satisfy
                       the "camera EXIF required" filter because scanners write
                       EXIF Make and Model exactly like cameras. They are useful
                       and they are not camera output, so they get labelled
                       rather than dropped.

    split, tier_order  distribution.md says in bold that the split is a property
                       of the cover, decided once and never recomputed per tier.
                       Neither field existed, so the whole nesting guarantee was
                       unbacked.

    artist             96 rows read "Unknown authorUnknown author". Commons
                       returns these as HTML with a hidden microformat copy of
                       the text, and the tag stripper concatenated both. It
                       corrupts precisely the field a CC BY user depends on.

WHY THE ORDERING IS APPEND-STABLE AND THE SPLIT IS NOT ORDERED AT ALL
---------------------------------------------------------------------
These are different jobs and conflating them is how tiered corpora poison the
results built on them.

`split` decides train or test. It has to be a property of the cover itself, so
it is derived from the cover's Commons content hash and a fixed salt. Add ninety
thousand covers later and not one existing assignment moves. Somebody who trains
on Lite and evaluates on Core therefore cannot test on an image they trained on,
which is the failure this field exists to prevent.

`tier_order` decides which covers are in Nano, Lite and Core: a tier is a prefix
of one ordering, not a fresh sample. That ordering is assigned once, here, and
recorded. New covers append after the existing ones rather than interleaving,
because a hash sort would reshuffle everything and Nano's first 200 would change
every time the corpus grew. A prefix that stops being a prefix is worse than no
tiers at all.
"""
from __future__ import annotations

import argparse
import hashlib
import os
import json
import pathlib
import random
import re
import sys
from collections import Counter

#: Resolvable URL per licence short name. A short name is not a licence: it does
#: not say what the terms are and it cannot be checked by a machine.
LICENCE_URLS = {
    "CC0": "https://creativecommons.org/publicdomain/zero/1.0/",
    "CC BY 1.0": "https://creativecommons.org/licenses/by/1.0/",
    "CC BY 2.0": "https://creativecommons.org/licenses/by/2.0/",
    "CC BY 2.5": "https://creativecommons.org/licenses/by/2.5/",
    "CC BY 3.0": "https://creativecommons.org/licenses/by/3.0/",
    "CC BY 4.0": "https://creativecommons.org/licenses/by/4.0/",
}

#: Licences that oblige a downstream user to credit the author.
ATTRIBUTION_REQUIRED = {"CC BY 1.0", "CC BY 2.0", "CC BY 2.5", "CC BY 3.0", "CC BY 4.0"}

#: Scanner and reprographic hardware, matched against EXIF Make and Model.
#:
#: These are not guesses. Every string here was observed in this corpus by the
#: review that prompted this file, and each one is a device that photographs a
#: flat original rather than a scene. The Phase One backs are the subtle case:
#: they are cameras, and on a book copy rig they are producing scans, so the
#: title patterns below carry that decision rather than the hardware string.
SCANNER_PATTERNS = re.compile(
    r"coolscan|canoscan|scanjet|perfection|expression\s*\d+xl|digibook|copibook"
    r"|suprascan|cruse|plustek|scanntech|imacon|flextight|epson\s*gt-|microtek"
    r"|opticfilm|scanner|scanmaker|powerlook|duoscan",
    re.I,
)

#: Titles that say a flat original was digitised, whatever the hardware reports.
DIGITISATION_PATTERNS = re.compile(
    r"\(page\s*\d+\)|\bDPLA\b|\bNARA\b|- DPLA -|txu-oclc|\bfolio\b"
    r"|\bplate\s+[IVXLC]+\b|bestanddeelnr|\bMM\.[A-Z]\.\d|\bNT\d{3,}"
    r"|accession\s*(no|number)|inv\.?\s*nr|\bnegatief\b|glass\s+negative",
    re.I,
)

#: Medium-format digital backs. These ARE cameras, and they are also the
#: standard tool for photographing flat originals on a museum or archive copy
#: rig, which is a scan by every meaning that matters to a steganalysis user.
#:
#: The hardware cannot tell you which happened, and neither can we. So a back on
#: this list does not change the class on its own; it only records that the
#: class is less certain, through `capture_class_basis`. Guessing here would be
#: worse than the gap, because a wrong label is acted on and a recorded
#: uncertainty is filtered.
REPRO_BACKS = re.compile(
    r"phase\s*one|\bleaf\b|aptus|betterlight|sinarback|\biXH\b|\bIQ[0-9]|P65\+"
    r"|hasselblad\s+h\d|\bcfv\b",
    re.I,
)


def undouble(text: str) -> str:
    """Collapse a string that is exactly itself twice over.

    Commons returns artist and credit as HTML carrying a hidden microformat copy
    of the same text, and a tag stripper concatenates both. The result is an
    exact self-doubling, optionally with a space at the join.

    This is deliberately conservative: it fires only on an exact repeat of at
    least four characters. A real name that happens to be its own doubling would
    be extraordinary, and a half-match is left alone rather than guessed at.
    """
    if not text:
        return text
    t = text.strip()
    n = len(t)
    if n >= 8 and n % 2 == 0 and t[: n // 2] == t[n // 2:]:
        return t[: n // 2]
    # The same thing with a single separator at the join.
    for sep in (" ", ", ", " - "):
        if sep in t:
            half = (n - len(sep)) / 2
            if half >= 4 and half == int(half):
                a, b = t[: int(half)], t[int(half) + len(sep):]
                if a == b and t[int(half): int(half) + len(sep)] == sep:
                    return a
    return t


def capture_class(row: dict) -> tuple[str, str]:
    """The class, and the reason for it.

    The reason is not decoration. Two of these classes are inferred from a
    pattern match and a downstream user deciding whether to trust them needs to
    know which rule fired, not just what it concluded. A label without a basis is
    an assertion; with one it is evidence.
    """
    exif = row.get("exif") or {}
    hardware = f"{exif.get('Make', '')} {exif.get('Model', '')}".strip()
    title = row.get("title") or ""

    if hardware and SCANNER_PATTERNS.search(hardware):
        return "scanner", "scanner hardware in EXIF"
    if DIGITISATION_PATTERNS.search(title):
        return "scanner", "digitisation identifier in title"
    if hardware and REPRO_BACKS.search(hardware):
        # A camera, on hardware that is equally at home on a copy stand.
        return "camera", "medium format back, could be reprographic"
    if hardware:
        return "camera", "camera hardware in EXIF"
    return "unknown", "no capture hardware recorded"


def attribution_for(row: dict) -> tuple[str | None, bool]:
    """A credit line a downstream user can paste, and whether they must.

    Follows the order Creative Commons itself recommends: title, creator,
    source, licence. Where the creator is missing or unusable the line says so
    explicitly rather than quietly omitting it, because a user who cannot tell
    the difference between "no attribution needed" and "we lost the name" will
    assume the first.
    """
    licence = (row.get("licence") or "").strip()
    required = licence in ATTRIBUTION_REQUIRED
    artist = undouble(row.get("artist") or "").strip()
    title = (row.get("title") or "").strip()
    source = row.get("descriptionurl") or row.get("source_url") or ""

    unusable = artist.lower() in {"", "unknown", "unknown author", "various",
                                  "various authors", "anonymous", "n/a", "-"}
    if unusable:
        artist_part = "author not recorded by the source" if required else None
    else:
        artist_part = artist

    parts = []
    if title:
        parts.append(f'"{title}"')
    if artist_part:
        parts.append(f"by {artist_part}")
    if licence:
        parts.append(licence)
    if source:
        parts.append(f"via Wikimedia Commons, {source}")
    return (", ".join(parts) if parts else None), required


def stable_split(row: dict, salt: str, test_fraction: float) -> str:
    """Train or test, from the cover's own identity.

    Keyed on the Commons content hash rather than on position, filename or
    arrival order, so the assignment survives the corpus growing, being
    reordered, or being rebuilt from scratch.
    """
    key = row.get("commons_sha1") or row.get("sha256") or row.get("file")
    digest = hashlib.sha256(f"{salt}:{key}".encode()).digest()
    value = int.from_bytes(digest[:8], "big") / float(1 << 64)
    return "test" if value < test_fraction else "train"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("manifest")
    ap.add_argument("--out", default=None, help="default: rewrite in place, atomically")
    ap.add_argument("--salt", default="pentimento-v1",
                    help="fixed, recorded, and never changed once published: "
                         "changing it reassigns every cover's split")
    ap.add_argument("--test-fraction", type=float, default=0.2)
    ap.add_argument("--order-seed", type=int, default=20260918)
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    path = pathlib.Path(args.manifest)
    if not path.is_file():
        print(f"no manifest at {path}", file=sys.stderr)
        return 2

    rows = [json.loads(l) for l in path.read_text().splitlines() if l.strip()]
    print(f"{len(rows)} rows")

    # Existing tier_order values are authoritative and never reassigned, so a
    # second run on a grown corpus appends rather than reshuffles.
    already = {r["file"]: r["tier_order"] for r in rows if "tier_order" in r}
    fresh = [r for r in rows if r["file"] not in already]
    rng = random.Random(args.order_seed)
    rng.shuffle(fresh)
    next_order = (max(already.values()) + 1) if already else 0
    for r in fresh:
        already[r["file"]] = next_order
        next_order += 1
    if already and fresh:
        print(f"assigned tier_order to {len(fresh)} cover(s), "
              f"keeping {len(rows) - len(fresh)} existing")

    stats = Counter()
    for r in rows:
        before = r.get("artist")
        r["artist"] = undouble(before or "") or None
        if before and r["artist"] != before:
            stats["artist_undoubled"] += 1

        r["capture_class"], r["capture_class_basis"] = capture_class(r)
        stats[f"capture_{r['capture_class']}"] += 1
        if "reprographic" in r["capture_class_basis"]:
            stats["capture_camera_uncertain"] += 1

        licence = (r.get("licence") or "").strip()
        r["licence_url"] = LICENCE_URLS.get(licence)
        if licence and r["licence_url"] is None:
            stats["licence_no_url"] += 1

        attribution, required = attribution_for(r)
        r["attribution"] = attribution
        r["attribution_required"] = required
        if required:
            stats["attribution_required"] += 1
            if attribution and "author not recorded" in attribution:
                stats["attribution_unresolvable"] += 1

        r["split"] = stable_split(r, args.salt, args.test_fraction)
        stats[f"split_{r['split']}"] += 1
        r["tier_order"] = already[r["file"]]
        r["split_salt"] = args.salt

    rows.sort(key=lambda r: r["tier_order"])

    print("\nwhat changed")
    for key in sorted(stats):
        print(f"  {key:<28} {stats[key]}")

    # The nesting guarantee, checked rather than asserted.
    orders = [r["tier_order"] for r in rows]
    assert orders == list(range(len(orders))), "tier_order is not a dense 0..n-1 range"
    for size, name in ((200, "Nano"), (1000, "Lite"), (len(rows), "Core")):
        if size <= len(rows):
            tier = rows[:size]
            test = sum(1 for r in tier if r["split"] == "test")
            print(f"  {name:<6} {size:>6} covers, {test:>5} test "
                  f"({test / size:.1%}), prefix of the next tier by construction")

    if args.dry_run:
        print("\ndry run: nothing written")
        return 0

    out = pathlib.Path(args.out) if args.out else path
    tmp = out.with_suffix(out.suffix + f".repair-{os.getpid()}")
    tmp.write_text("".join(json.dumps(r) + "\n" for r in rows))
    tmp.replace(out)
    print(f"\nwritten: {out}")
    return 0


if __name__ == "__main__":

    raise SystemExit(main())
