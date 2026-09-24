#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Push a packed tier to the archives, slowly, resumably, and not by accident.

THIS FILE IS DELIBERATELY STANDALONE. DO NOT FOLD IT INTO A PACKAGE.
--------------------------------------------------------------------
`upload-in-container.sh` bind-mounts THIS ONE FILE into the upload container:

    -v "$REPO/tools/release/upload_tier.py:/upload_tier.py:ro"

That container holds write tokens for four public archives, runs unattended for
six hours, and is given `--cap-drop=ALL` and a read-only root so that a
credential leak has as little to reach as possible. Nothing else is mounted, so
this module may not import from `generators/`: the import would simply fail
inside the container. Mounting the whole package instead would widen what a
credential-holding process can read, which is the one property that container
exists to preserve.

So it keeps its own copy of anything it needs, and that duplication is the
price of the isolation rather than an oversight.

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
import re
import sys
import time
import urllib.error
import urllib.parse
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
    """`status` carries the HTTP code where one caused the failure.

    Callers that need to tell "not there" from "not allowed" read it. Matching
    on the message text instead reads whatever the server put in the body, so
    a 401 whose body happens to mention 404 would be taken for a missing
    repository and answered by creating one.

    `retryable` is set only where the failure came from the connection layer,
    and it is a separate field from `status` because the absence of a status
    does not imply one. Most of the status-less failures raised here are
    permanent and local: no ia-metadata.json in the packed directory, an item
    that has been darkened, a body that is not JSON. Treating "no status" as
    "the line dropped" made the uploader sleep two minutes and repeat each of
    those five times before reporting the thing it already knew.
    """

    def __init__(self, *args, status: int | None = None,
                 retryable: bool = False):
        super().__init__(*args)
        self.status = status
        self.retryable = retryable


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
    # discharges CC BY for the thousands of covers that require it, and
    # publishing a
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


def ia_item_exists(item: str) -> bool:
    """Whether the Archive already holds this identifier.

    The metadata endpoint answers 200 with an empty body for an identifier
    nobody has taken, rather than 404, so presence is the `metadata` key
    rather than the status code.
    """
    try:
        with urllib.request.urlopen(f"https://archive.org/metadata/{item}",
                                    timeout=120) as response:
            body = json.loads(response.read().decode("utf-8"))
    except urllib.error.URLError as e:
        raise UploadError(
            f"could not ask the archive whether {item} exists, {e.reason}") from e

    metadata = body.get("metadata") or {}
    # A DARKENED item still answers with metadata. DATASHEET.md promises
    # photographers that a request about their own work is acted on, and that
    # an item can be darkened on request; treating dark as present lets a
    # resumed upload quietly undo a takedown that was honoured, harming the
    # one person the corpus has already been told it harmed.
    if str(metadata.get("is_dark", "")).lower() in ("true", "1"):
        raise UploadError(
            f"{item} is DARKENED on the archive. Somebody asked for it to be "
            f"taken down and that was acted on. Publishing into it would "
            f"reverse a withdrawal. Resolve that first, deliberately.")
    return bool(metadata)


def _ia_headers(packed: pathlib.Path, required: bool = True) -> dict[str, str]:
    """Item metadata as the Archive's S3 endpoint wants it.

    The endpoint creates the item on the first PUT and only then, so the
    metadata has to ride along with it. Sending nothing gives a 404: there is
    no bucket, and nothing asked for one to be made.

    `required=False` is for a part JOINING an item another part created and
    described. Only `main` may decide that, because it is the only caller that
    has checked the item is really there.
    """
    source = packed / "ia-metadata.json"
    if not source.is_file():
        if not required:
            return {"x-amz-auto-make-bucket": "1"}
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


