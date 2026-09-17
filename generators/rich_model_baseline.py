#!/usr/bin/env python3
"""Train a rich-model detector on one arm, so "nobody can see this" is measured.

THE QUESTION THIS ANSWERS
-------------------------
Rounds 3 and 4 measured two detectors against adaptive embedding and found both
at chance. It is tempting to write that up as "adaptive embedding defeats
detection", and that would be wrong, because both detectors come from the same
family. One is a CNN trained on simple hiding and the other is an ensemble of
classical LSB-replacement models. Neither was ever going to see HUGO.

Two tools failing for the same reason is one data point, not two.

The published state of the art for this problem is a rich-model feature set with
an ensemble classifier trained on the specific scheme and payload. That is the
thing to beat, and if it separates these arms then the honest report says the
gap is a training-set gap rather than a limit of the field. If it does not
separate them, then the claim is earned.

This builds that baseline, on our own corpus, and reports it the same way every
other arm is reported.

WHY THE SPLIT IS BY COVER AND NOT BY IMAGE
------------------------------------------
Every stego image here is a modified copy of a specific cover, and the two are
far more alike than any two unrelated photographs. Split them at random and a
cover lands in training while its own stego version lands in the test set. The
classifier then recognises the photograph rather than the payload, and the test
number is inflated by an amount nobody can estimate afterwards.

So the split is on the cover, and a cover takes its stego partner with it,
whichever side it falls.

    covers 0..699  ──> TRAIN   (cover + its stego)
    covers 700..999 ──> TEST   (cover + its stego)

This is the same pairing discipline the corpus itself was built under, applied
one layer up. Getting it wrong here would void the result in exactly the way the
outguess quality confound voided round 3's first attempt.

WHAT IT REPORTS
---------------
AUC, TPR at 1% and 10% false positives, and accuracy, on the held-out half only.
The out-of-bag error from the classifier's own subspace search is reported
beside them, because a large gap between out-of-bag and test error is the signal
that the training set was too small to trust.
"""
from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time

import numpy as np

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from fld_ensemble import FldEnsemble  # noqa: E402
from score_arms import roc_auc, tpr_at_fpr  # noqa: E402

#: Which Aletheia extractor suits which domain. Using a spatial feature set on
#: JPEG-domain stego is not a neutral choice: it decodes to pixels first and
#: measures a weaker version of the signal, which would understate the baseline
#: and therefore overstate our conclusion.
EXTRACTOR_FOR = {"spatial": "srm", "jpeg": "dctr"}


def run(cmd: list[str], timeout: int, what: str) -> subprocess.CompletedProcess:
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as e:
        raise RuntimeError(f"{what} exceeded {timeout}s. Command: {' '.join(cmd)}") from e
    if proc.returncode != 0:
        tail = (proc.stderr or proc.stdout or "")[-2000:]
        raise RuntimeError(f"{what} failed (exit {proc.returncode}).\n{tail}")
    return proc


def stage(rows: list[dict], corpus: pathlib.Path, dest: pathlib.Path, key: str) -> int:
    """Copy one side of the split into a flat directory for the extractor.

    Hard links where possible: the extractor only reads, and a thousand copies
    of a 512x512 PNG is disk we do not need to spend. Falls back to a copy when
    the staging directory is on another filesystem, which it will be if somebody
    points --work at a different mount.
    """
    dest.mkdir(parents=True, exist_ok=True)
    n = 0
    for row in rows:
        src = corpus / row[key]
        if not src.is_file():
            raise FileNotFoundError(f"manifest names {row[key]}, which is not on disk")
        # The name must carry the cover id, because the extractor sorts by name
        # and the label file is how features are matched back to images.
        target = dest / f"{row['_cover_id']}{src.suffix}"
        if target.exists():
            continue
        try:
            os.link(src, target)
        except OSError:
            shutil.copy2(src, target)
        n += 1
    return n


