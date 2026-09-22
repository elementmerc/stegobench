#!/usr/bin/env python3
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
import sys

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
                checked += 1
                if container_of(clean.read_bytes()) != container_of(
                        stego.read_bytes()):
                    mismatched.append(f"{arm}/{row['stego']}")

    if missing:
        report.fail("pairs", f"{len(missing):,} pair(s) have a half that is not "
                             f"on disk, e.g. {missing[:3]}")
    if mismatched:
        report.fail("pairs",
                    f"{len(mismatched):,} pair(s) differ in the CONTAINER as "
                    f"well as the payload, e.g. {mismatched[:3]}. A detector "
                    f"reads that difference instead of the payload")
    if nothing_checked("pairs", checked, report, "pairs"):
        return
    if not missing and not mismatched:
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
    else:
        report.skip("packed", "no --release given, so the packed archives were "
                              "never opened and a stale pack would not show")
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
    else:
        for check in ("pairs", "stale", "provenance"):
            report.skip(check, "no --arms given, so no stego pair was examined")

    if args.docs and args.release:
        check_figures(pathlib.Path(args.docs), pathlib.Path(args.release),
                      covers, report)
    else:
        report.skip("figures", "no --docs given, so no published number was "
                               "compared with the corpus it describes")

    order = ["covers", "licences", "digests", "pool", "pairs", "stale",
             "provenance", "packed", "figures"]
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
