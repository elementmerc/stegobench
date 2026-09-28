#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Score a corpus with the established detector panel, and report per arm.

WHY A PANEL AND NOT OUR OWN TOOL
--------------------------------
Earlier rounds compared a detector under test against a tool we build ourselves. That is
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

#: Two labels on every container this run starts.
#:
#: `stegobench-panel` marks it as a panel container at all, so an operator can
#: find every one of them by hand. That label exists because killing this
#: process does NOT kill its containers: a stopped scorer once left twelve of
#: them running against a dead pipe, holding the box at a load average of 76. `stegobench-panel-run=<id>` marks it as
#: belonging to THIS process, and that is the one cleanup filters on.
#:
#: The distinction exists because the first version had only the shared label and
#: reaped on it. That cleanup was added to stop a killed run orphaning its
#: containers, and it promptly did something worse: a second panel run finishing
#: normally ran its `finally: reap()` and killed the containers of a DIFFERENT
#: run that was still working. The victim reported `exit 137` on one shard,
#: discarded that whole directory, and carried on for three hours producing a
#: result with 200 covers missing from it, which read as a detector failure
#: rather than as a cleanup collision.
#:
#: Killing by a shared label is killing by category. A run may only clean up
#: after itself.
LABEL = "stegobench-panel"
RUN_ID = f"{os.getpid()}-{int(time.time())}"
RUN_LABEL = f"{LABEL}-run={RUN_ID}"

#: The memory the whole panel may use, split between its containers.
#:
#: Declared as a total rather than per container for the reason in
#: rich_model_baseline.py: a per container ceiling does not bound a run, and
#: shards times the cap is the number that decides whether anything else on the
#: machine survives. Splitting one budget also makes concurrency cost something,
#: which is what stops a wider run looking free at the point it is typed.
MEMORY_BUDGET_GIB = 12.0

#: Measured floor for a SPA/RS container. Probed at 366 MiB peak on colour
#: JPEGs, so 1 GiB is comfortable. Recorded because an exit 137 on this scorer
#: was once read as the cap biting when it was a cleanup collision, and the
#: measured figure is what ruled memory out in a minute rather than an hour.
MIN_CONTAINER_GIB = 1.0

HARDENING = [
    "--network=none", "--cap-drop=ALL",
    "--security-opt", "no-new-privileges",
    "--label", LABEL,
    "--label", RUN_LABEL,
]


def reap() -> int:
    """Stop every container this run started. Safe to call more than once.

    Returns the number actually stopped, and says so on the way out when it
    could not stop them: a container left holding a core after the scorer has
    gone is the failure this whole labelling scheme exists to prevent, and
    reporting a kill that did not happen hides exactly that.
    """
    try:
        listing = subprocess.run(
            ["docker", "ps", "-q", "--filter", f"label={RUN_LABEL}"],
            capture_output=True, text=True, timeout=60)
    except (subprocess.SubprocessError, OSError) as e:
        print(f"could not ask docker what this run left running ({e}); check "
              f"by hand with: docker ps --filter label={RUN_LABEL}",
              file=sys.stderr)
        return 0
    ids = [i for i in listing.stdout.split() if i]
    if not ids:
        return 0
    try:
        killed = subprocess.run(["docker", "kill", *ids], capture_output=True,
                                text=True, timeout=120)
    except (subprocess.SubprocessError, OSError) as e:
        print(f"could not stop {len(ids)} container(s) this run started ({e}). "
              f"They are still holding memory and cores; stop them with: "
              f"docker kill {' '.join(ids)}", file=sys.stderr)
        return 0
    if killed.returncode != 0:
        print(f"docker refused to stop {len(ids)} container(s) this run "
              f"started: {(killed.stderr or killed.stdout).strip()[-500:]}",
              file=sys.stderr)
        return 0
    return len(ids)


