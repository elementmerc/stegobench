#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Rebuild already-fetched covers through the current pipeline.

WHY THIS EXISTS
---------------
A corpus assembled in two formats is a corpus with a confound in it. The first
452 covers were cut from the centre of their sources and carry none of the
compression or ISO metadata the fetcher now records. Left alone they would sit
beside later covers that differ in crop policy, which is precisely the kind of
silent difference between arms that makes a result impossible to interpret
afterwards.

Throwing them away would also work and would be simpler. It is not chosen
because the expensive part of acquiring a cover is not the download: it is the
API calls that found a file which was permissively licensed, carried camera
EXIF, passed the suitability gate and was not already held. That selection is
recorded in the old manifest and is worth keeping.

WHAT IS AND IS NOT PRESERVED
----------------------------
Preserved: which source files were selected, and all their Commons provenance.

Not preserved: the pixels. A different crop position means a different picture,
so every cover is downloaded again and cut afresh. The new file has a new
sha256 and a new perceptual hash, and its predecessor is not a duplicate of it
in any sense a reader should care about.

This is why the rebuild writes to a new directory with its own deduplication
store rather than editing in place. The old store describes images that will no
longer exist; keeping it would mean carrying phantom entries that reject
nothing useful and might reject something real.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import pathlib
import sys
import time

from PIL import Image

