#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Does the cover set actually meet the standard it claims? Check, and repair.

WHAT "PRISTINE" MEANS HERE, AND WHY EACH CLAUSE IS IN IT
--------------------------------------------------------
A cover is the control half of a matched pair. Everything a detector reports is
a difference between the cover and its stego twin, so any property that varies
between covers for a reason unrelated to the payload is an uncontrolled variable
in every result drawn from the corpus. The standard is therefore not tidiness,
it is the definition of a usable control.

1. **Pixels and nothing else.** No ICC profile, no EXIF, no text chunks, no
   ancillary data of any kind. This is the clause that prompted the file:
   207 of the first 400 covers had inherited an ICC profile from their source
   and 193 had not, because Pillow carries a source's ancillary data through a
   crop. Half the clean covers carrying a variable-size extra chunk is exactly
   the shape the structural arm is built to detect, so it confounds the arm
   that matters most. One such profile was also large enough to trip Pillow's
   decompression guard on re-open, which is how it was noticed at all.

2. **Nothing after the end marker.** A PNG ends at IEND. Appended bytes are the
   structural arm's entire subject, so a clean cover carrying any is not clean.

3. **The declared geometry.** Every cover square, the same side, the mode its
   manifest row claims. A pair whose halves differ in size is not a pair.

4. **The digest the manifest promises.** If a file's sha256 does not match its
   row, the manifest is describing something that is not there, and every
   downstream reference to it is wrong.

5. **Texture above the floor.** Enforced at fetch time by `cover_quality`, and
   re-checked here because a gate that is never audited is a gate nobody knows
   is working.

6. **Uniqueness.** No two covers sharing a sha256, and no two within the
   perceptual thresholds. Also enforced at fetch time, also worth proving.

REPAIR, AND WHY IT IS SAFE HERE
-------------------------------
`--fix` rewrites each cover from its raw pixel data, which strips everything in
clause 1 and 2 while leaving every sample untouched. The picture is bit for bit
identical; only the container changes. That means the perceptual hashes do not
move and the corpus does not need re-fetching.

The sha256 does change, so the manifest is rewritten and the deduplication store
is rebuilt from the repaired files. Rebuilding is cheaper and more honest than
patching digests in place: the store is keyed on sha256 and a half-updated store
is worse than no store.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import pathlib
import sys

from PIL import Image

import cover_quality
from dedup import DedupStore, fingerprint_bytes

PNG_MAGIC = b"\x89PNG\r\n\x1a\n"
IEND = b"IEND"

#: Keys Pillow exposes that mean the file carries more than its pixels. The
#: geometry and mode keys it also puts in `info` are not contamination.
BENIGN_INFO = {"transparency", "gamma", "aspect", "srgb", "interlace"}


def ancillary(path: pathlib.Path) -> dict:
    """What this file carries besides its pixels."""
    with Image.open(path) as img:
        info = dict(img.info or {})
        exif = img.getexif()
    carried = {k: len(v) if isinstance(v, (bytes, str)) else 1
               for k, v in info.items() if k not in BENIGN_INFO}
    if exif and len(exif):
        carried["exif_tags"] = len(exif)
    return carried


def trailing_bytes(raw: bytes) -> int:
    """Bytes after the PNG end marker's chunk. Should always be zero."""
    if not raw.startswith(PNG_MAGIC):
        return -1
    index = raw.rfind(IEND)
    if index < 0:
        return -1
    # IEND is followed by a 4 byte CRC and then the file must end.
    return len(raw) - (index + len(IEND) + 4)


def bare_png(path: pathlib.Path) -> bytes:
    """The same picture with nothing else in the container."""
    with Image.open(path) as img:
        img.load()
        naked = Image.frombytes(img.mode, img.size, img.tobytes())
    buf = io.BytesIO()
    naked.save(buf, format="PNG", optimize=False)
    return buf.getvalue()


