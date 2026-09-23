#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Write the files a published dataset needs and this one was missing.

WHY THIS FILE EXISTS
--------------------
An audit of the packed release against what a research corpus is expected to
ship found the pixels and the licensing in good order and the paperwork absent.
A downloader got ten tar files, three JSON files and no entry point: no README,
nothing to cite, no machine-readable description, and no statement of how the
corpus should be split, despite cross-arm leakage being a documented
correctness hazard in this corpus's own design notes.

Every file here is DERIVED from the manifests rather than typed, for the reason
`distribution.md` gives: a human copying a licence string into a web form is
the failure mode being designed out, and it is how Dresden became CC0 and UCID
became MIT in the mirrors this corpus exists to correct.

WHAT IT WRITES
--------------
README.md          the entry point, with the non-comparability warning first
CITATION.cff       GitHub and Zenodo both read this to render a citation
croissant.json     ML Commons metadata, which Kaggle and HuggingFace index
DATASHEET.md       Gebru et al., the questions a reviewer will ask anyway
SPLITS.md          how to split without leaking a cover across the boundary
SHA256SUMS-<part>  so `sha256sum -c` works without parsing an index
ATTRIBUTION.md     the credit lines for the 54% that require one
ATTRIBUTION.csv    the same, joinable
load_pentimento.py a reader that runs with nothing installed

Usage::

    python release_metadata.py --release ~/pentimento/release/core \\
                               --arms-index ~/pentimento/release/core-arms/pentimento-core-arms-index.json \\
                               --version 1.0.0
"""
from __future__ import annotations

import argparse
import csv
import datetime
import hashlib
import io
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from tiers import TierError, tier_cover_names  # noqa: E402

#: The one sentence that must appear before anything else, everywhere. A reader
#: who takes a number from here and compares it with a BOSSbase number has been
#: misled, and the corpus is responsible for saying so first rather than in a
#: footnote.
NOT_COMPARABLE = (
    "This is a JPEG-decompressed spatial corpus. It is NOT comparable to "
    "BOSSbase, whose covers were never JPEG compressed. Detector numbers "
    "measured here and numbers measured on BOSSbase cannot be placed in the "
    "same table."
)


def digest_of(path: pathlib.Path) -> str:
    """The sha256 of a file, in blocks, matching `publish_tier.digest_of`."""
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for block in iter(lambda: fh.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def load(path: pathlib.Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def split_arms(arms: dict | None) -> tuple[int, int, int, int]:
    """Stego arms and clean arms counted separately.

    The clean arms are the other half of the pairs, not more pairs. Counting
    them as stego samples overstated the corpus by the size of its own control
    group, in three files that each did the arithmetic for themselves.
    """
    stego_total = stego_count = clean_total = clean_count = 0
    for a in (arms or {}).get("arms", []):
        if a["arm"].startswith("clean"):
            clean_count += 1
            clean_total += a["samples"]
        else:
            stego_count += 1
            stego_total += a["samples"]
    return stego_total, stego_count, clean_total, clean_count


def readme(cover_index: dict, licences: dict, arms: dict | None, version: str) -> str:
    tier = cover_index.get("tier", "Core")
    total_covers = cover_index.get("samples", licences.get("total", 0))
    cover_shards = len(cover_index.get("shards", []))
    stego_total, stego_count, clean_total, clean_count = split_arms(arms)
    # THE RATE UNIT COLUMN.
    #
    # The table listed `hugo-0400` and `steghide-0500` as adjacent rows with a
    # bare sample count, and nothing in any SHIPPED file said that the four
    # digits mean three incompatible quantities: bits per pixel for the
    # spatial arms, bits per non-zero AC coefficient for the JPEG ones, and a
    # fraction of capacity for steghide. All of that lived on the docs site,
    # which is not part of any download. A reader who curls a shard, as this
    # README's own quickstart tells them to, could rank schemes "at rate 0.2"
    # while comparing three different things.
    #
    # The per-sample JSON has carried `rate_unit` correctly all along. This
    # puts it where a human reads it.
    UNITS = {
        "hugo": "bits per pixel", "wow": "bits per pixel",
        "suniward": "bits per pixel", "hill": "bits per pixel",
        "mipod": "bits per pixel",
        "juniward": "bits per non-zero AC", "uerd": "bits per non-zero AC",
        "steghide": "fraction of capacity",
        "outguess": "fraction of capacity",
        "append_after_eoi": "not a rate: a fixed trailer",
    }

    def unit_of(arm_name: str) -> str:
        if arm_name.startswith("clean"):
            return "n/a, clean"
        return UNITS.get(arm_name.rsplit("-", 1)[0], "unstated")

    arm_lines = "\n".join(
        f"| `{a['arm']}` | {a['samples']:,} | {len(a['shards'])} | "
        f"{unit_of(a['arm'])} |"
        for a in sorted((arms or {}).get("arms", []), key=lambda x: x["arm"]))

    attribution = licences.get("attribution_required", 0)
    pct = licences.get("attribution_required_pct", 0)
    table = "\n".join(
        f"| {name} | {count:,} |"
        for name, count in sorted(licences.get("licences", {}).items(),
                                  key=lambda kv: -kv[1])
    )

    # YAML FRONTMATTER, for HuggingFace.
    #
    # This file is uploaded to the repository root, where HuggingFace treats it
    # as the dataset card. Without a frontmatter block the page renders with no
    # licence chip and the licence filter does not surface the dataset at all,
    # so a reader arriving through HF saw a card whose licensing paragraph was
    # a hundred lines down. The Internet Archive and Kaggle ignore the block;
    # it is valid Markdown everywhere else.
    return f"""---
license: cc-by-4.0
license_details: >-
  The collection is CC BY 4.0, the strictest obligation present. Individual
  cover photographs are third-party Wikimedia Commons works under CC0, public
  domain or CC BY 1.0 through 4.0, each credited in ATTRIBUTION.csv and
  carrying its own licence in its record.
pretty_name: "Pentimento {tier}"
task_categories:
  - image-classification
