#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Separate "the detector saw the payload" from "the detector saw the encoder".

THE RESULT THIS EXISTS TO DOUBT
--------------------------------
A detector under test came out BELOW chance on every outguess arm: it called
the stego half less suspicious than its own cover. An inversion is a strong
claim, so it earns a strong check.

The reason to doubt it is in the numbers already collected. The three outguess
arms span a tenfold payload range, 5% to 50% of capacity, and the effect does
not move:

    outguess/0050   AUC 0.3496
    outguess/0200   AUC 0.3513      a tenfold payload range,
    outguess/0500   AUC 0.3488      and no gradient at all

A payload effect grows with payload. A flat line across ten times the payload
is the signature of something that is present in equal measure in all three
arms, and there is an obvious candidate: **outguess rewrites the whole JPEG**.
Every arm was re-encoded by outguess's own libjpeg, while the clean half was
written by Pillow. The quality confound was already fixed by building at
`-p 95` to match the cover; the encoder difference was not, and cannot be, as
long as one side is written by outguess and the other is not.

For comparison, steghide does not re-encode. It embeds into the coefficients
that are already there, and its arms sit at 0.498 to 0.505: chance, exactly
where a detector that cannot see steghide should be.

THE TWO CONTROL ARMS
--------------------
Both start from the identical clean covers and neither carries a meaningful
payload, so anything they show is not hiding.

**`nullog`: outguess's own writer, with as close to no payload as it allows.**
One byte. Outguess refuses an empty payload outright (`mmap: Invalid argument`,
zero-byte output), so one byte is the floor, and it is 30 changed bits in about
90,000 usable ones: **0.03% of capacity**, three orders of magnitude below the
smallest real arm. This arm extends the payload sweep downwards. If the effect
is the payload it must vanish here. If the effect is the encoder it will sit
exactly where the other three sit.

**`nullpillow`: the same cover decoded and re-encoded by Pillow at the same
quality.** This one asks a different question: is any re-encode enough, or is
it outguess's writer specifically? A shift here would mean the detector is
sensitive to a second compression generation in general; no shift here plus a
shift in `nullog` points at outguess's libjpeg in particular.

    payload:   0.03%      5%        20%       50%
               nullog  outguess/0050  /0200   /0500
               └────────── if these four agree, it is the encoder ─────────┘

    nullpillow: no outguess anywhere. Isolates re-encoding from outguess.

WHAT EACH OUTCOME MEANS, DECIDED BEFORE THE NUMBERS ARRIVE
-----------------------------------------------------------
Writing this down first is the point of a control; a prediction made after
seeing the answer is not one.

| nullog | nullpillow | Reading |
|---|---|---|
| shifts like the real arms | flat | The inversion is outguess's encoder. The finding must be withdrawn as a payload result |
| shifts like the real arms | also shifts | The detector responds to re-compression generally. Same withdrawal, broader cause |
| flat | flat | The inversion survives: it really is the payload, and a very small one does not trigger it |

Usage::

    python3 generators/recompression_control.py \\
        --corpus ~/ssprobe/round3-q95 --out ~/ssprobe/round3-recompress
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time

from PIL import Image

#: One byte, because outguess will not take zero. Everything about this arm
#: depends on that being stated rather than rounded to "no payload".
NULL_PAYLOAD = b"\x00"

OUTGUESS_IMAGE = "stegobench/outguess:pinned"


class ControlError(RuntimeError):
    pass