def reconcile_ia_metadata(packed: pathlib.Path, item: str,
                          creds: dict[str, str], log,
                          apply: bool = True) -> None:
    """Make an EXISTING item's metadata match `ia-metadata.json`.

    WHY THIS IS NOT COVERED BY THE UPLOAD.

    `_ia_headers` rides `x-archive-meta-*` along with every PUT, and that is
    how a NEW item gets its title, licence and description: the S3 endpoint
    applies them when `x-amz-auto-make-bucket` creates the item. On an item
    that already exists they are ignored. Silently: the PUT succeeds, the file
    lands, and the metadata is whatever it was.

    Measured on 2026-09-22. `pentimento-core-v1` had been created by hand on
    the 19th and its description read "5,429 covers (54.3%) require
    attribution" when the corpus holds 5,453 at 54.5%. Re-uploading the
    corrected README would have replaced the file and left that sentence
    standing in the item's public, search-indexed description, which is the
    first thing a reader sees and the last thing anybody would think to check.

    So metadata is reconciled explicitly, through the metadata API, which is
    the only route that edits an item in place.
    """
    source = packed / "ia-metadata.json"
    if not source.is_file():
        # A part that JOINS an item carries no metadata of its own. `core-arms`
        # goes into the item `core` created and described, and only the covers
        # part ships `ia-metadata.json`, so this is the normal case for the
        # arms rather than a fault. It was fatal until 2026-09-23, which killed
        # the live release between step 1 and step 2, 45 GB from done.
        #
        # It is only safe once the item is REALLY there. A part with no
        # metadata landing in an item nobody made creates one, publicly, with
        # no title, no licence and no description, so prove it rather than
        # assume it.
        if ia_item_exists(item):
            # Proving the item EXISTS is not proving it says the right thing.
            # This is the path that publishes 45 GB into whatever description
            # is already there, and a wrong description is the exact defect
            # this function was written for: "5,429 covers" stood on a public,
            # search-indexed item for four days. The sibling part carries the
            # description for the item they share, so reconcile against that
            # rather than returning blind.
            sibling = packed.parent / packed.name.removesuffix("-arms")
            if sibling != packed and (sibling / "ia-metadata.json").is_file():
                log(f"  this part carries no item metadata of its own; "
                    f"checking {item} against the part that describes it")
                source = sibling / "ia-metadata.json"
            else:
                log(f"  this part carries no item metadata of its own, and "
                    f"{item} already exists. NOTE: nothing here could check "
                    f"what it says, because the part that describes it is not "
                    f"beside this one.")
                return
        else:
            raise UploadError(
                f"no ia-metadata.json in {packed}, and the item {item} does "
                f"not exist yet. The part that carries the metadata has to be "
                f"uploaded first, or the item is created with no title, no "
                f"licence and no description.")
    want = json.loads(source.read_text(encoding="utf-8"))

    current_url = f"https://archive.org/metadata/{item}"
    with urllib.request.urlopen(current_url, timeout=120) as response:
        current = json.loads(response.read().decode("utf-8"))
    have = current.get("metadata") or {}
    if not have:
        log(f"  {item} does not exist yet; its metadata will ride with the "
            f"first upload, which is what creates it.")
        return

    #: Fields the Archive will not let an ordinary account change, and which
    #: fail the WHOLE patch if included - the API rejects the request rather
    #: than skipping the offending op. Measured 2026-09-23:
    #:
    #:   HTTP 400 {"success":false,"error":"Not authorized to add collection(s)"}
    #:
    #: and it took the description correction down with it, so a figure that
    #: was publicly wrong stayed wrong because of a field nobody could have
    #: set. Collection membership is an Archive-side change; ask them.
    STAFF_ONLY = {"collection"}

    # Only the fields we actually assert, and only where they differ. A patch
    # that rewrites everything would clobber fields the Archive maintains
    # itself, and a patch of no-ops is a write nobody can audit.
    changes = {}
    for key, value in want.items():
        if key == "identifier":
            continue
        if key in STAFF_ONLY:
            if have.get(key) != value:
                log(f"  NOTE: {key} is {have.get(key)!r} and should be "
                    f"{value!r}, but the Archive only lets its own staff set "
                    f"it. Request the move; it cannot be patched from here.")
            continue
        mine = value if not isinstance(value, (list, tuple)) else list(value)
        theirs = have.get(key)
        if isinstance(theirs, (list, tuple)):
            theirs = list(theirs)
        if mine != theirs:
            changes[key] = mine

    if not changes:
        log(f"  {item} metadata already matches ia-metadata.json")
        return

    for key in sorted(changes):
        before = str(have.get(key, "<absent>"))
        after = str(changes[key])
        log(f"    {key}: {before[:70]!r} -> {after[:70]!r}")

    if not apply:
        log(f"  DRY RUN: {len(changes)} metadata field(s) WOULD be updated")
        return

    patch = json.dumps([{"op": "add", "path": f"/{k}", "value": v}
                        for k, v in changes.items()])
    body = urllib.parse.urlencode({
        "-target": "metadata",
        "-patch": patch,
        "access": creds["IA_ACCESS_KEY"],
        "secret": creds["IA_SECRET_KEY"],
    }).encode()
    request = urllib.request.Request(current_url, data=body, method="POST")
    with urllib.request.urlopen(request, timeout=300) as response:
        result = json.loads(response.read().decode("utf-8") or "{}")
    if not result.get("success"):
        raise UploadError(f"metadata update refused for {item}: {result}")
    log(f"  {item}: {len(changes)} metadata field(s) updated")


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
        # `main` has already established that an item exists for a part with no
        # metadata of its own, so the absence here is the arms joining the
        # covers' item rather than a release that forgot to describe itself.
        for key, value in _ia_headers(packed, required=False).items():
            request.add_header(key, value)
        try:
            with urllib.request.urlopen(request, timeout=7200) as response:
                if response.status not in (200, 201):
                    raise UploadError(f"{name}: archive returned {response.status}",
                                      status=response.status)
        except urllib.error.HTTPError as e:
            detail = e.read()[:300].decode("utf-8", "replace").strip()
            # `status` is carried, because `with_retries` reads it to tell a
            # server that said no from a line that dropped. Without it a 401
            # here was indistinguishable from a reset and was retried five
            # times, two minutes of sleeping to be refused again.
            raise UploadError(f"{name}: archive returned {e.code} {e.reason}. "
                              f"{detail}", status=e.code) from e
        except urllib.error.URLError as e:
            raise UploadError(f"{name}: could not reach the archive, {e.reason}",
                              retryable=True) from e


