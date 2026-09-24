#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Every invariant this corpus must satisfy before it is published, in one run.

WHY A TOOL RATHER THAN A CHECKLIST
----------------------------------
These checks have been run by hand, in an ad hoc order, several times this
week, and each time the transcript was the only record that they passed. That
is the same failure this corpus exists to argue against: a claim whose evidence
you have to take on trust.

Worse, a checklist run by hand is run in the order the person remembers, and
the invariants here are not independent. Replacing a cover keeps its filename,
so a stale arm is invisible in every field an arm manifest records; the tier
prefix guarantee is only meaningful if `tier_order` is dense; a packed shard
can match its own index while indexing the wrong corpus.

So this is one command, it refuses rather than warns, and it says which check
failed rather than "verification failed".

WHAT IT DOES NOT DO
-------------------
It does not fix anything, and it does not publish. It reads, and it returns
non-zero if the corpus is not fit to leave the machine.

THE CHECKS, AND WHY EACH ONE IS HERE
------------------------------------
`covers`
    Count, density of `tier_order`, and uniqueness of filename, Commons pageid
    and image digest. Density is load bearing: a tier is a PREFIX of
    `tier_order`, and that is the whole reason Nano nests inside Lite inside
    Core. A hole moves every boundary after it.

`licences`
    No licence outside the permitted set, and a credit line on every cover that
    needs one. A corpus arguing that provenance should be traceable cannot ship
    a row reading "author not recorded by the source".

`digests`
    The bytes on disk are the bytes the manifest claims. Sampled by default
    because a full pass over 10,000 covers plus 340,000 arm images is slow;
    `--full` does all of it and is what a release runs.

`packed`
    Do the PACKED tiers still hold the corpus the manifest describes? Nesting
    itself is not worth checking: a tier is derived as a prefix of one
    `tier_order`, so given a dense ordering it holds by construction. What can
    go wrong is staleness. A tier packed before the covers were replaced still
    holds the old images, its own index agrees with itself, its shard digests
    are correct, and nothing anywhere says the corpus moved underneath it.

`packed-arms`
    The same question for the packed ARMS, which `packed` deliberately leaves
    alone. A fresh arm pack is sound by construction, because `pack_arms`
    hashes every file as it reads it, so what this catches is an OLD arm pack
    left in place by a rebuild: correct shard digests, an index that agrees
    with itself, right counts, previous corpus. Needs `--arms` as well as
    `--release`, because the arms as built are what it compares against.

`figures`
    The numbers in the published prose match the corpus that shipped. The
    manifest being right does not make the README right: they are checked by
    different things, and for one release they disagreed by 24 covers.

`links`
    Every URL in the shipped prose answers. `DATASHEET.md` went to three public
    mirrors telling photographers to open an issue at a repository that was
    private, so the address was a 404 for the whole life of the release, and
    `CITATION.cff` names the same one as `repository-code`. A reviewer called a
    dead objection channel the one thing that would fail their chain of custody
    standard. Needs `--check-urls`, because it is the only check that reaches
    off this machine and an offline run must not go red for being offline.

`pool`
    The JPEG cover pool is dense, because `build_adaptive_arms.py` indexes it
    positionally and one gap mispairs every cover after it.

`pairs`
    Both halves of every pair exist, and their containers are structurally
    identical. This is the check that would have caught 80,000 separable
    images: the stego half went through one more jpeglib write than the clean
    half, and jpeglib prepends a JFIF APP0 every time.

`provenance`
    Every arm row names the DIGEST of the cover it was built from, not only its
    filename. A backfill replaces a cover in place under the same name, so a
    filename cannot witness the swap and a whole arm can be derived from an
    image that no longer exists while every other field still agrees.

`stale`
    No arm derives from a cover that has since been replaced. A backfill keeps
    the cover's FILENAME, so nothing in an arm manifest changes when the image
    underneath it does. The only witness is the cover's digest, which arm rows
    do not record, so this compares the clean half's recorded digest against
    the clean half on disk and reports any arm older than the cover it claims.

Usage::

    python verify_release.py --covers ~/pentimento/covers/commons \\
                             --arms ~/pentimento/arms/core \\
                             --release ~/pentimento/release \\
                             --sample 500
