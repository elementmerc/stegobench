#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Push a packed tier to the archives, slowly, resumably, and not by accident.

DRY RUN IS THE DEFAULT AND STAYS THE DEFAULT
---------------------------------------------
Uploading 45 GB to four public archives is not reversible in any useful sense.
An Internet Archive item can be darkened but its identifier is taken; a
HuggingFace repository can be deleted but not un-mirrored; a torrent, once
seeded, is out. So this refuses to send a byte unless `--live` is passed, and
`--live` is not implied by anything else. Every other flag is safe.

A dry run does everything except the transfer: it reads the index, re-hashes
every file and checks it against the digest that was recorded when the release
was packed, resolves the credentials, works out what a resume would still have
to send, and prints the plan with its size and its estimated duration. If the
dry run is not clean the live run would not have been either.

WHY 2 MB/s, AND WHY THE BUDGET IS SHARED
-----------------------------------------
This is a domestic connection that other people are using. An upload that
saturates it for a day is a decision about somebody else's video call.

**The 2 MB/s is the whole line, not one upload's share of it.** Three
destinations at 2 MB/s each is 6 MB/s, which is the opposite of the intent. So
the budget lives in one small file and every uploader reserves from it under a
lock, whichever machine-local process it belongs to:

    uploader A ──┐
    uploader B ──┼──► [ one reservation file, flock ] ──► 2 MB/s total
    uploader C ──┘

Each reads how many bytes it wants to send, takes the next free slot on the
line, and sleeps until that slot arrives. Uploads may therefore run
concurrently without exceeding the budget, and adding a fourth destination
slows the others rather than stacking on top of them.

The throttle is at the file read rather than at the network layer, because
shaping there needs NET_ADMIN and this runs in a container with no capabilities
at all.

WHY A CONTAINER, AND WHAT IT IS ALLOWED TO TOUCH
-------------------------------------------------
`upload-in-container.sh` runs this with `--cap-drop=ALL`, a read-only root
filesystem, the release directory mounted read only, and nothing else. The
credentials arrive as environment variables from a file that is sourced at the
point of use and never appears on a command line: a past session put a token in
a curl argument and it ended up baked into thirteen permission rules in an
editor's settings file.

ORDERING, WHICH THE CODE ENFORCES
----------------------------------
The torrent carries the Internet Archive item as its web seed, so the Archive
upload has to land first or the torrent has no seed and is a file nobody can
fetch. `publish_tier.py torrent` already refuses to build one without a seed;
this refuses to run the torrent step before the archive step has a record of
success.

RESUMING
--------
Every completed file is recorded in a state file beside the release, keyed by
destination and by the file's digest. A re-run skips what is already up and
verifies rather than re-sends. An interrupted six hour upload costs minutes.

Usage::

    # the default, and what you should run first
    python3 tools/release/upload_tier.py --packed ~/pentimento/release/core-arms \
        --destination internetarchive --item pentimento-core-v1

    # only after a clean dry run, and only deliberately
    python3 tools/release/upload_tier.py ... --live
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import sys
import time
import urllib.error
import urllib.request

try:
    import fcntl
except ImportError:  # not POSIX
    fcntl = None

#: Bytes per second. A domestic line other people are using.
DEFAULT_RATE = 2 * 1024 * 1024

#: Read granularity. Small enough that the throttle is smooth, large enough
#: that the syscall overhead does not dominate at the target rate.
CHUNK = 256 * 1024

#: Heartbeat interval, so a detached run is visibly alive in its log.
BEAT_SECONDS = 60

DESTINATIONS = ("internetarchive", "huggingface", "kaggle", "torrent")

#: Which environment variables each destination needs. They are read from the
#: environment and never written anywhere, never logged, and never passed as an
#: argument to anything.
CREDENTIALS = {
    "internetarchive": ("IA_ACCESS_KEY", "IA_SECRET_KEY"),
    "huggingface": ("HF_TOKEN",),
    "kaggle": ("KAGGLE_USERNAME", "KAGGLE_KEY"),
    "torrent": (),
}