#: How many times a network-level failure is retried, and the first delay.
#: A 48 GB upload over a domestic line runs for most of a day, and a reset in
#: that window is not an exception, it is the weather.
RETRIES = 5
RETRY_BACKOFF = 4.0

#: The HTTP answers that are the line rather than the request. A 429 is the
#: host asking to be asked later and a 5xx is it having a bad minute; both come
#: back differently on their own. Every other 4xx is a decision about this
#: request and repeating it only says no more slowly.
RETRYABLE_STATUSES = frozenset({429, 500, 502, 503, 504})


def retryable(e: BaseException) -> bool:
    """Whether trying the same thing again could plausibly work.

    Read rather than guessed. The earlier rule was "an UploadError with no
    status came from the connection layer", and that was wrong in both
    directions: the Archive's own HTTP wrapper did not record a status, so a
    401 was retried five times, and half the status-less failures raised in
    this file are permanent local refusals that a retry cannot touch.
    """
    if isinstance(e, UploadError):
        return e.retryable or e.status in RETRYABLE_STATUSES
    if isinstance(e, urllib.error.HTTPError):
        return e.code in RETRYABLE_STATUSES
    # URLError, ConnectionError, TimeoutError and the bare OSError beneath
    # them all mean the request did not complete.
    return True


def with_retries(attempt, what: str, log, retries: int = RETRIES):
    """Run `attempt`, retrying only what a retry can actually fix.

    A CONNECTION-level failure means the request did not complete: the peer
    reset it, the name did not resolve, the socket timed out. Trying again is
    the right answer and the only one that gets a day-long upload finished.

    A refusal is NOT retried. The server answered; it said no. Repeating a 401
    or a 400 just says no more slowly, and for a destination that creates
    public artefacts a blind retry is how one bad request becomes five. The
    exceptions are in `RETRYABLE_STATUSES`, which are the answers that say
    "not now" rather than "no".

    Added 2026-09-24 after `[Errno 104] Connection reset by peer` killed the
    live publish 127 files into the 770-file arms step, having already lost a
    separate run eight hours earlier. Nothing in the uploader retried anything.
    """
    delay = RETRY_BACKOFF
    for remaining in range(retries, -1, -1):
        try:
            return attempt()
        except (urllib.error.HTTPError, UploadError, urllib.error.URLError,
                ConnectionError, TimeoutError, OSError) as e:
            if not remaining or not retryable(e):
                raise
            why = e
        # The real reason, not the word "connection": a log saying every
        # failure was a connection failure is how a permanent refusal hid
        # behind two minutes of sleeping.
        log(f"    {what}: {why}; retrying in {delay:.0f}s "
            f"({remaining} attempt(s) left)")
        time.sleep(delay)
        delay *= 2
    # Only reachable if `retries` is negative, which would mean the caller
    # asked for no attempt at all. Loud rather than a silent None.
    raise UploadError(f"{what}: gave up after {retries} retries")