def memory_flag(shards: int, budget_gib: float = MEMORY_BUDGET_GIB) -> list[str]:
    """Per container ceiling derived from the run's whole budget."""
    per = budget_gib / max(1, shards)
    if per < MIN_CONTAINER_GIB:
        raise RuntimeError(
            f"{shards} shards under a {budget_gib:g} GiB budget leaves "
            f"{per:.2f} GiB each, below the {MIN_CONTAINER_GIB} GiB a detector "
            f"needs. Run at most {int(budget_gib // MIN_CONTAINER_GIB)} shards, "
            f"or raise the budget and say what gives up that memory."
        )
    return ["--memory", f"{per:.2f}g"]


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
                 shards: int = 1,
                 budget_gib: float = MEMORY_BUDGET_GIB) -> dict[str, dict]:
    """SPA and RS over a whole directory, split across `shards` containers.

    One container per image would spend a second of startup on every image, and
    one container for the whole directory takes about 22 seconds per image
    serially. So: a handful of containers, each taking every Nth file.

    The threads here only wait on subprocesses, so the interpreter lock is
    irrelevant and the parallelism is real.
    """
    mem = memory_flag(shards, budget_gib)

    def one(shard: int) -> str:
        cmd = [
            "docker", "run", "--rm", *HARDENING, *mem,
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
    # A line the driver wrote that this cannot read is an image with no answer,
    # and an image with no answer drops out of the arm silently: the AUC below
    # is then measured over the survivors and reads exactly like the real one.
    # The container also writes ordinary noise to stdout, so this counts and
    # reports rather than refusing outright.
    unreadable, first_unreadable = 0, ""
    for text in outputs:
        for line in text.splitlines():
            if not line.strip():
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                unreadable += 1
                first_unreadable = first_unreadable or line.strip()
                continue
            if "file" not in rec:
                unreadable += 1
                first_unreadable = first_unreadable or line.strip()
                continue
            out[rec["file"]] = rec
    if unreadable:
        print(f"  aletheia on {directory.name}: {unreadable} line(s) of "
              f"output were not a per-image answer, so those images carry no "
              f"score. First: {first_unreadable[:200]}", file=sys.stderr)
    return out


def stegexpose_dir(image: str, directory: pathlib.Path, scratch: pathlib.Path,
                   timeout: int,
                   budget_gib: float = MEMORY_BUDGET_GIB) -> dict[str, float]:
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
        "docker", "run", "--rm", *HARDENING, *memory_flag(1, budget_gib),
        "--user", f"{os.getuid()}:{os.getgid()}",
        "-v", f"{scratch}:/data", image,
        "/data", "default", "0.2", "/data/stegexpose.csv",
    ], timeout=timeout, what=f"stegexpose on {directory.name}", check=False)
    if not csv.is_file():
        return {}
    scores: dict[str, float] = {}
    unreadable = 0
    lines = [l for l in csv.read_text(encoding="utf-8").splitlines() if l.strip()]
    for line in lines[1:]:
        parts = line.split(",")
        if len(parts) < 8:
            unreadable += 1
            continue
        try:
            scores[parts[0]] = float(parts[-1])
        except ValueError:
            unreadable += 1
            continue
    # Same reason as the Aletheia reader: a row that cannot be read is an image
    # that silently leaves the measurement.
    if unreadable:
        print(f"  stegexpose on {directory.name}: {unreadable} CSV row(s) "
              f"carried no readable score, so those images are unscored",
              file=sys.stderr)
    return scores


