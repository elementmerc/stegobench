#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Fetch royalty-free cover images from Pexels, with provenance.

This is the "real world web" arm of the corpus. The research corpora (ALASKA2,
BOSSbase, Cassavia) are curated, uniformly processed, and in BOSSbase's case
developed from RAW. Photographs from a stock site are none of those things: they
have been through somebody's editing pipeline, saved as JPEG at an unknown
quality, and resized by the service. That is exactly why the arm is worth
having. A detector calibrated on curated corpora and deployed against whatever
arrives in a real queue meets this distribution, not that one.

CROP, NOT RESIZE
----------------
Covers are centre-cropped to the target size and never resampled.

Resizing averages neighbouring pixels together, which rewrites the entire
least-significant-bit plane: the exact statistics every spatial steganalysis
method reads. A resized cover is a different kind of object from a camera
original, and a corpus built by resizing measures the resampler as much as the
hiding. Cropping takes a window of the original pixels and changes none of them.

(The size sweep in `sizesweep.py` does resize, deliberately, because there the
image size IS the variable under test and every arm has to come from the same
source image. That is the one place it is the right call.)

REPRODUCIBILITY
---------------
Search results on a live service change over time, so re-running a search is not
a way to rebuild a corpus. The manifest records each photo's Pexels ID and the
exact source URL used, and the rebuild path is to fetch those IDs directly
(`GET /v1/photos/{id}`) rather than to search again.

That manifest is also what makes the corpus publishable. The Pexels licence
permits modification freely but restricts redistributing the photographs
themselves as a stock resource, so the intended distribution model is the
derived stego images plus this manifest plus a downloader, rather than a bundle
of somebody else's photographs. Nothing here decides that question; it records
what a decision would need.