import cover_quality
import provenance
from dedup import DedupError, DedupStore, fingerprint_bytes
from fetch_commons import PERMISSIVE, DiversityCaps, ShareQuota, fetch_bytes


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--manifest", required=True,
                    help="the manifest.jsonl of the covers to rebuild")
    ap.add_argument("--out", required=True, help="a NEW directory for the rebuild")
    ap.add_argument("--dedup-db", required=True, help="a NEW dedup store")
    ap.add_argument("--size", type=int, default=512)
    ap.add_argument("--delay", type=float, default=1.2)
    ap.add_argument("--max-per-uploader", type=int, default=40)
    ap.add_argument("--max-per-camera", type=int, default=40)
    ap.add_argument("--max-iso-share", type=float, default=0.5)
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)

    source = pathlib.Path(args.manifest)
    if not source.is_file():
        print(f"no manifest at {source}", file=sys.stderr)
        return 2
    rows = [json.loads(l) for l in source.read_text(encoding="utf-8").splitlines() if l.strip()]
    if not rows:
        print(f"{source} is empty", file=sys.stderr)
        return 1

    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest_path = out / "manifest.jsonl"
    rejected_path = out / "rejected.jsonl"

    done = set()
    if manifest_path.exists():
        done = {json.loads(l)["pageid"]
                for l in manifest_path.read_text(encoding="utf-8").splitlines() if l.strip()}
        print(f"resuming: {len(done)} of {len(rows)} already rebuilt")

    try:
        store = DedupStore(args.dedup_db)
    except DedupError as e:
        print(f"dedup store: {e}", file=sys.stderr)
        return 2
    if store.count() and not done:
        print(f"{args.dedup_db} already holds {store.count()} images but this "
              "rebuild has written nothing. Point --dedup-db at a new file: "
              "the old store describes crops that will not exist any more.",
              file=sys.stderr)
        store.close()
        return 2

    caps = DiversityCaps(args.max_per_uploader, args.max_per_camera)
    iso_quota = ShareQuota("iso band", args.max_iso_share)
    written = len(done)
    outcome = {"rebuilt": 0, "download": 0, "decode": 0, "too_small": 0,
               "flat": 0, "duplicate": 0, "over_cap": 0, "over_quota": 0,
               "licence": 0, "error": 0}
    last_beat = time.monotonic()

    with manifest_path.open("a") as mf, rejected_path.open("a") as rf:
        for row in rows:
            if row["pageid"] in done:
                continue
            try:
                # The licence is re-checked rather than trusted. These rows were
                # written before the permissive-only ruling was final, and a
                # rebuild is the last chance to catch one that should not be here.
                if (row.get("licence") or "").lower() not in PERMISSIVE:
                    outcome["licence"] += 1
                    continue

                try:
                    raw = fetch_bytes(row["source_url"])
                except RuntimeError as e:
                    outcome["download"] += 1
                    print(f"  cannot refetch {row['title']}: {e}", file=sys.stderr)
                    continue

                img = Image.open(io.BytesIO(raw))
                img.load()
                profile = provenance.jpeg_profile(img)
                img = (img.convert("L") if img.mode in ("L", "I;16", "I")
                       else img.convert("RGB"))

                box = provenance.crop_box(
                    img.size[0], img.size[1], args.size,
                    f"commons:{row['pageid']}",
                )
                cropped = img.crop(box)

                exif = row.get("exif") or {}
                band = provenance.iso_band(exif)
                uploader = row.get("uploader")
                camera = DiversityCaps.camera_key(exif)

                for reason, detail in (("over_cap", caps.refusal(uploader, camera)),
                                       ("over_quota", iso_quota.refusal(band))):
                    if detail:
                        outcome[reason] += 1
                        rf.write(json.dumps({
                            "pageid": row["pageid"], "title": row["title"],
                            "reason": reason, "detail": detail,
                        }) + "\n")
                        rf.flush()
                        break
                else:
                    quality = cover_quality.assess(cropped)
                    if not quality.usable:
                        outcome["flat"] += 1
                        measured = quality.as_dict()
                        rf.write(json.dumps({
                            "pageid": row["pageid"], "title": row["title"],
                            "reason": "unusable_cover", "detail": quality.reason,
                            "texture": measured["texture"],
                            "clipped": measured["clipped"],
                        }) + "\n")
                        rf.flush()
                        continue

                    # Rebuild from raw pixels so nothing rides along. Pillow carries
                    # the source's ancillary data through a crop, and 207 of the first
                    # 400 covers inherited an ICC profile from their originals while
                    # 193 did not. A corpus where half the clean covers carry a
                    # variable-size ancillary chunk has an uncontrolled variable in
                    # it, and the structural arm is specifically about data appended
                    # to a file, so this is a confound rather than untidiness. One
                    # profile was also large enough to trip Pillow's decompression
                    # guard on re-open, which is the error that surfaced it.
                    bare = Image.frombytes(cropped.mode, cropped.size, cropped.tobytes())
                    buf = io.BytesIO()
                    bare.save(buf, format="PNG", optimize=False)
                    payload = buf.getvalue()

                    decision = store.offer(fingerprint_bytes(payload), "commons",
                                           str(row["pageid"]))
                    if not decision.accepted:
                        outcome["duplicate"] += 1
                        rf.write(json.dumps({
                            "pageid": row["pageid"], "title": row["title"],
                            "reason": "duplicate",
                            "detail": decision.match.describe(),
                        }) + "\n")
                        rf.flush()
                        continue

                    name = f"{written:05d}.png"
                    path = out / name
                    part = path.with_suffix(".png.part")
                    part.write_bytes(payload)
                    part.replace(path)

                    rebuilt = dict(row)
                    rebuilt.update({
                        "file": name,
                        "compression": profile,
                        "pristine": provenance.pristine(row.get("original_mime"),
                                                        profile),
                        "iso_band": band,
                        "crop_box": list(box),
                        "crop": args.size,
                        "method": "random_crop",
                        "mode": cropped.mode,
                        "quality": quality.as_dict(),
                        "sha256": hashlib.sha256(payload).hexdigest(),
                        "rebuilt_from": row.get("sha256"),
                    })
                    mf.write(json.dumps(rebuilt) + "\n")
                    mf.flush()
                    caps.record(uploader, camera)
                    iso_quota.record(band)
                    written += 1
                    outcome["rebuilt"] += 1

                if time.monotonic() - last_beat >= 60:
                    print(f"  ... {written}/{len(rows)} rebuilt, {outcome}")
                    last_beat = time.monotonic()
                time.sleep(args.delay)
            except Exception as e:  # noqa: BLE001 - one bad row, not one dead run
                outcome["error"] += 1
                print(f"  skipping {row.get('title', row['pageid'])}: "
                      f"{type(e).__name__}: {e}", file=sys.stderr)
                rf.write(json.dumps({
                    "pageid": row["pageid"], "title": row.get("title"),
                    "reason": "error", "detail": f"{type(e).__name__}: {e}",
                }) + "\n")
                rf.flush()

    store.close()
    print(f"rebuilt {written} of {len(rows)} covers into {out}")
    print(f"  {outcome}")
    return 0 if written else 1


if __name__ == "__main__":
    raise SystemExit(main())
