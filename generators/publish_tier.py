#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Turn a packed tier into a release, from one source of truth.

THE FAILURE THIS IS DESIGNED AGAINST
------------------------------------
`cover-source-licensing.md` catalogues corpora whose mirrors advertise a licence
the corpus does not have: Dresden as CC0, UCID as MIT, BOSSbase as Apache. In
every case a human typed a licence string into an upload form and it drifted
from what the files actually carry. That is licence laundering, and it is the
single most likely way this corpus becomes the thing it was built to correct.

So no licence string is ever typed. Every platform's metadata is derived, here,
from the per-file manifest, and if the manifest changes the metadata changes with
it. `prepare` is pure Python with no dependencies and no credentials, so the
derivation can be inspected and diffed before anything is uploaded anywhere.

THE MIXED-LICENCE DECISION, STATED RATHER THAN BURIED
-----------------------------------------------------
Every hosting platform wants ONE licence for the item. This corpus has seven,
across 10,000 files, and no single one of them is true of the whole.

Taking the loosest, CC0, would be laundering: it would tell a user they owe no
attribution for 5,429 covers that require it.

So the collection-level licence is the **strictest obligation present**, CC BY
4.0. A user who complies with it is compliant for every file in the corpus,
including the CC0 and public domain ones, because CC BY's obligations are a
superset. The metadata then says plainly that per-file licences vary and the
manifest is authoritative, so nobody is misled into doing more work than they
owe either.

That is the conservative direction. Being wrong here costs a stranger a lawsuit,
and being over-careful costs them a credit line they did not strictly need.

WHY ACADEMIC TORRENTS GETS WEB SEEDS
------------------------------------
A torrent nobody seeds is a dead link, and seeding a corpus indefinitely from a
laptop is not a plan. The torrent therefore carries the Internet Archive item as
a web seed (BEP 19), so the data stays available from the Archive whether or not
any peer is online, and peers accelerate it when they are.