class UploadError(RuntimeError):
    pass


class SharedBudget:
    """One bandwidth budget for every uploader on this machine.

    The file holds a single number: the wall-clock time at which the line is
    next free. To send `n` bytes a process locks the file, takes the slot from
    `max(now, next_free)`, pushes `next_free` out by `n / rate`, unlocks, and
    sleeps until its slot arrives. Reservations are therefore handed out in
    arrival order and the sum of all senders is the rate, not a multiple of it.

    Wall clock rather than a monotonic clock, because monotonic clocks are not
    comparable between processes. A clock step is survivable: the clamp below
    turns any absurd reservation back into "now" rather than sleeping for a
    week.
    """

    #: No single reservation can legitimately be further ahead than a few
    #: chunks' worth of line time. Anything beyond this is a stale file from a
    #: killed run, or a clock that moved, and is ignored rather than obeyed.
    MAX_WAIT = 300.0

    def __init__(self, path: pathlib.Path | None, rate: int):
        if path is not None and fcntl is None:
            # Refusing beats carrying on. A budget that cannot be shared is a
            # budget that is silently taken once per uploader, which is the
            # exact failure it exists to prevent, and nothing downstream would
            # report it: every upload would look correctly throttled on its own.
            raise UploadError(
                "the shared bandwidth budget needs file locking, which this "
                "platform does not provide. Run the uploader on a POSIX system, "
                "or pass --budget '' to limit this process alone and accept "
                "that several uploaders will then exceed the rate together.")
        self._path = path
        self._rate = rate
        if path is not None:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.touch(exist_ok=True)

    def reserve(self, size: int) -> None:
        """Block until this many bytes may be sent."""
        if self._rate <= 0 or size <= 0:
            return
        cost = size / self._rate
        if self._path is None:
            time.sleep(cost)
            return

        now = time.time()
        with self._path.open("r+") as handle:
            fcntl.flock(handle.fileno(), fcntl.LOCK_EX)
            try:
                raw = handle.read().strip()
                try:
                    next_free = float(raw) if raw else now
                except ValueError:
                    next_free = now
                start = max(now, next_free)
                if start - now > self.MAX_WAIT:
                    start = now
                handle.seek(0)
                handle.truncate()
                handle.write(f"{start + cost:.6f}\n")
                handle.flush()
                os.fsync(handle.fileno())
            finally:
                fcntl.flock(handle.fileno(), fcntl.LOCK_UN)

        delay = start - time.time()
        if delay > 0:
            time.sleep(delay)


class ThrottledReader:
    """A file-like object that will not be read faster than the budget allows.

    Every chunk is reserved from the shared budget before it is handed over, so
    the limit holds across every uploader rather than per uploader.
    """

    def __init__(self, path: pathlib.Path, budget: SharedBudget, on_progress=None):
        self._handle = path.open("rb")
        self._budget = budget
        self._sent = 0
        self._on_progress = on_progress
        self.length = path.stat().st_size

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
        return False

    def read(self, size: int = -1) -> bytes:
        want = CHUNK if size is None or size < 0 else min(size, CHUNK)
        chunk = self._handle.read(want)
        if not chunk:
            return b""
        self._budget.reserve(len(chunk))
        self._sent += len(chunk)
        if self._on_progress:
            self._on_progress(self._sent, self.length)
        return chunk

    def __iter__(self):
        while True:
            chunk = self.read(CHUNK)
            if not chunk:
                return
            yield chunk

    def close(self) -> None:
        self._handle.close()