def extract(image: str, extractor: str, images: pathlib.Path,
            out_file: pathlib.Path, timeout: int) -> tuple[np.ndarray, list[str]]:
    """Run the extractor in the container and read back features aligned to names."""
    out_file.parent.mkdir(parents=True, exist_ok=True)
    if not out_file.is_file():
        run([
            "docker", "run", "--rm",
            "--network=none", "--cap-drop=ALL",
            "--security-opt", "no-new-privileges",
            "--user", f"{os.getuid()}:{os.getgid()}",
            "-v", f"{images}:/images:ro",
            "-v", f"{out_file.parent}:/out",
            image, extractor, "/images", f"/out/{out_file.name}",
        ], timeout=timeout, what=f"{extractor} on {images.name}")

    label_file = out_file.with_name(out_file.name + ".label")
    if not label_file.is_file():
        raise RuntimeError(
            f"{extractor} produced {out_file.name} but no .label file beside it, "
            "so features cannot be matched to images. Delete the .fea and re-run."
        )
    names = [l.strip() for l in label_file.read_text().splitlines() if l.strip()]
    features = np.loadtxt(out_file, dtype=np.float64)
    if features.ndim == 1:
        features = features.reshape(1, -1)
    if len(features) != len(names):
        raise RuntimeError(
            f"{len(features)} feature rows against {len(names)} labels in "
            f"{out_file.name}. The two files disagree, so nothing can be trusted."
        )
    return features, names


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--corpus", required=True)
    ap.add_argument("--arm", required=True, help="e.g. suniward/0400")
    ap.add_argument("--work", required=True, help="scratch for staged images and features")
    ap.add_argument("--out", default=None, help="result JSON (default: in --work)")
    ap.add_argument("--image", default="stegobench/aletheia-rich:pinned")
    ap.add_argument("--extractor", default=None,
                    help="srm or dctr; default follows the arm's domain")
    ap.add_argument("--test-frac", type=float, default=0.3)
    ap.add_argument("--learners", type=int, default=200)
    ap.add_argument("--seed", type=int, default=20260917)
    ap.add_argument("--timeout", type=int, default=86400,
                    help="per extraction; rich models are slow and this is a whole job")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    corpus = pathlib.Path(args.corpus)
    manifest = corpus / "manifest.jsonl"
    if not manifest.is_file():
        print(f"no manifest at {manifest}", file=sys.stderr)
        return 2

    rows = [json.loads(l) for l in manifest.read_text().splitlines() if l.strip()]
    rows = [r for r in rows if r["arm"] == args.arm]
    if not rows:
        arms = sorted({json.loads(l)["arm"] for l in manifest.read_text().splitlines() if l.strip()})
        print(f"no arm {args.arm!r}. Available:\n  " + "\n  ".join(arms), file=sys.stderr)
        return 2

    # One cover may legitimately appear once per arm; guard anyway, because a
    # duplicate would put the same image on both sides of the split.
    seen: dict[str, dict] = {}
    for r in rows:
        if r["clean"] in seen:
            print(f"cover {r['clean']} appears twice in {args.arm}", file=sys.stderr)
            return 2
        r["_cover_id"] = pathlib.Path(r["clean"]).stem
        seen[r["clean"]] = r

    domain = rows[0].get("domain", "spatial")
    extractor = args.extractor or EXTRACTOR_FOR.get(domain)
    if extractor not in ("srm", "srmq1", "dctr", "gfr"):
        print(f"unknown extractor {extractor!r} for domain {domain!r}", file=sys.stderr)
        return 2

    rng = np.random.default_rng(args.seed)
    order = rng.permutation(len(rows))
    n_test = max(1, int(round(len(rows) * args.test_frac)))
    test_idx = set(order[:n_test].tolist())
    train = [r for i, r in enumerate(rows) if i not in test_idx]
    test = [r for i, r in enumerate(rows) if i in test_idx]

    print(f"arm {args.arm}: {len(rows)} pairs, domain {domain}, extractor {extractor}")
    print(f"split by cover: {len(train)} train pairs, {len(test)} test pairs")
    if len(train) < 100:
        print("WARNING: fewer than 100 training pairs. A rich-model ensemble needs "
              "thousands; this number will be noisy and should be read as a floor.",
              file=sys.stderr)

    work = pathlib.Path(args.work)
    started = time.monotonic()
    sets: dict[str, np.ndarray] = {}
    for split, split_rows in (("train", train), ("test", test)):
        for side, key in (("clean", "clean"), ("stego", "stego")):
            name = f"{split}_{side}"
            images = work / "images" / name
            staged = stage(split_rows, corpus, images, key)
            print(f"  staged {staged:>5} new into {name}")
            feats, names = extract(args.image, extractor, images,
                                   work / "features" / f"{name}.fea", args.timeout)
            print(f"  {name}: {feats.shape[0]} vectors of {feats.shape[1]} features "
                  f"({time.monotonic() - started:.0f}s elapsed)")
            sets[name] = feats

    x_train = np.vstack([sets["train_clean"], sets["train_stego"]])
    y_train = np.hstack([np.zeros(len(sets["train_clean"])),
                         np.ones(len(sets["train_stego"]))])
    x_test = np.vstack([sets["test_clean"], sets["test_stego"]])
    y_test = np.hstack([np.zeros(len(sets["test_clean"])),
                        np.ones(len(sets["test_stego"]))])

    print(f"training on {len(x_train)} vectors, {x_train.shape[1]} features")
    clf = FldEnsemble(n_estimators=args.learners, seed=args.seed, verbose=True).fit(
        x_train, y_train)
    print(f"  chose d_sub {clf.d_sub_}, out-of-bag error {clf.oob_error_:.4f}")

    scores = clf.decision_function(x_test).tolist()
    labels = [bool(v) for v in y_test]
    result = {
        "arm": args.arm,
        "domain": domain,
        "extractor": extractor,
        "classifier": "FLD ensemble, bagged Fisher discriminants on random subspaces",
        "features": int(x_train.shape[1]),
        "train_pairs": len(train),
        "test_pairs": len(test),
        "d_sub": int(clf.d_sub_),
        "oob_error": round(float(clf.oob_error_), 4),
        "auc": round(roc_auc(scores, labels), 4),
        "tpr_at_1pct_fpr": round(tpr_at_fpr(scores, labels, 0.01), 4),
        "tpr_at_10pct_fpr": round(tpr_at_fpr(scores, labels, 0.10), 4),
        "accuracy": round(clf.score(x_test, y_test), 4),
        "seed": args.seed,
        "elapsed_seconds": round(time.monotonic() - started, 1),
    }

    out = pathlib.Path(args.out) if args.out else work / f"result-{args.arm.replace('/', '-')}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, indent=2) + "\n")

    print()
    for k in ("auc", "tpr_at_1pct_fpr", "tpr_at_10pct_fpr", "accuracy", "oob_error"):
        print(f"  {k:<18} {result[k]}")
    print(f"\nwritten: {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
