#!/usr/bin/env python3
"""Score a corpus with the established detector panel, and report per arm.

WHY A PANEL AND NOT OUR OWN TOOL
--------------------------------
Earlier rounds compared StegaShield against Stegcore, which we build. That is
marking our own homework: a reader does not have to believe we cheated to
discount the comparison, they only have to notice the incentive.

So the yardstick is other people's work:

    Aletheia     SPA and RS, structural estimators of LSB replacement. The
                 reference implementation in this field, and what Stegcore's
                 own detectors are held to parity against.
    StegExpose   the fusion of four classical statistics. Widely reached for,
                 and the tool a practitioner is most likely to have tried.
    zsteg        a structural scanner. It does not estimate a payload; it looks
                 for readable content in bit planes and in the file, which is a
                 different question and the right one for the container arm.

None of these is the state of the art against modern embedding, and the report
should not pretend otherwise. They are the honest floor: what a competent person
with free tools would achieve today. The state-of-the-art comparison is a
separate measurement, in `rich_model_baseline.py`, because it has to be trained
per scheme and cannot be run as a panel.

WHAT EACH DETECTOR CONTRIBUTES, AND WHERE IT IS SILENT
------------------------------------------------------
SPA and RS model LSB *replacement* in the *spatial* domain. They are strong
there and blind by construction elsewhere, and saying so up front is what stops
a null result being read as a finding. When they sit at chance against
JPEG-domain or adaptive embedding, that is the tool being used outside its
stated range, not evidence about the difficulty of the problem.

zsteg is the one that matters for the container arm, and it is scored on whether
it reported anything at all, not on a threshold.
"""
from __future__ import annotations

import argparse
import json
import os
from concurrent import futures
import pathlib
import signal
import subprocess
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from score_arms import roc_auc, tpr_at_fpr  # noqa: E402

HERE = pathlib.Path(__file__).resolve().parent
IMAGE_EXT = {".png", ".jpg", ".jpeg", ".bmp"}

#: Every container this script starts carries this label, so they can all be
#: found and stopped as a group.
#:
#: This exists because killing this process does NOT kill them. Measured: the
#: scorer was stopped mid-run and its twelve containers carried on for as long
#: as they were left alone, holding the machine at a load average of 76 while
#: writing to a pipe whose reader had gone. Orphaned work that still costs the
#: box is worse than work that fails, because nothing reports it.
LABEL = "stegobench-panel"

HARDENING = [
    "--network=none", "--cap-drop=ALL",
    "--security-opt", "no-new-privileges",
    "--label", LABEL,
]


def reap() -> int:
    """Stop every container this run started. Safe to call more than once."""
    try:
        listing = subprocess.run(
            ["docker", "ps", "-q", "--filter", f"label={LABEL}"],
            capture_output=True, text=True, timeout=60)
    except (subprocess.SubprocessError, OSError):
        return 0
    ids = [i for i in listing.stdout.split() if i]
    if not ids:
        return 0
    try:
        subprocess.run(["docker", "kill", *ids], capture_output=True,
                       text=True, timeout=120)
    except (subprocess.SubprocessError, OSError):
        pass
    return len(ids)


def run(cmd: list[str], timeout: int, what: str, check: bool = True):
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        raise RuntimeError(f"{what} exceeded {timeout}s") from None
    if check and proc.returncode != 0:
        raise RuntimeError(f"{what} failed (exit {proc.returncode}): "
                           f"{(proc.stderr or proc.stdout)[-1500:]}")
    return proc


def aletheia_dir(image: str, directory: pathlib.Path, timeout: int,
                 shards: int = 1) -> dict[str, dict]:
    """SPA and RS over a whole directory, split across `shards` containers.

    One container per image would spend a second of startup on every image, and
    one container for the whole directory takes about 22 seconds per image
    serially. So: a handful of containers, each taking every Nth file.

    The threads here only wait on subprocesses, so the interpreter lock is
    irrelevant and the parallelism is real.
    """
    def one(shard: int) -> str:
        cmd = [
            "docker", "run", "--rm", *HARDENING,
            "--user", f"{os.getuid()}:{os.getgid()}",
            "-v", f"{directory}:/images:ro",
            "-v", f"{HERE}:/driver:ro",
            "--entrypoint", "python3", image,
            "/driver/aletheia_scores.py", "/images",
        ]
        if shards > 1:
            cmd += ["--shards", str(shards), "--shard", str(shard)]
        return run(cmd, timeout=timeout,
                   what=f"aletheia shard {shard} on {directory.name}").stdout

    outputs: list[str] = []
    if shards <= 1:
        outputs.append(one(0))
    else:
        with futures.ThreadPoolExecutor(max_workers=shards) as pool:
            for text in pool.map(one, range(shards)):
                outputs.append(text)

    out: dict[str, dict] = {}
    for text in outputs:
        for line in text.splitlines():
            if not line.strip():
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            out[rec["file"]] = rec
    return out


