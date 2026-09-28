#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Does Commons hold never-compressed CAMERA originals, and how many?

WHY THIS QUESTION
-----------------
Every one of Pentimento's 10,000 covers was a JPEG before we cropped it, so the
corpus is a JPEG-decompressed spatial one and cannot claim BOSSbase parity. The
obvious fix is a never-compressed arm, and the fetcher already supports it:
`suitable()` accepts TIFF and PNG, and `provenance.pristine()` identifies them.

So why did a fetch that accepts TIFF return zero TIFFs? Two hypotheses, and they
call for different actions:

    supply    Commons holds almost no never-compressed camera photographs, in
              which case no filter change helps and the arm needs another source.
    filters   They exist and our own limits excluded them, most likely the 6 MB
              size cap, since a camera-resolution TIFF is tens of megabytes.

This measures which. It counts a funnel rather than fetching anything.

WHAT THE FIRST PASS FOUND, AND WHY THIS SECOND PASS EXISTS
-----------------------------------------------------------
Uniform random sampling of the file namespace gave 17 non-JPEG images in 200,
of which 6 passed licence, dimension and size together. Then 119 TIFFs sampled
from search returned **zero** with a camera Make in EXIF: the filenames were
manuscript scans, maps and artwork reproductions.

That is a strong signal and a weak sample, drawn from search ranking rather than
at random, so it could be biased toward documents. This asks directly instead,
with queries a camera photograph would match, and reports the hit rate per query
so the answer carries its own error bars.

The method is checked against a positive control: the same EXIF extraction run
over JPEGs, where the answer must be "most of them". A method that finds no
cameras anywhere is broken, not evidence.
"""
from __future__ import annotations

import argparse
import collections
import json
import sys
import time
import urllib.parse
import urllib.request

UA = "PentimentoProbe/1.0 (steganalysis corpus research; via Wikimedia Commons)"
API = "https://commons.wikimedia.org/w/api.php"
PERMISSIVE = ("cc0", "cc by", "public domain", "pd")

#: Queries a camera photograph would match, restricted to TIFF. Deliberately
#: varied: a single query's ranking is not a sample of anything.
QUERIES = [
    "filemime:tiff Nikon",
    "filemime:tiff Canon EOS",
    "filemime:tiff photograph landscape",
    "filemime:tiff portrait photo",
    "filemime:tiff DSLR",
    "filemime:tiff camera raw converted",
]

#: The positive control. If this does not find cameras, the extraction is wrong
#: and every negative result above is meaningless.
CONTROL_QUERY = "filemime:jpeg Nikon"


def api(params: dict, tries: int = 4) -> dict:
    """One API call, with backoff. Commons rate-limits and says so with a 429."""
    last = None
    for attempt in range(tries):
        try:
            query = urllib.parse.urlencode(dict(params, format="json", formatversion="2"))
            req = urllib.request.Request(API + "?" + query, headers={"User-Agent": UA})
            with urllib.request.urlopen(req, timeout=45) as response:
                return json.loads(response.read())
        except Exception as e:  # noqa: BLE001 - retried, then surfaced
            last = e
            time.sleep(8 * (attempt + 1))
    raise RuntimeError(f"API failed after {tries} attempts: {last}")


def camera_make(imageinfo: dict) -> str:
    """EXIF Make, extracted exactly as fetch_commons.exif_of does it.

    Same shape, same key, same flattening. If the two ever diverge, this probe
    stops measuring what the fetcher would actually accept.
    """
    entries = imageinfo.get("metadata")
    if not isinstance(entries, list):
        return ""
    raw = {m.get("name"): m.get("value") for m in entries if isinstance(m, dict)}
    return str(raw.get("Make", "") or "").strip()


def examine(query: str, limit: int, mime: str, delay: float) -> tuple[collections.Counter, list, int]:
    stats: collections.Counter = collections.Counter()
    found: list = []
    result = api({"action": "query", "list": "search", "srsearch": query,
                  "srnamespace": "6", "srlimit": str(limit)})
    titles = [x["title"] for x in result["query"]["search"]]
    total = result["query"]["searchinfo"].get("totalhits", 0)
    time.sleep(delay)

    for start in range(0, len(titles), 20):
        chunk = titles[start:start + 20]
        info = api({"action": "query", "titles": "|".join(chunk), "prop": "imageinfo",
                    "iiprop": "size|mime|metadata|extmetadata"})
        for page in (info.get("query", {}).get("pages") or []):
            ii = (page.get("imageinfo") or [{}])[0]
            if ii.get("mime") != mime:
                continue
            stats["checked"] += 1
            make = camera_make(ii)
            if not make:
                continue
            stats["with_camera_make"] += 1
            em = ii.get("extmetadata") or {}
            licence = (em.get("LicenseShortName", {}) or {}).get("value", "") or ""
            permissive = any(p in licence.lower() for p in PERMISSIVE)
            big = min(ii.get("width", 0), ii.get("height", 0)) >= 512
            if permissive and big:
                stats["usable"] += 1
                found.append((ii.get("size", 0) // 1024, make, licence,
                              page.get("title", "")[:46]))
        time.sleep(delay)
    return stats, found, total


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--limit", type=int, default=40, help="results per query")
    ap.add_argument("--delay", type=float, default=4.0,
                    help="seconds between calls. Commons is donated "
                         "infrastructure and returns 429 when pushed")
    args = ap.parse_args(argv)

    # A caller that redirected stdout may have put something there that
    # cannot be reconfigured, and losing the line buffering is a cosmetic
    # loss where crashing on it is a real one.
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(line_buffering=True)
    overall: collections.Counter = collections.Counter()
    makes: collections.Counter = collections.Counter()
    hits: list = []

    print("POSITIVE CONTROL: the same extraction over JPEGs")
    try:
        stats, _, total = examine(CONTROL_QUERY, args.limit, "image/jpeg", args.delay)
        rate = stats["with_camera_make"] / max(1, stats["checked"])
        print(f"  {stats['with_camera_make']}/{stats['checked']} JPEGs carry a camera Make "
              f"({rate:.0%}), from {total:,} hits")
        if rate < 0.3:
            print("  CONTROL FAILED: the extraction finds almost no cameras even on "
                  "JPEG, so every negative below is a broken method, not a finding.",
                  file=sys.stderr)
            return 2
    except RuntimeError as e:
        print(f"  control could not run: {e}", file=sys.stderr)
        return 2

    print("\nTIFF, by query")
    for query in QUERIES:
        try:
            stats, found, total = examine(query, args.limit, "image/tiff", args.delay)
        except RuntimeError as e:
            print(f"  {query:<44} failed: {e}", file=sys.stderr)
            continue
        overall.update(stats)
        hits.extend(found)
        for _, make, _, _ in found:
            makes[make[:24]] += 1
        print(f"  {query:<44} {total:>8,} hits | checked {stats['checked']:>3} "
              f"| camera {stats['with_camera_make']:>3} | usable {stats['usable']:>3}")

    print(f"\nTOTAL  checked {overall['checked']}, with a camera Make "
          f"{overall['with_camera_make']}, usable {overall['usable']}")
    if overall["checked"]:
        print(f"       camera rate {overall['with_camera_make'] / overall['checked']:.1%}")
    for make, count in makes.most_common(10):
        print(f"  {count:>4}  {make}")
    for kb, make, licence, title in hits[:12]:
        print(f"  {kb:>8} KB | {make:<20} | {licence:<14} | {title}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
