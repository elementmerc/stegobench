#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Turn a scored corpus into `result-v1` documents.

WHY THIS EXISTS
---------------
Numbers that live only in a log are numbers nobody can check. This writes each
measurement as a `result-v1` document, which is the format stegobench asks
other people to report in, and `stegobench validate` then checks the output.
A format whose author does not use it is a proposal, not a standard.

THE TWO MISTAKES IT IS BUILT NOT TO MAKE
----------------------------------------
**An arm is scored only against the covers it was actually run on.** outguess
covers 160 of the 200, and pooling all 200 into the negative side moved its AUC
from 0.49 to 0.44, which is a five point error in the direction of a more
dramatic finding. The 40 covers it never touched are not a control for it. That
error was made here on 2026-09-18 and nearly reached a report.

**A file with no score is counted, never skipped quietly.** The panel writes a
placeholder record before it scores, so the same file appears twice: once empty
and once filled. Reading the first occurrence gives an empty set, reading the
last gives a partial one, and neither announces itself. The record kept is the
one with the most fields filled, and anything still unscored lands in
`n_error`, which `result-v1` requires precisely so that zero is a claim.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import pathlib
import re
import sys

from metrics import MetricsRefused, metrics

#: Detector name in panel.jsonl -> (subject name, image tag we built it from).
DETECTORS = {
    "aletheia_spa": ("aletheia-spa", "stegobench/aletheia"),
    "aletheia_rs": ("aletheia-rs", "stegobench/aletheia"),
    "stegexpose": ("stegexpose", "stegobench/stegexpose"),
}

FPRS = (0.01, 0.10)

#: Embedders driven in bits per pixel. Everything else that carries a rate is
#: driven as a fraction of the capacity the tool reports for that cover, which
#: is a different quantity and must not share a field with bpp: 0.4 bpp and 5%
#: of capacity are not comparable numbers and a reader seeing one column would
#: compare them.
BPP_EMBEDDERS = {"hugo", "wow", "suniward", "hill", "mipod", "juniward", "uerd",
                 "lsb", "lsbmatching"}


def cover_id(name: str) -> str:
    """The cover this file came from, whatever directory depth it sits at.

    `outguess/0200/00042.jpg` and `clean/00042.jpg` are the same cover, and
    that correspondence is what makes the pairing check possible.
    """
    m = re.search(r"(\d+)\.[A-Za-z]+$", name)
    return m.group(1) if m else name


def best_records(path: pathlib.Path) -> list[dict]:
    """One record per file: the one with the most detector fields filled."""
    best: dict[str, dict] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        filled = sum(1 for d in DETECTORS if row.get(d) is not None)
        prev = best.get(row["file"])
        if prev is None or filled > sum(1 for d in DETECTORS if prev.get(d) is not None):
            best[row["file"]] = row
    return list(best.values())


def arm_metrics(pos: list[float], neg: list[float]) -> dict:
    """Every reported figure for one arm, from `stegobench metrics`.

    The arithmetic used to live here, in a second implementation that disagreed
    with the Rust one at the operating point sitting exactly on the budget.
    This module keeps the per-arm logic that decides WHICH scores belong to a
    measurement, which is the part that is specific to a corpus; the numbers
    over them come from the one place that computes numbers.

    Takes the two sides separately, as the pairing logic above produces them,
    and hands them over as scores and labels, which is what a ranking needs.
    """
    return metrics(
        [*pos, *neg],
        [True] * len(pos) + [False] * len(neg),
        budgets=FPRS,
    )


