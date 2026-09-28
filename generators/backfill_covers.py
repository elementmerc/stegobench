#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Replace the covers we cannot publish, in place, keeping the tiers nested.

WHY IN PLACE RATHER THAN DROPPING
---------------------------------
`tier_order` is a dense 0..n-1 ordering, and a tier is the first *n* of it.
That is what makes Nano a prefix of Lite a prefix of Core, and it is the
guarantee that stops somebody training on Lite and evaluating on Core while
unknowingly testing on images they trained on.

Dropping 109 covers would renumber everything after each hole, so every tier
boundary moves and the corpus stops matching any checksum anyone has recorded.
Replacing a cover at its own `tier_order` leaves the ordering untouched: only
that one position changes, and only the arms derived from that one cover need
rebuilding.

It also keeps the count at exactly 10,000, which is not vanity. The corpus was
sized to match BOSSbase's cover count on purpose, because BOSSbase cannot be
republished and matching its size is the only honest way to claim comparable
scale.

WHAT A REPLACEMENT HAS TO SATISFY
---------------------------------
Everything the original failed, and it is checked rather than assumed. A
replacement drawn from the same pool by the same sampler can carry the same
problem, and swapping one unpublishable cover for another while reporting
success would be worse than leaving the first one in, because nobody would look
again.

So the candidate manifest passed in here must already have been through
`audit_pd_licences.py` and `select_unpublishable.py`. This tool refuses to use
a candidate that appears in the candidates' own unpublishable list.

WHAT IT DOES NOT DO
-------------------
It does not rebuild the arms. The 109 replaced covers need their stego and
clean halves regenerated, and the tool arms need rebuilding anyway. Run the
builders, then `pack_arms.py`.

Usage::

    python backfill_covers.py --covers ~/pentimento/covers/commons \\
                              --unpublishable ~/pentimento/covers/commons/unpublishable.json \\
                              --candidates ~/pentimento/covers/backfill \\
                              --candidate-rejects ~/pentimento/covers/backfill/unpublishable.json \\
                              --dry-run
