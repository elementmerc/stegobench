#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Build every arm of a tier, with bounded parallelism and a resumable plan.

WHY AN ORCHESTRATOR AND NOT A SHELL LOOP
----------------------------------------
The Core tier is 10,000 covers across roughly 35 arms. Measured on this
hardware: 1.96 seconds per adaptive pair, single core and CPU bound. One arm is
therefore about five and a half hours and the whole adaptive set is six days in
one process. It has to run wide, and running wide by hand is how this project
put a sixteen core box at a load average of 131 last week.

So concurrency is declared once, here, with a memory budget attached, and the
jobs are independent processes that each resume from their own manifest.

THE THREE THINGS THIS GETS RIGHT THAT A LOOP WOULD NOT
-------------------------------------------------------
**Resumable at job granularity.** Both builders skip a pair whose stego file
already exists, so a job killed at hour twenty restarts where it stopped. The
orchestrator additionally records which jobs have finished, so a rerun does not
re-enter a completed arm just to discover it has nothing to do.

**Bounded, and the bound costs something.** `--jobs` is not a free dial. Each
adaptive worker is one busy core and roughly half a gigabyte, so the default
leaves headroom rather than claiming the machine. The failure this avoids has a
name in this repo: eight extractor containers with no ceiling took out dbus,
pipewire, syncthing and a peer's language model for an afternoon.

**Ordered so a partial result is still a release.** Jobs run cheapest first and
in tier order, so an interrupted build leaves complete arms rather than thirty
half-built ones. A half-built arm is not publishable and is not obviously
distinguishable from a complete one without counting.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys
import time
from concurrent import futures

HERE = pathlib.Path(__file__).resolve().parent

#: Measured on a 16 core build box, 2026-09-18, single process and CPU bound.
SECONDS_PER_ADAPTIVE_PAIR = 1.96
#: The JPEG tool arms shell out to containers and are I/O bound, so they cost
#: far less CPU per pair and parallelise further than the adaptive ones.
SECONDS_PER_JPEG_COVER = 3.0

SPATIAL_SCHEMES = ("hugo", "wow", "suniward", "hill", "mipod")
JPEG_SCHEMES = ("juniward", "uerd")
RATES = (0.05, 0.1, 0.2, 0.4)


def plan_jobs(count: int, out: pathlib.Path, covers: pathlib.Path,
              manifest: pathlib.Path, jpeg_covers: pathlib.Path | None) -> list[dict]:
    """Every arm as an independent, resumable job.

    One job per (scheme, rate) rather than one per scheme: a scheme-level job is
    four times longer, so an interruption costs four times as much, and the
    parallelism is coarser for no benefit.
    """
    jobs: list[dict] = []

    # The JPEG tool arms first: they are the cheapest in CPU and they produce
    # the clean JPEG covers the DCT adaptive arms need as input.
    jobs.append({
        "name": "jpeg-tools",
        "kind": "jpeg",
        "est_seconds": count * SECONDS_PER_JPEG_COVER,
        "cmd": [sys.executable, str(HERE / "build_jpeg_arms.py"),
                "--covers", str(covers), "--manifest", str(manifest),
                "--out", str(out / "jpeg-tools"), "--count", str(count)],
    })

    for scheme in SPATIAL_SCHEMES:
        for rate in RATES:
            jobs.append({
                "name": f"{scheme}-{rate}",
                "kind": "adaptive",
                "est_seconds": count * SECONDS_PER_ADAPTIVE_PAIR,
                "cmd": [sys.executable, str(HERE / "build_adaptive_arms.py"),
                        "--covers", str(covers), "--manifest", str(manifest),
                        "--out", str(out / "adaptive"), "--count", str(count),
                        "--schemes", scheme, "--rates", str(rate)],
            })

    for scheme in JPEG_SCHEMES:
        for rate in RATES:
            cmd = [sys.executable, str(HERE / "build_adaptive_arms.py"),
                   "--covers", str(covers), "--manifest", str(manifest),
                   "--out", str(out / "adaptive"), "--count", str(count),
                   "--schemes", scheme, "--rates", str(rate)]
            if jpeg_covers:
                cmd += ["--jpeg-covers", str(jpeg_covers)]
            jobs.append({
                "name": f"{scheme}-{rate}",
                "kind": "adaptive-jpeg",
                "est_seconds": count * SECONDS_PER_ADAPTIVE_PAIR,
                "cmd": cmd,
                "needs": "jpeg-tools",
            })

    return jobs