def digest_of(path: pathlib.Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def emit(corpus_dir: pathlib.Path, out_dir: pathlib.Path, corpus_name: str,
         clean_prefix: str, image_digests: dict[str, str], domain: str,
         fmt: str, harness_version: str) -> int:
    panel = corpus_dir / "panel.jsonl"
    manifest = corpus_dir / "manifest.jsonl"
    if not panel.is_file():
        print(f"no panel.jsonl in {corpus_dir}", file=sys.stderr)
        return 1

    recs = best_records(panel)
    corpus_digest = digest_of(manifest) if manifest.is_file() else "sha256:unknown"

    clean = {cover_id(r["file"]): r for r in recs if r["file"].startswith(clean_prefix)}
    arms: dict[str, list[dict]] = {}
    for r in recs:
        if not r["file"].startswith(clean_prefix):
            arms.setdefault("/".join(r["file"].split("/")[:-1]), []).append(r)

    out_dir.mkdir(parents=True, exist_ok=True)
    written = 0
    # A measurement the metrics refused is named and counted rather than
    # skipped. A short run of documents under an exit code of zero reads as
    # "these are all the arms there were".
    refused: list[str] = []
    for arm_key, rows in sorted(arms.items()):
        parts = arm_key.split("/")
        embedder = parts[0]
        # The directory encodes the rate times a thousand in both families:
        # suniward/0400 is 0.400 bpp, outguess/0050 is 0.050 of capacity.
        rate = None
        if len(parts) > 1 and parts[1].isdigit() and int(parts[1]) > 0:
            rate = {
                "value": int(parts[1]) / 1000,
                "unit": "bpp" if embedder in BPP_EMBEDDERS else "capacity_fraction",
            }
        ids = {cover_id(r["file"]) for r in rows}

        for field, (subject, image) in DETECTORS.items():
            pos = [r[field] for r in rows if r.get(field) is not None]
            # The pairing rule: only covers this arm was actually run on.
            neg = [clean[i][field] for i in ids
                   if i in clean and clean[i].get(field) is not None]
            if not pos or not neg:
                continue
            # Failures on BOTH sides. A clean image this detector could not
            # score shrinks the negative set just as silently as a stego one
            # shrinks the positive set, and reporting only half of that would
            # make n_error a comforting number rather than an honest one.
            n_error = (sum(1 for r in rows if r.get(field) is None)
                       + sum(1 for i in ids
                             if i in clean and clean[i].get(field) is None))

            try:
                figures = arm_metrics(pos, neg)
            except MetricsRefused as e:
                refused.append(f"{arm_key} / {subject}: {e.reason}: {e}")
                continue

            doc = {
                "schema": "stegobench/result-v1",
                "subject": {"name": subject,
                            "version": image_digests.get(image, "sha256:unknown"),
                            "kind": "detector"},
                # Supplied, not fetched: this corpus was built here by the
                # generators beside this file. Nothing downloaded it, and
                # saying otherwise would vouch for bytes nobody saw arrive.
                # PAIRS, which is the stego side: one cover and its twin is one
                # pair, so 6 covers and 12 stego images is 12 and not 18. The
                # Rust scorer wrote items here for as long as both writers
                # existed, and one published field meaning two things is worse
                # than a missing one. See CorpusRef::pairs in
                # crates/stegobench-core/src/result.rs.
                "corpus": {"name": corpus_name, "digest": corpus_digest,
                           "source": "supplied", "pairs": len(rows)},
                "arm": {"embedder": embedder, "domain": domain, "format": fmt,
                        **({"rate": rate} if rate is not None else {})},
                "metrics": {
                    "auc": round(figures["auc"], 4),
                    "tpr_at_fpr": {f"{f}": round(figures["tpr_at_fpr"][f], 4)
                                   for f in FPRS},
                    "n_clean": len(neg),
                    "n_stego": len(pos),
                    "n_error": n_error,
                },
                "provenance": {
                    "plugins": [{"name": subject,
                                 "image": f"{image}@{image_digests.get(image, 'sha256:unknown')}",
                                 "determinism": "exact",
                                 # The image digest names bytes anybody can
                                 # pull, and the container is started with no
                                 # network. Two facts, two fields: they agree
                                 # here and they do not for a service entry,
                                 # which is why one field could not carry both.
                                 "pinned_by": "image-digest",
                                 "isolation": "sandbox-no-network"}],
                    "harness_version": harness_version,
                    "started_utc": dt.datetime.now(dt.timezone.utc)
                                     .strftime("%Y-%m-%dT%H:%M:%SZ"),
                    "elapsed_seconds": 0.0,
                    "network_reachable": False,
                },
                "declarations": {
                    "split_discipline": "not-applicable",
                    "pairing": "single-variable",
                    # Custom, because these arms are not a registered tier
                    # whose digest anybody declared in advance. The number is
                    # comparable with itself rather than with somebody else's.
                    "configuration": "custom",
                    "self_reported": False,
                },
            }
            name = f"{corpus_name}-{embedder}-{parts[1] if len(parts) > 1 else '0000'}-{subject}.json"
            (out_dir / name).write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n", encoding="utf-8")
            written += 1
    print(f"{corpus_name}: {written} result-v1 document(s) in {out_dir}")
    if refused:
        print(f"\n{len(refused)} measurement(s) produced no document because "
              f"the metrics refused them:\n  " + "\n  ".join(refused),
              file=sys.stderr)
        return 1
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--corpus-dir", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--name", required=True, help="corpus name for the result")
    ap.add_argument("--clean-prefix", default="clean")
    ap.add_argument("--domain", default="jpeg", choices=["spatial", "jpeg", "structural"])
    ap.add_argument("--format", default="jpeg")
    ap.add_argument("--harness-version", default="0.1.0")
    ap.add_argument("--image-digests", default="",
                    help="comma-separated image=sha256:... pairs")
    args = ap.parse_args(argv)

    digests = {}
    for pair in args.image_digests.split(","):
        if "=" in pair:
            k, v = pair.split("=", 1)
            digests[k.strip()] = v.strip()

    return emit(pathlib.Path(args.corpus_dir), pathlib.Path(args.out), args.name,
                args.clean_prefix, digests, args.domain, args.format,
                args.harness_version)


if __name__ == "__main__":
    raise SystemExit(main())
