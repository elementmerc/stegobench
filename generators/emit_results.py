#!/usr/bin/env python3
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
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        filled = sum(1 for d in DETECTORS if row.get(d) is not None)
        prev = best.get(row["file"])
        if prev is None or filled > sum(1 for d in DETECTORS if prev.get(d) is not None):
            best[row["file"]] = row
    return list(best.values())


def auc(pos: list[float], neg: list[float]) -> float:
    """Tie-aware ROC AUC, via the rank sum with average ranks.

    Ties are not a corner case here. Several of these detectors return the same
    score for many images, and the naive rank sum counts each tie as half a win
    in a way that reads as signal.
    """
    data = sorted([(v, 1) for v in pos] + [(v, 0) for v in neg])
    total = 0.0
    i = 0
    while i < len(data):
        j = i
        while j + 1 < len(data) and data[j + 1][0] == data[i][0]:
            j += 1
        rank = (i + j) / 2.0 + 1
        total += sum(rank for k in range(i, j + 1) if data[k][1] == 1)
        i = j + 1
    n1, n0 = len(pos), len(neg)
    return (total - n1 * (n1 + 1) / 2.0) / (n1 * n0)


def tpr_at_fpr(pos: list[float], neg: list[float], target: float) -> float:
    """Detection rate at a false-alarm budget, ties counted against us.

    The threshold is the score that lets no more than `target` of the clean
    images through. A detector returning one constant for everything scores
    zero here rather than appearing to work, which is the point.
    """
    allowed = int(len(neg) * target)
    cut = sorted(neg, reverse=True)[allowed - 1] if allowed >= 1 else max(neg)
    return sum(1 for v in pos if v > cut) / len(pos)


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

            doc = {
                "schema": "stegobench/result-v1",
                "subject": {"name": subject,
                            "version": image_digests.get(image, "sha256:unknown"),
                            "kind": "detector"},
                "corpus": {"name": corpus_name, "digest": corpus_digest,
                           "pairs": len(rows)},
                "arm": {"embedder": embedder, "domain": domain, "format": fmt,
                        **({"rate": rate} if rate is not None else {})},
                "metrics": {
                    "auc": round(auc(pos, neg), 4),
                    "tpr_at_fpr": {f"{f}": round(tpr_at_fpr(pos, neg, f), 4)
                                   for f in FPRS},
                    "n_clean": len(neg),
                    "n_stego": len(pos),
                    "n_error": n_error,
                },
                "provenance": {
                    "plugins": [{"name": subject,
                                 "image": f"{image}@{image_digests.get(image, 'sha256:unknown')}",
                                 "determinism": "exact"}],
                    "harness_version": harness_version,
                    "started_utc": dt.datetime.now(dt.timezone.utc)
                                     .strftime("%Y-%m-%dT%H:%M:%SZ"),
                    "elapsed_seconds": 0.0,
                    "network_reachable": False,
                },
                "declarations": {
                    "split_discipline": "not-applicable",
                    "pairing": "single-variable",
                    "self_reported": False,
                },
            }
            name = f"{corpus_name}-{embedder}-{parts[1] if len(parts) > 1 else '0000'}-{subject}.json"
            (out_dir / name).write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n")
            written += 1
    print(f"{corpus_name}: {written} result-v1 document(s) in {out_dir}")
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
