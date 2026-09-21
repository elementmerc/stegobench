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


class PartReader:
    """A window of a file, throttled, for a multipart upload.

    One part is a byte range rather than a whole file, and it has to report its
    own length, so this is a reader over `[offset, offset + size)` rather than
    a second use of the whole-file one.
    """

    def __init__(self, path: pathlib.Path, offset: int, size: int,
                 budget: SharedBudget):
        self._handle = path.open("rb")
        self._handle.seek(offset)
        self._budget = budget
        self._left = size
        self.length = size

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self._handle.close()
        return False

    def read(self, size: int = -1) -> bytes:
        if self._left <= 0:
            return b""
        want = CHUNK if size is None or size < 0 else min(size, CHUNK)
        chunk = self._handle.read(min(want, self._left))
        if not chunk:
            return b""
        self._budget.reserve(len(chunk))
        self._left -= len(chunk)
        return chunk

    def __iter__(self):
        while True:
            chunk = self.read(CHUNK)
            if not chunk:
                return
            yield chunk


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
    #
    # THIS LIST IS NOT A PREFERENCE. Four of these were missing from it while
    # the published documentation told people to fetch them by name: the
    # quickstart runs `curl -O $BASE/SHA256SUMS -O $BASE/load_pentimento.py`
    # and then `sha256sum -c`, all of which would have answered 404. The
    # attribution files are worse than an inconvenience: they are how a reader
    # discharges CC BY for the 5,429 covers that require it, and publishing a
    # CC BY corpus while withholding the credit list is the one failure this
    # corpus has no excuse for.
    #
    # Anything written into a packed directory that a reader is told about
    # belongs here. `packaged_extras` is the shared list so the packer and the
    # publisher cannot disagree about it.
    for extra in packaged_extras(packed):
        files[extra.name] = digest_of(extra)
    return files


#: Files that ship beside the shards. Names, not a glob, so a stray file in the
#: packed directory is never published by accident.
PACKAGED_EXTRAS = (
    "README.md",
    "LICENCES.md",
    "DATASHEET.md",
    "SPLITS.md",
    "CITATION.cff",
    "croissant.json",
    "licence-summary.json",
    "ATTRIBUTION.md",
    "ATTRIBUTION.csv",
    "load_pentimento.py",
    "SHA256SUMS-covers",
    "SHA256SUMS-arms",
)

#: Written for a destination's own use and meaningless to a downloader. The
#: Archive reads `ia-metadata.json` as headers on the first PUT and Kaggle
#: reads `dataset-metadata.json` through its own client; publishing them as
#: files would just be confusing.
NOT_PUBLISHED = ("ia-metadata.json", "dataset-metadata.json")


def packaged_extras(packed: pathlib.Path) -> list[pathlib.Path]:
    """The non-shard files in `packed` that belong in the release."""
    return [packed / name for name in PACKAGED_EXTRAS if (packed / name).is_file()]


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


def _ia_headers(packed: pathlib.Path) -> dict[str, str]:
    """Item metadata as the Archive's S3 endpoint wants it.

    The endpoint creates the item on the first PUT and only then, so the
    metadata has to ride along with it. Sending nothing gives a 404: there is
    no bucket, and nothing asked for one to be made.
    """
    source = packed / "ia-metadata.json"
    if not source.is_file():
        raise UploadError(
            f"no ia-metadata.json in {packed}. It is derived from the manifest "
            f"by publish_tier.py prepare, and without it the item would be "
            f"created with no title, no licence and no description.")
    meta = json.loads(source.read_text(encoding="utf-8"))
    headers = {"x-amz-auto-make-bucket": "1"}
    for key, value in meta.items():
        if key == "identifier":
            continue
        if isinstance(value, (list, tuple)):
            # Repeated fields are numbered from 00, which is how the Archive
            # takes more than one subject.
            for index, item in enumerate(value):
                headers[f"x-archive-meta{index:02d}-{key}"] = str(item)
        else:
            headers[f"x-archive-meta-{key}"] = str(value)
    return headers


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
        # Derivation turns one upload into a queue of server-side jobs. For a
        # corpus of tar shards there is nothing useful to derive and the queue
        # would run for days.
        request.add_header("x-archive-queue-derive", "0")
        for key, value in _ia_headers(packed).items():
            request.add_header(key, value)
        try:
            with urllib.request.urlopen(request, timeout=7200) as response:
                if response.status not in (200, 201):
                    raise UploadError(f"{name}: archive returned {response.status}")
        except urllib.error.HTTPError as e:
            detail = e.read()[:300].decode("utf-8", "replace").strip()
            raise UploadError(f"{name}: archive returned {e.code} {e.reason}. "
                              f"{detail}") from e
        except urllib.error.URLError as e:
            raise UploadError(f"{name}: could not reach the archive, {e.reason}") from e


