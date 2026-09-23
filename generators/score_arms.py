#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Score a built corpus with the detector under test and a reference detector, then report per arm.

WHAT IT REPORTS AND WHY NOT ACCURACY
------------------------------------
Accuracy on a paired corpus is a trap: an arm is half clean and half stego, so a
detector that answers "clean" to everything scores 50% and looks like a coin
rather than like the useless thing it is. Worse, a detector with a badly placed
threshold can score well on accuracy while separating the classes perfectly,
which is the opposite mistake and the one round 2 found in Stegcore.

So three numbers per arm, and they answer different questions:

**ROC AUC** asks whether the score *ranks* stego above clean, ignoring where the
threshold sits. It is the question "is the information there at all?" A value of
0.5 means the scores carry nothing; 1.0 means perfect separation even if the
verdict never fires. Computed tie-aware, because a detector that returns the
same float for many images, as some detectors do on appended data, would
otherwise be flattered by an arbitrary tie-break.

**TPR at a fixed FPR** asks what the detector is worth in a queue where false
positives cost real work. Reported at 1% and 10%.

**Verdict rate** asks what the shipped decision boundary actually does, which is
what a buyer experiences. The gap between this and AUC is the difference
between what a model knows and what a product says.

THE STRUCTURAL ARM IS READ DIFFERENTLY
--------------------------------------
For appended-data pairs the interesting statistic is not AUC at all, it is how
many pairs returned the *identical* score. Round 2 found 120 of 120 on PNG,
which says the model decodes the image and never looks at the container. That
count is reported separately because an AUC of 0.5 there could mean "no signal"
whereas byte-identical scores mean "did not look".
"""
from __future__ import annotations

import argparse
import json
import pathlib
import shutil
import subprocess
import sys
import time
import urllib.request

#: No default endpoint. A benchmark that ships one lab's network address as a
#: default produces results whose provenance nobody can check, and silently
#: scores against whatever happens to answer on that address. The caller says
#: which detector is under test.
DEFAULT_ENDPOINT = None


def post_image(endpoint: str, path: pathlib.Path, timeout: int = 120) -> dict:
    """One multipart upload, built by hand to avoid a dependency."""
    boundary = "----pentimento-boundary-7f3a"
    body = b"".join([
        f"--{boundary}\r\n".encode(),
        f'Content-Disposition: form-data; name="image"; filename="{path.name}"\r\n'.encode(),
        b"Content-Type: application/octet-stream\r\n\r\n",
        path.read_bytes(),
        f"\r\n--{boundary}--\r\n".encode(),
    ])
    req = urllib.request.Request(
        endpoint, data=body,
        headers={"Content-Type": f"multipart/form-data; boundary={boundary}"},
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def stegcore_score(binary: str, path: pathlib.Path) -> dict | None:
    """Stegcore's own verdict and score on the same file, for comparison."""
    result = subprocess.run([binary, "analyse", str(path), "--json"],
                            capture_output=True, check=False, timeout=180)
    if result.returncode not in (0, 1) or not result.stdout:
        return None
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError:
        return None
    # Stegcore wraps its results: {"ok": true, "data": [ {...} ]}. Reading the
    # envelope as the record silently yields None for every field, which then
    # reads as "the detector had no opinion" rather than as a parsing mistake.
    data = payload.get("data", payload)
    if isinstance(data, list):
        return data[0] if data else None
    return data if isinstance(data, dict) else None


def roc_auc(scores: list[float], labels: list[bool]) -> float | None:
    """Tie-aware AUC by the rank-sum identity.

    Ties get the average of the ranks they span, which is what makes a detector
    returning one constant score land at exactly 0.5 rather than at whatever the
    sort order happened to produce.
    """
    positives = sum(labels)
    negatives = len(labels) - positives
    if not positives or not negatives:
        return None
    order = sorted(range(len(scores)), key=lambda i: scores[i])
    ranks = [0.0] * len(scores)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and scores[order[j + 1]] == scores[order[i]]:
            j += 1
        shared = (i + j) / 2 + 1
        for k in range(i, j + 1):
            ranks[order[k]] = shared
        i = j + 1
    rank_sum = sum(r for r, l in zip(ranks, labels) if l)
    return (rank_sum - positives * (positives + 1) / 2) / (positives * negatives)