def digest(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def quality_of(row: dict) -> int:
    """The quality the real arms were built at, so the control matches them."""
    detail = row.get("detail") or {}
    quality = detail.get("reencoded_at_quality") or row.get("jpeg_quality")
    if not quality:
        raise ControlError(
            "the manifest does not record what quality the outguess arms were "
            "written at, and a control that guesses it controls for nothing")
    return int(quality)


def outguess_null(cover: pathlib.Path, dest: pathlib.Path, quality: int) -> dict:
    """The cover through outguess's writer, carrying one byte."""
    with tempfile.TemporaryDirectory() as tmp:
        work = pathlib.Path(tmp)
        shutil.copy2(cover, work / "cover.jpg")
        (work / "payload.bin").write_bytes(NULL_PAYLOAD)
        result = subprocess.run(
            ["docker", "run", "--rm", "--network", "none",
             "-v", f"{work}:/w", "-w", "/w", OUTGUESS_IMAGE,
             "-p", str(quality), "-d", "payload.bin", "cover.jpg", "out.jpg"],
            capture_output=True, timeout=300)
        produced = work / "out.jpg"
        if result.returncode != 0 or not produced.is_file() or not produced.stat().st_size:
            raise ControlError(
                "outguess refused: "
                + (result.stderr or b"").decode("utf-8", "replace").strip()[:300])
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(produced, dest)

    # Outguess reports what it changed on stderr. Recording it is what lets the
    # write-up say "0.03% of capacity" instead of "a negligible payload".
    text = (result.stderr or b"").decode("utf-8", "replace")
    changed = usable = None
    for line in text.splitlines():
        if "Extracting usable bits" in line:
            usable = int("".join(c for c in line.split(":")[1] if c.isdigit()))
        if line.startswith("Total bits changed"):
            changed = int("".join(c for c in line.split(":")[1].split("(")[0]
                                 if c.isdigit()))
    return {"bits_changed": changed, "usable_bits": usable,
            "changed_fraction": (round(changed / usable, 6)
                                 if changed and usable else None)}


def pillow_null(cover: pathlib.Path, dest: pathlib.Path, quality: int) -> dict:
    """The cover decoded and re-encoded by the encoder that first wrote it."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    with Image.open(cover) as img:
        img.load()
        bare = Image.frombytes(img.mode, img.size, img.tobytes()).convert("RGB")
    part = dest.with_suffix(".jpg.part")
    bare.save(part, format="JPEG", quality=quality, subsampling=0, optimize=False)
    part.replace(dest)
    return {"bits_changed": 0, "usable_bits": None, "changed_fraction": 0.0}


ARMS = {
    "nullog": (outguess_null, "outguess's own writer, one byte of payload"),
    "nullpillow": (pillow_null, "Pillow re-encode, no payload at all"),
}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--corpus", required=True,
                    help="the corpus whose clean arm and manifest to control")
    ap.add_argument("--out", required=True)
    ap.add_argument("--tool", default="outguess",
                    help="whose arms this is controlling, for the quality")
    ap.add_argument("--limit", type=int, default=0, help="0 means every cover")
    ap.add_argument("--arms", default=",".join(ARMS))
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    corpus = pathlib.Path(args.corpus)
    out = pathlib.Path(args.out)
    source = corpus / "manifest.jsonl"
    if not source.is_file():
        print(f"no manifest at {source}", file=sys.stderr)
        return 1

    rows = [json.loads(l) for l in source.read_text(encoding="utf-8").splitlines() if l.strip()]
    subject = [r for r in rows if r.get("tool") == args.tool]
    if not subject:
        print(f"no {args.tool} rows in {source}", file=sys.stderr)
        return 1
    try:
        quality = quality_of(subject[0])
    except ControlError as e:
        print(e, file=sys.stderr)
        return 1

    # One row per distinct clean cover, in manifest order, so the control arms
    # sit at the same positions as the arms they control.
    covers: list[str] = []
    seen: set[str] = set()
    for row in subject:
        clean = row["clean"]
        if clean not in seen:
            seen.add(clean)
            covers.append(clean)
    if args.limit:
        covers = covers[: args.limit]

    print(f"controlling {args.tool} at quality {quality}, {len(covers)} cover(s)")

    out.mkdir(parents=True, exist_ok=True)
    # The clean half is copied rather than referenced, so the control corpus is
    # a corpus on its own terms and can be scored without the original beside it.
    clean_dir = out / "clean"
    for clean in covers:
        dest = clean_dir / pathlib.PurePosixPath(clean).name
        if not dest.is_file():
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(corpus / clean, dest)

    manifest = out / "manifest.jsonl"
    done = set()
    if manifest.is_file():
        done = {json.loads(l)["stego"] for l in manifest.read_text(encoding="utf-8").splitlines()
                if l.strip()}
        print(f"resuming: {len(done)} already built")

    wanted = [a.strip() for a in args.arms.split(",") if a.strip()]
    counts = {"built": 0, "failed": 0, "skipped": 0}
    last_beat = time.monotonic()

    with manifest.open("a") as mf:
        for index, clean in enumerate(covers):
            name = pathlib.PurePosixPath(clean).name
            local_clean = clean_dir / name
            for arm in wanted:
                build, why = ARMS[arm]
                rel = f"{arm}/{name}"
                stego = out / rel
                if rel in done or stego.is_file():
                    counts["skipped"] += 1
                    continue
                try:
                    detail = build(local_clean, stego, quality)
                except Exception as e:  # noqa: BLE001 - one cover, not the run
                    counts["failed"] += 1
                    print(f"  {rel}: {type(e).__name__}: {e}", file=sys.stderr)
                    continue
                mf.write(json.dumps({
                    "arm": arm,
                    "tool": arm,
                    "rate": 0.0,
                    "control_for": args.tool,
                    "why": why,
                    "clean": f"clean/{name}",
                    "stego": rel,
                    "jpeg_quality": quality,
                    "payload_bytes": len(NULL_PAYLOAD) if arm == "nullog" else 0,
                    "clean_sha256": digest(local_clean),
                    "stego_sha256": digest(stego),
                    "detail": detail,
                }) + "\n")
                mf.flush()
                counts["built"] += 1

            if time.monotonic() - last_beat >= 30:
                print(f"  ... {index + 1}/{len(covers)} covers, {counts}")
                last_beat = time.monotonic()

    print(f"{counts} into {out}")
    return 0 if counts["built"] or counts["skipped"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