def _hf_api(url: str, token: str, body: bytes | None = None,
            method: str = "GET", content_type: str | None = None,
            accept: str | None = None, what: str = "hugging face") -> dict:
    request = urllib.request.Request(url, data=body, method=method)
    request.add_header("authorization", f"Bearer {token}")
    if content_type:
        request.add_header("content-type", content_type)
    if accept:
        request.add_header("accept", accept)
    try:
        with urllib.request.urlopen(request, timeout=300) as response:
            raw = response.read()
        return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as e:
        detail = e.read()[:300].decode("utf-8", "replace").strip()
        raise UploadError(f"{what}: {e.code} {e.reason}. {detail}") from e
    except urllib.error.URLError as e:
        raise UploadError(f"{what}: could not reach it, {e.reason}") from e


def _hf_commit(repo: str, token: str, lines: list[dict], summary: str) -> None:
    """One commit on main. The API takes newline-delimited JSON, not a list."""
    payload = [{"key": "header", "value": {"summary": summary, "description": ""}}]
    payload += lines
    body = ("\n".join(json.dumps(line) for line in payload) + "\n").encode()
    _hf_api(f"https://huggingface.co/api/datasets/{repo}/commit/main", token,
            body=body, method="POST", content_type="application/x-ndjson",
            what="hugging face commit")


def _hf_put_part(url: str, path: pathlib.Path, offset: int, size: int,
                 budget: SharedBudget) -> str:
    """One multipart part, throttled. Returns the ETag the completion needs."""
    with PartReader(path, offset, size, budget) as body:
        request = urllib.request.Request(url, data=body, method="PUT")
        request.add_header("content-length", str(size))
        try:
            with urllib.request.urlopen(request, timeout=3600) as response:
                etag = response.headers.get("ETag") or response.headers.get("etag")
        except urllib.error.HTTPError as e:
            raise UploadError(f"part upload refused: {e.code} {e.reason}") from e
        except urllib.error.URLError as e:
            raise UploadError(f"part upload could not be sent: {e.reason}") from e
    if not etag:
        raise UploadError("a part uploaded without returning an ETag, which the "
                          "completion call needs to assemble the file")
    return etag