"""
from __future__ import annotations

import argparse
import collections
import hashlib
import json
import pathlib
import shutil
import sys

#: Carried over from the replaced row rather than from the candidate, because
#: these describe the cover's PLACE in the corpus rather than the image.
POSITIONAL = ("file", "tier_order")


class BackfillError(RuntimeError):
    pass


def load_jsonl(path: pathlib.Path) -> list[dict]:
    if not path.is_file():
        raise BackfillError(f"no manifest at {path}")
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip()]


def digest_of(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def choose(candidates: list[dict], unusable: set[str], wanted: int,
           already_used: set[int] | None = None) -> list[dict]:
    """The first `wanted` candidates that are publishable and not already in.

    `already_used` is the set of Commons pageids the live corpus holds. Without
    it a second backfill run picks the same candidates as the first, because
    the pool is read in order and nothing in it records having been spent. The
    corpus would then carry the same photograph at two tier positions, with two
    different filenames and two different digests, and every check downstream
    would pass: the files exist, the digests match, the licences join. It would
    surface as a cover and its "unrelated" counterpart landing on opposite
    sides of a train/test split.
    """
    used = already_used or set()
    clean = [c for c in candidates
             if c["file"] not in unusable and c.get("pageid") not in used]
    if len(clean) < wanted:
        raise BackfillError(
            f"{wanted} replacements needed and only {len(clean)} of "
            f"{len(candidates)} candidates are publishable and unused. Fetch "
            f"more rather than lowering the standard: the covers being "
            f"replaced failed this same test.")
    return clean[:wanted]


def splice(replaced: dict, candidate: dict) -> dict:
    """The candidate's row, wearing the replaced cover's position.

    Everything about the IMAGE comes from the candidate. Only the two fields
    that describe where it sits in the corpus are carried over, and
    `replaces` records what it stands in for so the swap is not silent.
    """
    row = dict(candidate)
    for field in POSITIONAL:
        row[field] = replaced[field]
    row["replaces"] = {
        "pageid": replaced.get("pageid"),
        "title": replaced.get("title"),
        "licence": replaced.get("licence"),
        "reason": replaced.get("reason"),
    }
    return row


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--covers", required=True, help="the live cover directory")
    ap.add_argument("--unpublishable", required=True,
                    help="select_unpublishable.py output for the live corpus")
    ap.add_argument("--candidates", required=True,
                    help="a directory of freshly fetched covers")
    ap.add_argument("--candidate-rejects", default=None,
                    help="select_unpublishable.py output for the CANDIDATES. "
                         "Without it this refuses to run, because a "
                         "replacement that fails the same test is not a fix")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args(argv)

    # A caller that redirected stdout may have put something there that
    # cannot be reconfigured, and losing the line buffering is a cosmetic
    # loss where crashing on it is a real one.
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(line_buffering=True)
    covers = pathlib.Path(args.covers)
    candidates_dir = pathlib.Path(args.candidates)
    manifest_path = covers / "manifest.jsonl"

    try:
        rows = load_jsonl(manifest_path)
        candidates = load_jsonl(candidates_dir / "manifest.jsonl")
        doomed = json.loads(pathlib.Path(args.unpublishable).read_text(encoding="utf-8"))
    except (BackfillError, OSError, json.JSONDecodeError) as e:
        print(f"cannot start: {e}", file=sys.stderr)
        return 1

    if args.candidate_rejects:
        rejects = json.loads(pathlib.Path(args.candidate_rejects).read_text(encoding="utf-8"))
        unusable = {c["file"] for c in rejects.get("covers", [])}
        print(f"candidates: {len(candidates):,}, of which {len(unusable)} are "
              f"themselves unpublishable")
    else:
        print("refusing to run without --candidate-rejects. The covers being "
              "replaced failed a licence audit; a replacement that has not "
              "been through the same audit is not known to be better.",
              file=sys.stderr)
        return 1

    by_file = {r["file"]: r for r in rows}
    targets = []
    for entry in doomed.get("covers", []):
        row = by_file.get(entry["file"])
        if row is None:
            print(f"{entry['file']} is not in the manifest", file=sys.stderr)
            return 1
        targets.append(dict(row, reason=entry.get("reason")))

    print(f"replacing {len(targets):,} of {len(rows):,} covers")
    by_reason = collections.Counter(t["reason"] for t in targets)
    for reason, count in sorted(by_reason.items()):
        print(f"  {reason:16} {count:>4}")

    in_corpus = {r.get("pageid") for r in rows if r.get("pageid") is not None}
    try:
        chosen = choose(candidates, unusable, len(targets), in_corpus)
    except BackfillError as e:
        print(f"\n{e}", file=sys.stderr)
        return 1

    swaps = []
    for replaced, candidate in zip(targets, chosen):
        source = candidates_dir / candidate["file"]
        if not source.is_file():
            print(f"candidate {candidate['file']} is not on disk",
                  file=sys.stderr)
            return 1
        destination = covers / replaced["file"]
        row = splice(replaced, candidate)
        # The digest must describe the file that ends up at this name, and the
        # candidate's own digest already does: the bytes are copied unchanged.
        row["sha256"] = candidate["sha256"]
        swaps.append({
            "position": replaced["tier_order"],
            "file": replaced["file"],
            "reason": replaced["reason"],
            "out": {"title": replaced.get("title"),
                    "licence": replaced.get("licence")},
            "in": {"title": candidate.get("title"),
                   "licence": candidate.get("licence")},
            "source": str(source),
            "destination": str(destination),
        })
        if not args.dry_run:
            part = destination.with_suffix(".png.part")
            shutil.copy2(source, part)
            part.replace(destination)
            actual = digest_of(destination)
            if actual != candidate["sha256"]:
                print(f"{destination.name}: copied bytes do not match the "
                      f"candidate's recorded digest", file=sys.stderr)
                return 1
        by_file[replaced["file"]] = row

    print(f"\n{len(swaps):,} swap(s) prepared")
    for swap in swaps[:5]:
        print(f"  {swap['file']} @ {swap['position']:>5}  "
              f"{swap['reason']:14}  {str(swap['out']['licence']):16} -> "
              f"{swap['in']['licence']}")
    if len(swaps) > 5:
        print(f"  ... and {len(swaps) - 5:,} more")

    if args.dry_run:
        print("\ndry run: nothing written")
        return 0

    # Rewritten in the original order, so the manifest stays sorted by the
    # ordering the tiers depend on.
    ordered = sorted(by_file.values(), key=lambda r: r["tier_order"])
    if [r["tier_order"] for r in ordered] != list(range(len(ordered))):
        print("tier_order is no longer dense after the splice; refusing to "
              "write", file=sys.stderr)
        return 1

    part = manifest_path.with_suffix(".jsonl.part")
    part.write_text("".join(json.dumps(r, sort_keys=True) + "\n"
                            for r in ordered), encoding="utf-8")
    part.replace(manifest_path)

    log = covers / "backfill.json"
    log.write_text(json.dumps({"swaps": swaps}, indent=2) + "\n", encoding="utf-8")
    print(f"\nwritten: {manifest_path}\nwritten: {log}")
    print("\nThe arms for these covers are now stale. Rebuild them, then "
          "repack.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
