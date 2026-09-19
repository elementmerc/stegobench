#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Pack the stego arms into shards, so the other half of the corpus ships too.

WHY THIS EXISTS SEPARATELY FROM `pack_tier.py`
-----------------------------------------------
`pack_tier.py` packs covers. It takes `--covers` and nothing else, so until now
the 42 GB of stego arms had no route into a release at all: the corpus could be
built and measured but only half of it could be published. That was found by
auditing the release directory against the corpus and noticing the packed tier
predated the arms by a day.

Arms are not covers and the difference is load bearing:

**A sample is a pair, not a picture.** Every stego image is a modified copy of
one specific clean image, and the pairing is the whole point of the corpus. So
the clean half travels in its own arm (`clean-grey`, `clean-jpeg`,
`clean-jpeg-tools`), synthesised here from the `clean` column every stego row
carries, and every stego row records the cover it descends from, which is the
join key back to the cover tier.

**The JPEG arms name their cover indirectly, and the indirection is recorded.**
A spatial row carries `source_png` and joins to the cover manifest in one hop. A
JPEG DCT row carries `source_jpeg`, which is a positional name like `00000.jpg`
in the clean JPEG pool that `build_jpeg_arms.py` wrote. That builder recorded
`source_png` for every one of those, so the second hop is a lookup in its
manifest rather than a guess about ordering: `--jpeg-covers-manifest` supplies
it, `licence_join` in each sample says which route was taken, and a sample whose
cover cannot be named is refused rather than shipped.

**The licence is inherited and must travel.** A stego image is a derivative of
a Commons photograph. 54% of the covers require attribution, so 54% of the
stego images do too, and a shard that ships the pixels without the credit line
is a compliance failure rather than an inconvenience. Every sample JSON here is
the arm row joined to its cover's licence, artist, credit and description URL.
A reader who never opens the cover tier still has everything the licence asks
of them.

**One shard set per arm.** Arms are the unit people want: somebody evaluating
against WOW at 0.2 bpp should not download MiPOD to get it. Shards are named
`pentimento-core-<tool>-<rate>-NNNNN.tar` and each arm carries its own index.

WHAT A SHARD LOOKS LIKE
-----------------------
WebDataset layout, the same as the cover tier, so the same readers work::

    pentimento-core-wow-0200-00000.tar
      000000.png     the stego image
      000000.json    the arm row, with licence and attribution joined in
      000001.png
      000001.json

The member extension is the real one, so a JPEG arm ships `000000.jpg`. Readers
that pick a decoder by extension, which is every WebDataset reader, would
otherwise hand JPEG bytes to a PNG decoder.

The key is the position within the arm, and `source_png` inside the JSON is
what joins a sample back to its cover and to its clean counterpart.

DIGESTS ARE CHECKED, NOT TRUSTED
---------------------------------
Every file is hashed as it is read and compared with the digest the build
recorded. This is the last moment at which the bytes and the record are in the
same place, and a corpus that ships a file which does not match its own
manifest is worse than one that ships nothing.

Usage::

    python pack_arms.py --arms ~/pentimento/arms/core \\
                        --covers-manifest ~/pentimento/covers/commons/manifest.jsonl \\
                        --out ~/pentimento/release/core-arms