def tpr_at_fpr(scores: list[float], labels: list[bool], max_fpr: float) -> float | None:
    """The best true-positive rate reachable without exceeding `max_fpr`."""
    positives = sum(labels)
    negatives = len(labels) - positives
    if not positives or not negatives:
        return None
    best = 0.0
    for threshold in sorted(set(scores)):
        tp = sum(1 for s, l in zip(scores, labels) if l and s >= threshold)
        fp = sum(1 for s, l in zip(scores, labels) if not l and s >= threshold)
        if fp / negatives <= max_fpr:
            best = max(best, tp / positives)
    return best


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--corpus", required=True)
    ap.add_argument("--out", default=None, help="scores.jsonl (default: in corpus)")
    ap.add_argument("--endpoint", default=DEFAULT_ENDPOINT)
    ap.add_argument("--stegcore", default=None,
                    help="path to the stegcore binary; omit to skip that column")
    ap.add_argument("--limit", type=int, default=0, help="0 means every pair")
    ap.add_argument("--shards", type=int, default=1,
                    help="split the work across this many parallel scorers")
    ap.add_argument("--shard", type=int, default=0, help="which shard this is")
    ap.add_argument("--report-only", action="store_true",
                    help="skip scoring and just read what is already recorded")
    args = ap.parse_args(argv)
    if not args.endpoint:
        print("--endpoint is required: give the HTTP address of the detector "
              "under test. There is deliberately no default, because a default "
              "would score against whatever answered on it.", file=sys.stderr)
        return 2

    sys.stdout.reconfigure(line_buffering=True)
    corpus = pathlib.Path(args.corpus)
    manifest = corpus / "manifest.jsonl"
    if not manifest.is_file():
        print(f"no manifest at {manifest}", file=sys.stderr)
        return 2
    rows = [json.loads(l) for l in manifest.read_text(encoding="utf-8").splitlines() if l.strip()]
    if args.limit:
        rows = rows[: args.limit]

    out_path = pathlib.Path(args.out) if args.out else corpus / "scores.jsonl"
    scored: dict[str, dict] = {}
    if out_path.exists():
        for line in out_path.read_text(encoding="utf-8").splitlines():
            if line.strip():
                record = json.loads(line)
                scored[record["file"]] = record
        print(f"resuming: {len(scored)} files already scored")

    # Each clean cover appears in many rows; score it once.
    wanted: list[str] = []
    seen = set()
    for row in rows:
        for key in ("clean", "stego"):
            rel = row[key]
            if rel not in seen:
                seen.add(rel)
                wanted.append(rel)

    binary = args.stegcore
    if binary and not (shutil.which(binary) or pathlib.Path(binary).is_file()):
        print(f"no stegcore at {binary}; skipping that column", file=sys.stderr)
        binary = None

    todo = [r for r in wanted if r not in scored]
    if args.shards > 1:
        # Partitioned by position so the shards never touch the same file. They
        # all append to one scores file, which is safe for the same reason the
        # manifest is: every line is a few hundred bytes and an O_APPEND write
        # below PIPE_BUF is atomic on Linux.
        todo = [r for i, r in enumerate(todo) if i % args.shards == args.shard]
        print(f"shard {args.shard} of {args.shards}")
    if args.report_only:
        todo = []
    print(f"{len(wanted)} unique files, {len(todo)} for this scorer")
    last_beat = time.monotonic()
    failures = 0

    with out_path.open("a") as f:
        for n, rel in enumerate(todo, 1):
            path = corpus / rel
            if not path.is_file():
                continue
            record: dict = {"file": rel}
            try:
                response = post_image(args.endpoint, path)
                record["stegashield"] = {
                    "probability": response.get("stego_probability"),
                    "label": response.get("label"),
                }
            except Exception as e:  # noqa: BLE001 - one file, not the run
                failures += 1
                record["stegashield_error"] = f"{type(e).__name__}: {e}"
            if binary:
                verdict = stegcore_score(binary, path)
                if verdict:
                    record["stegcore"] = {
                        "verdict": verdict.get("verdict"),
                        "score": verdict.get("overall_score"),
                    }
            f.write(json.dumps(record) + "\n")
            f.flush()
            scored[rel] = record
            if time.monotonic() - last_beat >= 60:
                print(f"  ... {n}/{len(todo)} scored, {failures} failed")
                last_beat = time.monotonic()

    report(rows, scored, bool(binary))
    return 0