def _hf_api(url: str, token: str, body: bytes | None = None,
            method: str = "GET", content_type: str | None = None,
            accept: str | None = None, what: str = "hugging face",
            expect_json: bool = True) -> dict:
    """One HuggingFace request. `expect_json=False` where the body is ignored.

    Every body was parsed as JSON whether or not any caller read it, so an
    endpoint answering 200 with something else took the whole run down. That
    is what killed the live publish on 2026-09-24 at 02:22, 98 files into the
    45 GB arms step: the git LFS `verify` endpoint returned a non-JSON body,
    the upload it was confirming had already succeeded, and nothing reads its
    return value. Eight hours of line time were lost to a parse of something
    nobody wanted.
    """
    request = urllib.request.Request(url, data=body, method=method)
    request.add_header("authorization", f"Bearer {token}")
    if content_type:
        request.add_header("content-type", content_type)
    if accept:
        request.add_header("accept", accept)
    try:
        with urllib.request.urlopen(request, timeout=300) as response:
            raw = response.read()
        # Whitespace is not a body. `if raw` was true for b"\n".
        raw = raw.strip()
        if not raw:
            return {}
        try:
            return json.loads(raw)
        except json.JSONDecodeError as e:
            if expect_json:
                raise UploadError(
                    f"{what}: answered successfully with a body that is not "
                    f"JSON, which this call needs: "
                    f"{raw[:200].decode('utf-8', 'replace')!r}") from e
            return {}
    except urllib.error.HTTPError as e:
        detail = e.read()[:300].decode("utf-8", "replace").strip()
        raise UploadError(f"{what}: {e.code} {e.reason}. {detail}",
                          status=e.code) from e
    except urllib.error.URLError as e:
        raise UploadError(f"{what}: could not reach it, {e.reason}",
                          retryable=True) from e


def huggingface_repo_declared(packed: pathlib.Path) -> str | None:
    """The repository the SHIPPED card tells readers to load, or None.

    The card is the source of truth because it is the thing that goes public:
    a `--item` that disagrees with it creates one public name and leaves the
    card pointing at another, and neither is recoverable. The Archive copy of
    that card is already search-indexed and the HuggingFace name, once taken,
    is taken.

    Only the part carrying the card can answer. The arms part carries no
    README, which is exactly why it may not create a repository.
    """
    card = packed / "README.md"
    if not card.is_file():
        return None
    found = re.findall(r'load_dataset\(\s*"([^"]+)"',
                       card.read_text(encoding="utf-8"))
    return found[0] if found else None


