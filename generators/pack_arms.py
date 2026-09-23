#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
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

from publish_tier import canonical_licence
from tiers import TierError, tier_cover_names, tier_name

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
    for line in manifest.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        entry = {k: row[k] for k in INHERITED if k in row}
        # One spelling per licence in what ships, so a reader grouping by the
        # field gets one group per licence rather than one per spelling.
        if "licence" in entry:
            entry["licence"] = canonical_licence(entry["licence"])
        out[row["file"]] = entry
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
    for line in manifest.read_text(encoding="utf-8").splitlines():
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
    for line in manifest.read_text(encoding="utf-8").splitlines():
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


#: The eight bytes every PNG starts with.
PNG_MAGIC = b"\x89PNG\r\n\x1a\n"

#: THE SAME FOUR DIGITS MEAN THREE DIFFERENT THINGS.
#:
#: `hugo-0050` is 0.05 bits per pixel. `steghide-0050` is 5% of the capacity
#: steghide reports for that cover. `juniward-0050` is 0.05 bits per non-zero
#: AC coefficient. A reader comparing arms on the number in the name is
#: comparing three incompatible quantities, and only the adaptive arms said so
#: in their records.
RATE_UNITS = {
    "hugo": "bits per pixel",
    "wow": "bits per pixel",
    "suniward": "bits per pixel",
    "hill": "bits per pixel",
    "mipod": "bits per pixel",
    "juniward": "bits per non-zero AC coefficient",
    "uerd": "bits per non-zero AC coefficient",
    "steghide": "fraction of the capacity steghide reports",
    "outguess": "fraction of the capacity outguess reports",
    "append_after_eoi": "not a rate: a fixed trailer",
}

#: Where the payload lives, which decides which detectors can see it at all.
DOMAINS = {
    "hugo": "spatial", "wow": "spatial", "suniward": "spatial",
    "hill": "spatial", "mipod": "spatial",
    "juniward": "jpeg-dct", "uerd": "jpeg-dct",
    "steghide": "jpeg-dct", "outguess": "jpeg-dct",
    "append_after_eoi": "container",
}

#: What was done to the cover to make this file. The credit line has to say so:
#: CC BY asks for an indication of modification whenever a derivative is
#: published, and every image in this corpus is a derivative twice over, once
#: by the crop and once by the payload.
STEGO_MODIFICATION = "cropped, then modified to carry a hidden payload"
CLEAN_MODIFICATION = "cropped, and re-encoded as the control half of a pair"


def restate(cover: dict, arm: str) -> dict:
    """The cover's licence fields, with this derivative's own modification.

    The cover row's credit line describes a crop. This file is that crop with
    something further done to it, and a reader holding only this shard has no
    other way to learn that.
    """
    out = dict(cover)
    line = out.get("attribution")
    if line:
        modification = (CLEAN_MODIFICATION if arm.startswith("clean")
                        else STEGO_MODIFICATION)
        # The cover line already ends with its own ", cropped".
        if line.endswith(", cropped"):
            line = line[: -len(", cropped")]
        out["attribution"] = f"{line}, {modification}"
    return out


