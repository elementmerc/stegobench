#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Pack a tier into sharded archives, so the corpus is downloadable.

WHY NOT LOOSE FILES
-------------------
Version 1 is 10,000 covers and the full tier would be 100,000. Shipping loose
PNGs breaks the hosts we most want to be on: HuggingFace publishes its limits
plainly at under 10,000 files per folder and under 100,000 per repository, and
every other destination is slow to download a directory of small files even when
it permits one.

So a tier ships as tar shards in WebDataset layout, which is a format the major
dataset hosts understand and which streams without unpacking.

WHAT WEBDATASET LAYOUT MEANS, IN PLAIN TERMS
--------------------------------------------
A shard is an ordinary tar file. Inside it, everything belonging to one sample
shares a basename and differs only in extension:

    shard-00000.tar
      000000.png     the image
      000000.json    its manifest row
      000001.png
      000001.json
      ...

A reader walking the tar in order sees each sample's parts together, so it can
stream straight into training without an index and without random access. That
is the whole trick, and it is why the parts must be adjacent and in order.

THE THREE PROPERTIES THIS FILE IS RESPONSIBLE FOR
-------------------------------------------------
**Nesting.** Samples go into shards in `tier_order`, so shard 0 of Nano holds
the same samples as shard 0 of Core. A tier is a prefix of an ordering and the
packing must not quietly reorder it.

**Byte-for-byte reproducibility.** Two runs over the same input produce
identical tars. That needs more care than it sounds: tar records mtime, uid,
gid, username and group for every member, and the defaults leak the packing
machine's clock and account into the archive. All of them are pinned here.

**Verification, not assertion.** Every image is hashed while being packed and
checked against the digest the manifest already carries. A corpus that ships a
digest it never verified is worse than one that ships none, because the digest
is what a downstream user trusts instead of looking.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import pathlib
import sys
import tarfile
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from tiers import TierError, covers_in_tier_order, tier_name  # noqa: E402

#: Fixed timestamp for every tar member. Any real clock makes the archive differ
#: between runs, which would break the checksum comparison that proves one tier
#: is a prefix of another.
EPOCH = 1_600_000_000

#: Members are written as this owner, never the packing account's.
OWNER = ("pentimento", "pentimento")


def shard_plan(total: int, per_shard: int) -> list[tuple[int, int]]:
    """Half-open [start, end) ranges, in order."""
    return [(i, min(i + per_shard, total)) for i in range(0, total, per_shard)]


def add(tar: tarfile.TarFile, name: str, payload: bytes) -> None:
    """One member, with everything that varies between machines pinned."""
    info = tarfile.TarInfo(name)
    info.size = len(payload)
    info.mtime = EPOCH
    info.mode = 0o644
    info.uid = info.gid = 0
    info.uname, info.gname = OWNER
    info.type = tarfile.REGTYPE
    tar.addfile(info, io.BytesIO(payload))


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--covers", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--count", type=int, required=True,
                    help="tier size: 200 Nano, 1000 Lite, 10000 Core")
    ap.add_argument("--manifest", default=None,
                    help="default: manifest.jsonl beside the covers")
    ap.add_argument("--per-shard", type=int, default=1000,
                    help="samples per shard. 1000 keeps a cover shard near "
                         "300 MB, which resumes cheaply on a poor connection")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    covers = pathlib.Path(args.covers)
    manifest_path = pathlib.Path(args.manifest) if args.manifest else covers / "manifest.jsonl"
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    try:
        chosen = covers_in_tier_order(manifest_path, covers, args.count)
    except TierError as e:
        print(f"cannot select a tier: {e}", file=sys.stderr)
        return 1

    rows = {}
    for line in manifest_path.read_text().splitlines():
        if line.strip():
            row = json.loads(line)
            rows[row["file"]] = row

    name = tier_name(args.count)
    plan = shard_plan(len(chosen), args.per_shard)
    print(f"{name}: {len(chosen)} samples into {len(plan)} shard(s) "
          f"of up to {args.per_shard}")

    started = time.monotonic()
    index: list[dict] = []
    mismatches = 0
    for shard_no, (start, end) in enumerate(plan):
        shard = out / f"pentimento-{name.lower()}-{shard_no:05d}.tar"
        digest = hashlib.sha256()
        with tarfile.open(shard, "w", format=tarfile.PAX_FORMAT) as tar:
            for position in range(start, end):
                path = chosen[position]
                row = rows[path.name]
                payload = path.read_bytes()

                # The manifest's digest is checked here rather than trusted.
                # This is the last point at which the bytes and the record are
                # in the same place at the same time.
                actual = hashlib.sha256(payload).hexdigest()
                if row.get("sha256") and actual != row["sha256"]:
                    mismatches += 1
                    print(f"  DIGEST MISMATCH {path.name}: manifest "
                          f"{row['sha256'][:16]}, file {actual[:16]}", file=sys.stderr)
                    continue

                # The key is the tier position, not the original filename, so a
                # reader sees samples in tier order and shard membership is a
                # property of position rather than of an arbitrary name.
                key = f"{position:06d}"
                add(tar, f"{key}.png", payload)
                add(tar, f"{key}.json", json.dumps(row, sort_keys=True).encode())
                digest.update(payload)

        size = shard.stat().st_size
        shard_sha = hashlib.sha256(shard.read_bytes()).hexdigest()
        index.append({
            "shard": shard.name,
            "samples": end - start,
            "first_tier_order": start,
            "last_tier_order": end - 1,
            "bytes": size,
            "sha256": shard_sha,
        })
        print(f"  {shard.name}  {end - start:>5} samples  {size / 1e6:>8.1f} MB  "
              f"{shard_sha[:16]}")

    if mismatches:
        print(f"\n{mismatches} file(s) did not match their manifest digest and were "
              f"NOT packed. The tier is incomplete; fix the corpus and re-run.",
              file=sys.stderr)

    manifest_out = out / f"pentimento-{name.lower()}-index.json"
    manifest_out.write_text(json.dumps({
        "tier": name,
        "samples": len(chosen) - mismatches,
        "shards": index,
        "per_shard": args.per_shard,
        "layout": "webdataset",
        "note": "samples are keyed by tier_order, so a smaller tier is a prefix "
                "of a larger one and shard N is identical across tiers",
    }, indent=2) + "\n")

    print(f"\n{len(index)} shard(s), {sum(s['bytes'] for s in index) / 1e9:.2f} GB, "
          f"{time.monotonic() - started:.0f}s")
    print(f"index: {manifest_out}")
    return 1 if mismatches else 0


if __name__ == "__main__":
    raise SystemExit(main())