"""
from __future__ import annotations

import argparse
import collections
import hashlib
import io
import json
import pathlib
import sys
import tarfile
import time

#: Licence-bearing fields lifted from the cover row onto every derivative. If
#: the cover manifest gains a field that the licence depends on, it belongs
#: here, because a reader of an arm shard may never see the cover tier.
INHERITED = (
    "licence",
    "usage_terms",
    "artist",
    "credit",
    "descriptionurl",
    "attribution",
    "title",
)

#: 500 stego images per shard rather than the cover tier's 1000. Arm images are
#: larger on average and a shard that resumes cheaply on a poor connection is
#: worth more than one that is tidy.
DEFAULT_PER_SHARD = 500


class PackError(RuntimeError):
    pass


def load_cover_licences(manifest: pathlib.Path) -> dict[str, dict]:
    """The licence half of every cover row, keyed by filename."""
    out: dict[str, dict] = {}
    for line in manifest.read_text().splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        out[row["file"]] = {k: row[k] for k in INHERITED if k in row}
    if not out:
        raise PackError(f"no cover rows in {manifest}")
    return out


def load_jpeg_cover_map(manifest: pathlib.Path) -> dict[str, str]:
    """`00000.jpg` to the cover PNG it was made from, as the builder recorded it.

    `build_jpeg_arms.py` writes the clean JPEG pool and puts `source_png` on
    every row, so this is a record rather than an inference about ordering. It
    is checked for collisions because a name pointing at two different covers
    would silently attribute half the arm to the wrong photographer.
    """
    out: dict[str, str] = {}
    for line in manifest.read_text().splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        clean = row.get("clean")
        cover = row.get("source_png")
        if not clean or not cover:
            continue
        name = pathlib.PurePosixPath(clean).name
        if out.setdefault(name, cover) != cover:
            raise PackError(
                f"{manifest}: {name} is recorded against both {out[name]} and "
                f"{cover}, so the JPEG licence join is not trustworthy")
    return out


def cover_of(row: dict, jpeg_map: dict[str, str]) -> tuple[str | None, str]:
    """The cover a row descends from, and how that was established."""
    direct = row.get("source_png")
    if direct:
        return direct, "direct"
    indirect = row.get("source_jpeg")
    if indirect:
        mapped = jpeg_map.get(pathlib.PurePosixPath(indirect).name)
        if mapped:
            return mapped, "via clean JPEG"
    return None, "unresolved"


def arm_key(row: dict) -> str:
    """`wow-0200` from a row. Rates become integers so names sort and never
    carry a decimal point into a filename."""
    tool = str(row["tool"])
    if row.get("rate") is None:
        return tool
    return f"{tool}-{int(round(float(row['rate']) * 1000)):04d}"


def group_rows(manifest: pathlib.Path) -> dict[str, list[dict]]:
    """Every row, grouped into the arm it belongs to, in manifest order.

    Order is preserved deliberately: it is the order the build wrote, which is
    cover order, so a reader comparing two arms sees the same photographs at
    the same positions.
    """
    groups: dict[str, list[dict]] = collections.defaultdict(list)
    for line in manifest.read_text().splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        groups[arm_key(row)].append(row)
    return dict(groups)


def add(tar: tarfile.TarFile, name: str, payload: bytes) -> None:
    info = tarfile.TarInfo(name)
    info.size = len(payload)
    # Fixed metadata, so the same corpus packs to the same bytes on any machine
    # and on any day. A shard whose digest moves with the clock cannot be
    # checked against a published one.
    info.mtime = 0
    info.mode = 0o644
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    info.type = tarfile.REGTYPE
    tar.addfile(info, io.BytesIO(payload))


def pack_arm(
    name: str,
    rows: list[dict],
    arms_root: pathlib.Path,
    licences: dict[str, dict],
    jpeg_map: dict[str, str],
    out: pathlib.Path,
    per_shard: int,
    path_field: str,
    digest_field: str,
) -> dict:
    """One arm into shards. Returns its index."""
    shards: list[dict] = []
    mismatches: list[str] = []
    missing: list[str] = []
    unlicensed: list[str] = []
    position = 0

    for shard_no in range((len(rows) + per_shard - 1) // per_shard):
        chunk = rows[shard_no * per_shard : (shard_no + 1) * per_shard]
        shard = out / f"pentimento-core-{name}-{shard_no:05d}.tar"
        packed = 0
        with tarfile.open(shard, "w", format=tarfile.PAX_FORMAT) as tar:
            for row in chunk:
                rel = row[path_field]
                path = arms_root / rel
                if not path.exists():
                    missing.append(rel)
                    position += 1
                    continue
                payload = path.read_bytes()
                actual = hashlib.sha256(payload).hexdigest()
                declared = row.get(digest_field)
                if declared and actual != declared:
                    mismatches.append(rel)
                    position += 1
                    continue

                # The derivative inherits its cover's licence. Without this a
                # reader holding only this shard cannot discharge the
                # attribution that 54% of these images carry, so a sample whose
                # cover cannot be named is left out rather than shipped bare.
                cover_name, how = cover_of(row, jpeg_map)
                cover = licences.get(cover_name or "")
                if not cover:
                    unlicensed.append(rel)
                    position += 1
                    continue

                sample = dict(row)
                sample["sha256"] = actual
                sample["source_png"] = cover_name
                sample["licence_join"] = how
                sample["cover_licence"] = cover

                key = f"{position:06d}"
                add(tar, f"{key}{pathlib.PurePosixPath(rel).suffix}", payload)
                add(tar, f"{key}.json", json.dumps(sample, sort_keys=True).encode())
                packed += 1
                position += 1

        blob = shard.read_bytes()
        shards.append({
            "shard": shard.name,
            "samples": packed,
            "bytes": len(blob),
            "sha256": hashlib.sha256(blob).hexdigest(),
        })
        print(f"  {shard.name}  {packed:>5} samples  {len(blob) / 1e6:>8.1f} MB",
              flush=True)

    return {
        "arm": name,
        "samples": sum(s["samples"] for s in shards),
        "rows_in_manifest": len(rows),
        "shards": shards,
        "digest_mismatches": mismatches,
        "missing": missing,
        "unlicensed": unlicensed,
    }


#: The clean halves, synthesised from the `clean` column rather than from a
#: manifest of their own. Without them the corpus ships only the modified half
#: of every pair, and a paired corpus whose pairs are missing a side is not one.
def clean_arms(rows: list[dict], group: str) -> dict[str, list[dict]]:
    """Every distinct clean image in a group, as arms of its own.

    Keyed on the directory the builder put it in, because those directories are
    different images: `clean_jpeg` is written by the same encoder as its stego
    twin, while `clean` under `jpeg-tools` is the Pillow original the tools were
    handed. Collapsing them would break the pairing they exist to preserve.
    """
    arms: dict[str, list[dict]] = collections.defaultdict(list)
    seen: set[str] = set()
    for row in rows:
        rel = row.get("clean")
        if not rel or rel in seen:
            continue
        seen.add(rel)
        folder = pathlib.PurePosixPath(rel).parent.name
        name = f"clean-{group}" if folder == "clean" else folder.replace("_", "-")
        arms[name].append({
            "arm": name,
            "tool": "clean",
            "rate": None,
            "role": "clean",
            "domain": row.get("domain"),
            "file": rel,
            "sha256": row.get("clean_sha256"),
            "source_png": row.get("source_png"),
            "source_jpeg": row.get("source_jpeg"),
        })
    return dict(arms)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--arms", required=True, help="the built arms directory")
    ap.add_argument("--covers-manifest", required=True,
                    help="the cover manifest, for the licence join")
    ap.add_argument("--out", required=True)
    ap.add_argument("--jpeg-covers-manifest", default=None,
                    help="the manifest that recorded which cover each clean "
                         "JPEG was made from, for the DCT arms' licence join. "
                         "Defaults to <arms>/jpeg-tools/manifest.jsonl")
    ap.add_argument("--no-clean", action="store_true",
                    help="pack only the stego halves. The clean halves are "
                         "what make the pairs usable, so this is not the default")
    ap.add_argument("--per-shard", type=int, default=DEFAULT_PER_SHARD)
    ap.add_argument("--group", action="append", default=None,
                    help="arm group directory under --arms; repeatable. "
                         "Default: every directory holding a manifest.jsonl")
    ap.add_argument("--only", action="append", default=None,
                    help="pack only these arms, e.g. --only wow-0200")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    arms_root = pathlib.Path(args.arms)
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    try:
        licences = load_cover_licences(pathlib.Path(args.covers_manifest))
    except (PackError, OSError) as e:
        print(f"cannot read the cover manifest: {e}", file=sys.stderr)
        return 1
    print(f"licence rows: {len(licences)}")

    jpeg_manifest = pathlib.Path(
        args.jpeg_covers_manifest or arms_root / "jpeg-tools" / "manifest.jsonl")
    jpeg_map: dict[str, str] = {}
    if jpeg_manifest.exists():
        try:
            jpeg_map = load_jpeg_cover_map(jpeg_manifest)
        except (PackError, OSError) as e:
            print(f"cannot read the JPEG cover manifest: {e}", file=sys.stderr)
            return 1
        print(f"JPEG cover map: {len(jpeg_map)} clean JPEGs joined to covers")
    else:
        print(f"no JPEG cover manifest at {jpeg_manifest}: any DCT arm will "
              f"fail its licence join", file=sys.stderr)

    groups = args.group or [
        p.parent.name for p in sorted(arms_root.glob("*/manifest.jsonl"))
    ]
    if not groups:
        print(f"no arm groups with a manifest under {arms_root}", file=sys.stderr)
        return 1

    started = time.monotonic()
    indices: list[dict] = []
    for group in groups:
        manifest = arms_root / group / "manifest.jsonl"
        if not manifest.exists():
            print(f"skipping {group}: no manifest", file=sys.stderr)
            continue
        print(f"\n=== {group} ===")
        by_arm = group_rows(manifest)
        if not args.no_clean:
            every_row = [r for rows in by_arm.values() for r in rows]
            by_arm.update(clean_arms(every_row, group))
        for name, rows in sorted(by_arm.items()):
            if args.only and name not in args.only:
                continue
            # A stego row carries both halves; only the stego half is packed
            # under the arm's own name, because the clean half is packed once
            # as a clean arm rather than repeated under all four rates.
            path_field = "stego" if "stego" in rows[0] else "file"
            digest_field = "stego_sha256" if path_field == "stego" else "sha256"
            print(f"{name}: {len(rows)} rows")
            indices.append(pack_arm(
                name, rows, arms_root / group, licences, jpeg_map, out,
                args.per_shard, path_field, digest_field,
            ))

    total_samples = sum(i["samples"] for i in indices)
    total_bytes = sum(s["bytes"] for i in indices for s in i["shards"])
    bad = sum(len(i["digest_mismatches"]) for i in indices)
    gone = sum(len(i["missing"]) for i in indices)
    bare = sum(len(i["unlicensed"]) for i in indices)

    index_path = out / "pentimento-core-arms-index.json"
    index_path.write_text(json.dumps({
        "tier": "Core",
        "part": "arms",
        "arms": indices,
        "total_samples": total_samples,
        "total_bytes": total_bytes,
        "note": "Each sample JSON carries the arm row joined to its cover's "
                "licence under `cover_licence`. A stego image is a derivative "
                "of its cover and inherits that cover's terms.",
    }, indent=2, sort_keys=True) + "\n")

    print(f"\n{len(indices)} arm(s), {total_samples} samples, "
          f"{total_bytes / 1e9:.1f} GB, {time.monotonic() - started:.0f}s")
    print(f"index: {index_path}")
    if bad or gone or bare:
        # Three different failures, reported as three. An earlier version
        # counted the licence gaps as missing files and said "80000 missing
        # file(s)" when every file was present, which sent the diagnosis the
        # wrong way for an afternoon.
        print("", file=sys.stderr)
        if gone:
            print(f"{gone} file(s) named in the manifest are not on disk.",
                  file=sys.stderr)
        if bad:
            print(f"{bad} file(s) do not match the digest the build recorded.",
                  file=sys.stderr)
        if bare:
            print(f"{bare} sample(s) could not be joined to a cover licence. "
                  f"They are derivatives of licensed photographs, so they were "
                  f"left out rather than shipped without attribution.",
                  file=sys.stderr)
        print("None of those were packed. The tier is incomplete; see the "
              "index for the list.", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