def audit(root: pathlib.Path, manifest_path: pathlib.Path, size: int) -> dict:
    rows = {r["file"]: r for r in (
        json.loads(l) for l in manifest_path.read_text(encoding="utf-8").splitlines() if l.strip()
    )}
    findings = {
        "carries_metadata": [], "trailing_data": [], "wrong_geometry": [],
        "digest_mismatch": [], "below_texture_floor": [], "not_in_manifest": [],
        "missing_file": [], "duplicate_digest": [],
    }
    digests: dict[str, str] = {}
    checked = 0

    for path in sorted(root.glob("*.png")):
        checked += 1
        row = rows.get(path.name)
        if row is None:
            findings["not_in_manifest"].append(path.name)
            continue

        raw = path.read_bytes()
        digest = hashlib.sha256(raw).hexdigest()
        if digest != row.get("sha256"):
            findings["digest_mismatch"].append(path.name)
        if digest in digests:
            findings["duplicate_digest"].append(f"{path.name} = {digests[digest]}")
        digests[digest] = path.name

        if trailing_bytes(raw) != 0:
            findings["trailing_data"].append(path.name)
        carried = ancillary(path)
        if carried:
            findings["carries_metadata"].append(f"{path.name} {carried}")

        with Image.open(path) as img:
            if img.size != (size, size):
                findings["wrong_geometry"].append(f"{path.name} {img.size}")
            if cover_quality.texture(img) < cover_quality.TEXTURE_FLOOR:
                findings["below_texture_floor"].append(path.name)

    for name in rows:
        if not (root / name).is_file():
            findings["missing_file"].append(name)

    return {"checked": checked, "findings": findings}


def repair(root: pathlib.Path, manifest_path: pathlib.Path,
           dedup_db: pathlib.Path | None) -> int:
    """Rewrite every cover from its pixels, then rebuild manifest and store."""
    lines = [l for l in manifest_path.read_text(encoding="utf-8").splitlines() if l.strip()]
    rows = [json.loads(l) for l in lines]
    changed = 0

    for row in rows:
        path = root / row["file"]
        if not path.is_file():
            continue
        before = path.read_bytes()
        after = bare_png(path)
        if after == before:
            continue
        # Atomic, so an interrupted repair cannot leave a truncated cover.
        part = path.with_suffix(".png.part")
        part.write_bytes(after)
        part.replace(path)
        row["sha256"] = hashlib.sha256(after).hexdigest()
        row["container"] = "pixels only, ancillary chunks stripped"
        changed += 1

    manifest_path.write_text("".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")
    print(f"rewrote {changed} of {len(rows)} covers and their manifest rows")

    if dedup_db and changed:
        # The store is keyed on sha256, which every repaired file has changed.
        # A half-updated store is worse than none, so it is rebuilt.
        fresh = pathlib.Path(str(dedup_db) + ".rebuilding")
        for suffix in ("", "-wal", "-shm"):
            stale = pathlib.Path(str(fresh) + suffix)
            if stale.exists():
                stale.unlink()
        store = DedupStore(fresh)
        admitted = rejected = 0
        for row in rows:
            path = root / row["file"]
            if not path.is_file():
                continue
            decision = store.offer(fingerprint_bytes(path.read_bytes()),
                                   "commons", str(row.get("pageid", row["file"])))
            if decision.accepted:
                admitted += 1
            else:
                rejected += 1
                print(f"  {row['file']} collides: {decision.match.describe()}")
        store.close()
        for suffix in ("", "-wal", "-shm"):
            old = pathlib.Path(str(dedup_db) + suffix)
            new = pathlib.Path(str(fresh) + suffix)
            if new.exists():
                new.replace(old)
            elif old.exists():
                old.unlink()
        print(f"rebuilt the dedup store: {admitted} admitted, {rejected} collisions")
    return changed


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("covers")
    ap.add_argument("--manifest", default=None)
    ap.add_argument("--dedup-db", default=None)
    ap.add_argument("--size", type=int, default=512)
    ap.add_argument("--fix", action="store_true",
                    help="rewrite covers from their pixels, then re-audit")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    root = pathlib.Path(args.covers)
    manifest_path = pathlib.Path(args.manifest) if args.manifest else root / "manifest.jsonl"
    if not root.is_dir():
        print(f"not a directory: {root}", file=sys.stderr)
        return 2
    if not manifest_path.is_file():
        print(f"no manifest at {manifest_path}", file=sys.stderr)
        return 2

    if args.fix:
        repair(root, manifest_path,
               pathlib.Path(args.dedup_db) if args.dedup_db else None)

    result = audit(root, manifest_path, args.size)
    print(f"\naudited {result['checked']} covers against the pristine standard")
    clean = True
    for name, items in result["findings"].items():
        if items:
            clean = False
            print(f"  {name}: {len(items)}")
            for item in items[:5]:
                print(f"      {item}")
            if len(items) > 5:
                print(f"      ... and {len(items) - 5} more")
    if clean:
        print("  PRISTINE: pixels only, nothing appended, geometry and digests "
              "agree, texture above the floor, no duplicates")
    return 0 if clean else 1


if __name__ == "__main__":
    raise SystemExit(main())