"""
from __future__ import annotations

import argparse
import collections
import hashlib
import json
import pathlib
import random
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from pack_arms import container_of, cover_of, load_jpeg_cover_map  # noqa: E402

#: Licences this corpus may carry. Anything else is either a mistake or a
#: decision nobody recorded, and both are worth refusing over.
PERMITTED_LICENCES = {
    "CC0", "CC BY 1.0", "CC BY 2.0", "CC BY 2.5", "CC BY 3.0", "CC BY 4.0",
    "Public domain",
}

#: Artist values that do not discharge an attribution requirement. Kept in step
#: with `manifest_repair.UNUSABLE_ARTIST`; imported rather than restated where
#: that import is available.
try:
    from manifest_repair import UNUSABLE_ARTIST
except ImportError:  # pragma: no cover - the constant is the contract
    UNUSABLE_ARTIST = {"", "unknown", "unknown author", "not recorded"}

#: Which of the published files are read for links. Taken by SUFFIX from
#: `release_metadata.PUBLISHED_EXTRAS` rather than listed here, so a document
#: added to the release is scanned without anybody remembering to add it: the
#: Markdown prose, plus the structured files that carry addresses
#: (`CITATION.cff`, `croissant.json`, `licence-summary.json`). The checksum
#: files, the CSV and the loader script hold no prose and are left out.
PROSE_SUFFIXES = (".md", ".cff", ".json")

#: Every request is bounded, because this is the one check that waits on a
#: machine nobody here controls. A host that accepts a connection and then
#: never answers would otherwise hang a release gate indefinitely.
URL_TIMEOUT = 10.0
URL_REDIRECT_CAP = 5
#: At most this many distinct URLs per run, spread across hosts rather than
#: taken in order (see `_spread`). `ATTRIBUTION.md` alone carries one Commons
#: link per credited photograph, and checking them in sorted order would spend
#: the whole budget on one host and never reach the repository link that is
#: the reason this check exists.
URL_CAP = 200
#: A per-file read cap, so a corrupt or accidentally enormous document cannot
#: pull the whole release into memory. Exceeding it is reported, not ignored.
PROSE_BYTES_CAP = 8 << 20
#: Some hosts refuse an unidentified client outright, which would read as a
#: dead link when the page is fine.
URL_USER_AGENT = "pentimento-verify-release/1.0 (+link check)"

#: Deliberately excludes the closing brackets and the quote characters, so a
#: link written as Markdown, as JSON or inside a sentence ends where the prose
#: resumes rather than swallowing the punctuation after it.
URL_PATTERN = re.compile(r"https?://[^\s<>\"'\\)\]}|]+")

#: Answers that say nothing about whether the page exists: the host declined to
#: talk to us this minute. A rate limit must not fail a release.
UNSETTLED_STATUSES = frozenset({408, 425, 429})


class _CappedRedirects(urllib.request.HTTPRedirectHandler):
    max_redirections = URL_REDIRECT_CAP


def fetch_status(url: str, timeout: float = URL_TIMEOUT) -> tuple[int | None, str]:
    """Ask a URL whether it is there, and report what happened.

    Returns ``(status, how)``. A status of None means no answer arrived at all,
    which is a different thing from a 404 and is reported differently: the page
    may be perfectly alive and this machine simply off the network.

    HEAD first, because a link check has no use for the body and some of these
    documents are large. A host that will not do HEAD says so with a 4xx of its
    own, and GET is tried once before its refusal is believed.
    """
    opener = urllib.request.build_opener(_CappedRedirects())
    last = "no request was made"
    for method in ("HEAD", "GET"):
        request = urllib.request.Request(
            url, method=method, headers={"User-Agent": URL_USER_AGENT})
        try:
            with opener.open(request, timeout=timeout) as response:
                return response.status, method
        except urllib.error.HTTPError as e:
            # 405 and 501 are the standard "I do not do that verb"; 400 and 403
            # are what several CDNs send instead. Anything else is the host's
            # real answer about this address.
            if method == "HEAD" and e.code in (400, 403, 405, 501):
                last = f"HEAD refused with {e.code}"
                continue
            return e.code, method
        except Exception as e:
            # Deliberately broad: DNS, TLS, reset, timeout and the malformed
            # URL all arrive as different types and mean the same thing here,
            # that nothing answered. The reason is carried, never discarded.
            return None, f"{type(e).__name__}: {e}"
    return None, last


def urls_in(text: str) -> list[str]:
    """Every URL in a document, in the order it appears, trailing punctuation
    removed. A sentence ending "...at https://example.org." names a host, not a
    path with a full stop on it."""
    found = []
    for raw in URL_PATTERN.findall(text):
        url = raw.rstrip(".,;:!?")
        if url:
            found.append(url)
    return found


def _spread(urls: list[str], cap: int) -> list[str]:
    """Take at most `cap` URLs, one host at a time, round robin.

    Taking the first `cap` in any stable order hands the whole budget to
    whichever host appears most, and in this release that is Commons with one
    link per credited photograph. The link that has actually been broken, the
    repository address in `DATASHEET.md` and `CITATION.cff`, appears twice. So
    every distinct host gets looked at before any host gets looked at twice.
    """
    by_host: dict[str, list[str]] = collections.OrderedDict()
    for url in urls:
        host = urllib.parse.urlsplit(url).netloc.lower()
        by_host.setdefault(host, []).append(url)

    taken: list[str] = []
    queues = [q for q in by_host.values()]
    while queues and len(taken) < cap:
        queues = [q for q in queues if q]
        for queue in queues:
            taken.append(queue.pop(0))
            if len(taken) >= cap:
                break
    return taken



class Report:
    """Collects failures per check so one run reports all of them."""

    def __init__(self) -> None:
        self.failures: dict[str, list[str]] = collections.defaultdict(list)
        self.notes: dict[str, str] = {}
        self.skipped: dict[str, str] = {}

    def fail(self, check: str, message: str) -> None:
        self.failures[check].append(message)

    def note(self, check: str, message: str) -> None:
        self.notes[check] = message

    def skip(self, check: str, why: str) -> None:
        """A check that was not run at all, and what would run it.

        Omitting the line entirely is the same fault `nothing_checked` exists
        to prevent, one level up: a check that never ran leaves no trace, so
        the summary reads as though everything was examined. `packed` is the
        one that proves it, because it is the check that found two stale
        covers in an otherwise clean pack.
        """
        self.skipped[check] = why

    @property
    def ok(self) -> bool:
        return not self.failures


def nothing_checked(check: str, count: int, report: Report,
                    what: str) -> bool:
    """Refuse a check that examined nothing, rather than passing it.

    "0 pairs, containers identical" reads as a pass and means the opposite:
    that nothing was looked at. **Could not look** and **looked and found
    nothing wrong** render as the same clean line, and the clean line is the
    one people act on. This is the single most common fault found across this
    fleet today, so it is a helper rather than a habit.
    """
    if count:
        return False
    report.fail(check, f"checked no {what} at all. That is not a pass: it "
                       f"means this check could not look, and a check that "
                       f"cannot look must not report clean")
    return True


def load_rows(path: pathlib.Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip()]


def digest(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for block in iter(lambda: fh.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def check_covers(rows: list[dict], expected: int, report: Report) -> None:
    if len(rows) != expected:
        report.fail("covers", f"{len(rows):,} rows, expected {expected:,}")

    orders = sorted(r.get("tier_order") for r in rows)
    if orders != list(range(len(rows))):
        holes = set(range(len(rows))) - set(orders)
        report.fail("covers",
                    f"tier_order is not dense 0..{len(rows) - 1}; "
                    f"{len(holes)} hole(s), first {sorted(holes)[:5]}. A tier "
                    f"is a prefix of this ordering, so a hole moves every "
                    f"boundary after it")

    for field in ("file", "pageid", "sha256"):
        values = [r.get(field) for r in rows]
        if len(set(values)) != len(values):
            duplicated = [v for v, n in collections.Counter(values).items()
                          if n > 1]
            report.fail("covers",
                        f"{field} is not unique: {len(duplicated)} repeated, "
                        f"e.g. {duplicated[:3]}")
    report.note("covers", f"{len(rows):,} rows, dense, all unique")


def check_licences(rows: list[dict], report: Report) -> None:
    seen = collections.Counter(r.get("licence") for r in rows)
    outside = set(seen) - PERMITTED_LICENCES
    if outside:
        report.fail("licences", f"licence(s) outside the permitted set: "
                                f"{sorted(outside)}")

    unattributable = [r["file"] for r in rows
                      if r.get("attribution_required")
                      and (r.get("artist") or "").strip().lower()
                      in UNUSABLE_ARTIST]
    if unattributable:
        report.fail("licences",
                    f"{len(unattributable):,} cover(s) require attribution and "
                    f"record no usable author, e.g. {unattributable[:3]}")

    missing_credit = [r["file"] for r in rows if not r.get("attribution")]
    if missing_credit:
        report.fail("licences",
                    f"{len(missing_credit):,} cover(s) have no credit line, "
                    f"e.g. {missing_credit[:3]}")
    report.note("licences",
                f"{len(seen)} licence value(s), 0 unattributable, "
                f"{len(rows) - len(missing_credit):,} credit lines")


def check_packed(release: pathlib.Path, rows: list[dict],
                 sample: int, report: Report) -> None:
    """Do the PACKED tiers still hold the corpus the manifest describes?

    Nesting is not the interesting question. A tier is derived as a prefix of
    one `tier_order`, so given a dense ordering the prefix property is true by
    construction and checking it proves only that the code did what it says.

    The question that can actually go wrong is staleness. A tier packed before
    the covers were replaced still holds the old images, its own index agrees
    with itself, its shard digests are correct, and nothing anywhere says the
    corpus moved underneath it. The shard members are keyed by tier position,
    so comparing a member's bytes against the cover manifest at that position
    settles it.
    """
    if not release.is_dir():
        report.fail("packed", f"no release directory at {release}")
        return

    import tarfile

    by_order = {r["tier_order"]: r for r in rows}
    indexes = sorted(release.rglob("pentimento-*-index.json"))
    # The cover tiers only. An arm index keys its members by position within
    # the arm rather than by `tier_order`, so the cover manifest cannot say
    # anything about one; the arms are checked against the arms as built, in
    # `check_packed_arms`.
    indexes = [p for p in indexes if "arms" not in p.name]
    if not indexes:
        report.fail("packed", f"no packed cover tier under {release}")
        return

    stale, unreadable, checked, tiers = [], [], 0, []
    for index_path in indexes:
        index = json.loads(index_path.read_text(encoding="utf-8"))
        tier = index.get("tier", index_path.parent.name)
        tiers.append(tier)
        for shard in index.get("shards", []):
            tar_path = index_path.parent / shard["shard"]
            if not tar_path.is_file():
                unreadable.append(str(tar_path.name))
                continue
            first = shard.get("first_tier_order", 0)
            wanted = {f"{n:06d}.png" for n in
                      _positions(first, shard.get("last_tier_order", first),
                                 sample)}
            with tarfile.open(tar_path) as tar:
                for member in tar:
                    if member.name not in wanted:
                        continue
                    fh = tar.extractfile(member)
                    if fh is None:
                        continue
                    order = int(member.name.split(".")[0])
                    row = by_order.get(order)
                    if row is None:
                        stale.append(f"{tier}:{member.name} has no cover at "
                                     f"tier_order {order}")
                        continue
                    checked += 1
                    if hashlib.sha256(fh.read()).hexdigest() != row["sha256"]:
                        stale.append(f"{tier}:{member.name}")

    if unreadable:
        report.fail("packed", f"{len(unreadable)} shard(s) named in an index "
                              f"are not on disk, e.g. {unreadable[:3]}")
    if stale:
        report.fail("packed",
                    f"{len(stale):,} packed cover(s) differ from the manifest "
                    f"at their tier position, e.g. {stale[:3]}. The tier was "
                    f"packed before the covers changed and nothing in it says "
                    f"so; repack before publishing")
    if not unreadable and nothing_checked("packed", checked, report,
                                          "packed members"):
        return
    if not unreadable and not stale:
        report.note("packed", f"{checked:,} members across {len(tiers)} tier(s) "
                              f"match the manifest at their position")


def check_attribution(release: pathlib.Path, rows: list[dict],
                      report: Report) -> None:
    """Does the shipped credit list actually credit everybody it must?

    Every numeric claim in this release has a gate behind it. The one artefact
    that DISCHARGES the licence obligation had none: `check_licences` validates
    the manifest's rows, and the figure check deliberately skips
    `ATTRIBUTION.*` as data rather than claims. Both are right about their own
    scope, and between them a truncated write or a tier packed before a
    backfill produces a credit list short of rows while every check passes.

    The people who lose by that are photographers, and they never find out.

    So: for each packed cover tier, the credit list must exist, and must carry
    exactly one row per attribution-requiring cover IN THAT TIER. The tier
    scoping matters as much as the count. Shipping Core's 5,453 credit lines
    with a 200 cover Nano names photographers whose work is not in the
    download, which is its own false statement.
    """
    import csv
    import io

    indexes = sorted(p for p in release.rglob("pentimento-*-index.json")
                     if "arms" not in p.name)
    if not indexes:
        report.fail("attribution", f"no packed cover tier under {release}")
        return

    by_order = sorted(rows, key=lambda r: r["tier_order"])
    missing, wrong, checked, tiers = [], [], 0, []
    for index_path in indexes:
        index = json.loads(index_path.read_text(encoding="utf-8"))
        tier = index.get("tier", index_path.parent.name)
        tiers.append(tier)
        packed = index.get("samples", 0)
        # A tier is a PREFIX of tier_order, which `check_covers` proves dense,
        # so the tier's own covers are the first n of that ordering.
        in_tier = by_order[:packed]
        want = {r["file"] for r in in_tier if r.get("attribution_required")}

        csv_path = index_path.parent / "ATTRIBUTION.csv"
        md_path = index_path.parent / "ATTRIBUTION.md"
        for p in (csv_path, md_path):
            if not p.is_file():
                missing.append(f"{tier}: no {p.name}")
        if not csv_path.is_file():
            continue

        seen: list[str] = []
        with io.StringIO(csv_path.read_text(encoding="utf-8")) as fh:
            for row in csv.DictReader(fh):
                name = (row.get("file") or row.get("cover") or "").strip()
                if name:
                    seen.append(name)
        counts = collections.Counter(seen)
        absent = sorted(want - set(counts))
        extra = sorted(set(counts) - want)
        repeated = sorted(n for n, c in counts.items() if c > 1)
        checked += len(want)

        if absent:
            wrong.append(f"{tier}: {len(absent):,} cover(s) require "
                         f"attribution and are not credited, e.g. {absent[:3]}")
        if extra:
            wrong.append(f"{tier}: {len(extra):,} credited cover(s) are not in "
                         f"this tier or do not require it, e.g. {extra[:3]}")
        if repeated:
            wrong.append(f"{tier}: {len(repeated):,} cover(s) credited more "
                         f"than once, e.g. {repeated[:3]}")

    if missing:
        report.fail("attribution", f"{len(missing)} credit list(s) are not "
                                   f"present, e.g. {missing[:3]}")
    if wrong:
        for message in wrong[:4]:
            report.fail("attribution", message)
    if missing or wrong:
        return
    if nothing_checked("attribution", checked, report, "credit lines"):
        return
    report.note("attribution",
                f"{checked:,} required credit line(s) across {len(tiers)} "
                f"tier(s), each present exactly once")


def arm_truth(arm_root: pathlib.Path) -> dict[str, dict[str, str]]:
    """What each arm holds TODAY, grouped exactly as the packer groups it.

    Derived by calling `pack_arms`' own grouping rather than by restating it,
    because an independent reimplementation of which rows become which arm is
    a second thing that can drift, and a checker that drifts into agreeing with
    the pack it is checking is the fault this file keeps finding elsewhere.

    Returns ``{arm: {relative path: digest}}``.
    """
    from pack_arms import clean_arms, group_rows

    truth: dict[str, dict[str, str]] = {}
    for manifest in sorted(arm_root.glob("*/manifest.jsonl")):
        group = manifest.parent.name
        by_arm = group_rows(manifest)
        every_row = [r for rows in by_arm.values() for r in rows]
        by_arm.update(clean_arms(every_row, group))
        for arm, rows in by_arm.items():
            if not rows:
                continue
            path_field = "stego" if "stego" in rows[0] else "file"
            digest_field = ("stego_sha256" if path_field == "stego"
                            else "sha256")
            held = truth.setdefault(arm, {})
            for row in rows:
                rel, digest_value = row.get(path_field), row.get(digest_field)
                if rel and digest_value:
                    held[rel] = digest_value
    return truth


def check_packed_arms(release: pathlib.Path, arm_root: pathlib.Path,
                      sample: int, report: Report) -> None:
    """Do the packed ARMS still hold the arms that were built?

    `check_packed` deliberately skips any index whose name contains "arms", so
    until now only the cover tiers were ever compared with anything outside
    themselves. A FRESH arm pack is sound by construction, because `pack_arms`
    hashes every file as it reads it and leaves out anything that disagrees.
    The gap is an OLD arm pack left in place: its shard digests are correct,
    its index agrees with itself, every count comes out right, and it holds
    the previous corpus. Nothing in the release says so.

    So the packed bytes are compared against the arm manifests as they stand
    now. Each member carries the row it was packed from, which names its path
    within the built arm, and that path is what the current manifest is asked
    about. A rebuild changes the digest and a repack is required; a pack left
    behind by a rebuild fails here instead of shipping.
    """
    import tarfile

    indexes = sorted(release.rglob("pentimento-*-arms-index.json"))
    if not indexes:
        report.fail("packed-arms", f"no packed arms index under {release}")
        return

    truth = arm_truth(arm_root)
    if not truth:
        report.fail("packed-arms", f"no arm manifests under {arm_root}, so the "
                                   f"packs have nothing to be checked against")
        return

    stale, unreadable, unknown, checked, tiers = [], [], [], 0, []
    for index_path in indexes:
        index = json.loads(index_path.read_text(encoding="utf-8"))
        tiers.append(index.get("tier", index_path.parent.name))
        for arm_index in index.get("arms", []):
            arm = arm_index["arm"]
            held = truth.get(arm)
            if held is None:
                unknown.append(f"{arm} is packed but no longer built")
                continue
            for shard in arm_index.get("shards", []):
                tar_path = index_path.parent / shard["shard"]
                if not tar_path.is_file():
                    unreadable.append(tar_path.name)
                    continue
                with tarfile.open(tar_path) as tar:
                    names = tar.getnames()
                    stems = sorted({n.rsplit(".", 1)[0] for n in names})
                    for stem in _pick(stems, sample):
                        sidecar = tar.extractfile(f"{stem}.json")
                        payload_name = next(
                            (n for n in names
                             if n.startswith(f"{stem}.")
                             and not n.endswith(".json")), None)
                        if sidecar is None or payload_name is None:
                            unreadable.append(f"{tar_path.name}:{stem}")
                            continue
                        row = json.loads(sidecar.read())
                        rel = row.get("stego") or row.get("file")
                        want = held.get(rel or "")
                        if want is None:
                            unknown.append(f"{arm}:{rel}")
                            continue
                        fh = tar.extractfile(payload_name)
                        if fh is None:
                            unreadable.append(f"{tar_path.name}:{payload_name}")
                            continue
                        checked += 1
                        if hashlib.sha256(fh.read()).hexdigest() != want:
                            stale.append(f"{arm}:{rel}")

    if unreadable:
        report.fail("packed-arms",
                    f"{len(unreadable)} packed shard(s) or member(s) could not "
                    f"be read, e.g. {unreadable[:3]}")
    if unknown:
        report.fail("packed-arms",
                    f"{len(unknown):,} packed sample(s) name something the "
                    f"built arms no longer hold, e.g. {unknown[:3]}. The pack "
                    f"predates a rebuild; repack before publishing")
    if stale:
        report.fail("packed-arms",
                    f"{len(stale):,} packed arm sample(s) differ from the arm "
                    f"that was built, e.g. {stale[:3]}. The pack was made "
                    f"before the arms changed and nothing in it says so; "
                    f"repack before publishing")
    if unreadable or unknown or stale:
        return
    if nothing_checked("packed-arms", checked, report, "packed arm samples"):
        return
    report.note("packed-arms",
                f"{checked:,} arm sample(s) across {len(tiers)} tier(s) match "
                f"the arms as built")


def _pick(items: list[str], sample: int) -> list[str]:
    if len(items) <= sample:
        return items
    return sorted(random.Random(20260921).sample(items, sample))


def _positions(first: int, last: int, sample: int) -> list[int]:
    span = list(range(first, last + 1))
    if len(span) <= sample:
        return span
    return random.Random(20260921).sample(span, sample)


def check_pool(pool: pathlib.Path, expected: int, report: Report) -> None:
    if not pool.is_dir():
        report.fail("pool", f"no JPEG cover pool at {pool}")
        return
    names = sorted(p.name for p in pool.glob("*.jpg"))
    if len(names) != expected:
        report.fail("pool", f"{len(names):,} JPEGs, expected {expected:,}. The "
                            f"adaptive builder indexes this pool by position, "
                            f"so a gap mispairs every cover after it")
    for i, name in enumerate(names):
        if name != f"{i:05d}.jpg":
            report.fail("pool", f"position {i} is {name}, not {i:05d}.jpg")
            break
    if not report.failures.get("pool"):
        report.note("pool", f"{len(names):,} JPEGs, dense and aligned")


def check_digests(rows: list[dict], root: pathlib.Path, sample: int | None,
                  report: Report, check: str = "digests") -> None:
    chosen = rows if sample is None else _sample(rows, sample)
    missing, wrong = [], []
    for row in chosen:
        path = root / row["file"]
        if not path.is_file():
            missing.append(row["file"])
        elif digest(path) != row["sha256"]:
            wrong.append(row["file"])
    if missing:
        report.fail(check, f"{len(missing):,} file(s) named in the manifest are "
                           f"not on disk, e.g. {missing[:3]}")
    if wrong:
        report.fail(check, f"{len(wrong):,} file(s) do not match their recorded "
                           f"digest, e.g. {wrong[:3]}")
    if nothing_checked(check, len(chosen), report, "files"):
        return
    if not missing and not wrong:
        report.note(check, f"{len(chosen):,} checked, all present and matching"
                           + ("" if sample is None else f" (sampled)"))


def _sample(rows: list[dict], n: int) -> list[dict]:
    if len(rows) <= n:
        return rows
    rng = random.Random(20260921)
    return rng.sample(rows, n)


def check_pairs(arm_root: pathlib.Path, sample_per_arm: int,
                report: Report) -> None:
    """Both halves present, and structurally identical containers."""
    manifests = sorted(arm_root.rglob("manifest.jsonl"))
    if not manifests:
        report.fail("pairs", f"no arm manifests under {arm_root}")
        return

    arms, missing, mismatched, checked = set(), [], [], 0
    unknown_format: list[str] = []
    for path in manifests:
        base = path.parent
        by_arm: dict[str, list[dict]] = collections.defaultdict(list)
        for row in load_rows(path):
            by_arm[row.get("arm", "<none>")].append(row)
        for arm, rows in by_arm.items():
            arms.add(arm)
            for row in _sample(rows, sample_per_arm):
                clean, stego = base / row["clean"], base / row["stego"]
                if not (clean.is_file() and stego.is_file()):
                    missing.append(f"{arm}/{row['stego']}")
                    continue
                clean_shape = container_of(clean.read_bytes())
                stego_shape = container_of(stego.read_bytes())
                # A format `container_of` does not understand returns None.
                # Counting that as a match would add it to `checked` and
                # report it clean, which is a check that examined nothing
                # saying everything is fine.
                if clean_shape is None or stego_shape is None:
                    unknown_format.append(f"{arm}/{row['stego']}")
                    continue
                checked += 1
                if clean_shape != stego_shape:
                    mismatched.append(f"{arm}/{row['stego']}")

    if missing:
        report.fail("pairs", f"{len(missing):,} pair(s) have a half that is not "
                             f"on disk, e.g. {missing[:3]}")
    if unknown_format:
        report.fail("pairs",
                    f"{len(unknown_format):,} pair(s) are in a format the "
                    f"container comparison does not understand, e.g. "
                    f"{unknown_format[:3]}. They were NOT examined, and a pair "
                    f"nothing examined must not ship as a verified one")
    if mismatched:
        report.fail("pairs",
                    f"{len(mismatched):,} pair(s) differ in the CONTAINER as "
                    f"well as the payload, e.g. {mismatched[:3]}. A detector "
                    f"reads that difference instead of the payload")
    if nothing_checked("pairs", checked, report, "pairs"):
        return
    if not missing and not mismatched and not unknown_format:
        report.note("pairs", f"{checked:,} pairs across {len(arms)} arms, "
                             f"containers identical")


def check_provenance(arm_root: pathlib.Path, rows: list[dict],
                     report: Report) -> None:
    """Can every arm prove which cover it came from?

    `source_png` is a filename, and `backfill_covers.py` replaces a cover in
    place keeping that filename, so the field cannot witness a swap. The digest
    can, and `stamp_source_digests.py` writes it under a precondition that
    makes it true rather than assumed. A published arm without one is an arm
    whose provenance rests on nobody having replaced anything, which is exactly
    the kind of claim this corpus exists to refuse.
    """
    known = {r["file"]: r["sha256"] for r in rows}

    # The DCT arms are built from the clean JPEG pool, so they carry
    # `source_jpeg` rather than `source_png`. Skipping a row whose direct field
    # is absent exempted all 80,000 of them from this check, and the exemption
    # rendered as a smaller number rather than as a warning.
    jpeg_manifest = arm_root / "jpeg-tools" / "manifest.jsonl"
    jpeg_map = (load_jpeg_cover_map(jpeg_manifest)
                if jpeg_manifest.is_file() else {})

    known_rows = 0
    unstamped, wrong, checked, unresolved = 0, [], 0, 0
    for path in sorted(arm_root.rglob("manifest.jsonl")):
        for row in load_rows(path):
            known_rows += 1
            source, _ = cover_of(row, jpeg_map)
            if not source:
                unresolved += 1
                continue
            stamped = row.get("source_sha256")
            if not stamped:
                unstamped += 1
                continue
            checked += 1
            if known.get(source) != stamped:
                wrong.append(row.get("stego", source))

    if unresolved:
        report.fail("provenance",
                    f"{unresolved:,} arm row(s) name no cover at all, directly "
                    f"or through the clean JPEG pool, so this check could not "
                    f"look at them. Skipping them would report on the rest and "
                    f"read as a pass")
    if unstamped:
        report.fail("provenance",
                    f"{unstamped:,} arm row(s) carry no source_sha256, so "
                    f"nothing in them survives a cover being replaced under "
                    f"the same filename. Run stamp_source_digests.py")
    if wrong:
        report.fail("provenance",
                    f"{len(wrong):,} arm row(s) name a cover digest the "
                    f"manifest does not agree with, e.g. {wrong[:3]}. The arm "
                    f"was built from an image that is no longer there")
    if not unstamped and nothing_checked("provenance", checked, report,
                                         "arm rows"):
        return
    if not unstamped and not wrong and not unresolved:
        # Both numbers, deliberately. "261,997 rows checked" looked clean while
        # 80,000 rows were being skipped for carrying a different field name,
        # and only the total says whether the check saw the whole corpus.
        report.note("provenance", f"{checked:,} of {known_rows:,} rows name "
                                  f"the cover they were built from, by content")


def check_stale(arm_root: pathlib.Path, sample_per_arm: int,
                report: Report) -> None:
    """No arm half differs from the digest its manifest recorded.

    A backfill replaces a cover in place, keeping its filename, so nothing an
    arm manifest records changes when the image underneath it does. The clean
    half's recorded digest is the only witness: rebuild it and the digest
    moves, leave it and the digest still matches a file built from a cover that
    no longer exists.
    """
    manifests = sorted(arm_root.rglob("manifest.jsonl"))
    stale, checked = [], 0
    for path in manifests:
        base = path.parent
        rows = [r for r in load_rows(path) if r.get("clean_sha256")]
        for row in _sample(rows, sample_per_arm):
            clean = base / row["clean"]
            if not clean.is_file():
                continue
            checked += 1
            if digest(clean) != row["clean_sha256"]:
                stale.append(row["clean"])
    if nothing_checked("stale", checked, report, "clean halves"):
        return
    if stale:
        report.fail("stale",
                    f"{len(stale):,} clean half/halves no longer match the "
                    f"digest their arm recorded, e.g. {stale[:3]}")
    else:
        report.note("stale", f"{checked:,} clean halves match their recorded "
                             f"digest")


def check_figures(docs: pathlib.Path, release: pathlib.Path,
                  covers: pathlib.Path, report: Report) -> None:
    """Do the published figures match the corpus that is about to ship?

    The other checks read the manifest. Nothing read the PROSE derived from it,
    and prose is what a user acts on. A stale `licence-summary.json` put "5,429
    covers require attribution" into the shipped README while the manifest said
    5,453, and every check here passed, because none of them was looking at a
    sentence.

    `check_docs_figures` derives each figure from the packed index rather than
    restating it, so this cannot drift into agreeing with itself.
    """
    try:
        from check_docs_figures import FigureError, check, derive
    except ImportError as e:  # pragma: no cover - the import is the contract
        report.fail("figures", f"the figure check could not be imported "
                               f"({e}), so no published number was compared "
                               f"with the corpus")
        return
    try:
        figures = derive(release, covers)
        problems, notes = check(docs, figures)
    except FigureError as e:
        report.fail("figures", str(e))
        return
    for problem in problems:
        report.fail("figures", problem)
    if not problems:
        checked = sum(1 for n in notes if "as shipped" in n)
        if nothing_checked("figures", checked, report, "published figures"):
            return
        report.note("figures", f"{checked} published figure(s) match the "
                               f"corpus that shipped")


def check_links(release: pathlib.Path, report: Report,
                fetch=fetch_status, cap: int = URL_CAP,
                timeout: float = URL_TIMEOUT) -> None:
    """Does every address in the shipped prose answer?

    `DATASHEET.md` went to three public mirrors telling photographers to open
    an issue at `https://github.com/elementmerc/pentimento`, and that address
    was a 404 for the whole life of the release, because the repository was
    private. `CITATION.cff` names the same one as `repository-code`. A forensic
    analyst reviewing the corpus called a dead objection channel the single
    thing that would fail their chain of custody standard, and no check here
    was looking at a link.

    Every other check reads bytes on this machine. This one reaches off it, so
    it runs only when asked for, and it separates the three answers a link can
    give: gone, alive, and nobody said. The middle one is the release's
    problem; the last one is usually the checker's.
    """
    try:
        from release_metadata import PUBLISHED_EXTRAS
    except ImportError as e:  # pragma: no cover - the import is the contract
        report.fail("links", f"the list of published files could not be "
                             f"imported ({e}), so no shipped link was checked "
                             f"and a hardcoded list would drift from it")
        return

    if not release.is_dir():
        report.fail("links", f"no release directory at {release}")
        return

    wanted = {name for name in PUBLISHED_EXTRAS
              if name.endswith(PROSE_SUFFIXES)}
    documents = sorted(p for p in release.rglob("*") if p.name in wanted)
    if not documents:
        report.fail("links", f"no published prose under {release}, so this "
                             f"check had nothing to read. The release ships "
                             f"{len(wanted)} such file(s) per tier")
        return

    seen: dict[str, list[str]] = collections.OrderedDict()
    oversized = []
    for path in documents:
        try:
            size = path.stat().st_size
        except OSError as e:
            report.fail("links", f"{path.name} could not be read ({e})")
            continue
        if size > PROSE_BYTES_CAP:
            oversized.append(f"{path.name} ({size:,} bytes)")
            continue
        where = f"{path.parent.name}/{path.name}"
        text = path.read_text(encoding="utf-8", errors="replace")
        for url in urls_in(text):
            origins = seen.setdefault(url, [])
            if where not in origins:
                origins.append(where)

    if oversized:
        report.fail("links", f"{len(oversized)} published document(s) are "
                             f"larger than the {PROSE_BYTES_CAP:,} byte read "
                             f"cap and were NOT scanned, e.g. {oversized[:3]}")

    chosen = _spread(list(seen), cap)
    dead, unreachable, unsettled, checked = [], [], [], 0
    for url in chosen:
        status, how = fetch(url, timeout)
        checked += 1
        origin = ", ".join(seen[url][:2])
        if status is None:
            unreachable.append(f"{url} (in {origin}): {how}")
        elif status in UNSETTLED_STATUSES or status >= 500:
            unsettled.append(f"{url} (in {origin}): {status}")
        elif status >= 400:
            dead.append(f"{url} (in {origin}): {status}")

    if dead:
        report.fail("links",
                    f"{len(dead):,} address(es) in the shipped prose do not "
                    f"exist: {dead[:3]}. A reader following one of these has "
                    f"no way to reach us, and the objection channel a licence "
                    f"depends on is the worst of them to lose")
    if unreachable:
        report.fail("links",
                    f"{len(unreachable):,} address(es) gave no answer at all: "
                    f"{unreachable[:3]}. That is NOT the same as a 404. The "
                    f"page may be perfectly alive and this machine off the "
                    f"network, so confirm connectivity before believing it")
    if dead or unreachable or oversized:
        return
    # Settled, not checked. A run where every host rate-limited us looked at
    # 200 addresses and proved nothing about any of them, and "200 addresses
    # answer" is the clean line people act on.
    settled = checked - len(unsettled)
    if nothing_checked("links", settled, report,
                       "addresses that gave a usable answer"):
        return

    tail = ""
    if unsettled:
        # A rate limit is the host declining to talk this minute. Failing the
        # release over it would teach everyone to pass --check-urls twice and
        # believe the second answer, which is worse than saying what happened.
        tail = (f"; {len(unsettled)} host(s) declined to answer this run "
                f"(rate limit or server error), e.g. {unsettled[:2]}, and "
                f"were NOT proven either way")
    if len(seen) > len(chosen):
        tail += (f"; capped at {cap:,} of {len(seen):,} distinct address(es), "
                 f"spread across hosts")
    report.note("links", f"{settled:,} address(es) across {len(documents)} "
                         f"published document(s) answer{tail}")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--covers", required=True)
    ap.add_argument("--arms", default=None)
    ap.add_argument("--release", default=None,
                    help="the packed release directory, to check for a stale pack")
    ap.add_argument("--jpeg-pool", default=None,
                    help="the JPEG cover pool the adaptive arms index")
    ap.add_argument("--expect", type=int, default=10000)
    ap.add_argument("--sample", type=int, default=500,
                    help="covers to digest-check; --full overrides")
    ap.add_argument("--sample-per-arm", type=int, default=40)
    ap.add_argument("--docs", default=None,
                    help="a documentation directory whose published figures "
                         "should match this corpus, e.g. pentimento/docs or "
                         "the release directory itself")
    ap.add_argument("--full", action="store_true",
                    help="check every file rather than a sample")
    ap.add_argument("--check-urls", action="store_true",
                    help="fetch every address in the shipped prose, which is "
                         "the only check that needs a network")
    args = ap.parse_args(argv)

    # Line buffering is so a long run's progress reaches a tail as it happens.
    # A redirected stdout may not be a real stream, and losing the buffering is
    # a cosmetic loss where crashing on it would be a real one.
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(line_buffering=True)
    covers = pathlib.Path(args.covers)
    manifest = covers / "manifest.jsonl"
    if not manifest.is_file():
        print(f"no cover manifest at {manifest}", file=sys.stderr)
        return 1
    rows = load_rows(manifest)

    report = Report()
    check_covers(rows, args.expect, report)
    check_licences(rows, report)
    check_digests(rows, covers, None if args.full else args.sample, report)
    if args.release:
        check_packed(pathlib.Path(args.release), rows,
                     10 ** 9 if args.full else args.sample_per_arm, report)
        check_attribution(pathlib.Path(args.release), rows, report)
    else:
        report.skip("packed", "no --release given, so the packed archives were "
                              "never opened and a stale pack would not show")
        report.skip("attribution",
                    "no --release given, so the shipped credit lists were "
                    "never compared with the covers that require one")
    if args.jpeg_pool:
        check_pool(pathlib.Path(args.jpeg_pool), args.expect, report)
    else:
        report.skip("pool", "no --jpeg-pool given, so nothing confirmed the "
                            "pool the adaptive arms index by POSITION is dense")
    if args.arms:
        arm_root = pathlib.Path(args.arms)
        per_arm = 10 ** 9 if args.full else args.sample_per_arm
        check_pairs(arm_root, per_arm, report)
        check_stale(arm_root, per_arm, report)
        check_provenance(arm_root, rows, report)
        if args.release:
            check_packed_arms(pathlib.Path(args.release), arm_root,
                              per_arm, report)
        else:
            report.skip("packed-arms",
                        "no --release given, so the packed arms were never "
                        "opened and an arm pack left behind by a rebuild "
                        "would not show")
    else:
        for check in ("pairs", "stale", "provenance"):
            report.skip(check, "no --arms given, so no stego pair was examined")
        report.skip("packed-arms",
                    "no --arms given, so the packed arms had nothing current "
                    "to be compared against")

    if args.docs and args.release:
        check_figures(pathlib.Path(args.docs), pathlib.Path(args.release),
                      covers, report)
    else:
        report.skip("figures", "no --docs given, so no published number was "
                               "compared with the corpus it describes")

    if args.check_urls and args.release:
        check_links(pathlib.Path(args.release), report)
    elif args.check_urls:
        report.skip("links", "no --release given, so there was no shipped "
                             "prose to take addresses from")
    else:
        report.skip("links", "no --check-urls given, so no published address "
                             "was fetched and a dead objection channel would "
                             "not show. This one needs a network")

    order = ["covers", "licences", "digests", "pool", "pairs", "stale",
             "provenance", "packed", "packed-arms", "attribution", "figures",
             "links"]
    print()
    for check in order:
        if check in report.failures:
            print(f"  FAIL  {check}")
            for message in report.failures[check]:
                print(f"          {message}")
        elif check in report.notes:
            print(f"  ok    {check:9} {report.notes[check]}")
        elif check in report.skipped:
            print(f"  ----  {check:9} NOT RUN: {report.skipped[check]}")

    if report.ok:
        # Named, not counted. "every checked invariant holds" is true of a run
        # that checked one thing, and the reader acts on the sentence rather
        # than on which arguments they happened to pass.
        if report.skipped:
            print(f"\nevery invariant that ran holds, but {len(report.skipped)} "
                  f"did NOT run: {', '.join(sorted(report.skipped))}. This is "
                  f"not a clean bill for the corpus, only for what was looked "
                  f"at.")
            return 0
        print("\nevery checked invariant holds"
              + ("" if args.full else "; run --full before publishing"))
        return 0
    print(f"\n{sum(len(v) for v in report.failures.values())} problem(s) across "
          f"{len(report.failures)} check(s). This corpus is not fit to publish.",
          file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