That means the Archive upload must land BEFORE the torrent is published, and the
ordering is enforced rather than documented.
"""
from __future__ import annotations

import argparse
import collections
import hashlib
import json
import os
import pathlib
import sys

#: The collection licence this corpus DECLARES. See the header for why the
#: strictest obligation is the honest choice rather than the loosest.
#:
#: It is a declaration, and `strictest_obligation()` below is what checks it is
#: still true. Until 2026-09-21 this constant was the whole mechanism: the run
#: printed "strictest present, not loosest" while nothing had looked at a single
#: licence in the manifest. One share-alike cover reaching the corpus would have
#: published the collection under a licence that did not cover it, and the
#: sentence asserting otherwise would have printed exactly the same.
COLLECTION_LICENCE = "CC BY 4.0"

#: Obligation classes, loosest first. Within a class, version differences are
#: not ordered: CC BY 2.0 and CC BY 4.0 impose the same KIND of obligation, and
#: the collection licence covers a user who complies with it either way. What
#: matters is whether a class appears that CC BY does not cover.
OBLIGATION_CLASSES = ("none", "attribution")

#: Beyond these, the declared collection licence is not a superset and
#: publishing under it would understate what a user owes.
UNCOVERED_MARKERS = ("-sa", "share", "nc", "noncommercial", "non-commercial",
                     "nd", "noderiv", "no-deriv", "gfdl")


def obligation_class(licence: str) -> str:
    """Which obligation a per-file licence imposes, or 'uncovered'.

    Coarse on purpose. The question is not which licence this is, it is
    whether a user who complies with the collection licence is thereby
    compliant for this file. Anything this cannot place is 'uncovered', because
    an unrecognised licence is exactly the case where guessing is worst.
    """
    lowered = licence.strip().lower().replace(" ", "-")
    if any(marker in lowered for marker in UNCOVERED_MARKERS):
        return "uncovered"
    if lowered.startswith(("cc0", "public-domain", "no-rights",
                           "copyrighted-free-use")):
        return "none"
    if lowered.startswith("cc-by"):
        return "attribution"
    return "uncovered"


def strictest_obligation(licences) -> tuple[str, list[str]]:
    """The strictest class present, and the licences that are not covered.

    Returns the class name and every distinct licence value the declared
    collection licence does not account for. An empty second element is what
    makes the printed claim true rather than decorative.
    """
    seen = {obligation_class(l): [] for l in ("none", "attribution")}
    uncovered = []
    strictest = "none"
    for licence in licences:
        kind = obligation_class(licence)
        if kind == "uncovered":
            if licence not in uncovered:
                uncovered.append(licence)
            continue
        if OBLIGATION_CLASSES.index(kind) > OBLIGATION_CLASSES.index(strictest):
            strictest = kind
    return strictest, sorted(uncovered)
COLLECTION_LICENCE_URL = "https://creativecommons.org/licenses/by/4.0/"

#: Kaggle accepts an enumerated licence, and none of its values means "mixed".
#: "other" plus an explicit description is the honest answer; picking CC0-1.0
#: because it is on the list would be exactly the drift this file prevents.
KAGGLE_LICENCE = "other"

IA_COLLECTION = "opensource_media"


def bencode(value) -> bytes:
    """Minimal bencode, so torrent creation needs no dependency.

    A torrent file is bencoded and the format is four rules. Writing them is
    cheaper than adding a dependency to a release path, and a release path with
    fewer moving parts is one that still works in a year.
    """
    if isinstance(value, int):
        return b"i" + str(value).encode() + b"e"
    if isinstance(value, bytes):
        return str(len(value)).encode() + b":" + value
    if isinstance(value, str):
        return bencode(value.encode())
    if isinstance(value, list):
        return b"l" + b"".join(bencode(v) for v in value) + b"e"
    if isinstance(value, dict):
        # Keys must be sorted, and that is not cosmetic: the infohash is a hash
        # of this structure, so an unsorted dict produces a different torrent
        # for identical data.
        items = sorted(value.items())
        return b"d" + b"".join(bencode(k) + bencode(v) for k, v in items) + b"e"
    raise TypeError(f"cannot bencode {type(value).__name__}")


#: Commons writes the same licence under more than one spelling. Left alone
#: they become separate rows in the published table, so "Public Domain" appears
#: beside "Public domain" with a count of one against a count of nearly two
#: thousand, and anyone grouping by the field gets two groups for one licence.
#: The raw value stays in the manifest; this is what the published table and
#: the per-sample record use.
CANONICAL_LICENCE = {
    "public domain": "Public domain",
    "cc0": "CC0",
}


def canonical_licence(value: str | None) -> str | None:
    """One spelling per licence, chosen once rather than per source."""
    if not value:
        return value
    return CANONICAL_LICENCE.get(value.strip().lower(), value.strip())


def digest_of(path: pathlib.Path) -> str:
    """The sha256 of a file, read in blocks so a large manifest does not sit
    in memory twice."""
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for block in iter(lambda: fh.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def licence_summary(manifest: pathlib.Path, count: int | None = None) -> dict:
    """Everything the platforms need to know about licensing, from the files.

    `count` restricts this to the covers ACTUALLY IN THE TIER, and leaving it out
    was a bug worth keeping the memory of. The first version summarised the whole
    manifest, so a Nano release carrying 200 images would have advertised 10,000
    covers and a licence breakdown describing files it did not contain.

    That is the exact drift this module exists to prevent, committed by the
    module itself. Deriving metadata from a manifest is not sufficient; it has to
    be derived from the rows that ship.
    """
    counts: collections.Counter = collections.Counter()
    urls: dict[str, str] = {}
    attribution_required = 0
    capture: collections.Counter = collections.Counter()
    total = 0
    rows = [json.loads(l) for l in manifest.read_text(encoding="utf-8").splitlines() if l.strip()]
    rows.sort(key=lambda r: r.get("tier_order", 0))
    if count is not None:
        rows = rows[:count]
    for row in rows:
        total += 1
        licence = canonical_licence(row.get("licence")) or "unrecorded"
        counts[licence] += 1
        if row.get("licence_url"):
            urls[licence] = row["licence_url"]
        if row.get("attribution_required"):
            attribution_required += 1
        capture[row.get("capture_class") or "unknown"] += 1
    return {
        # WHAT THIS WAS COMPUTED FROM, so it can witness its own staleness.
        #
        # On 2026-09-22 the shipped README said 5,429 covers require
        # attribution and the manifest said 5,453. Nothing was wrong with
        # either number: `release_metadata` read a summary written three days
        # and one cover backfill earlier, and no field in the file said which
        # manifest it described. A derived artefact that does not name its
        # source cannot be checked against it, and the reader sees a figure
        # that renders exactly like a current one.
        #
        # The digest rather than a timestamp, for the same reason arm rows
        # carry `source_sha256`: a copy, a restore or a clock skew all forge
        # an mtime, and none of them changes the bytes.
        "source_manifest_sha256": digest_of(manifest),
        "source_manifest_rows": total,
        "total": total,
        "licences": dict(counts.most_common()),
        "licence_urls": urls,
        "attribution_required": attribution_required,
        "attribution_required_pct": round(100 * attribution_required / max(1, total), 1),
        "capture_class": dict(capture.most_common()),
    }


def licences_markdown(summary: dict, tier: str) -> str:
    """The file a human reads before using the corpus."""
    lines = [
        f"# Licensing, Pentimento {tier}",
        "",
        "**Every file in this corpus carries its own licence, and the manifest is "
        "authoritative.** This page is a summary of what those licences are; it "
        "does not replace them.",
        "",
        f"The collection is published as **{COLLECTION_LICENCE}** "
        f"({COLLECTION_LICENCE_URL}). That is the strictest obligation present in "
        "the corpus, not the loosest. Complying with it is sufficient for every "
        "file here. Where the manifest records a looser licence for a particular "
        "image, you may rely on that instead.",
        "",
        "## What is actually in here",
        "",
        "| Licence | Covers | Attribution required | Terms |",
        "|---|---|---|---|",
    ]
    for licence, count in summary["licences"].items():
        url = summary["licence_urls"].get(licence, "")
        required = "yes" if licence.upper().startswith("CC BY") else "no"
        link = f"[{licence}]({url})" if url else licence
        lines.append(f"| {link} | {count:,} | {required} | |")
    lines += [
        "",
        f"**{summary['attribution_required']:,} of {summary['total']:,} covers "
        f"({summary['attribution_required_pct']}%) require attribution.** Each "
        "one carries a ready-made credit line in its manifest row, under "
        "`attribution`. Where the original source did not record an author, that "
        "line says so rather than omitting it.",
        "",
        "## Capture class",
        "",
        "Not every image is camera output, and the ones that are not are labelled "
        "rather than removed.",
        "",
        "| Class | Covers |",
        "|---|---|",
    ]
    for cls, count in summary["capture_class"].items():
        lines.append(f"| {cls} | {count:,} |")
    lines += [
        "",
        "`capture_class_basis` on each row records which rule decided the class, "
        "so you can disagree with a particular one.",
        "",
        "## What this corpus is",
        "",
        "**Every cover was a JPEG before it was cropped.** This is a "
        "JPEG-decompressed spatial corpus and it is not comparable to BOSSbase, "
        "which is RAW-developed and never compressed. The 8x8 block lattice "
        "survives in the pixels and its phase is recoverable from the `crop_box` "
        "published in the manifest. Thresholds calibrated here will not transfer "
        "unchanged to never-compressed covers.",
        "",
        "That is stated plainly because it decides whether this corpus suits your "
        "experiment.",
    ]
    return "\n".join(lines) + "\n"


def make_torrent(shards: list[pathlib.Path], name: str, trackers: list[str],
                 web_seeds: list[str], piece_length: int = 1 << 22) -> bytes:
    """A multi-file torrent over the packed shards, with web seeds."""
    files, pieces, buffer = [], bytearray(), bytearray()
    for shard in sorted(shards):
        files.append({"length": shard.stat().st_size, "path": [shard.name]})
        with shard.open("rb") as handle:
            while chunk := handle.read(1 << 20):
                buffer += chunk
                while len(buffer) >= piece_length:
                    pieces += hashlib.sha1(bytes(buffer[:piece_length])).digest()
                    del buffer[:piece_length]
    if buffer:
        pieces += hashlib.sha1(bytes(buffer)).digest()

    torrent = {
        "announce": trackers[0],
        "announce-list": [[t] for t in trackers],
        # The torrent names the command a recipient would run to rebuild
        # this, so it has to be the command that actually exists.
        "created by": "pentimento publish-tier",
        "info": {
            "name": name,
            "piece length": piece_length,
            "pieces": bytes(pieces),
            "files": files,
        },
    }
    if web_seeds:
        torrent["url-list"] = web_seeds
    return bencode(torrent)


def cmd_prepare(args) -> int:
    packed = pathlib.Path(args.packed)
    manifest = pathlib.Path(args.manifest)
    index_files = sorted(packed.glob("*-index.json"))
    if not index_files:
        print(f"no packed index under {packed}. Run pack_tier.py first.", file=sys.stderr)
        return 1
    index = json.loads(index_files[0].read_text(encoding="utf-8"))
    tier = index["tier"]
    summary = licence_summary(manifest, index["samples"])

    # THE DECLARED COLLECTION LICENCE HAS TO BE CHECKED, NOT ASSERTED.
    #
    # Publishing under a licence that does not cover every file in the
    # collection understates what a recipient owes, and this is the corpus
    # whose whole argument is that licensing should be traceable. Refusing here
    # costs a rebuild; being wrong costs a stranger.
    strictest, uncovered = strictest_obligation(summary["licences"])
    if uncovered:
        print(f"\nREFUSING TO PREPARE: {len(uncovered)} licence value(s) are "
              f"not covered by {COLLECTION_LICENCE}:", file=sys.stderr)
        for licence in uncovered:
            print(f"  {licence:24} {summary['licences'][licence]:>6,} cover(s)",
                  file=sys.stderr)
        print(f"\nA user who complies with {COLLECTION_LICENCE} would NOT be "
              f"compliant for those files, so declaring it would understate "
              f"what they owe. Either remove them with "
              f"select_unpublishable.py, or change the declared collection "
              f"licence deliberately.", file=sys.stderr)
        return 1

    identifier = f"pentimento-{tier.lower()}-v1"
    total_bytes = sum(s["bytes"] for s in index["shards"])
    blurb = (
        f"Pentimento {tier}: {summary['total']:,} permissively licensed cover "
        f"images for steganalysis research, 512x512, with per-file provenance "
        f"and licensing. JPEG-decompressed spatial corpus; see LICENCES.md. "
        f"{summary['attribution_required']:,} covers "
        f"({summary['attribution_required_pct']}%) require attribution and each "
        f"carries a ready-made credit line."
    )

    (packed / "LICENCES.md").write_text(licences_markdown(summary, tier), encoding="utf-8")

    # Internet Archive. licenseurl is a single field on a mixed corpus, so it
    # carries the strictest obligation and the description says where the truth
    # lives.
    ia_meta = {
        "identifier": identifier,
        "mediatype": "data",
        "collection": IA_COLLECTION,
        "title": f"Pentimento {tier}: permissively licensed steganalysis covers",
        "description": blurb,
        "licenseurl": COLLECTION_LICENCE_URL,
        "subject": ["steganalysis", "steganography", "image forensics",
                    "dataset", "computer vision"],
        "creator": "Daniel Iwugo",
    }
    (packed / "ia-metadata.json").write_text(json.dumps(ia_meta, indent=2) + "\n", encoding="utf-8")

    kaggle_meta = {
        "title": f"Pentimento {tier} steganalysis covers",
        "id": f"{args.kaggle_user or 'USERNAME'}/{identifier}",
        "licenses": [{"name": KAGGLE_LICENCE}],
        "subtitle": f"{summary['total']:,} permissively licensed 512x512 covers",
        "description": blurb,
    }
    (packed / "dataset-metadata.json").write_text(json.dumps(kaggle_meta, indent=2) + "\n", encoding="utf-8")

    (packed / "licence-summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")

    print(f"tier {tier}: {summary['total']:,} covers, "
          f"{len(index['shards'])} shard(s), {total_bytes / 1e9:.2f} GB")
    print(f"collection licence: {COLLECTION_LICENCE}, and {strictest} is the "
          f"strictest obligation actually present across "
          f"{len(summary['licences'])} licence value(s)")
    for licence, count in summary["licences"].items():
        print(f"  {count:>6,}  {licence}")
    print(f"\nattribution required on {summary['attribution_required']:,} "
          f"({summary['attribution_required_pct']}%)")
    print("\nwritten: LICENCES.md, ia-metadata.json, dataset-metadata.json, "
          "licence-summary.json")
    return 0


def cmd_torrent(args) -> int:
    packed = pathlib.Path(args.packed)
    index_files = sorted(packed.glob("*-index.json"))
    if not index_files:
        print(f"no packed index under {packed}", file=sys.stderr)
        return 1
    index = json.loads(index_files[0].read_text(encoding="utf-8"))
    tier = index["tier"]
    identifier = f"pentimento-{tier.lower()}-v1"

    if not args.web_seed and not args.allow_no_seed:
        print(
            "refusing to build a torrent with no web seed.\n"
            "  A torrent nobody seeds is a dead link, and seeding this from a\n"
            "  laptop is not a plan. Upload to the Internet Archive first, then\n"
            "  pass --web-seed https://archive.org/download/" + identifier + "/\n"
            "  Override with --allow-no-seed if you really mean to seed it yourself.",
            file=sys.stderr)
        return 1

    shards = sorted(packed.glob("*.tar"))
    if not shards:
        print(f"no shards under {packed}", file=sys.stderr)
        return 1

    blob = make_torrent(shards, identifier, args.tracker, args.web_seed)
    out = packed / f"{identifier}.torrent"
    out.write_bytes(blob)
    print(f"{out}  {len(blob):,} bytes over {len(shards)} shard(s)")
    print(f"web seeds: {args.web_seed or 'NONE'}")
    print("\nAcademic Torrents has no upload API. Upload this file at\n"
          "  https://academictorrents.com/upload.php\n"
          "and paste the description from LICENCES.md.")
    return 0


def cmd_upload(args) -> int:
    packed = pathlib.Path(args.packed)
    missing = [f for f in ("ia-metadata.json", "LICENCES.md") if not (packed / f).is_file()]
    if missing:
        print(f"run prepare first; missing {', '.join(missing)}", file=sys.stderr)
        return 1

    needed = {
        "internetarchive": ("IA_ACCESS_KEY", "IA_SECRET_KEY"),
        "kaggle": ("KAGGLE_USERNAME", "KAGGLE_KEY"),
    }[args.destination]
    absent = [k for k in needed if not os.environ.get(k)]
    if absent:
        print(f"missing credentials in the environment: {', '.join(absent)}",
              file=sys.stderr)
        print("Credentials are read from the environment and never written to a "
              "file in this repository.", file=sys.stderr)
        return 1

    print(f"would upload {packed} to {args.destination}")
    print("Upload is not wired yet: the SDKs are not installed on this box and "
          "installing them is a deliberate step, not something a release script "
          "should do to a machine on its own.")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="command", required=True)

    p = sub.add_parser("prepare", help="derive every platform's metadata from the manifest")
    p.add_argument("--packed", required=True)
    p.add_argument("--manifest", required=True)
    p.add_argument("--kaggle-user", default=None)
    p.set_defaults(func=cmd_prepare)

    t = sub.add_parser("torrent", help="build a .torrent over the packed shards")
    t.add_argument("--packed", required=True)
    t.add_argument("--tracker", action="append",
                   default=["udp://tracker.opentrackr.org:1337/announce"])
    t.add_argument("--web-seed", action="append", default=[])
    t.add_argument("--allow-no-seed", action="store_true")
    t.set_defaults(func=cmd_torrent)

    u = sub.add_parser("upload", help="push to one destination")
    u.add_argument("--packed", required=True)
    u.add_argument("--destination", required=True,
                   choices=("internetarchive", "kaggle"))
    u.set_defaults(func=cmd_upload)

    args = ap.parse_args(argv)
    sys.stdout.reconfigure(line_buffering=True)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
