#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Give an already-built arm the clean half it should have had.

WHY A MIGRATION RATHER THAN A REBUILD
--------------------------------------
A tool that rewrites the container leaves its encoder's signature on every file
it writes. Paired against a clean half written by anything else, the pair
differs in the encoder as well as in the payload, and a detector reading that
difference is measuring the encoder.

`build_jpeg_arms.py` now pairs such a tool against its own writer. Corpora
built before it did are still correct in their stego half: nothing about those
bytes was wrong, and they were embedded from exactly the cover they should
have been. Only the clean half of the pair was the wrong file.

So this builds the missing clean half and repoints the rows, rather than
spending hours re-running embeddings that would come out byte-identical.

WHAT IT CHANGES, AND WHAT IT LEAVES ALONE
------------------------------------------
Changed, for rows whose tool rewrites the container:

    clean          -> clean_<tool>/NNNNN.jpg
    clean_sha256   -> that file's digest
    pairing        -> "writer-matched"
    detail.clean_half -> how the clean half was written

Untouched: every stego file, every stego digest, every payload, every row
belonging to a tool that edits coefficients in place. The old clean directory
stays where it is, because it is still the correct clean half for those tools
and it is still the cover as a JPEG.

The manifest is rewritten through a temporary file and renamed over the
original, so an interruption leaves the old one intact rather than half of a
new one.

Usage::

    python3 generators/repair_writer_matched_pairs.py \\
        --arms ~/pentimento/arms/core/jpeg-tools --jobs 8
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys
import time
from concurrent import futures

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from embedders import EmbedError  # noqa: E402
from tools import build  # noqa: E402


def digest(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--arms", required=True, help="a built arm directory")
    ap.add_argument("--jobs", type=int, default=8,
                    help="concurrent tool runs. Each is a container start")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    root = pathlib.Path(args.arms)
    manifest = root / "manifest.jsonl"
    if not manifest.is_file():
        print(f"no manifest at {manifest}", file=sys.stderr)
        return 1

    rewriting = {e.id: e for e in build() if e.rewrites_container}
    if not rewriting:
        print("no registered tool rewrites the container, so nothing to repair")
        return 0
    print(f"tools that rewrite the container: {', '.join(sorted(rewriting))}")

    rows = [json.loads(l) for l in manifest.read_text(encoding="utf-8").splitlines() if l.strip()]
    affected = [r for r in rows
                if r.get("tool") in rewriting
                and r.get("pairing") != "writer-matched"]
    if not affected:
        print(f"{len(rows)} row(s), none needing repair")
        return 0

    # One clean half per (tool, cover), not per row: four rates over the same
    # cover share it, and building it four times would quadruple the work for
    # four identical files.
    wanted: dict[tuple[str, str], pathlib.Path] = {}
    settings: dict[tuple[str, str], dict] = {}
    for row in affected:
        stem = pathlib.PurePosixPath(row["clean"]).name
        key = (row["tool"], stem)
        wanted[key] = root / row["clean"]
        settings[key] = row

    print(f"{len(affected)} row(s) over {len(wanted)} clean half/halves to build")
    if args.dry_run:
        for tool, stem in sorted(wanted)[:5]:
            print(f"  would write clean_{tool}/{stem}")
        print("DRY RUN, nothing written")
        return 0

    built: dict[tuple[str, str], dict] = {}
    failed: list[str] = []
    started = time.monotonic()
    done_count = 0
    last_beat = time.monotonic()

    def one(key: tuple[str, str]) -> tuple[tuple[str, str], dict | None, str]:
        tool, stem = key
        target = root / f"clean_{tool}" / stem
        if target.is_file():
            return key, {"writer": tool, "resumed": True}, ""
        try:
            # Configured from the row, not from the tool's defaults. A clean
            # half written with different settings is not this pair's clean
            # half, and the difference is invisible until somebody measures it.
            configured = rewriting[tool].configured_for(settings[key])
            detail = configured.matched_clean(wanted[key], target)
        except (EmbedError, OSError) as e:
            return key, None, f"clean_{tool}/{stem}: {e}"
        return key, detail, ""

    with futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for key, detail, error in pool.map(one, sorted(wanted)):
            done_count += 1
            if detail is None:
                failed.append(error)
                print(f"  {error}", file=sys.stderr)
            else:
                built[key] = detail
            if time.monotonic() - last_beat >= 30:
                last_beat = time.monotonic()
                rate = done_count / max(time.monotonic() - started, 1)
                left = (len(wanted) - done_count) / max(rate, 1e-9)
                print(f"  ... {done_count}/{len(wanted)}, "
                      f"{len(failed)} failed, about {left / 60:.0f} min left")

    repaired = 0
    for row in rows:
        tool = row.get("tool")
        if tool not in rewriting or row.get("pairing") == "writer-matched":
            continue
        stem = pathlib.PurePosixPath(row["clean"]).name
        key = (tool, stem)
        if key not in built:
            continue  # its clean half could not be built; leave the row as it was
        target = root / f"clean_{tool}" / stem
        row["clean"] = str(target.relative_to(root)).replace("\\", "/")
        row["clean_sha256"] = digest(target)
        row["pairing"] = "writer-matched"
        row.setdefault("detail", {})["clean_half"] = built[key]
        repaired += 1

    part = manifest.with_suffix(".jsonl.part")
    part.write_text("".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")
    part.replace(manifest)

    print(f"\n{repaired} row(s) repaired, {len(built)} clean half/halves, "
          f"{len(failed)} failed, {(time.monotonic() - started) / 60:.1f} min")
    if failed:
        print(f"{len(failed)} clean half/halves could not be built, and their "
              f"rows were left as they were rather than repointed at a file "
              f"that is not there.", file=sys.stderr)
        return 2
    print("Repack the affected arms: the shards still carry the old clean half.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