def container_of(payload: bytes) -> tuple | None:
    """Everything about a file that the payload should NOT have changed.

    For a JPEG: every marker and its length before the start of scan. That is
    the quantisation tables, the Huffman tables, the frame header and any
    application segments. A payload hidden in DCT coefficients lives after the
    start of scan, so an honest pair agrees on all of it.

    For a PNG: the sequence of chunk types, and the lengths of everything that
    is not image data. A payload hidden in pixels changes IDAT contents and,
    through compression, IDAT lengths; it does not move a header.

    A RUN OF IDAT CHUNKS COUNTS AS ONE. Pillow splits image data at 64 KiB, so
    a cover whose pixels compress to 60,179 bytes ships one IDAT and its stego
    twin at 67,817 ships two. Counting them separately flagged 1,863 perfectly
    good pairs, and the giveaway was the shape of it: the count rose with the
    payload rate, 18 at 0.05 bpp and 218 at 0.4. That is the payload making the
    data less compressible, which is what a payload does, not the writer
    leaving a mark. The number of IDAT chunks is a function of the compressed
    length, and the compressed length is already excluded on purpose.

    Anything else: `None`, meaning no opinion. A format this does not
    understand must not be silently declared matched OR mismatched, and every
    format the corpus actually carries is handled above.

    IT USED TO RETURN THE EMPTY TUPLE for that case, and the callers compare
    two results for inequality. `() != ()` is false, so two files in an
    unrecognised format were counted as CHECKED and declared identical - a
    check that examined nothing and reported clean, which is the single fault
    this codebase keeps finding. Unreachable today, because every file is JPEG
    or PNG; one new format away from being the quietest bug in the release.
    `None` makes the caller decide, and both callers now refuse it.
    """
    if payload[:2] == b"\xff\xd8":
        markers = []
        i = 2
        while i < len(payload) - 1:
            if payload[i] != 0xFF:
                break
            marker = payload[i + 1]
            if marker == 0xDA:
                break
            if marker in (0xD8, 0xD9):
                i += 2
                continue
            length = int.from_bytes(payload[i + 2:i + 4], "big")
            markers.append((marker, length))
            i += 2 + length
        return ("jpeg", tuple(markers))

    if payload[:8] == PNG_MAGIC:
        chunks = []
        i = 8
        while i + 8 <= len(payload):
            length = int.from_bytes(payload[i:i + 4], "big")
            kind = payload[i + 4:i + 8]
            # IDAT length tracks the compressed size, which a payload is
            # entitled to change, and so does the number of IDATs. A run of
            # them collapses to one entry; where the image data sits relative
            # to every other chunk still has to match.
            if kind == b"IDAT":
                if not chunks or chunks[-1][0] != b"IDAT":
                    chunks.append((kind, None))
            else:
                chunks.append((kind, length))
            if kind == b"IEND":
                break
            i += 12 + length
        return ("png", tuple(chunks))

    return None


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
    slug: str = "core",
) -> dict:
    """One arm into shards. Returns its index."""
    shards: list[dict] = []
    mismatches: list[str] = []
    missing: list[str] = []
    unlicensed: list[str] = []
    mispaired: list[str] = []
    container_mismatches: list[str] = []
    position = 0

    # An arm where some samples are paired against the tool's own writer and
    # some are not is internally inconsistent, and the two kinds are not
    # comparable: measured on outguess, the mismatched pairing moved a
    # detector's AUC from 0.50 to 0.36 with no payload involved. So if any
    # sample in the arm is writer-matched, every sample has to be, and the
    # rest are left out rather than quietly averaged in.
    expects_matched = any(r.get("pairing") == "writer-matched" for r in rows)

    for shard_no in range((len(rows) + per_shard - 1) // per_shard):
        chunk = rows[shard_no * per_shard : (shard_no + 1) * per_shard]
        shard = out / f"pentimento-{slug}-{name}-{shard_no:05d}.tar"
        packed = 0
        with tarfile.open(shard, "w", format=tarfile.PAX_FORMAT) as tar:
            for row in chunk:
                rel = row[path_field]
                if expects_matched and row.get("pairing") != "writer-matched":
                    mispaired.append(rel)
                    position += 1
                    continue
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

                # THE CONTAINER GATE.
                #
                # Twice now a pair has differed in its container rather than in
                # its payload, and both times everything else passed: the
                # digests matched, the licences joined, the counts came out
                # right. outguess wrote the stego half with a different encoder
                # from the clean half, worth 0.14 of AUC with no payload at
                # all. Then the JPEG arms wrote the stego half one jpeglib pass
                # further than the clean half, worth a perfect classifier on
                # 80,000 images across eight arms.
                #
                # A shared writer was checked both times and was true both
                # times. What was never checked is the only thing that settles
                # it: that the two files agree byte for byte everywhere except
                # where the payload lives. So compare the containers, and leave
                # out any pair that does not match rather than describing it as
                # a pair.
                clean_rel = row.get("clean")
                if clean_rel:
                    clean_path = arms_root / clean_rel
                    if clean_path.exists():
                        clean_shape = container_of(clean_path.read_bytes())
                        stego_shape = container_of(payload)
                        # `None` is "this format is not understood", which is
                        # not the same as "the two agree". Refusing is the
                        # only safe reading: a pair this cannot examine must
                        # not be shipped as a pair it has examined.
                        if clean_shape is None or stego_shape is None:
                            container_mismatches.append(rel)
                            position += 1
                            continue
                        if clean_shape != stego_shape:
                            container_mismatches.append(rel)
                            position += 1
                            continue

                sample = dict(row)
                # THE ARM NAME THE RELEASE PUBLISHES, not the one the builder
                # used internally. The two differed, and for one arm they
                # differed in the word rather than the punctuation: the
                # appended-data shards are `append_after_eoi-0000` in every
                # shard name, prose table, figure and docs entry, while the
                # records inside them said `structural/0000`. A loader
                # grouping by `record["arm"]`, which is what the docs tell a
                # reader the field is for, produced a label matching nothing
                # published and looking like an undocumented 40th arm.
                #
                # Every other arm differed only as `wow/0200` against
                # `wow-0200`, which is just as unjoinable.
                sample["arm"] = name
                sample["sha256"] = actual
                sample["source_png"] = cover_name
                sample["licence_join"] = how
                sample["cover_licence"] = restate(cover, name)

                # RECORDED, NOT ASSUMED. The reviewer's point was that an
                # absent `pairing` field is ambiguous between "checked and
                # matched" and "nobody looked", and that ambiguity is exactly
                # what hid two container defects. Every sample that reaches
                # this line has had its container compared with its clean
                # half, so the field says what was verified rather than what
                # the builder intended.
                sample["pairing"] = (
                    "container-verified" if clean_rel and (arms_root / clean_rel).exists()
                    else "no-clean-half")
                sample.setdefault("domain", DOMAINS.get(
                    str(row.get("tool", "")), "unknown"))
                sample.setdefault("rate_unit", RATE_UNITS.get(
                    str(row.get("tool", "")), "unstated"))

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
        "mispaired": mispaired,
        "container_mismatches": container_mismatches,
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
    ap.add_argument("--count", type=int, default=10000,
                    help="tier size: 200 Nano, 1000 Lite, 10000 Core. Rows are "
                         "kept when their COVER is in the tier, so an arm tier "
                         "is a prefix for the same reason the cover tier is")
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

    tier = tier_name(args.count)
    slug = tier.lower()
    try:
        in_tier = tier_cover_names(pathlib.Path(args.covers_manifest), args.count)
    except TierError as e:
        print(f"cannot select a tier: {e}", file=sys.stderr)
        return 1
    print(f"tier: {tier} ({len(in_tier)} covers)")

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
            if args.count < 10000:
                # Selected on the COVER, not on the arm row's own position. An
                # arm can be short of a full 10,000 (outguess is, by 1,884), so
                # taking its first n would pick a different set of covers per
                # arm and the tiers would stop lining up across arms.
                kept = []
                for r in rows:
                    cover, _ = cover_of(r, jpeg_map)
                    if cover and pathlib.PurePosixPath(cover).name in in_tier:
                        kept.append(r)
                print(f"{name}: {len(rows)} rows, {len(kept)} in {tier}")
                rows = kept
                if not rows:
                    continue
            # A stego row carries both halves; only the stego half is packed
            # under the arm's own name, because the clean half is packed once
            # as a clean arm rather than repeated under all four rates.
            path_field = "stego" if "stego" in rows[0] else "file"
            digest_field = "stego_sha256" if path_field == "stego" else "sha256"
            print(f"{name}: {len(rows)} rows")
            indices.append(pack_arm(
                name, rows, arms_root / group, licences, jpeg_map, out,
                args.per_shard, path_field, digest_field, slug,
            ))

    total_samples = sum(i["samples"] for i in indices)
    total_bytes = sum(s["bytes"] for i in indices for s in i["shards"])
    bad = sum(len(i["digest_mismatches"]) for i in indices)
    gone = sum(len(i["missing"]) for i in indices)
    bare = sum(len(i["unlicensed"]) for i in indices)
    odd = sum(len(i["mispaired"]) for i in indices)
    boxed = sum(len(i.get("container_mismatches", [])) for i in indices)

    index_path = out / f"pentimento-{slug}-arms-index.json"
    index_path.write_text(json.dumps({
        "tier": tier,
        "part": "arms",
        "arms": indices,
        "total_samples": total_samples,
        "total_bytes": total_bytes,
        "note": "Each sample JSON carries the arm row joined to its cover's "
                "licence under `cover_licence`. A stego image is a derivative "
                "of its cover and inherits that cover's terms.",
    }, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    print(f"\n{len(indices)} arm(s), {total_samples} samples, "
          f"{total_bytes / 1e9:.1f} GB, {time.monotonic() - started:.0f}s")
    print(f"index: {index_path}")
    if bad or gone or bare or odd or boxed:
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
        if odd:
            print(f"{odd} sample(s) are paired against a different encoder from "
                  f"the rest of their arm, so they measure the encoder rather "
                  f"than the payload and were left out.", file=sys.stderr)
        if boxed:
            print(f"{boxed} sample(s) differ from their clean half in the "
                  f"container rather than in the payload: a marker, a table or "
                  f"a chunk that no payload should have moved. A detector can "
                  f"read that difference without doing any steganalysis, so "
                  f"the pair measures the writer and was left out.",
                  file=sys.stderr)
        print("None of those were packed. The tier is incomplete; see the "
              "index for the list.", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