def ensure_huggingface_repo(repo: str, creds: dict[str, str], log,
                            apply: bool = True,
                            may_create: bool = True) -> None:
    """Make sure the dataset repository exists before a single byte is sent.

    Nothing in this uploader created it. That was survivable while the
    repository happened to exist, and stopped being survivable the moment one
    was deleted: `preupload` answers 404 for a repository that is not there,
    so the run fails on its first file having already spent the verify pass
    and the operator's evening. This is the pre-flight the failure earned.

    Created PUBLIC. A private dataset repository would upload 48 GB to an
    address no reader can reach, which is the failure this corpus already had
    once: the 2026-09-19 run left a private repository holding one shard.
    """
    token = creds["HF_TOKEN"]
    url = f"https://huggingface.co/api/datasets/{repo}"
    try:
        info = _hf_api(url, token, what=f"{repo}: hugging face repository")
    except UploadError as e:
        if e.status != 404:
            raise
    else:
        # EXISTING is not the same as REACHABLE. The 2026-09-19 failure left a
        # PRIVATE repository holding one shard, and that run took this branch:
        # a repository was already there. Checking only the create path guards
        # the case that did not happen.
        if info.get("private"):
            raise UploadError(
                f"{repo} exists but is PRIVATE. Publishing into it would send "
                f"the corpus to an address no reader can reach, which is what "
                f"happened on 2026-09-19, and the docs already link to it. "
                f"Make it public first.")
        log(f"    the dataset repository {repo} exists and is public")
        return

    if not may_create:
        raise UploadError(
            f"{repo} does not exist, and this part carries no README.md, so "
            f"creating it here would publish a repository with no licence, no "
            f"dataset card and no credit list for the photographers. Upload "
            f"the part that carries the card first.")

    if not apply:
        log(f"    the dataset repository {repo} DOES NOT EXIST; a live run "
            f"would create it, public")
        return

    if "/" not in repo:
        raise UploadError(
            f"{repo}: a dataset repository is named <owner>/<name>, and this "
            f"has no owner, so there is nothing to create it under")
    organization, _, name = repo.partition("/")
    log(f"    creating the dataset repository {repo}, public")
    try:
        _hf_api("https://huggingface.co/api/repos/create", token,
                body=json.dumps({"type": "dataset", "name": name,
                                 "organization": organization,
                                 "private": False}).encode(),
                method="POST", content_type="application/json",
                what=f"{repo}: hugging face repository creation")
    except UploadError as e:
        # Two parts go to one repository, so this runs twice per release, and
        # a re-run after an interruption runs it again. "It is already there"
        # is the outcome we wanted, not a failure.
        if e.status != 409:
            raise
        log(f"    {repo} was created by something else first, which is fine")


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
            # Confirms the object landed. Nothing reads what it answers, and
            # it does not always answer JSON.
            _hf_api(verify["href"], token,
                    body=json.dumps({"oid": oid, "size": size}).encode(),
                    method="POST", content_type="application/vnd.git-lfs+json",
                    what=f"{name}: hugging face verify", expect_json=False)

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
    ap.add_argument("--metadata-only", action="store_true",
                    help="Internet Archive only: reconcile the item's "
                         "metadata and send no files. For correcting what an "
                         "item SAYS about itself without re-uploading what it "
                         "holds, which is a 48 GB difference")
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

    # METADATA ONLY. An item's description is public the moment the item is,
    # and it can be wrong while every file in it is right - which is what
    # happened here: "5,429 covers (54.3%)" stood in a search-indexed
    # description for four days over a corpus holding 5,453. Correcting that
    # should not require re-sending 48 GB, and making it require that is how
    # a wrong sentence survives.
    if args.metadata_only:
        if args.destination != "internetarchive":
            print("--metadata-only applies to the Internet Archive, whose "
                  "item metadata is separate from its files. Other "
                  "destinations carry their metadata in the files "
                  "themselves.", file=sys.stderr)
            return 2
        creds = credentials_for(args.destination)
        reconcile_ia_metadata(pathlib.Path(args.packed), args.item, creds, log,
                              apply=bool(args.live))
        if not args.live:
            log("DRY RUN complete. Nothing was changed. Re-run with --live.")
        return 0

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
        # Read-only, and worth doing in a dry run precisely because item
        # metadata is the part that does NOT ride along with the files. A dry
        # run that lists the shards and says nothing about a stale description
        # is the report that let 5,429 stand for three days.
        if args.destination == "internetarchive":
            try:
                log("  item metadata, which does not travel with the files:")
                reconcile_ia_metadata(packed, args.item, creds, log,
                                      apply=False)
            except (UploadError, urllib.error.URLError, OSError) as e:
                log(f"  could not read the item's current metadata: {e}")
        unchecked = []
        if args.destination == "huggingface":
            try:
                log("  the repository the files need to land in:")
                declared = huggingface_repo_declared(packed)
                if declared and declared != args.item:
                    unchecked.append(
                        f"--item is {args.item} but the shipped card names "
                        f"{declared}")
                ensure_huggingface_repo(args.item, creds, log, apply=False,
                                        may_create=declared is not None)
            except (UploadError, urllib.error.URLError, OSError) as e:
                log(f"  could not check the repository: {e}")
                unchecked.append(str(e))
        if unchecked:
            # The dry run is THE proving step before a live publish, so a
            # precondition it could not check must not be reported as one it
            # checked. An expired token printed one line in the middle of a
            # long output and the run still ended "complete" and exited zero.
            print(f"\nDRY RUN INCOMPLETE: {len(unchecked)} precondition(s) "
                  f"could not be checked. Nothing was sent.", file=sys.stderr)
            for line in unchecked:
                print(f"  {line}", file=sys.stderr)
            return 3
        log("DRY RUN complete. Nothing was sent. Re-run with --live to publish.")
        return 0

    # BEFORE the files, not after. If the run is interrupted halfway the item
    # should already describe itself correctly; an item carrying new shards
    # under a stale description is the worse of the two partial states, and
    # the description is what a reader sees first.
    if args.destination == "internetarchive":
        reconcile_ia_metadata(packed, args.item, creds, log)
    if args.destination == "huggingface":
        declared = huggingface_repo_declared(packed)
        if declared and declared != args.item:
            print(f"\n--item is {args.item}, but the card this part ships "
                  f"sends every reader to {declared}. Publishing would take "
                  f"one public name and leave the card pointing at another, "
                  f"and neither can be undone.", file=sys.stderr)
            return 2
        ensure_huggingface_repo(args.item, creds, log,
                                may_create=declared is not None)

    started = time.monotonic()
    for position, name in enumerate(pending, 1):
        log(f"[{position}/{len(pending)}] {name}")
        try:
            # Per FILE, because that is the unit the state file already makes
            # idempotent: a completed file is skipped on the next pass, and a
            # half-sent one is simply sent again. Both destinations key stored
            # objects by digest, so a repeat costs bandwidth and nothing else.
            with_retries(
                lambda: send(packed, name, args.item, creds, budget, log),
                name, log)
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