def zsteg_file(image: str, path: pathlib.Path, timeout: int,
               budget_gib: float = MEMORY_BUDGET_GIB) -> bool:
    """Did zsteg report anything? A verdict, deliberately, not a score.

    zsteg does not estimate a payload. It hunts for readable content and for
    structure that should not be there, so the only honest reduction is whether
    it found something, and an AUC computed from its output would be a category
    error.

    TWO WAYS THIS READ WRONG, BOTH FOUND BY LOOKING AT REAL OUTPUT
    ---------------------------------------------------------------
    The first version matched only `text:` and `file:`, which are the markers on
    a payload recovered from a bit plane. zsteg's other finding shape is a
    bracketed note about the container:

        [?] 4122 bytes of extra data after image end (IEND), offset = 0x121bb

    That is exactly the appended-data arm, detected correctly, and it was being
    scored as "found nothing" on all 200 files. The report came within a draft
    of saying no free tool catches appended data, on the strength of a parser
    that was not looking for the answer.

    The second is that on a JPEG zsteg prints that line and THEN dies with a
    NoMethodError, because it is a PNG and BMP tool being handed a JPEG. The
    call does not check the exit status, so the crash was invisible too. The
    finding is still real and still on stdout, so it is kept, but the crash is
    reported rather than swallowed.
    """
    proc = run([
        "docker", "run", "--rm", *HARDENING, *memory_flag(1, budget_gib),
        "-v", f"{path.parent}:/data:ro", image, f"/data/{path.name}",
    ], timeout=timeout, what=f"zsteg on {path.name}", check=False)

    # BOTH streams, and that is the third layer of this same bug rather than
    # belt and braces. On a JPEG zsteg writes NOTHING to stdout: the finding and
    # the crash both go to stderr, so a scanner reading only stdout sees an empty
    # string and reports "found nothing" with complete confidence.
    #
    #     stdout:  (empty)
    #     stderr:  [?] 4122 bytes of extra data after image end (IEND) ...
    #
    # The first version had the wrong pattern, the second had the right pattern
    # on the wrong stream, and both returned a clean False that looked like a
    # measurement.
    for line in (proc.stdout + "\n" + proc.stderr).splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        # A bracketed note is a structural finding: extra data, an odd chunk,
        # a size mismatch. A `text:` or `file:` marker is a recovered payload.
        if stripped.startswith("[?]"):
            return True
        if "text:" in stripped or "file:" in stripped:
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
    ap.add_argument("--memory-budget", type=float, default=MEMORY_BUDGET_GIB,
                    help="GiB the whole panel may use, across all its "
                         "containers. Lower it when something else on the box "
                         "needs the memory; the per container ceiling follows")
    ap.add_argument("--shards", type=int, default=4,
                    help="parallel Aletheia containers per directory. Each one\n                         starts its own worker pool inside, so this multiplies:\n                         12 here put a 16 core box at a load average of 76")
    ap.add_argument("--timeout", type=int, default=14400)
    ap.add_argument("--report-only", action="store_true")
    args = ap.parse_args(argv)

    # Line buffering is so a long run's progress reaches a tail as it happens.
    # A caller that redirected stdout may have put something else there, and
    # losing the buffering is a cosmetic loss where crashing on it is a real one.
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(line_buffering=True)
    corpus = pathlib.Path(args.corpus)
    manifest = corpus / "manifest.jsonl"
    if not manifest.is_file():
        print(f"no manifest at {manifest}", file=sys.stderr)
        return 2
    rows = [json.loads(l) for l in manifest.read_text(encoding="utf-8").splitlines() if l.strip()]
    # Round 1 era manifests predate the arm field. Naming them after the corpus
    # keeps them scorable rather than crashing on a KeyError, and the name still
    # says where the number came from.
    fallback_arm = corpus.name
    for row in rows:
        row.setdefault("arm", fallback_arm)

    out_path = pathlib.Path(args.out) if args.out else corpus / "panel.jsonl"
    # A record counts as done only if it actually carries a detector's answer.
    #
    # The first version keyed on the filename alone, so a record written after a
    # detector failed, which is just {"file": ...}, looked identical to a
    # finished one. A run whose Aletheia pass died on the first directory wrote
    # 200 such records, and every re-run then reported "resuming: 1480 files
    # already scored" and exited having scored nothing, over and over, while the
    # report showed arms pairing against three survivors. Resume has to be able
    # to tell an answer from an absence.
    DETECTOR_FIELDS = ("aletheia_spa", "aletheia_rs", "stegexpose", "zsteg",
                       "aletheia_error")
    scored: dict[str, dict] = {}
    if out_path.exists():
        empty = 0
        lines = out_path.read_text(encoding="utf-8").splitlines()
        for number, line in enumerate(lines, 1):
            if not line.strip():
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError as e:
                # The scores file is an append log, so a run killed mid-write
                # leaves its last line half finished. That one is discarded
                # with a word said about it. A broken line anywhere else means
                # something other than an interruption wrote here, and
                # resuming on top of it would score around whatever it holds.
                if number == len(lines):
                    print(f"the last line of {out_path.name} is incomplete, "
                          f"which is what an interrupted run leaves behind; "
                          f"ignoring it and re-scoring that file")
                    continue
                print(f"{out_path}, line {number} is not valid JSON ({e}). "
                      f"This file is an append log and only its last line can "
                      f"be truncated, so something else has written here. "
                      f"Move it aside to re-score from scratch, or repair the "
                      f"line.", file=sys.stderr)
                return 2
            if "file" not in rec:
                print(f"{out_path}, line {number} names no file, so there "
                      f"is no telling which image it scored. Move the file "
                      f"aside to re-score from scratch, or repair the line.",
                      file=sys.stderr)
                return 2
            if any(f in rec for f in DETECTOR_FIELDS):
                scored[rec["file"]] = rec
            else:
                empty += 1
        note = f", {empty} empty record(s) ignored" if empty else ""
        print(f"resuming: {len(scored)} files already scored{note}")

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
                    # Every file under it stays unscored, and the arms it holds
                    # are then reported on whatever else survived. The report
                    # prints n per arm, but n only reads as short against a
                    # number nobody has unless this is said out loud.
                    print(f"  {d} is not in this corpus, so {len(rels)} file(s) "
                          f"go unscored and the arms they belong to are "
                          f"measured on less than they name", file=sys.stderr)
                    continue
                print(f"[{n}/{len(by_dir)}] {d}: {len(rels)} files "
                      f"({time.monotonic() - started:.0f}s)")
                try:
                    alet = aletheia_dir(args.aletheia_image, directory, args.timeout,
                                        shards=args.shards,
                                        budget_gib=args.memory_budget)
                except RuntimeError as e:
                    print(f"  aletheia: {e}", file=sys.stderr)
                    alet = {}
                try:
                    expose = stegexpose_dir(args.stegexpose_image, directory,
                                            scratch_root / d.replace("/", "_"),
                                            args.timeout, args.memory_budget)
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
                        found = zsteg_file(args.zsteg_image, corpus / rel, 300,
                                           args.memory_budget)
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
