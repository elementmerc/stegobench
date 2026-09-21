#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Name the covers this corpus has no business redistributing, and say why.

WHY THIS EXISTS
---------------
The corpus's reason to exist is that its licences are traceable, in a field
where BOSSbase and ALASKA2 cannot be republished at all and where mirrors of
Dresden and UCID carry licences their sources never granted. That claim is only
worth making if we are stricter with ourselves than the mirrors were.

An audit against Commons found four groups that fail it. They are listed here
rather than handled quietly, because a corpus that drops images without saying
which ones is asking to be taken on trust, and taking a corpus on trust is the
habit this one exists to break.

THE FOUR GROUPS
---------------
`share-alike`
    Recorded as public domain, but the file page carries CC BY-SA. Share-alike
    is a copyleft obligation: derivatives must be licensed alike. Every stego
    image here is a derivative, and the collection is published CC BY 4.0, so
    keeping these would breach the term AND falsify the collection statement,
    which says CC BY is the strictest obligation present.

`unattributable`
    Requires attribution and records no author, so the credit line reads "by
    author not recorded by the source". CC BY asks for the author as
    designated. A placeholder does not discharge that; it documents that we
    could not.

`us-only`
    Public domain on United States grounds alone, with nothing said about the
    source country. The publisher is in the United Kingdom and the corpus is
    served worldwide. "Public domain in the US" is not a redistribution basis
    here, and a reader in Berlin cannot rely on it either.

`ungrounded`
    Recorded as public domain with no recoverable ground on the file page. Not
    necessarily wrong, and not verifiable, which for this corpus is the same
    thing: an unverifiable licence claim is what it exists to argue against.

WHAT IT DOES NOT DO
-------------------
It selects. It does not delete, and it does not choose replacements. Run
`backfill_covers.py` with its output.

Usage::

    python select_unpublishable.py --manifest ~/pentimento/covers/commons/manifest.jsonl \\
                                   --audited ~/pentimento/covers/commons/manifest.pd-audited.jsonl \\
                                   --out ~/pentimento/covers/commons/unpublishable.json
"""
from __future__ import annotations

import argparse
import collections
import json
import pathlib
import sys

#: A PD template naming the United States and nothing else. A file that also
#: carries a source-country ground (`PD-old-70`, `PD-Russia-expired`) is fine:
#: the US tag is then one of several, not the only one.
US_ONLY_PREFIXES = ("PD-US", "PD-1996", "PD-USGov")

#: A ground that says the work is free everywhere, or that the author released
#: it themselves. Either settles the jurisdiction question.
UNIVERSAL_PREFIXES = ("PD-self", "PD-user", "PD-author", "CC0",
                      "PD-old", "PD-art", "PD-Art", "PD-scan", "PD-because",
                      "No rights reserved", "Copyrighted free use")


def is_us_only(grounds: list[str]) -> bool:
    """Every stated ground rests on United States law.

    Asked as "are they ALL US" rather than "is any US", because a file tagged
    both PD-USGov and PD-old-70 is free on a ground that does not depend on
    where the reader is.
    """
    if not grounds:
        return False
    if any(g.startswith(UNIVERSAL_PREFIXES) for g in grounds):
        return False
    return all(g.startswith(US_ONLY_PREFIXES) for g in grounds)


def classify(row: dict, page_licences: dict[str, list[str]]) -> str | None:
    """Which group a row falls into, or None if it is publishable."""
    licence = (row.get("licence") or "").strip()
    lowered = licence.lower()

    # Checked first: it is the only group that is an outright breach rather
    # than a gap, and a row can be in more than one.
    stated = page_licences.get(row.get("title", ""), [])
    if any(s.lower().startswith(("cc-by-sa", "cc-sa", "gfdl")) for s in stated):
        if not lowered.startswith("cc by-sa"):
            return "share-alike"

    if row.get("attribution_required") and not (row.get("artist") or "").strip():
        return "unattributable"

    if lowered.startswith("public domain"):
        grounds = row.get("pd_grounds")
        if grounds is None:
            return None  # not audited; not this tool's call to make
        if not grounds:
            return "ungrounded"
        if is_us_only(grounds):
            return "us-only"

    return None


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--manifest", required=True)
    ap.add_argument("--audited", default=None,
                    help="manifest carrying pd_grounds, from audit_pd_licences.py")
    ap.add_argument("--page-licences", default=None,
                    help="JSON mapping file title to the licence templates on "
                         "its Commons page, from audit_pd_licences.py "
                         "--cross-check --report")
    ap.add_argument("--out", default=None)
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    source = pathlib.Path(args.audited or args.manifest)
    if not source.is_file():
        print(f"no manifest at {source}", file=sys.stderr)
        return 1
    rows = [json.loads(line) for line in source.read_text().splitlines()
            if line.strip()]

    page_licences: dict[str, list[str]] = {}
    if args.page_licences:
        path = pathlib.Path(args.page_licences)
        if path.is_file():
            page_licences = json.loads(path.read_text())
        else:
            print(f"note: no page licences at {path}, so the share-alike group "
                  f"cannot be found", file=sys.stderr)

    groups: dict[str, list[dict]] = collections.defaultdict(list)
    for row in rows:
        group = classify(row, page_licences)
        if group:
            groups[group].append({
                "file": row["file"],
                "title": row.get("title"),
                "licence": row.get("licence"),
                "pd_grounds": row.get("pd_grounds"),
                "descriptionurl": row.get("descriptionurl"),
                "tier_order": row.get("tier_order"),
                "reason": group,
            })

    total = sum(len(v) for v in groups.values())
    print(f"{total:,} of {len(rows):,} covers cannot be published as recorded\n")
    for group in ("share-alike", "unattributable", "us-only", "ungrounded"):
        picked = groups.get(group, [])
        if not picked:
            continue
        print(f"{group:16} {len(picked):>5}")
        for row in picked[:3]:
            print(f"                  {row['file']}  {row['descriptionurl'] or ''}")
        if len(picked) > 3:
            print(f"                  ... and {len(picked) - 3:,} more")

    if args.out:
        flat = [r for v in groups.values() for r in v]
        flat.sort(key=lambda r: r["tier_order"] if r["tier_order"] is not None else 0)
        pathlib.Path(args.out).write_text(
            json.dumps({"total": total,
                        "by_reason": {k: len(v) for k, v in sorted(groups.items())},
                        "covers": flat}, indent=2) + "\n")
        print(f"\nwritten: {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