def digest_of(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def load_index(packed: pathlib.Path) -> dict[str, str]:
    """Every publishable file in the release, mapped to its recorded digest.

    The index is the record of what was packed, and a file whose bytes no
    longer match it must not go out under the same name as the published
    digest. So this is the authority and the directory listing is not.
    """
    candidates = sorted(packed.glob("*index.json"))
    if not candidates:
        raise UploadError(
            f"no index JSON under {packed}. The index is what says which files "
            f"belong to this release and what their digests are; without it an "
            f"upload is a directory listing, which is not the same thing.")

    files: dict[str, str] = {}
    for index_path in candidates:
        index = json.loads(index_path.read_text(encoding="utf-8"))
        files[index_path.name] = digest_of(index_path)
        for arm in index.get("arms", []):
            for shard in arm.get("shards", []):
                files[shard["shard"]] = shard["sha256"]
        for shard in index.get("shards", []):
            files[shard["shard"]] = shard["sha256"]

    # The metadata a reader needs in order to use or cite the corpus travels
    # with it. A shard set with no datasheet is a pile of tar files.
    for extra in ("README.md", "LICENCES.md", "DATASHEET.md", "SPLITS.md",
                  "CITATION.cff", "croissant.json"):
        path = packed / extra
        if path.is_file():
            files[extra] = digest_of(path)
    return files


def verify(packed: pathlib.Path, files: dict[str, str]) -> list[str]:
    """Check the bytes against the index before anything leaves the machine."""
    problems: list[str] = []
    for name, declared in sorted(files.items()):
        path = packed / name
        if not path.is_file():
            problems.append(f"{name}: named in the index, not on disk")
            continue
        if digest_of(path) != declared:
            problems.append(f"{name}: does not match its recorded digest")
    return problems


def load_state(path: pathlib.Path) -> dict:
    if path.is_file():
        return json.loads(path.read_text(encoding="utf-8"))
    return {"destinations": {}}


def save_state(path: pathlib.Path, state: dict) -> None:
    part = path.with_suffix(".json.part")
    part.write_text(json.dumps(state, indent=2, sort_keys=True) + "\n",
                    encoding="utf-8")
    part.replace(path)


def credentials_for(destination: str) -> dict[str, str]:
    needed = CREDENTIALS[destination]
    absent = [k for k in needed if not os.environ.get(k)]
    if absent:
        raise UploadError(
            f"{destination} needs {', '.join(needed)} in the environment and "
            f"{', '.join(absent)} is not set. Source the credential file at the "
            f"point of use; never put a token on a command line, where it is "
            f"recorded by the shell, by the process table and by anything "
            f"watching either.")
    return {k: os.environ[k] for k in needed}


def put_internetarchive(packed: pathlib.Path, name: str, item: str,
                        creds: dict[str, str], budget: SharedBudget, log) -> None:
    """One file into an Archive item, over its S3 compatible endpoint.

    Deliberately no SDK. The endpoint is a single authenticated PUT, and adding
    a dependency to a release path means one more package that runs with our
    privileges on the machine holding the keys.
    """
    url = f"https://s3.us.archive.org/{item}/{name}"
    path = packed / name

    def beat(sent: int, total: int, _last=[0.0]) -> None:
        now = time.monotonic()
        if now - _last[0] >= BEAT_SECONDS:
            _last[0] = now
            log(f"    {name}: {sent / 1e6:.0f} of {total / 1e6:.0f} MB")

    with ThrottledReader(path, budget, on_progress=beat) as body:
        request = urllib.request.Request(url, data=body, method="PUT")
        request.add_header("authorization",
                           f"LOW {creds['IA_ACCESS_KEY']}:{creds['IA_SECRET_KEY']}")
        request.add_header("content-length", str(body.length))
        request.add_header("x-archive-queue-derive", "0")
        try:
            with urllib.request.urlopen(request, timeout=600) as response:
                if response.status not in (200, 201):
                    raise UploadError(f"{name}: archive returned {response.status}")
        except urllib.error.HTTPError as e:
            raise UploadError(f"{name}: archive returned {e.code} {e.reason}") from e
        except urllib.error.URLError as e:
            raise UploadError(f"{name}: could not reach the archive: {e.reason}") from e


def put_huggingface(packed: pathlib.Path, name: str, repo: str,
                    creds: dict[str, str], budget: SharedBudget, log) -> None:
    """One file into a HuggingFace dataset repository.

    Deliberately no SDK. `huggingface_hub` pulls in a large dependency tree and
    does its own retrying and its own progress bars, neither of which can see
    the shared budget. The upload API is a PUT of the whole file to a
    pre-authorised URL, which the reader can stream into.
    """
    path = packed / name
    size = path.stat().st_size

    # Step one: ask where to put it. Large files go to a storage backend
    # rather than straight into git, and the API decides which.
    preupload = urllib.request.Request(
        f"https://huggingface.co/api/datasets/{repo}/preupload/main",
        data=json.dumps({"files": [{"path": name, "size": size,
                                    "sample": ""}]}).encode(),
        method="POST")
    preupload.add_header("authorization", f"Bearer {creds['HF_TOKEN']}")
    preupload.add_header("content-type", "application/json")
    try:
        with urllib.request.urlopen(preupload, timeout=120) as response:
            plan = json.loads(response.read())
    except urllib.error.HTTPError as e:
        raise UploadError(
            f"{name}: hugging face refused the upload plan, {e.code} {e.reason}. "
            f"Check the repository {repo} exists and the token can write to it."
        ) from e
    except urllib.error.URLError as e:
        raise UploadError(f"{name}: could not reach hugging face: {e.reason}") from e

    entry = (plan.get("files") or [{}])[0]
    if entry.get("uploadMode") == "regular" and not entry.get("shouldIgnore"):
        raise UploadError(
            f"{name}: hugging face wants this file committed through git rather "
            f"than streamed, which this uploader does not do. Files this size "
            f"belong in LFS; check the repository's .gitattributes.")

    def beat(sent: int, total: int, _last=[0.0]) -> None:
        now = time.monotonic()
        if now - _last[0] >= BEAT_SECONDS:
            _last[0] = now
            log(f"    {name}: {sent / 1e6:.0f} of {total / 1e6:.0f} MB")

    upload_url = entry.get("uploadUrl")
    if not upload_url:
        raise UploadError(
            f"{name}: hugging face returned no upload URL, so there is nowhere "
            f"to send it. Response keys: {sorted(entry)}")

    with ThrottledReader(path, budget, on_progress=beat) as body:
        request = urllib.request.Request(upload_url, data=body, method="PUT")
        request.add_header("content-length", str(body.length))
        try:
            with urllib.request.urlopen(request, timeout=3600) as response:
                if response.status not in (200, 201):
                    raise UploadError(f"{name}: hugging face returned "
                                      f"{response.status}")
        except urllib.error.HTTPError as e:
            raise UploadError(f"{name}: hugging face returned {e.code} "
                              f"{e.reason}") from e
        except urllib.error.URLError as e:
            raise UploadError(f"{name}: could not reach hugging face: "
                              f"{e.reason}") from e


def unsupported(destination: str, reason: str):
    def _send(*_args, **_kwargs):
        raise UploadError(f"{destination}: {reason}")
    return _send


SENDERS = {
    "internetarchive": put_internetarchive,
    "huggingface": put_huggingface,
    # Kaggle has no per-file upload endpoint: a dataset version is created by
    # pushing the whole directory through its own client, which does its own
    # chunking and cannot be made to draw from the shared budget. Doing it by
    # hand from the release directory is the honest answer until that is worth
    # building.
    "kaggle": unsupported(
        "kaggle",
        "creates a dataset version from a whole directory rather than file by "
        "file, so it cannot share the bandwidth budget with the other "
        "destinations. Run it on its own, after the others have finished."),
}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--packed", required=True, help="a packed release directory")
    ap.add_argument("--destination", required=True, choices=DESTINATIONS)
    ap.add_argument("--item", default=None,
                    help="the Internet Archive identifier, or the HuggingFace "
                         "or Kaggle dataset slug")
    ap.add_argument("--rate", type=int, default=DEFAULT_RATE,
                    help=f"bytes per second for the WHOLE line, shared with "
                         f"every other uploader using the same --budget. "
                         f"Default {DEFAULT_RATE} (2 MB/s)")
    ap.add_argument("--budget", default=None,
                    help="the shared reservation file. Every uploader that "
                         "names the same one shares the rate between them. "
                         "Defaults to one beside the state file")
    ap.add_argument("--state", default=None,
                    help="where the resume record lives. Defaults to beside "
                         "the release; give it its own path when the release "
                         "is mounted read only")
    ap.add_argument("--live", action="store_true",
                    help="actually send. Without this nothing leaves the machine")
    ap.add_argument("--skip-verify", action="store_true",
                    help="do not re-hash every file first. Only for a re-run "
                         "minutes after a clean one")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)

    def log(message: str) -> None:
        print(f"[{time.strftime('%H:%M:%S')}] {message}")

    packed = pathlib.Path(args.packed)
    if not packed.is_dir():
        print(f"no release directory at {packed}", file=sys.stderr)
        return 1

    mode = "LIVE" if args.live else "DRY RUN, nothing will be sent"
    log(f"{args.destination}: {mode}")

    try:
        files = load_index(packed)
        creds = credentials_for(args.destination)
    except UploadError as e:
        print(f"\n{e}", file=sys.stderr)
        return 1

    total = sum((packed / n).stat().st_size for n in files if (packed / n).is_file())
    log(f"{len(files)} file(s), {total / 1e9:.1f} GB, "
        f"about {total / args.rate / 3600:.1f} hours at {args.rate / 1e6:.1f} MB/s "
        f"if nothing else is sharing the line")

    if not args.skip_verify:
        log("checking every file against the index before anything leaves")
        problems = verify(packed, files)
        if problems:
            print("\nthe release does not match its own index:", file=sys.stderr)
            for line in problems:
                print(f"  {line}", file=sys.stderr)
            print("Nothing was sent. Repack before publishing.", file=sys.stderr)
            return 2
        log("every file matches its recorded digest")

    state_file = pathlib.Path(args.state) if args.state else packed / ".upload-state.json"
    budget_file = pathlib.Path(args.budget) if args.budget else (
        state_file.parent / ".upload-budget")
    if args.budget == "":
        budget_file = None
    try:
        budget = SharedBudget(budget_file, args.rate)
    except UploadError as e:
        print(f"\n{e}", file=sys.stderr)
        return 1
    state = load_state(state_file)
    done = state["destinations"].setdefault(args.destination, {})

    if args.destination == "torrent":
        # The torrent carries the Archive item as its web seed, so a torrent
        # built before the Archive upload lands points at nothing.
        archive = state["destinations"].get("internetarchive", {})
        if len(archive) < len(files):
            print(f"\nthe Internet Archive upload is not complete "
                  f"({len(archive)} of {len(files)} files), and the torrent "
                  f"uses that item as its web seed. Finish the archive first.",
                  file=sys.stderr)
            return 3
        log("the archive item is complete, so the torrent has a seed")
        log("build it with publish_tier.py torrent, then upload it by hand: "
            "Academic Torrents has no upload API")
        return 0

    if not args.item:
        print(f"\n--item is required for {args.destination}", file=sys.stderr)
        return 1

    send = SENDERS[args.destination]
    pending = [n for n, d in sorted(files.items()) if done.get(n) != d]
    log(f"{len(pending)} file(s) still to send, {len(files) - len(pending)} already up")

    if not args.live:
        for name in pending[:5]:
            log(f"  would send {name}")
        if len(pending) > 5:
            log(f"  ... and {len(pending) - 5} more")
        log("DRY RUN complete. Nothing was sent. Re-run with --live to publish.")
        return 0

    started = time.monotonic()
    for position, name in enumerate(pending, 1):
        log(f"[{position}/{len(pending)}] {name}")
        try:
            send(packed, name, args.item, creds, budget, log)
        except UploadError as e:
            print(f"\n{e}", file=sys.stderr)
            print("The state file records what did land, so a re-run resumes "
                  "rather than starting again.", file=sys.stderr)
            save_state(state_file, state)
            return 4
        done[name] = files[name]
        save_state(state_file, state)

    log(f"{len(pending)} file(s) sent in {(time.monotonic() - started) / 3600:.1f} hours")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