tags:
  - steganalysis
  - steganography
  - image-forensics
  - cover-source-mismatch
size_categories:
  - {"100K<n<1M" if stego_total >= 100000 else "10K<n<100K"}
---

# Pentimento {tier}, v{version}

**{NOT_COMPARABLE}**

A steganalysis corpus of {total_covers:,} permissively licensed cover
photographs and {stego_total:,} matched stego pairs across {stego_count} arms,
where every image carries its own licence and every file carries its own
checksum. A further {clean_count} arms hold the {clean_total:,} clean halves
those pairs are measured against.

## What makes it different

Three things, each of which exists because something else was missing.

**Adaptive schemes alongside real tools.** REVEAL, from the Netherlands
Forensic Institute, covers more than 50 end-user hiding tools and excludes
content-adaptive academic schemes on purpose. This corpus carries HUGO, WOW,
S-UNIWARD, HILL, MiPOD, J-UNIWARD and UERD as separate labelled arms.

**Per-source arms, not a blend.** The ranking of embedding schemes is known to
invert when the cover source changes. A corpus that blends sources hides
exactly the effect a user most needs to measure, so the arms here stay
separate and labelled.

**Rebuildable byte for byte.** Every generator is seeded and every file is
recorded with a sha256 and the count of samples actually changed. Results in
the steganalysis literature differ by several accuracy points on the choice of
data split alone, same network and same algorithm, so a corpus that cannot be
rebuilt identically cannot support a comparison.

## Layout