def report(rows: list[dict], scored: dict[str, dict], with_stegcore: bool) -> None:
    arms: dict[str, list[dict]] = {}
    for row in rows:
        arms.setdefault(row["arm"], []).append(row)

    print(f"\n{'arm':<22} {'n':>4}  {'AUC':>6} {'TPR@1%':>7} {'TPR@10%':>8} "
          f"{'flagged':>8}  identical")
    print("-" * 78)
    for arm in sorted(arms):
        pairs = arms[arm]
        scores: list[float] = []
        labels: list[bool] = []
        identical = 0
        counted = 0
        clean_seen = set()
        for row in pairs:
            c = scored.get(row["clean"], {}).get("stegashield", {})
            s = scored.get(row["stego"], {}).get("stegashield", {})
            if c.get("probability") is None or s.get("probability") is None:
                continue
            counted += 1
            if c["probability"] == s["probability"]:
                identical += 1
            scores.append(s["probability"])
            labels.append(True)
            if row["clean"] not in clean_seen:
                clean_seen.add(row["clean"])
                scores.append(c["probability"])
                labels.append(False)
        if not counted:
            continue
        auc = roc_auc(scores, labels)
        t1 = tpr_at_fpr(scores, labels, 0.01)
        t10 = tpr_at_fpr(scores, labels, 0.10)
        flagged = sum(
            1 for row in pairs
            if scored.get(row["stego"], {}).get("stegashield", {}).get("label")
            not in (None, "clean")
        )
        print(f"{arm:<22} {counted:>4}  "
              f"{auc if auc is None else round(auc, 3):>6} "
              f"{'-' if t1 is None else round(t1, 3):>7} "
              f"{'-' if t10 is None else round(t10, 3):>8} "
              f"{flagged / counted:>7.0%}  {identical}/{counted}")

    if with_stegcore:
        print("\nStegcore on the same images")
        print(f"{'arm':<22} {'n':>4}  {'AUC':>6} {'flagged':>8}")
        print("-" * 46)
        for arm in sorted(arms):
            pairs = arms[arm]
            scores, labels, clean_seen, counted, flagged = [], [], set(), 0, 0
            for row in pairs:
                c = scored.get(row["clean"], {}).get("stegcore", {})
                s = scored.get(row["stego"], {}).get("stegcore", {})
                if c.get("score") is None or s.get("score") is None:
                    continue
                counted += 1
                if s.get("verdict") not in (None, "clean", "Clean"):
                    flagged += 1
                scores.append(s["score"])
                labels.append(True)
                if row["clean"] not in clean_seen:
                    clean_seen.add(row["clean"])
                    scores.append(c["score"])
                    labels.append(False)
            if not counted:
                continue
            auc = roc_auc(scores, labels)
            print(f"{arm:<22} {counted:>4}  "
                  f"{auc if auc is None else round(auc, 3):>6} "
                  f"{flagged / counted:>7.0%}")


if __name__ == "__main__":
    raise SystemExit(main())