def run_job(job: dict, log_dir: pathlib.Path, timeout: int) -> dict:
    log = log_dir / f"{job['name']}.log"
    started = time.monotonic()
    try:
        with log.open("w") as handle:
            proc = subprocess.run(job["cmd"], stdout=handle, stderr=subprocess.STDOUT,
                                  timeout=timeout)
        code = proc.returncode
    except subprocess.TimeoutExpired:
        code = -1
        log.write_text(log.read_text() + f"\nTIMED OUT after {timeout}s\n")
    return {"name": job["name"], "returncode": code,
            "seconds": round(time.monotonic() - started, 1), "log": str(log)}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--covers", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--count", type=int, default=10000)
    ap.add_argument("--manifest", default=None)
    ap.add_argument("--jpeg-covers", default=None,
                    help="clean JPEG covers for the DCT arms. Defaults to the "
                         "ones the jpeg-tools job writes")
    ap.add_argument("--jobs", type=int, default=10,
                    help="concurrent builders. Each adaptive worker is one busy "
                         "core and about half a gigabyte, so this is a real cost "
                         "and not a free dial")
    ap.add_argument("--timeout", type=int, default=172800, help="per job")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    covers = pathlib.Path(args.covers)
    manifest = pathlib.Path(args.manifest) if args.manifest else covers / "manifest.jsonl"
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    log_dir = out / "logs"
    log_dir.mkdir(exist_ok=True)

    jpeg_covers = pathlib.Path(args.jpeg_covers) if args.jpeg_covers else out / "jpeg-tools" / "clean"
    jobs = plan_jobs(args.count, out, covers, manifest, jpeg_covers)

    state_path = out / "build-state.json"
    state = json.loads(state_path.read_text()) if state_path.is_file() else {"done": {}}
    pending = [j for j in jobs if j["name"] not in state["done"]]

    total_cpu = sum(j["est_seconds"] for j in pending)
    print(f"{len(jobs)} arm job(s), {len(pending)} still to run")
    print(f"estimated {total_cpu / 3600:.1f} CPU-hours, "
          f"about {total_cpu / 3600 / args.jobs:.1f} hours wall clock at "
          f"--jobs {args.jobs}")
    if args.dry_run:
        for j in pending:
            print(f"  {j['name']:<20} {j['kind']:<14} ~{j['est_seconds'] / 3600:.1f}h")
        return 0

    # The DCT arms need clean JPEGs, which the jpeg-tools job produces. Running
    # them first is a dependency, not an optimisation, so it is a separate wave
    # rather than a hint to the scheduler.
    waves = [
        [j for j in pending if not j.get("needs")],
        [j for j in pending if j.get("needs")],
    ]

    started = time.monotonic()
    failures = 0
    for wave_no, wave in enumerate(waves, 1):
        if not wave:
            continue
        print(f"\nwave {wave_no}: {len(wave)} job(s), {args.jobs} at a time")
        with futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            submitted = {pool.submit(run_job, j, log_dir, args.timeout): j for j in wave}
            for future in futures.as_completed(submitted):
                result = future.result()
                ok = result["returncode"] == 0
                failures += 0 if ok else 1
                mark = "ok " if ok else "FAIL"
                print(f"  {mark} {result['name']:<20} {result['seconds'] / 60:>7.1f} min"
                      + ("" if ok else f"  exit {result['returncode']}, see {result['log']}"))
                if ok:
                    state["done"][result["name"]] = result
                    state_path.write_text(json.dumps(state, indent=2) + "\n")

    elapsed = time.monotonic() - started
    print(f"\n{len(state['done'])}/{len(jobs)} arms complete, "
          f"{failures} failed, {elapsed / 3600:.1f}h elapsed")
    if failures:
        print("Re-run this command to retry only the failed arms; completed ones "
              "are recorded in build-state.json and are not re-entered.")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
