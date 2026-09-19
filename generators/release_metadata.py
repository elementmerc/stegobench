#!/usr/bin/env python3
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
README.md        the entry point, with the non-comparability warning first
CITATION.cff     GitHub and Zenodo both read this to render a citation
croissant.json   ML Commons metadata, which Kaggle and HuggingFace index
DATASHEET.md     Gebru et al., the questions a reviewer will ask anyway
SPLITS.md        how to split without leaking a cover across the boundary

Usage::

    python release_metadata.py --release ~/pentimento/release/core \\
                               --arms-index ~/pentimento/release/core-arms/pentimento-core-arms-index.json \\
                               --version 1.0.0
"""
from __future__ import annotations

import argparse
import datetime
import json
import pathlib
import sys

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


def load(path: pathlib.Path) -> dict:
    return json.loads(path.read_text())


def readme(cover_index: dict, licences: dict, arms: dict | None, version: str) -> str:
    total_covers = cover_index.get("samples", licences.get("total", 0))
    cover_shards = len(cover_index.get("shards", []))
    arm_lines, arm_total, arm_count = "", 0, 0
    if arms:
        arm_count = len(arms.get("arms", []))
        arm_total = arms.get("total_samples", 0)
        rows = []
        for a in sorted(arms.get("arms", []), key=lambda x: x["arm"]):
            rows.append(f"| `{a['arm']}` | {a['samples']:,} | {len(a['shards'])} |")
        arm_lines = "\n".join(rows)

    attribution = licences.get("attribution_required", 0)
    pct = licences.get("attribution_required_pct", 0)
    table = "\n".join(
        f"| {name} | {count:,} |"
        for name, count in sorted(licences.get("licences", {}).items(),
                                  key=lambda kv: -kv[1])
    )

    return f"""# Pentimento Core, v{version}

**{NOT_COMPARABLE}**

A steganalysis corpus of {total_covers:,} permissively licensed cover
photographs and {arm_total:,} matched stego pairs across {arm_count} arms,
where every image carries its own licence and every file carries its own
checksum.

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
recorded with a sha256 and the count of samples actually changed. A nine point
accuracy swing has been measured in the literature from the data split alone,
so a corpus that cannot be rebuilt identically cannot support a comparison.

## Layout

Shards are [WebDataset](https://github.com/webdataset/webdataset) tar files.
Inside each, a sample's parts share a basename:

```
pentimento-core-00000.tar
  000000.png     the image
  000000.json    its manifest row, licence included
```

They stream without unpacking, and every major dataset loader reads them.

| Part | Files | Shards |
|---|---|---|
| Covers | {total_covers:,} | {cover_shards} |
{arm_lines}

Each part carries an index JSON with a sha256 per shard. Verify before use.

## Licensing, in one paragraph

Every file carries its own licence and the manifest is authoritative. The
collection is published as **CC BY 4.0**, which is the strictest obligation
present, not the loosest: complying with it satisfies every file here. Where
the manifest records something looser for a given image, rely on that instead.

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
"""


def citation(version: str, today: str) -> str:
    return f"""cff-version: 1.2.0
message: "If you use this corpus, please cite it as below."
title: "Pentimento Core: a licence-traceable steganalysis corpus"
abstract: >-
  {NOT_COMPARABLE}
  A steganalysis corpus of permissively licensed cover photographs with matched
  stego pairs across adaptive spatial schemes, JPEG schemes and real end-user
  tools, kept as separate labelled arms rather than a blend, with per-file
  licensing and per-file checksums.
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


def croissant(cover_index: dict, licences: dict, arms: dict | None,
              version: str, today: str) -> dict:
    """ML Commons Croissant, which is what Kaggle and HuggingFace actually index."""
    distribution = [{
        "@type": "cr:FileSet",
        "@id": "covers",
        "name": "covers",
        "description": f"{cover_index.get('samples', 0)} cover images as WebDataset shards.",
        "encodingFormat": "application/x-tar",
        "includes": "pentimento-core-*.tar",
    }]
    if arms:
        distribution.append({
            "@type": "cr:FileSet",
            "@id": "arms",
            "name": "arms",
            "description": (f"{arms.get('total_samples', 0)} stego images across "
                            f"{len(arms.get('arms', []))} labelled arms."),
            "encodingFormat": "application/x-tar",
            "includes": "pentimento-core-*-*.tar",
        })
    return {
        "@context": {
            "@vocab": "https://schema.org/",
            "cr": "http://mlcommons.org/croissant/",
            "sc": "https://schema.org/",
        },
        "@type": "sc:Dataset",
        "conformsTo": "http://mlcommons.org/croissant/1.0",
        "name": "pentimento-core",
        "version": version,
        "datePublished": today,
        "description": NOT_COMPARABLE + " A steganalysis corpus with per-file "
                       "licensing and per-file checksums.",
        "license": "https://creativecommons.org/licenses/by/4.0/",
        "creator": {"@type": "Person", "name": "Daniel Iwugo"},
        "keywords": ["steganalysis", "steganography", "image forensics",
                     "dataset", "cover source mismatch"],
        "distribution": distribution,
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

**How many?** {n:,} covers and {arms.get('total_samples', 0) if arms else 0:,}
stego images across {len(arms.get('arms', [])) if arms else 0} arms.

**Is any information missing?** Some covers carry no recorded author. Those
cases say so in the `attribution` field rather than omitting it.

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
"""


def splits() -> str:
    return """# Splitting this corpus without leaking

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

Every sample JSON carries `source_png`, the cover it descends from. Partition on
that field, then take whole groups:

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

## Across arms too

The same cover appears in every arm. If you train on `wow-0200` and test on
`hugo-0200` without partitioning by cover, the photographs are shared across the
boundary even though the arms are different, and the leak is the same leak.

## Reporting

State the fold rule you used. Nine points of accuracy have been measured in the
literature from the data split alone, same network, same algorithm, so a result
without its split rule is not comparable to anything.
"""


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--release", required=True, help="the packed cover release directory")
    ap.add_argument("--arms-index", default=None, help="the packed arms index JSON")
    ap.add_argument("--version", default="1.0.0")
    args = ap.parse_args(argv)

    rel = pathlib.Path(args.release)
    if not rel.is_dir():
        print(f"no release directory at {rel}", file=sys.stderr)
        return 1

    cover_index_path = rel / "pentimento-core-index.json"
    licences_path = rel / "licence-summary.json"
    for p in (cover_index_path, licences_path):
        if not p.exists():
            print(f"missing {p.name}; run pack_tier.py first", file=sys.stderr)
            return 1

    cover_index = load(cover_index_path)
    licences = load(licences_path)
    arms = load(pathlib.Path(args.arms_index)) if args.arms_index and \
        pathlib.Path(args.arms_index).exists() else None
    if args.arms_index and arms is None:
        print(f"note: {args.arms_index} not found, writing covers-only metadata",
              file=sys.stderr)

    today = datetime.date.today().isoformat()
    written = {
        "README.md": readme(cover_index, licences, arms, args.version),
        "CITATION.cff": citation(args.version, today),
        "croissant.json": json.dumps(
            croissant(cover_index, licences, arms, args.version, today),
            indent=2, sort_keys=True) + "\n",
        "DATASHEET.md": datasheet(cover_index, licences, arms),
        "SPLITS.md": splits(),
    }
    for name, body in written.items():
        (rel / name).write_text(body)
        print(f"  {name}  {len(body):>6} bytes")

    print(f"\n{len(written)} file(s) written to {rel}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