def put_huggingface(packed: pathlib.Path, name: str, repo: str,
                    creds: dict[str, str], budget: SharedBudget, log) -> None:
    """One file into a HuggingFace dataset repository.

    Deliberately no SDK: `huggingface_hub` does its own chunking and its own
    retrying, neither of which can see the shared bandwidth budget, and a
    release path with fewer moving parts is one that still works in a year.

    HuggingFace decides per file how it wants the bytes. Small ones are
    committed inline; anything large goes through git LFS, and for a file of
    any size worth calling large that means a multipart upload whose parts are
    signed separately. Both paths end in a commit, and the commit is what makes
    the file exist.
    """
    token = creds["HF_TOKEN"]
    path = packed / name
    size = path.stat().st_size

    plan = _hf_api(
        f"https://huggingface.co/api/datasets/{repo}/preupload/main", token,
        body=json.dumps({"files": [{"path": name, "size": size, "sample": ""}]}).encode(),
        method="POST", content_type="application/json",
        what=f"{name}: hugging face preupload")
    entry = (plan.get("files") or [{}])[0]
    mode = entry.get("uploadMode")

    if entry.get("shouldIgnore"):
        raise UploadError(f"{name}: the repository's .gitattributes ignores this path")

    if mode == "regular":
        # Small enough to travel inside the commit. Still reserved from the
        # budget, so a few hundred small files cannot jump the queue.
        budget.reserve(size)
        import base64
        _hf_commit(repo, token, [{"key": "file", "value": {
            "path": name,
            "content": base64.b64encode(path.read_bytes()).decode(),
            "encoding": "base64"}}], f"Add {name}")
        return

    if mode != "lfs":
        raise UploadError(f"{name}: hugging face asked for an upload mode this "
                          f"uploader does not know, {mode!r}")

    oid = digest_of(path)
    batch = _hf_api(
        f"https://huggingface.co/datasets/{repo}.git/info/lfs/objects/batch", token,
        body=json.dumps({"operation": "upload",
                         "transfers": ["basic", "multipart"],
                         "hashAlgo": "sha_256",
                         "objects": [{"oid": oid, "size": size}]}).encode(),
        method="POST", content_type="application/vnd.git-lfs+json",
        accept="application/vnd.git-lfs+json",
        what=f"{name}: hugging face LFS batch")

    obj = (batch.get("objects") or [{}])[0]
    if obj.get("error"):
        raise UploadError(f"{name}: hugging face refused the object, {obj['error']}")
    actions = obj.get("actions") or {}
    upload = actions.get("upload")

    if upload is None:
        # No upload action means the server already holds these bytes. Only the
        # pointer is missing, so commit it and stop.
        log(f"    {name}: already stored, committing the pointer")
    else:
        header = dict(upload.get("header") or {})
        chunk_size = header.pop("chunk_size", None)
        parts = sorted((k for k in header if k.isdigit()), key=int)

        if parts and chunk_size:
            chunk_size = int(chunk_size)
            log(f"    {name}: {len(parts)} part(s) of {chunk_size / 1e6:.0f} MB")
            etags = []
            for index, key in enumerate(parts):
                offset = index * chunk_size
                this = min(chunk_size, size - offset)
                etags.append({"partNumber": int(key),
                              "etag": _hf_put_part(header[key], path, offset,
                                                   this, budget)})
                log(f"    {name}: part {index + 1} of {len(parts)} done")
            _hf_api(upload["href"], token,
                    body=json.dumps({"oid": oid, "parts": etags}).encode(),
                    method="POST", content_type="application/json",
                    what=f"{name}: hugging face multipart completion")
        else:
            def beat(sent: int, total: int, _last=[0.0]) -> None:
                now = time.monotonic()
                if now - _last[0] >= BEAT_SECONDS:
                    _last[0] = now
                    log(f"    {name}: {sent / 1e6:.0f} of {total / 1e6:.0f} MB")

            with ThrottledReader(path, budget, on_progress=beat) as body:
                request = urllib.request.Request(upload["href"], data=body, method="PUT")
                request.add_header("content-length", str(body.length))
                for key, value in header.items():
                    request.add_header(key, value)
                try:
                    with urllib.request.urlopen(request, timeout=7200) as response:
                        if response.status not in (200, 201, 204):
                            raise UploadError(f"{name}: hugging face returned "
                                              f"{response.status}")
                except urllib.error.HTTPError as e:
                    raise UploadError(f"{name}: hugging face returned {e.code} "
                                      f"{e.reason}") from e
                except urllib.error.URLError as e:
                    raise UploadError(f"{name}: could not reach hugging face, "
                                      f"{e.reason}") from e

        verify = actions.get("verify")
        if verify:
            _hf_api(verify["href"], token,
                    body=json.dumps({"oid": oid, "size": size}).encode(),
                    method="POST", content_type="application/vnd.git-lfs+json",
                    what=f"{name}: hugging face verify")

    _hf_commit(repo, token, [{"key": "lfsFile", "value": {
        "path": name, "algo": "sha256", "oid": oid, "size": size}}],
        f"Add {name}")


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