def stegexpose_dir(image: str, directory: pathlib.Path, scratch: pathlib.Path,
                   timeout: int) -> dict[str, float]:
    """The fusion score per file.

    StegExpose writes its CSV into the directory it was given, so the images are
    copied to scratch rather than the corpus being mounted writable. A detector
    that can write into the corpus it is judging is a corpus that cannot be
    trusted afterwards.
    """
    scratch.mkdir(parents=True, exist_ok=True)
    for src in sorted(directory.iterdir()):
        if src.suffix.lower() in IMAGE_EXT:
            dst = scratch / src.name
            if not dst.exists():
                try:
                    os.link(src, dst)
                except OSError:
                    dst.write_bytes(src.read_bytes())
    csv = scratch / "stegexpose.csv"
    if csv.exists():
        csv.unlink()
    run([
        "docker", "run", "--rm", *HARDENING,
        "--user", f"{os.getuid()}:{os.getgid()}",
        "-v", f"{scratch}:/data", image,
        "/data", "default", "0.2", "/data/stegexpose.csv",
    ], timeout=timeout, what=f"stegexpose on {directory.name}", check=False)
    if not csv.is_file():
        return {}
    scores: dict[str, float] = {}
    lines = [l for l in csv.read_text().splitlines() if l.strip()]
    for line in lines[1:]:
        parts = line.split(",")
        if len(parts) < 8:
            continue
        try:
            scores[parts[0]] = float(parts[-1])
        except ValueError:
            continue
    return scores


def zsteg_file(image: str, path: pathlib.Path, timeout: int) -> bool:
    """Did zsteg report anything? A verdict, deliberately, not a score.

    zsteg does not estimate a payload. It hunts for readable content, so the
    only honest reduction is whether it found something, and an AUC computed
    from a boolean would be a category error.
    """
    proc = run([
        "docker", "run", "--rm", *HARDENING,
        "-v", f"{path.parent}:/data:ro", image, f"/data/{path.name}",
    ], timeout=timeout, what=f"zsteg on {path.name}", check=False)
    for line in proc.stdout.splitlines():
        stripped = line.strip()
        # Progress output is a run of "b1,r,lsb,xy .." with no finding attached;
        # a real hit carries a payload description after the channel spec.
        if stripped and ".. " in line and "text:" in line or "file:" in line:
            return True
    return False


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--corpus", required=True)
    ap.add_argument("--out", default=None, help="panel.jsonl (default: in corpus)")
    ap.add_argument("--scratch", default="/tmp/panel-scratch")
    ap.add_argument("--aletheia-image", default="stegobench/aletheia:pinned")
    ap.add_argument("--stegexpose-image", default="stegobench/stegexpose:pinned")
    ap.add_argument("--zsteg-image", default="stegobench/zsteg:pinned")
    ap.add_argument("--zsteg-arms", default="",
                    help="comma separated arms to run zsteg on; it is slow and "
                         "only meaningful on structural arms")
    ap.add_argument("--shards", type=int, default=4,
                    help="parallel Aletheia containers per directory. Each one\n                         starts its own worker pool inside, so this multiplies:\n                         12 here put a 16 core box at a load average of 76")
    ap.add_argument("--timeout", type=int, default=14400)
    ap.add_argument("--report-only", action="store_true")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    corpus = pathlib.Path(args.corpus)
    manifest = corpus / "manifest.jsonl"
    if not manifest.is_file():
        print(f"no manifest at {manifest}", file=sys.stderr)
        return 2
    rows = [json.loads(l) for l in manifest.read_text().splitlines() if l.strip()]

    out_path = pathlib.Path(args.out) if args.out else corpus / "panel.jsonl"
    scored: dict[str, dict] = {}
    if out_path.exists():
        for line in out_path.read_text().splitlines():
            if line.strip():
                rec = json.loads(line)
                scored[rec["file"]] = rec
        print(f"resuming: {len(scored)} files already scored")

    wanted: list[str] = []
    seen = set()
    for row in rows:
        for key in ("clean", "stego"):
            if row[key] not in seen:
                seen.add(row[key])
                wanted.append(row[key])

    if not args.report_only:
        # Group by directory so each detector starts once per directory rather
        # than once per image.
        by_dir: dict[str, list[str]] = {}
        for rel in wanted:
            if rel not in scored:
                by_dir.setdefault(str(pathlib.PurePath(rel).parent), []).append(rel)

        scratch_root = pathlib.Path(args.scratch)
        started = time.monotonic()
        with out_path.open("a") as f:
            for n, (d, rels) in enumerate(sorted(by_dir.items()), 1):
                directory = corpus / d
                if not directory.is_dir():
                    continue
                print(f"[{n}/{len(by_dir)}] {d}: {len(rels)} files "
                      f"({time.monotonic() - started:.0f}s)")
                try:
                    alet = aletheia_dir(args.aletheia_image, directory, args.timeout,
                                        shards=args.shards)
                except RuntimeError as e:
                    print(f"  aletheia: {e}", file=sys.stderr)
                    alet = {}
                try:
                    expose = stegexpose_dir(args.stegexpose_image, directory,
                                            scratch_root / d.replace("/", "_"),
                                            args.timeout)
                except RuntimeError as e:
                    print(f"  stegexpose: {e}", file=sys.stderr)
                    expose = {}
                for rel in rels:
                    name = pathlib.PurePath(rel).name
                    rec: dict = {"file": rel}
                    a = alet.get(name)
                    if a and "error" not in a:
                        rec["aletheia_spa"] = a.get("spa")
                        rec["aletheia_rs"] = a.get("rs")
                    elif a:
                        rec["aletheia_error"] = a["error"]
                    if name in expose:
                        rec["stegexpose"] = expose[name]
                    f.write(json.dumps(rec) + "\n")
                    scored[rel] = rec
                f.flush()

        zsteg_arms = [a for a in args.zsteg_arms.split(",") if a.strip()]
        if zsteg_arms:
            print(f"\nzsteg on {len(zsteg_arms)} arm(s)")
            with out_path.open("a") as f:
                for row in rows:
                    if row["arm"] not in zsteg_arms:
                        continue
                    for key in ("clean", "stego"):
                        rel = row[key]
                        if scored.get(rel, {}).get("zsteg") is not None:
                            continue
                        found = zsteg_file(args.zsteg_image, corpus / rel, 300)
                        rec = dict(scored.get(rel, {"file": rel}))
                        rec["zsteg"] = found
                        f.write(json.dumps(rec) + "\n")
                        scored[rel] = rec
                    f.flush()

    report(rows, scored)
    return 0


