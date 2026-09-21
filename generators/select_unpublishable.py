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

An audit against Commons found five groups that fail it. They are listed here
rather than handled quietly, because a corpus that drops images without saying
which ones is asking to be taken on trust, and taking a corpus on trust is the
habit this one exists to break.

THE FIVE GROUPS
---------------
`share-alike`
    The file page offers share-alike and nothing matching what we recorded.
    Share-alike is a copyleft obligation: derivatives must be licensed alike.
    Every stego image here is a derivative, and the collection is published
    CC BY 4.0, so keeping these would breach the term AND falsify the
    collection statement, which says CC BY is the strictest obligation present.

`misrecorded`
    The page grants something, and it is not what the manifest says. Not
    necessarily stricter; still wrong, and wrong in the field this corpus
    claims to get right.

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

A FIRST VERSION OF THIS FILE SELECTED 1,177 COVERS, and 1,068 of those were
sound. Two rules were too strict, and both were too strict in the same way:
they read a single fact off a page and did not ask what the page as a whole
granted.

    - Any page mentioning share-alike was flagged. Commons files are very
      often offered under several licences at once, `Cc-by-4.0` beside `GFDL`
      being the commonest, and taking the permissive one is the point of a
      multi-licence offer rather than an abuse of it.
    - Any `PD-USGov` ground was read as United States only. A work of the US
      federal government is uncopyrighted by statute rather than by a term
      expiring, and is treated as free worldwide.

Dropping 12% of a corpus is not a safe default just because it errs towards
caution. It throws away work, it changes what the corpus measures, and it
would have been done on a rule nobody checked.

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

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from manifest_repair import UNUSABLE_ARTIST  # noqa: E402

#: Grounds that rest on a United States COPYRIGHT TERM having run out, and say
#: nothing about the source country. A photograph first published in Hungary in
#: 1930 can be out of copyright in the United States and in copyright at home,
#: and this corpus is served from the United Kingdom to everywhere.
#:
#: `PD-USGov*` is deliberately NOT here, though it names the United States in
#: every one of its 576 appearances. A work of the US federal government is
#: uncopyrighted by statute rather than by expiry, the government does not
#: assert copyright in it abroad either, and Commons hosts it on that basis.
#: Treating those as jurisdiction-limited would have dropped 576 sound covers,
#: which is what a first version of this file did.
US_EXPIRY_PREFIXES = ("PD-US", "PD-1996")

#: A ground that says the work is free everywhere, or that the author released
#: it themselves. Either settles the jurisdiction question.
UNIVERSAL_PREFIXES = ("PD-self", "PD-user", "PD-author", "CC0",
                      "PD-old", "PD-art", "PD-Art", "PD-scan", "PD-because",
                      "No rights reserved", "Copyrighted free use")


def is_us_only(grounds: list[str]) -> bool:
    """Every stated ground is a United States copyright term expiring.

    Asked as "are they ALL US expiry" rather than "is any", because a file
    tagged both `PD-US-expired` and `PD-old-70` is free on a ground that does
    not depend on where the reader is.
    """
    if not grounds:
        return False
    if any(g.startswith(UNIVERSAL_PREFIXES) for g in grounds):
        return False
    if any(g.startswith("PD-USGov") for g in grounds):
        return False
    return all(g.startswith(US_EXPIRY_PREFIXES) for g in grounds)


def matches(stated: str, recorded: str) -> bool:
    """Whether a template on the page grants what the manifest recorded.

    Coarse, and deliberately generous about version: `Cc-by-3.0` on the page
    against `CC BY 3.0` recorded is the same grant, and this is not the place
    to argue about 3.0 versus 4.0.
    """
    s = stated.lower().replace("_", "-")
    r = recorded.lower().replace(" ", "-")
    if r.startswith("public-domain"):
        return s.startswith(("pd", "cc-pd", "cc-zero", "no-rights",
                             "copyrighted-free-use"))
    if r.startswith("cc0"):
        return s.startswith(("cc-zero", "cc0", "cc-pd", "pd"))
    if r.startswith("cc-by-sa"):
        return s.startswith(("cc-by-sa", "cc-sa"))
    if r.startswith("cc-by"):
        # A share-alike tag does NOT satisfy a plain CC BY record: the two
        # carry different obligations and we would be publishing under the
        # looser one.
        return s.startswith("cc-by") and not s.startswith(("cc-by-sa", "cc-sa"))
    return False


def classify(row: dict, page_licences: dict[str, list[str]]) -> str | None:
    """Which group a row falls into, or None if it is publishable."""
    licence = (row.get("licence") or "").strip()
    lowered = licence.lower()

    # A COMMONS FILE MAY OFFER SEVERAL LICENCES AND THE USER PICKS ONE.
    #
    # The question is not "does this page mention share-alike" but "does it
    # offer anything matching what we recorded". A file offered as CC BY 2.5 or
    # CC BY-SA 3.0 or GFDL is one we may take under CC BY 2.5, which is the
    # permissive option and the one recorded.
    #
    # Asking the first question flagged 522 covers, almost all of them dual
    # licensed and correctly recorded, because GFDL sits beside a CC BY tag on
    # a great many Commons files. The share-alike group is for a page that
    # offers share-alike AND NOTHING ELSE while the manifest claims otherwise.
    stated = page_licences.get(row.get("title", ""), [])
    if stated and not any(matches(s, licence) for s in stated):
        if any(s.lower().startswith(("cc-by-sa", "cc-sa", "gfdl"))
               for s in stated):
            return "share-alike"
        return "misrecorded"

    # The SAME rule the credit line uses. An earlier version tested only for an
    # empty string, so twelve covers whose artist read "Unknown author" passed
    # here and then produced a credit line saying "author not recorded by the
    # source". One definition, imported, rather than two that agree by habit.
    if (row.get("attribution_required")
            and (row.get("artist") or "").strip().lower() in UNUSABLE_ARTIST):
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
    for group in ("share-alike", "misrecorded", "unattributable", "us-only",
                  "ungrounded"):
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