The API key is read from the environment, never from a file in the repository
and never from an argument, since arguments are visible to every process on the
machine.
"""
import argparse
import hashlib
import json
import os
import pathlib
import sys
import time
import io
import urllib.error
import urllib.parse
import urllib.request

from PIL import Image

API = "https://api.pexels.com/v1/search"

# Pexels returns 403 to the default `Python-urllib/x.y` user agent, on both the
# API and the CDN. Identifying the client honestly is the fix, and it is also
# the polite thing to send to a free service being scraped for a corpus.
UA = "stegobench/0.1 (research corpus builder; https://github.com/elementmerc/stegobench)"

# A spread of subjects rather than one. A single query returns a visually
# coherent set, and a corpus of 400 photographs of the same thing measures that
# thing rather than the distribution it is standing in for.
DEFAULT_QUERIES = [
    "street",
    "portrait",
    "landscape",
    "food",
    "architecture",
    "animal",
    "texture",
    "city night",
    "forest",
    "interior",
]


def api_get(url: str, key: str, retries: int = 4) -> dict:
    """One API call, with backoff. Rate-limit headers are surfaced, not guessed."""
    last = "no attempt"
    for attempt in range(1, retries + 1):
        req = urllib.request.Request(url, headers={"Authorization": key, "User-Agent": UA})
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                # Pexels omits this header on some responses and returns -1 on
                # others, so only a real non-negative count is worth warning on.
                remaining = r.headers.get("x-ratelimit-remaining")
                try:
                    left = int(remaining) if remaining is not None else -1
                except ValueError:
                    left = -1
                if 0 <= left < 50:
                    print(f"  warning: {left} API calls left this period", file=sys.stderr)
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            if e.code == 429:
                # Their limit, their pace. Back off hard rather than hammer it.
                wait = 60 * attempt
                print(f"  rate limited, waiting {wait}s", file=sys.stderr)
                time.sleep(wait)
                last = "429"
                continue
            last = f"HTTP {e.code}"
        except Exception as e:  # noqa: BLE001 - network, and the message is the point
            last = str(e)
        time.sleep(2 * attempt)
    raise RuntimeError(f"pexels API failed after {retries} attempts: {last}")


def fetch_bytes(url: str, retries: int = 4) -> bytes:
    last = "no attempt"
    for attempt in range(1, retries + 1):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": UA})
            with urllib.request.urlopen(req, timeout=60) as r:
                return r.read()
        except Exception as e:  # noqa: BLE001
            last = str(e)
            time.sleep(2 * attempt)
    raise RuntimeError(f"download failed after {retries} attempts: {last}")


def centre_crop(img: Image.Image, size: int) -> Image.Image:
    w, h = img.size
    if w < size or h < size:
        raise ValueError(f"image is {w}x{h}, smaller than the {size}px crop")
    left = (w - size) // 2
    top = (h - size) // 2
    return img.crop((left, top, left + size, top + size))


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True, help="directory to write covers into")
    ap.add_argument("--count", type=int, default=200, help="total covers wanted")
    ap.add_argument("--size", type=int, default=512, help="square crop side")
    ap.add_argument("--queries", default=",".join(DEFAULT_QUERIES))
    ap.add_argument("--per-page", type=int, default=80, help="API page size, max 80")
    args = ap.parse_args(argv)

    key = os.environ.get("PEXELS_API_KEY")
    if not key:
        print(
            "PEXELS_API_KEY is not set. Source the env file rather than passing "
            "the key as an argument, which every process on the machine can read.",
            file=sys.stderr,
        )
        return 2

    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest_path = out / "manifest.jsonl"

    # Resume: an interrupted fetch should cost the images it did not get, not the
    # ones it did, and should not re-spend API calls on them either.
    seen_ids = set()
    if manifest_path.exists():
        for line in manifest_path.read_text(encoding="utf-8").splitlines():
            if line.strip():
                seen_ids.add(json.loads(line)["pexels_id"])
        print(f"resuming: {len(seen_ids)} covers already fetched")

    queries = [q.strip() for q in args.queries.split(",") if q.strip()]
    per_query = max(1, args.count // len(queries))
    written = len(seen_ids)
    skipped = {"too_small": 0, "download": 0, "decode": 0}

    with manifest_path.open("a") as mf:
        for query in queries:
            if written >= args.count:
                break
            page = 1
            got = 0
            while got < per_query and written < args.count:
                url = f"{API}?query={urllib.parse.quote(query)}&per_page={args.per_page}&page={page}"
                try:
                    data = api_get(url, key)
                except RuntimeError as e:
                    print(f"  {query}: {e}", file=sys.stderr)
                    break
                photos = data.get("photos", [])
                if not photos:
                    break
                for p in photos:
                    if got >= per_query or written >= args.count:
                        break
                    if p["id"] in seen_ids:
                        continue
                    # `original` is the unresized upload. Anything else is the
                    # service's own resample, which is the thing being avoided.
                    src = p["src"]["original"]
                    try:
                        raw = fetch_bytes(src)
                    except RuntimeError:
                        skipped["download"] += 1
                        continue
                    try:
                        img = Image.open(io.BytesIO(raw)).convert("RGB")
                    except Exception:  # noqa: BLE001
                        skipped["decode"] += 1
                        continue
                    try:
                        cropped = centre_crop(img, args.size)
                    except ValueError:
                        skipped["too_small"] += 1
                        continue

                    name = f"{written:05d}.png"
                    path = out / name
                    cropped.save(path)
                    mf.write(
                        json.dumps(
                            {
                                "file": name,
                                "pexels_id": p["id"],
                                "source_url": src,
                                "page_url": p.get("url"),
                                "photographer": p.get("photographer"),
                                "photographer_url": p.get("photographer_url"),
                                "original_size": [p.get("width"), p.get("height")],
                                "query": query,
                                "crop": args.size,
                                "method": "centre_crop",
                                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                                "licence": "Pexels licence, https://www.pexels.com/license/",
                            }
                        )
                        + "\n"
                    )
                    mf.flush()
                    seen_ids.add(p["id"])
                    written += 1
                    got += 1
                page += 1

    print(f"wrote {written} covers of {args.count} requested into {out}")
    if any(skipped.values()):
        print(f"  skipped: {skipped}")
    print(f"  manifest: {manifest_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