Shards are [WebDataset](https://github.com/webdataset/webdataset) tar files.
Inside each, a sample's parts share a basename:

```
pentimento-{tier.lower()}-00000.tar
  000000.png     the image
  000000.json    its manifest row, licence included
```

They stream without unpacking, and every major dataset loader reads them.

| Part | Files | Shards | What the rate means |
|---|---|---|---|
| Covers | {total_covers:,} | {cover_shards} | n/a |
{arm_lines}

**The four digits in an arm name are not one quantity.** `hugo-0200` is 0.2
bits per pixel; `juniward-0200` is 0.2 bits per non-zero AC coefficient;
`steghide-0200` is 0.2 of the capacity steghide itself reports. Ranking arms
on the number in the name compares three different things. Every sample's JSON
record carries `rate_unit`, which is the authoritative answer per file.

**Every JPEG arm is written at quality 95**, and the adaptive spatial arms are
**simulated at the optimal embedding rate rather than by a real STC coder** -
the `coding` field on each record says so. Both are ordinary practice and both
change what a result means, so neither should be discovered after the fact.

Each part ships its own checksum file, `SHA256SUMS-covers` and
`SHA256SUMS-arms`, so verifying a download is one command:

```
sha256sum -c SHA256SUMS-covers
```

Do it before use: a shard that arrived truncated reads as a smaller corpus
rather than as an error.

Tiers nest. Nano is the first 200 covers of the same ordering Lite's first
1,000 and Core's 10,000 follow, so you can develop against a small tier and
evaluate on a larger one without the two overlapping in a way that flatters
the result.

## Licensing, in one paragraph

Every file carries its own licence, and **each file's own `.json` member inside
the shards is the authoritative record** of it. `ATTRIBUTION.csv` beside this
file is an extract of those records for the covers that require a credit line,
and is the one to read if you want the licences without downloading the
shards. (This paragraph used to point at "the manifest", which does not ship as
a standalone file; a reader who wanted to check a licence before committing to
48 GB was sent to something they could not find.)
The collection is published as **CC BY 4.0**, which is the strictest obligation
present, not the loosest: complying with it satisfies every file here. Where a
file's own record names something looser, rely on that instead.

| Licence | Covers |
|---|---|
{table}

**{attribution:,} of {total_covers:,} covers ({pct}%) require attribution**, and
each carries a ready-made credit line in its manifest row under
`attribution`. Stego images are derivatives and inherit their cover's terms;
every stego sample JSON carries the cover's licence under `cover_licence`, so
a reader holding only one arm can still discharge the obligation.

See `LICENCES.md` for the full statement.

## Before you train on it

Read `SPLITS.md`. A cover and its stego versions are far more alike than any
two unrelated photographs, so a random split puts a cover in training and its
own stego copy in test, and the classifier learns the photograph rather than
the payload. Split by cover, never by image.

## Citing

See `CITATION.cff`, which GitHub and Zenodo both render automatically.

## Provenance

Covers come from Wikimedia Commons under permissive licences, deduplicated by
perceptual hash across sources and sessions, and capped per photographer and
per camera body so no single prolific uploader dominates the sample. Rejected
candidates are recorded with their reason rather than silently dropped.

A caveat on that last sentence, kept because it was not true once. Cover
SELECTION records its rejections. The arm BUILDERS did not: a failure during
embedding was written to stderr and the loop continued, so 118 covers were
missing from every spatial arm with nothing in the corpus saying so, and the
build log that held the reason had been superseded. The images turned out to
have been written without their manifest rows, they were rebuilt, and the
arms are complete. The builders now record what they skip.
"""


def citation(version: str, today: str, tier: str = "Core") -> str:
    """The CFF record.

    It carried no `url`, no `repository-code` and no `contact` until
    2026-09-22. The README tells a reader that GitHub and Zenodo render this
    file automatically, and rendered, it produced a citation with no way to
    find the dataset or reach anybody about it. That matters MORE while the
    DOI is deferred, not less: every citation minted in the interim is the one
    without a resolvable identifier, and those references are permanent in the
    literature.

    The abstract now also says the covers are third-party photographs. Without
    that, `creator: Daniel Iwugo` beside `license: CC-BY-4.0` reads to any
    machine as a claim of authorship over 10,000 other people's work, which is
    the exact over-claim this corpus exists to argue against.
    """
    return f"""cff-version: 1.2.0
message: "If you use this corpus, please cite it as below."
title: "Pentimento {tier}: a licence-traceable steganalysis corpus"
url: "https://archive.org/details/pentimento-{tier.lower()}-v1"
repository-code: "https://github.com/elementmerc/pentimento"
contact:
  - family-names: Iwugo
    given-names: Daniel
abstract: >-
  {NOT_COMPARABLE}
  A steganalysis corpus of permissively licensed cover photographs with matched
  stego pairs across adaptive spatial schemes, JPEG schemes and real end-user
  tools, kept as separate labelled arms rather than a blend, with per-file
  licensing and per-file checksums.
  The cover photographs are third-party works from Wikimedia Commons, each
  retaining its own licence and credited individually in ATTRIBUTION.csv; the
  corpus is the assembly, the labelling and the derived stego images.
type: dataset
version: "{version}"
date-released: "{today}"
license: CC-BY-4.0
keywords:
  - steganalysis
  - steganography
  - image forensics
  - dataset
  - cover source mismatch
authors:
  - family-names: Iwugo
    given-names: Daniel
"""


#: The official Croissant 1.0 context, verbatim. An abbreviated one looks
#: harmless and is not: the reference validator resolves `recordSet`, `field`,
#: `source` and the rest THROUGH this map, so a record carrying a shortened
#: context fails to expand and is rejected before anything in it is read.
CROISSANT_CONTEXT = {
    "@language": "en",
    "@vocab": "https://schema.org/",
    "citeAs": "cr:citeAs",
    "column": "cr:column",
    "conformsTo": "dct:conformsTo",
    "cr": "http://mlcommons.org/croissant/",
    "rai": "http://mlcommons.org/croissant/RAI/",
    "data": {"@id": "cr:data", "@type": "@json"},
    "dataType": {"@id": "cr:dataType", "@type": "@vocab"},
    "dct": "http://purl.org/dc/terms/",
    "equivalentProperty": "cr:equivalentProperty",
    "examples": {"@id": "cr:examples", "@type": "@json"},
    "extract": "cr:extract",
    "field": "cr:field",
    "fileProperty": "cr:fileProperty",
    "fileObject": "cr:fileObject",
    "fileSet": "cr:fileSet",
    "format": "cr:format",
    "includes": "cr:includes",
    "isLiveDataset": "cr:isLiveDataset",
    "jsonPath": "cr:jsonPath",
    "key": "cr:key",
    "md5": "cr:md5",
    "parentField": "cr:parentField",
    "path": "cr:path",
    "recordSet": "cr:recordSet",
    "references": "cr:references",
    "regex": "cr:regex",
    "repeated": "cr:repeated",
    "replace": "cr:replace",
    "samplingRate": "cr:samplingRate",
    "sc": "https://schema.org/",
    "separator": "cr:separator",
    "source": "cr:source",
    "subField": "cr:subField",
    "transform": "cr:transform",
}

#: Where the canonical copy lives. The Archive is the one host that needs no
#: account and makes no promise it can later withdraw.
ARCHIVE_ITEM = "https://archive.org/download/pentimento-{slug}-v1"


def croissant(cover_index: dict, licences: dict, arms: dict | None,
              version: str, today: str) -> dict:
    """ML Commons Croissant, which is what Kaggle and HuggingFace actually index.

    The shape matters as much as the content. A `cr:FileSet` has to be
    contained in something, and a record set has to say which fields it has and
    where each one is extracted from, or a reader can discover that the dataset
    exists and still not be able to load a single sample from it.
    """
    tier = cover_index.get("tier", "Core")
    slug = tier.lower()
    stego_total, stego_count, clean_total, clean_count = split_arms(arms)

    distribution = [
        {
            "@type": "cr:FileObject",
            "@id": "archive",
            "name": "archive",
            # THE `sha256` BELOW IS A PLACEHOLDER, AND IT HAS TO BE.
            #
            # A panel flagged it on 2026-09-22 as a false assertion: a GitHub
            # issue URL sitting in a checksum field, in a record HuggingFace
            # mirrors and keeps in history, undercutting the one claim this
            # corpus rests on at exactly the point a machine reads it. That
            # reading is right, and the obvious fix - drop the key - was tried
            # the same night and is WRONG. mlcroissant refuses the record:
            #
            #   ValidationError: [Metadata > FileObject(archive)] At least one
            #   of these properties should be defined: ['md5', 'sha256'].
            #
            # It is an error, not a warning, so dropping the field trades a
            # cosmetic falsehood for an unpublishable record. This FileObject
            # is an Archive ITEM, a container with no bytes of its own, and
            # Croissant has no way to say that - which is what issue 80 is
            # about and why the URL is the community's answer to it.
            #
            # So it stays, and the description beside it now says what it is,
            # so the next reader finds an explanation rather than a bug. The
            # REAL digests, which is what actually matters, are per shard in
            # the FileSets below and in SHA256SUMS-covers / SHA256SUMS-arms.
            "description": "The Internet Archive item holding every shard. "
                           "NOTE: this is an item, not a file, so it has no "
                           "checksum of its own. Croissant requires one on a "
                           "FileObject, so the sha256 field carries a pointer "
                           "to the upstream issue about that requirement "
                           "rather than a digest. Do not parse it. The real "
                           "per-shard digests are in SHA256SUMS-covers and "
                           "SHA256SUMS-arms, and on each FileSet below.",
            "contentUrl": ARCHIVE_ITEM.format(slug=slug),
            "encodingFormat": "text/html",
            "sha256": "https://github.com/mlcommons/croissant/issues/80",
        },
        {
            "@type": "cr:FileSet",
            "@id": "cover-shards",
            "name": "cover-shards",
            "description": f"{cover_index.get('samples', 0):,} cover images as "
                           "WebDataset tar shards.",
            "containedIn": {"@id": "archive"},
            "encodingFormat": "application/x-tar",
            # Anchored on the digits, so this does NOT also swallow the arm
            # shards, whose names carry a tool and a rate between the tier and
            # the shard number.
            "includes": f"pentimento-{slug}-[0-9][0-9][0-9][0-9][0-9].tar",
        },
        {
            "@type": "cr:FileSet",
            "@id": "cover-images",
            "name": "cover-images",
            "description": "The cover images inside the shards.",
            "containedIn": {"@id": "cover-shards"},
            "encodingFormat": "image/png",
            "includes": "*.png",
        },
        {
            "@type": "cr:FileSet",
            "@id": "cover-records",
            "name": "cover-records",
            "description": "One record per cover, carrying its licence and provenance.",
            "containedIn": {"@id": "cover-shards"},
            "encodingFormat": "application/json",
            "includes": "*.json",
        },
    ]
    if arms:
        distribution.append({
            "@type": "cr:FileSet",
            "@id": "arm-shards",
            "name": "arm-shards",
            "description": (f"{stego_total:,} stego images across {stego_count} "
                            f"labelled arms, plus {clean_total:,} clean halves "
                            f"in {clean_count} more."),
            "containedIn": {"@id": "archive"},
            "encodingFormat": "application/x-tar",
            "includes": f"pentimento-{slug}-*-[0-9][0-9][0-9][0-9][0-9].tar",
        })
        # Split out for the same reason the covers are: the image and its
        # record are two files sharing a basename, and a record set drawing on
        # one FileSet cannot address both. The arms ship JPEG as well as PNG,
        # because a JPEG arm's member keeps its real extension.
        distribution.append({
            "@type": "cr:FileSet",
            "@id": "arm-images",
            "name": "arm-images",
            "description": "The sample images inside the arm shards.",
            "containedIn": {"@id": "arm-shards"},
            "encodingFormat": "image/png",
            "includes": "*.png",
        })
        distribution.append({
            "@type": "cr:FileSet",
            "@id": "arm-records",
            "name": "arm-records",
            "description": "One record per sample, carrying its arm, its rate "
                           "and unit, and the cover licence it inherits.",
            "containedIn": {"@id": "arm-shards"},
            "encodingFormat": "application/json",
            "includes": "*.json",
        })

    # The image and its record are two files that share a basename, so a reader
    # assembling a sample has to join them. Croissant will not infer that: a
    # record set drawing on two file sets without a declared join is rejected
    # by the reference validator, which is how the previous record went out
    # unreadable. The key is the basename, and every field taken from the JSON
    # side references it.
    KEY = {
        "fileSet": {"@id": "cover-images"},
        "extract": {"fileProperty": "filename"},
        "transform": {"regex": "^(.*)\\.png$"},
    }

    # The same join for the arms. The regex has to strip either extension,
    # because a JPEG arm's member ships as .jpg and a spatial one as .png;
    # anchoring on .png alone would silently key every JPEG arm on a filename
    # that still carries its suffix, and the join would match nothing.
    ARM_KEY = {
        "fileSet": {"@id": "arm-images"},
        "extract": {"fileProperty": "filename"},
        "transform": {"regex": "^(.*)\\.(?:png|jpg)$"},
    }

    #: Which key a record set joins on, per file set. A field reading from the
    #: JSON side has to declare the join back to the image side or mlcroissant
    #: refuses the record; and it has to be the RIGHT key. This was implicit
    #: ("anything that is not cover-images joins on KEY"), which silently gave
    #: the arm fields the cover join and produced:
    #:
    #:   ValidationError: You try to use the sources with names ('arm-images',
    #:   'arm-records') as sources, but you didn't declare a join between them.
    #:
    #: Naming it per file set is the difference between a rule that happens to
    #: hold for one record set and one that states what it means.
    JOIN_KEY = {
        "cover-images": None,   # this IS the key side
        "cover-records": KEY,
        "arm-images": None,     # likewise, for the arms
        "arm-records": ARM_KEY,
    }

    def field(fid: str, name: str, description: str, data_type: str,
              file_set: str, extract: dict) -> dict:
        spec = {
            "@type": "cr:Field",
            "@id": fid,
            "name": name,
            "description": description,
            "dataType": data_type,
            "source": {"fileSet": {"@id": file_set}, "extract": extract},
        }
        join = JOIN_KEY.get(file_set, KEY)
        if join is not None:
            spec["references"] = dict(join)
        return spec

    record_sets = [{
        "@type": "cr:RecordSet",
        "@id": "covers",
        "name": "covers",
        "description": "One record per cover photograph.",
        "field": [
            dict(field("covers/key", "key",
                       "The sample key, which is the cover's position in tier "
                       "order. It is what joins a cover to its stego versions.",
                       "sc:Text", "cover-images", {"fileProperty": "filename"}),
                 source=dict(KEY)),
            field("covers/image", "image", "The cover image itself.",
                  "sc:ImageObject", "cover-images", {"fileProperty": "content"}),
            field("covers/licence", "licence",
                  "The licence this specific image carries. Authoritative.",
                  "sc:Text", "cover-records", {"jsonPath": "$.licence"}),
            field("covers/attribution", "attribution",
                  "A ready-made credit line, where the licence requires one.",
                  "sc:Text", "cover-records", {"jsonPath": "$.attribution"}),
            field("covers/source_url", "source_url",
                  "Where the original came from on Wikimedia Commons.",
                  "sc:URL", "cover-records", {"jsonPath": "$.descriptionurl"}),
        ],
    }]

    # THE ARMS, which had no recordSet at all until 2026-09-22.
    #
    # `covers` was the only record set, so every automated consumer -
    # HuggingFace's viewer, MLCommons tooling, anything reading the Croissant
    # record - presented Pentimento as a 10,000 image dataset with a licence
    # column. The 341,997 stego samples appeared only inside a FileSet's
    # free-text description.
    #
    # `source_png` as a `references` join to `covers/key` is the load-bearing
    # part. The central instruction of this corpus is split by cover, never by
    # image, and without that reference a loader has no machine-readable way
    # to know which samples share a photograph. The warning existed in prose
    # only, which is to say it did not exist for the tools that need it.
    if arms:
        record_sets.append({
            "@type": "cr:RecordSet",
            "@id": "samples",
            "name": "samples",
            "description": f"One record per stego or clean sample. "
                           f"{stego_total:,} stego samples across "
                           f"{stego_count} arms and {clean_total:,} clean "
                           f"across {clean_count}. Every sample descends from "
                           f"one cover; partition on source_png, not on key.",
            "field": [
                dict(field("samples/key", "key",
                           "The sample key: its position within its arm. It "
                           "is NOT the cover key, and two arms use it for "
                           "different photographs.",
                           "sc:Text", "arm-images",
                           {"fileProperty": "filename"}),
                     source=dict(ARM_KEY)),
                field("samples/image", "image", "The sample image.",
                      "sc:ImageObject", "arm-images",
                      {"fileProperty": "content"}),
                field("samples/arm", "arm",
                      "Which arm this sample belongs to, as the shard names "
                      "it.", "sc:Text", "arm-records", {"jsonPath": "$.arm"}),
                field("samples/rate", "rate",
                      "The embedding rate, in the units given by rate_unit.",
                      "sc:Float", "arm-records", {"jsonPath": "$.rate"}),
                field("samples/rate_unit", "rate_unit",
                      "What the rate MEANS. Bits per pixel and bits per "
                      "non-zero AC coefficient are different quantities, so "
                      "two arms at 0.2 are not comparable without this.",
                      "sc:Text", "arm-records",
                      {"jsonPath": "$.rate_unit"}),
                field("samples/cover_licence", "cover_licence",
                      "The credit line inherited from the cover this sample "
                      "was derived from.",
                      "sc:Text", "arm-records",
                      {"jsonPath": "$.cover_licence.attribution"}),
                # `references` here declares the join to `arm-images`, which
                # is what mlcroissant requires of any field read from the JSON
                # side. It CANNOT also carry the cross-record-set join to
                # `covers/key`: a field has one `references`, and pointing it
                # at the cover key was tried and left the arm join undeclared,
                # which mlcroissant refuses.
                #
                # So the join to the covers is stated in the description
                # instead, where a human reads it. That is weaker than a
                # machine-readable edge and it is what the format allows here.
                # The VALUE is the cover's filename, identical to `covers/key`,
                # so a loader can still join on it; it just is not told to.
                field("samples/source_png", "source_png",
                      "The cover this sample was derived from, as a filename "
                      "identical to the `key` field of the `covers` record "
                      "set. JOIN ON THIS to split without leaking: a cover "
                      "and every sample made from it must stay on the same "
                      "side of any train/test boundary.",
                      "sc:Text", "arm-records", {"jsonPath": "$.source_png"}),
            ],
        })

    return {
        "@context": CROISSANT_CONTEXT,
        "@type": "sc:Dataset",
        "conformsTo": "http://mlcommons.org/croissant/1.0",
        "name": f"pentimento-{slug}",
        "version": version,
        "datePublished": today,
        "description": NOT_COMPARABLE + " A steganalysis corpus with per-file "
                       "licensing and per-file checksums.",
        "license": "https://creativecommons.org/licenses/by/4.0/",
        "url": ARCHIVE_ITEM.format(slug=slug),
        "creator": {"@type": "Person", "name": "Daniel Iwugo"},
        "citeAs": (f"Iwugo, D. ({today[:4]}). Pentimento {tier}: a "
                   f"licence-traceable steganalysis corpus ({version})."),
        "keywords": ["steganalysis", "steganography", "image forensics",
                     "dataset", "cover source mismatch"],
        "distribution": distribution,
        "recordSet": record_sets,
    }


def datasheet(cover_index: dict, licences: dict, arms: dict | None) -> str:
    n = cover_index.get("samples", 0)
    return f"""# Datasheet

Following Gebru et al., *Datasheets for Datasets*. These are the questions a
reviewer asks anyway; answering them here is cheaper than answering them in
correspondence.

## Motivation

**Why was it created?** No single corpus covered real end-user hiding tools and
academic content-adaptive schemes together, across spatial and JPEG domains, at
camera diversity, down to low payload, as separate labelled arms. Everything
with good adaptive coverage was JPEG-only, derived from BOSSbase, and licensed
so restrictively it could not be redistributed.

**Who funded it?** Nobody. It was built to support an independent evaluation and
released because the corpus outlives the evaluation.

## Composition

**What do instances represent?** Photographs, and modified copies of those
photographs carrying a hidden payload. A pair is one photograph twice, differing
only in the embedded bits.

**How many?** {n:,} covers and {split_arms(arms)[0]:,}
stego images across {split_arms(arms)[1]} arms, plus {split_arms(arms)[2]:,}
clean halves in {split_arms(arms)[3]} more.

**Is any information missing?** Not for attribution: every cover requiring a
credit line carries a usable author. Covers whose author could not be
recovered were removed and replaced during the build rather than shipped with
a placeholder, so no `attribution` field reads "author not recorded".

**Does it contain people?** Photographs from Wikimedia Commons may include
people incidentally. No instance is labelled by, or selected for, any attribute
of a person, and the corpus supports no inference about individuals.

**Is it a sample?** Yes. {n:,} covers out of a much larger Commons pool,
deduplicated by perceptual hash and capped per uploader and per camera body so
no single prolific contributor dominates. The cap is recorded in the manifest
so the sampling is reproducible.

## Collection

**How was it collected?** Through the Wikimedia Commons API, filtered to
permissive licences, with per-file provenance recorded at fetch time.

**Over what period?** September 2026.

**Were people told?** The images are published under licences that permit reuse
and redistribution, which is the basis on which they are included. No separate
consent was sought and none is required by those licences.

## Preprocessing

**What was done?** Covers were resized and encoded to a common size and format.
**This makes them JPEG-decompressed spatial images, which is why they are not
comparable to BOSSbase**, whose covers were never JPEG compressed. Rejected
candidates are recorded with their reason.

**Is the raw data available?** Every row records the source URL, so the original
can be refetched from Commons.

## Uses

**What is it for?** Training and evaluating steganalysis detectors, and
measuring how a detector transfers across cover sources.

**What should it NOT be used for?** Any comparison against a corpus whose covers
were never JPEG compressed. Any claim about individuals appearing in the
photographs. Any task where a random train and test split is used, which leaks
covers across the boundary; see `SPLITS.md`.

## Distribution

**How is it distributed?** As WebDataset tar shards with a per-shard sha256,
under CC BY 4.0 as the strictest obligation present in the corpus.

## Maintenance

**Who maintains it?** The author. Corrections and errata are published against
the version they affect; versions are not silently replaced.

**How do I report a problem, or ask for something to be removed?** Open an
issue at <https://github.com/elementmerc/pentimento>. Requests from a
photographer about their own work are acted on.

**What can actually be withdrawn?** The honest answer differs per destination,
and it is better than it was. A HuggingFace repository can be deleted and a
Kaggle dataset removed. An Internet Archive item can be darkened on request,
though its identifier stays taken.

**No torrent is published for this corpus**, and that is the part worth saying
plainly, because a seeded torrent is the one thing on that list that could
never be recalled: darkening the Archive item would remove the web seed but not
the copies already in the swarm. The Archive generates a torrent of its own for
every public item; that one is the Archive's and follows the item.

So a removal request can be honoured everywhere this corpus is published. A
promise of errata is worth what its mechanism is worth, and here the mechanism
exists.
"""


def splits(rows: list[dict] | None = None) -> str:
    """The split document, with its counts taken from the tier's own rows.

    These were a hard-coded "8,032 covers against 1,968" until 2026-09-22, when
    a panel counted the manifest and found 8,029 against 1,971. The literal had
    been right when it was written and the corpus moved underneath it, which is
    the same fault that shipped a wrong attribution figure to a public archive.

    Worse than being wrong, it was wrong in EVERY tier: the same absolute
    numbers shipped with Nano, whose real figures are 167 and 33, so a Nano
    reader was told their 200 cover download held 8,032 training covers.

    So the counts are derived, and when they cannot be, the sentence that would
    have carried them is not written at all. A document that quietly drops a
    figure is recoverable; one that states a confident wrong number is not.
    """
    if rows:
        train = sum(1 for r in rows if r.get("split") == "train")
        test = sum(1 for r in rows if r.get("split") == "test")
        counts = f", {train:,} covers against {test:,},"
    else:
        counts = ""
    return f"""# Splitting this corpus without leaking

**Split by cover, never by image.** This is the one instruction in this corpus
that will silently inflate your results if you ignore it.

Every stego image is a modified copy of one specific cover, and the two are far
more alike than any two unrelated photographs. Split at random and a cover lands
in training while its own stego version lands in test. The classifier then
recognises the photograph rather than the payload, and the test score measures
memorisation.

This is a documented defect in published corpora, and it is invisible unless you
look for it: the number simply comes out better than it should.

## How

Every sample JSON in an arm shard carries `source_png`, the cover it descends
from. The cover shards name the same value `file` instead, so a fold function
written against one raises `KeyError` on the other; `record.get("source_png") or
record["file"]` covers both.

Partition on that value, then take whole groups:

```python
import json, tarfile, hashlib

def fold(source_png: str, folds: int = 5) -> int:
    # Deterministic and independent of ordering, so two people who split the
    # same corpus the same way get the same folds.
    h = hashlib.sha256(source_png.encode()).hexdigest()
    return int(h[:8], 16) % folds
```

A cover and every stego image derived from it land in the same fold, whatever
arm they came from.

## The `split` field is a second, different partition

Cover records also carry a fixed `split` of `train` or `test`{counts} and a
`split_salt` naming the string those labels were derived
under. It is partitioned by cover, so it does not leak either, but it is NOT the
same partition as `fold()` above. Pick one and stay with it; a run that uses
both puts the same photograph on both sides of the boundary.

`split` is recorded in the data, so two readers get identical sets without
reimplementing anything, and it is the one to quote for a headline number. It
rides on the cover records only, so using it from an arm means joining against
the cover tier. `fold()` needs nothing but the arm you already have, and gives
you k folds rather than one holdout.

## Across arms too

The same cover appears in every arm. If you train on `wow-0200` and test on
`hugo-0200` without partitioning by cover, the photographs are shared across the
boundary even though the arms are different, and the leak is the same leak.

## Reporting

State the fold rule you used. Results in the literature differ by several
accuracy points on the choice of data split alone, same network and same
algorithm, so a result without its split rule is not comparable to anything.

This corpus publishes no detector results of its own, and the sentence above
is a statement about the literature rather than a measurement made here. If
you want a number to quote, measure it and say how.
"""


def sha256sums(index: dict, extra: dict[str, str]) -> str:
    """A `sha256sum -c` file for one part of the release.

    The shard digests are taken from the pack index rather than recomputed. The
    index is what the packer verified as it wrote, so re-hashing 45 GB here
    would not be an independent check, it would be the same arithmetic run
    twice at considerable cost. Checking a DOWNLOAD against this file is the
    independent check, and it is the reader who performs it.

    `extra` carries the small hand-readable files, which are hashed directly
    because they are written in this same run and are a few kilobytes each.
    """
    lines = []
    for shard in index.get("shards", []):
        lines.append(f"{shard['sha256']}  {shard['shard']}")
    for arm in index.get("arms", []):
        for shard in arm.get("shards", []):
            lines.append(f"{shard['sha256']}  {shard['shard']}")
    # Sorted by name so two runs produce byte-identical output regardless of
    # the order the arms happen to appear in the index.
    lines.sort(key=lambda line: line.split("  ", 1)[1])
    lines.extend(f"{digest}  {name}" for name, digest in sorted(extra.items()))
    return "\n".join(lines) + "\n"


def attribution(rows: list[dict]) -> str:
    """The credit lines for every cover that requires one, in one place.

    The obligation is real and it is the reader's: 54% of these covers are
    CC BY. Every row already carries a ready-made `attribution` string, but a
    reader discharging the licence should not have to open 10,000 JSON records
    inside tar shards to collect them. Deriving the list costs nothing and is
    the difference between a licence that can be complied with and one that
    technically could be.
    """
    required = sorted(
        (r for r in rows if r.get("attribution_required")),
        key=lambda r: r["file"],
    )
    by_licence: dict[str, int] = {}
    for r in required:
        by_licence[r.get("licence", "unknown")] = by_licence.get(r.get("licence", "unknown"), 0) + 1

    out = [
        "# Attribution",
        "",
        f"**{len(required):,} of {len(rows):,} covers require attribution.** Every credit "
        "line below is reproduced from the image's own manifest row, which remains "
        "authoritative if the two ever disagree.",
        "",
        "Stego images are derivatives and inherit their cover's terms, so crediting the "
        "cover credits every image derived from it. If you used one arm rather than the "
        "whole corpus, each sample record names its cover under `source_png`, and you "
        "need only the lines for the covers you actually used.",
        "",
        "`ATTRIBUTION.csv` beside this file carries the same list in a form you can join "
        "against.",
        "",
        "## What is here",
        "",
        "| Licence | Covers |",
        "|---|---|",
    ]
    for licence, count in sorted(by_licence.items(), key=lambda kv: -kv[1]):
        out.append(f"| {licence} | {count:,} |")
    out += ["", "## Credits", ""]
    for r in required:
        out.append(f"- `{r['file']}` {r.get('attribution', '')}")
    return "\n".join(out) + "\n"


def attribution_csv(rows: list[dict]) -> str:
    buf = io.StringIO()
    writer = csv.writer(buf, lineterminator="\n")
    writer.writerow(["file", "licence", "licence_url", "artist", "title",
                     "source", "attribution"])
    for r in sorted((r for r in rows if r.get("attribution_required")),
                    key=lambda r: r["file"]):
        writer.writerow([
            r.get("file", ""), r.get("licence", ""), r.get("licence_url", ""),
            r.get("artist", ""), r.get("title", ""), r.get("descriptionurl", ""),
            r.get("attribution", ""),
        ])
    return buf.getvalue()


def loader() -> str:
    """A reader that works with nothing installed, and one that scales.

    The documentation said "any loader that reads WebDataset works unchanged",
    which is true and is not a starting point. Somebody who has just downloaded
    45 GB wants a file they can run.
    """
    return '''#!/usr/bin/env python3
"""Read Pentimento shards, with or without the webdataset package.

Two ways in. The first needs nothing beyond the standard library and is enough
to look at the corpus; the second is what you would train on.

    python load_pentimento.py pentimento-core-00000.tar

Shards are ordinary tar files. Each sample is an image and a JSON record that
share a basename, so a sample is whatever group of members has the same stem.
"""
from __future__ import annotations

import io
import json
import sys
import tarfile
from collections.abc import Iterator


def samples(shard: str) -> Iterator[tuple[str, bytes, dict]]:
    """Yield (key, image bytes, record) from one shard. Standard library only.

    Streams rather than extracting, so a 1.3 GB shard costs one sample of
    memory rather than 1.3 GB of disk. Members of a sample are adjacent in the
    tar because the packer writes them that way, but this does not rely on it.
    """
    pending: dict[str, dict] = {}
    with tarfile.open(shard, "r|*") as tar:          # "r|*" is the streaming form
        for member in tar:
            if not member.isfile():
                continue
            key, _, suffix = member.name.rpartition(".")
            handle = tar.extractfile(member)
            if handle is None:
                continue
            payload = handle.read()
            slot = pending.setdefault(key, {})
            if suffix == "json":
                slot["record"] = json.loads(payload)
            else:
                slot["image"] = payload
            if "record" in slot and "image" in slot:
                yield key, slot["image"], slot["record"]
                del pending[key]
    if pending:
        raise ValueError(
            f"{len(pending)} incomplete sample(s) in {shard}, first: "
            f"{sorted(pending)[0]}. A truncated download is the usual cause; "
            f"check the shard against SHA256SUMS."
        )


def as_webdataset(pattern: str):
    """The training path. Needs `pip install webdataset`.

    `pattern` is a brace expression over shards, for example
    "pentimento-core-{00000..00009}.tar".

    Split by cover before you do this, not after. A cover and its stego
    versions are near-identical, so a random split puts a photograph on both
    sides of the boundary and the model learns the photograph. Every record
    carries `source_png`; partition on that. SPLITS.md has the rule.
    """
    import webdataset as wds

    return (
        wds.WebDataset(pattern)
        .decode("pil")
        .to_tuple("png;jpg;jpeg", "json")
    )


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    count = 0
    for key, image, record in samples(argv[1]):
        if count == 0:
            print(f"first sample: {key}")
            print(f"  bytes      {len(image):,}")
            print(f"  licence    {record.get('licence') or record.get('cover_licence')}")
            print(f"  cover      {record.get('source_png', 'this IS a cover')}")
        count += 1
    print(f"{count:,} samples in {argv[1]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
'''


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--release", required=True, help="the packed cover release directory")
    ap.add_argument("--arms-index", default=None, help="the packed arms index JSON")
    ap.add_argument("--covers-manifest", default=None,
                    help="manifest.jsonl for the covers, which carries the credit lines")
    ap.add_argument("--version", default="1.0.0")
    args = ap.parse_args(argv)

    rel = pathlib.Path(args.release)
    if not rel.is_dir():
        print(f"no release directory at {rel}", file=sys.stderr)
        return 1

    # Globbed rather than named, so Nano and Lite are packaged by the same
    # run as Core rather than by a second code path that can drift from it.
    found = sorted(rel.glob("pentimento-*-index.json"))
    found = [p for p in found if "arms" not in p.name]
    if not found:
        print(f"no pentimento-*-index.json in {rel}; run pack_tier.py first",
              file=sys.stderr)
        return 1
    cover_index_path = found[0]
    licences_path = rel / "licence-summary.json"
    # Each missing file names the tool that WRITES it. The two come from
    # different tools, and a message that names the wrong one sends the reader
    # round the same loop: re-running pack_tier.py never produces the licence
    # summary, so the error repeats and looks like a bug in the packer.
    produced_by = {
        cover_index_path.name: "pack_tier.py",
        licences_path.name: "publish_tier.py prepare",
    }
    for p in (cover_index_path, licences_path):
        if not p.exists():
            print(f"missing {p.name}; run {produced_by[p.name]} first",
                  file=sys.stderr)
            return 1

    cover_index = load(cover_index_path)
    licences = load(licences_path)

    # THE SUMMARY HAS TO DESCRIBE THE MANIFEST THIS RUN WAS GIVEN.
    #
    # Every licence figure below is rendered from `licences`, not recomputed
    # here, so a stale summary produces a README that is wrong in a way no
    # later check reads: `verify_release` examines the manifest, never the
    # prose derived from it. On 2026-09-22 that shipped a README saying 5,429
    # covers require attribution when 5,453 do, because this step ran without
    # `publish_tier prepare` and silently read a summary three days and one
    # cover backfill old.
    #
    # Compared by content rather than by mtime: a copy, a restore or a clock
    # skew all forge a timestamp and none of them changes the bytes.
    if args.covers_manifest:
        manifest_path = pathlib.Path(args.covers_manifest)
        if not manifest_path.is_file():
            print(f"no cover manifest at {manifest_path}", file=sys.stderr)
            return 1
        stamped = licences.get("source_manifest_sha256")
        if not stamped:
            print(f"{licences_path.name} does not say which manifest it was "
                  f"computed from, so it cannot be shown to be current. It "
                  f"predates the stamp; re-run `publish_tier.py prepare` to "
                  f"write one.", file=sys.stderr)
            return 1
        actual = digest_of(manifest_path)
        if stamped != actual:
            print(f"{licences_path.name} was computed from a different cover "
                  f"manifest than the one given here.\n"
                  f"  summary describes : {stamped[:16]}...\n"
                  f"  manifest given    : {actual[:16]}...\n"
                  f"Every licence figure in the README and LICENCES.md comes "
                  f"from that summary, so writing them now would publish "
                  f"numbers for a corpus that no longer exists. Re-run "
                  f"`publish_tier.py prepare` against this manifest first.",
                  file=sys.stderr)
            return 1
    arms = load(pathlib.Path(args.arms_index)) if args.arms_index and \
        pathlib.Path(args.arms_index).exists() else None
    if args.arms_index and arms is None:
        print(f"note: {args.arms_index} not found, writing covers-only metadata",
              file=sys.stderr)

    # The tier's own cover rows, read BEFORE the documents are written because
    # SPLITS.md now derives its counts from them rather than restating a
    # literal. Scoping happens here for the same reason it always did for
    # attribution: the manifest describes the whole corpus, and a smaller tier
    # that quotes Core's numbers is making a false statement about what was
    # downloaded.
    tier_rows: list[dict] | None = None
    if args.covers_manifest:
        manifest = pathlib.Path(args.covers_manifest)
        if not manifest.exists():
            print(f"no covers manifest at {manifest}", file=sys.stderr)
            return 1
        tier_rows = [json.loads(line) for line in
                     manifest.read_text(encoding="utf-8").splitlines()
                     if line.strip()]
        packed = cover_index.get("samples", len(tier_rows))
        if packed < len(tier_rows):
            try:
                in_tier = tier_cover_names(manifest, packed)
            except TierError as e:
                print(f"cannot select the tier for attribution: {e}",
                      file=sys.stderr)
                return 1
            tier_rows = [r for r in tier_rows if r.get("file") in in_tier]
            print(f"  attribution scoped to {cover_index.get('tier', '?')}: "
                  f"{len(tier_rows):,} covers")

    today = datetime.date.today().isoformat()
    written = {
        "README.md": readme(cover_index, licences, arms, args.version),
        "CITATION.cff": citation(args.version, today,
                                 cover_index.get("tier", "Core")),
        "croissant.json": json.dumps(
            croissant(cover_index, licences, arms, args.version, today),
            indent=2, sort_keys=True) + "\n",
        "DATASHEET.md": datasheet(cover_index, licences, arms),
        "SPLITS.md": splits(tier_rows),
        "load_pentimento.py": loader(),
    }

    if tier_rows is not None:
        # Scoped above, where SPLITS.md also needed it. Shipping Core's credit
        # lines with a 200 cover Nano would name photographers whose work is
        # not in the download, which is a false statement about what was used.
        written["ATTRIBUTION.md"] = attribution(tier_rows)
        written["ATTRIBUTION.csv"] = attribution_csv(tier_rows)
    else:
        print("note: no --covers-manifest, so no attribution list is written. "
              "54% of these covers require one.", file=sys.stderr)

    for name, body in written.items():
        (rel / name).write_text(body, encoding="utf-8")
        print(f"  {name}  {len(body):>7} bytes")

    # Last, because it hashes the files written above. Anything that changes
    # after this point invalidates it, which is why nothing does.
    def digests_of(directory: pathlib.Path) -> dict[str, str]:
        return {
            p.name: hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(directory.iterdir())
            if p.is_file() and not p.name.endswith(".tar")
            and not p.name.startswith("SHA256SUMS")
            and p.name != ".upload-budget"
        }

    # NAMED PER PART, because every destination is flat.
    #
    # The Internet Archive item and the HuggingFace repository each hold the
    # covers and the arms side by side in one namespace. Two files both called
    # SHA256SUMS land on the same name there, and the second silently replaces
    # the first, leaving a checksum file that covers 10 shards and claims to
    # cover 769.
    parts = [(rel, cover_index, "covers")]
    if args.arms_index and arms is not None:
        parts.append((pathlib.Path(args.arms_index).parent, arms, "arms"))
    for directory, index, part in parts:
        name = f"SHA256SUMS-{part}"
        body = sha256sums(index, digests_of(directory))
        (directory / name).write_text(body, encoding="utf-8")
        print(f"  {directory.name}/{name}  {len(body.splitlines())} entries")

    print(f"\n{len(written)} file(s) written to {rel}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