def report(rows: list[dict], scored: dict[str, dict]) -> None:
    arms: dict[str, list[dict]] = {}
    for row in rows:
        arms.setdefault(row["arm"], []).append(row)

    for label, field in (("Aletheia SPA", "aletheia_spa"),
                         ("Aletheia RS", "aletheia_rs"),
                         ("StegExpose", "stegexpose")):
        print(f"\n{label}")
        print(f"{'arm':<24} {'n':>5}  {'AUC':>6} {'TPR@1%':>7} {'TPR@10%':>8}")
        print("-" * 56)
        for arm in sorted(arms):
            scores, labels, clean_seen, n = [], [], set(), 0
            for row in arms[arm]:
                c = scored.get(row["clean"], {}).get(field)
                s = scored.get(row["stego"], {}).get(field)
                if c is None or s is None:
                    continue
                n += 1
                scores.append(s)
                labels.append(True)
                if row["clean"] not in clean_seen:
                    clean_seen.add(row["clean"])
                    scores.append(c)
                    labels.append(False)
            if not n:
                continue
            auc = roc_auc(scores, labels)
            t1 = tpr_at_fpr(scores, labels, 0.01)
            t10 = tpr_at_fpr(scores, labels, 0.10)
            print(f"{arm:<24} {n:>5}  "
                  f"{'-' if auc is None else round(auc, 3):>6} "
                  f"{'-' if t1 is None else round(t1, 3):>7} "
                  f"{'-' if t10 is None else round(t10, 3):>8}")

    flagged = {a: (0, 0, 0) for a in arms}
    any_zsteg = False
    for arm, pairs in arms.items():
        hit_s = sum(1 for r in pairs if scored.get(r["stego"], {}).get("zsteg"))
        hit_c = sum(1 for r in pairs if scored.get(r["clean"], {}).get("zsteg"))
        n = sum(1 for r in pairs if "zsteg" in scored.get(r["stego"], {}))
        flagged[arm] = (hit_s, hit_c, n)
        any_zsteg = any_zsteg or n > 0
    if any_zsteg:
        print("\nzsteg, reported something at all")
        print(f"{'arm':<24} {'n':>5}  {'on stego':>9} {'on clean':>9}")
        print("-" * 52)
        for arm in sorted(arms):
            hit_s, hit_c, n = flagged[arm]
            if not n:
                continue
            print(f"{arm:<24} {n:>5}  {hit_s / n:>8.0%} {hit_c / n:>9.0%}")


def _on_signal(signum, _frame):
    """Ctrl+C and SIGTERM must take the containers with them, not leave them."""
    n = reap()
    print(f"\nsignal {signum}: stopped {n} container(s)", file=sys.stderr)
    raise SystemExit(130)


if __name__ == "__main__":
    signal.signal(signal.SIGINT, _on_signal)
    signal.signal(signal.SIGTERM, _on_signal)
    try:
        raise SystemExit(main())
    finally:
        reap()
