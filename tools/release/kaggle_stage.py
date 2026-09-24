#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Choose, and stage, exactly the files that go to Kaggle.

WHY A STAGING DIRECTORY AT ALL
-------------------------------
Kaggle builds a dataset version from a WHOLE DIRECTORY and has no exclude
flag, so whatever sits in the directory you point it at goes public. Pointing
it at the packed release directory therefore published `ia-metadata.json`,
which is an instruction file for the Internet Archive and no part of the
corpus. That is the same fault that once put two unpublishable names into
`SHA256SUMS-covers`, and it was fixed there the same way it is fixed here: by
NAMING the published set rather than taking whatever the directory happens to
hold.

So nothing is chosen by looking at the directory. The shards come from the
pack index, the documents come from `release_metadata.PUBLISHED_EXTRAS`, and
the directory is consulted only to prove that each of those is really there
and to say what was left behind.

THE TWO CONTROL FILES, WHICH ARE NOT THE SAME
----------------------------------------------
Both are written for a host rather than for a reader, and they are treated
differently because the hosts read them differently.

`dataset-metadata.json` IS STAGED. The Kaggle client reads it out of the
directory it is given to learn the dataset's id, title, licence and
description (`get_dataset_metadata_file`), and refuses the push without it.
It does not upload it: `upload_files` skips that exact name. So it has to be
in the directory and it does not become a public file.

`ia-metadata.json` IS NOT STAGED. It is headers for the Archive's first PUT.
Kaggle has never read it, no downloader has any use for it, and it only ever
reached the public copy because it was sitting in the directory.

WHY HARD LINKS
--------------
A Core tier is about 3.3 GB of covers. Copying that for every publish is real
minutes and real disk, so each staged file is hard linked when the filesystem
allows it and copied when it does not. A hard link is a second name for the
same inode, indistinguishable from an ordinary file to anything that opens
it, so no client can mishandle one. (A symlink would also work here: the
client tests with `os.path.isfile`, which follows them. A hard link needs no
such argument.) Hard linking needs one filesystem, which is why the caller
puts the staging directory beside the release rather than in /tmp.

Usage::

    python3 tools/release/kaggle_stage.py --packed ~/pentimento/release/core \\
        --stage /path/to/staging
    python3 tools/release/kaggle_stage.py --packed ... --dry-run
"""
from __future__ import annotations

import argparse
import collections
import fnmatch
import json
import os
import pathlib
import shutil
import sys
from dataclasses import dataclass

#: The canonical list of published documents lives in the generators, and this
#: imports it rather than keeping a copy. `upload_tier.py` keeps a copy because
#: it is bind-mounted alone into a container that cannot see `generators/`;
#: this script has no such constraint, it runs on the host beside the whole
#: repository, and a third copy would only be a third thing to drift.
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[2] / "generators"))
try:
    from release_metadata import PUBLISHED_EXTRAS  # noqa: E402
except ImportError as exc:  # pragma: no cover - the import is the contract
    raise ImportError(
        "the list of published files could not be imported from "
        f"generators/release_metadata.py ({exc}). A hardcoded copy here would "
        "drift from it, so this refuses to guess."
    ) from exc

#: The pack index ships, and its name carries the tier, so it is matched rather
#: than named. This is the same rule as `release_metadata.publishes`.
INDEX_GLOB = "pentimento-*-index.json"

#: Kaggle reads this out of the pushed directory and skips uploading it, so it
#: is staged without ever becoming a published file. See the module docstring.
KAGGLE_CONTROL = "dataset-metadata.json"

#: Published documents that a legitimate release may not have written.
#: `release_metadata` writes `SHA256SUMS-arms` only when the tier was packed
#: with an arms index, so a covers-only release has none and refusing over it
#: would block a publish that is completely correct. Everything else in
#: `PUBLISHED_EXTRAS` is required.
OPTIONAL_EXTRAS = ("SHA256SUMS-arms",)

#: WHY THE SHARDS ARRIVE UNDER A DIFFERENT NAME.
#:
#: Kaggle extracts uploaded archives and offers no way to refuse, so the Core
#: cover tier landed as 20,014 loose files: ten shards expanded into 20,000
#: images and sidecars. On 2026-09-24 that broke the dataset outright. Kaggle's
#: own file listing returned HTTP 500 partway through enumerating them and the
#: Data Card stopped rendering, so a visitor was told the data was
#: inaccessible while the data was perfectly fine.
#:
#: Measured on a throwaway dataset, since deleted: Kaggle extracts `.tar` and
#: nothing else. `.tar.bin`, `.bin`, `.shard` and `.tardata` were all stored
#: intact. `.tar.bin` is the one used here because it keeps `tar` legible in
#: the name while saying the file is a stored blob.
#:
#: The shipped reader needs no change: `tarfile` sniffs the content rather
#: than trusting the extension, which was verified by reading a renamed shard
#: end to end.
KAGGLE_SHARD_SUFFIX = ".bin"

#: The checksum files, which cannot be staged verbatim once the shards are
#: renamed: they would name containers that are not on that mirror, which is
#: the exact defect that made `sha256sum -c` fail on the Kaggle copy for
#: months. They are rewritten instead, digests untouched and names corrected.
CHECKSUM_EXTRAS = ("SHA256SUMS-covers", "SHA256SUMS-arms")


def kaggle_name(name: str) -> str:
    """What a staged file is called on Kaggle.

    Only `.tar` moves, because only `.tar` is unpacked. Renaming anything else
    would be a cost with no purchase, and a reader has to be able to recognise
    what they downloaded.
    """
    return name + KAGGLE_SHARD_SUFFIX if name.endswith(".tar") else name


class StagingRefused(Exception):
    """The release directory cannot be staged, and nothing has been staged.

    Its own exception because every one of these means the same thing to the
    operator: stop, because half a corpus published is worse than none.
    """


@dataclass(frozen=True)
class Plan:
    """What would be sent, and what would deliberately not be."""

    packed: pathlib.Path
    indexes: tuple[str, ...]
    shards: tuple[str, ...]
    extras: tuple[str, ...]
    control: tuple[str, ...]
    absent_optional: tuple[str, ...]
    excluded: tuple[str, ...]

    @property
    def sources(self) -> tuple[str, ...]:
        """Every name READ FROM the release directory, sorted.

        Distinct from `staged`, because a shard is read as `.tar` and lands as
        `.tar.bin`. Keeping the two apart is what stops a rename being applied
        twice or looked for in the wrong directory.
        """
        return tuple(sorted(self.indexes + self.shards + self.extras + self.control))

    @property
    def staged(self) -> tuple[str, ...]:
        """Every name that goes INTO the staging directory, sorted."""
        return tuple(sorted(kaggle_name(n) for n in self.sources))

    @property
    def published(self) -> tuple[str, ...]:
        """The staged names that become public files, so without the control file."""
        return tuple(sorted(kaggle_name(n)
                            for n in self.indexes + self.shards + self.extras))


def _shard_names(index_path: pathlib.Path) -> list[str]:
    """The shard filenames an index declares, covers and arms alike."""
    try:
        index = json.loads(index_path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        raise StagingRefused(
            f"{index_path.name} could not be read as JSON ({exc}). It names "
            f"every shard that publishes, so nothing can be staged without it."
        ) from exc
    if not isinstance(index, dict):
        raise StagingRefused(
            f"{index_path.name} is not a JSON object, so it cannot be an index")

    groups = [index.get("shards", [])]
    for arm in index.get("arms", []) or []:
        if isinstance(arm, dict):
            groups.append(arm.get("shards", []))

    names: list[str] = []
    for group in groups:
        if not isinstance(group, list):
            raise StagingRefused(
                f"{index_path.name} has a shard list that is not a list")
        for entry in group:
            if not isinstance(entry, dict) or "shard" not in entry:
                raise StagingRefused(
                    f"{index_path.name} has a shard entry with no 'shard' key, "
                    f"so the published set cannot be derived from it")
            names.append(_safe_name(entry["shard"], index_path.name))
    return names


def _safe_name(name: object, source: str) -> str:
    """A filename from a data file, checked before it is joined to a path.

    The index is written on this machine, but it is still a file being read,
    and a name carrying a separator would stage something outside the release.
    """
    if not isinstance(name, str) or not name:
        raise StagingRefused(f"{source} names a shard that is not a filename")
    if name != pathlib.PurePosixPath(name).name or name in (".", ".."):
        raise StagingRefused(
            f"{source} names a shard as {name!r}, which is a path rather than "
            f"a filename. Nothing was staged.")
    return name


def plan(packed: pathlib.Path) -> Plan:
    """Work out the published set, and prove every member is on disk.

    Raises `StagingRefused` before anything is staged if it is not.
    """
    packed = pathlib.Path(packed)
    if not packed.is_dir():
        raise StagingRefused(f"no packed release directory at {packed}")

    present = {p.name for p in packed.iterdir()}
    files = {p.name for p in packed.iterdir() if p.is_file()}

    indexes = tuple(sorted(n for n in files if fnmatch.fnmatchcase(n, INDEX_GLOB)))
    if not indexes:
        raise StagingRefused(
            f"no pack index matching {INDEX_GLOB} in {packed}. The index names "
            f"every shard that publishes, so without it there is no published "
            f"set to stage.")

    shards: list[str] = []
    for name in indexes:
        shards.extend(_shard_names(packed / name))
    duplicates = sorted(n for n, count in collections.Counter(shards).items()
                        if count > 1)
    if duplicates:
        raise StagingRefused(
            f"the pack index or indexes name the same shard more than once "
            f"({', '.join(duplicates[:3])}), so the published set is ambiguous")

    missing = [n for n in shards if n not in files]
    extras: list[str] = []
    absent_optional: list[str] = []
    for name in PUBLISHED_EXTRAS:
        if name in files:
            extras.append(name)
        elif name in OPTIONAL_EXTRAS:
            absent_optional.append(name)
        else:
            missing.append(name)

    if missing:
        raise StagingRefused(
            f"{len(missing)} file(s) of the published set are not in {packed}: "
            f"{', '.join(sorted(missing)[:6])}"
            f"{' ...' if len(missing) > 6 else ''}. Nothing was staged, "
            f"because half a corpus published is worse than none.")

    control = (KAGGLE_CONTROL,) if KAGGLE_CONTROL in files else ()
    if not control:
        raise StagingRefused(
            f"no {KAGGLE_CONTROL} in {packed}. Kaggle reads it for the "
            f"dataset's id, title, licence and description, and refuses the "
            f"push without it.")

    staged = set(indexes) | set(shards) | set(extras) | set(control)
    excluded = tuple(sorted(
        n + ("/" if (packed / n).is_dir() else "") for n in present if n not in staged))

    return Plan(
        packed=packed,
        indexes=indexes,
        shards=tuple(sorted(shards)),
        extras=tuple(extras),
        control=control,
        absent_optional=tuple(absent_optional),
        excluded=excluded,
    )


def retarget_checksums(body: str, renamed: dict[str, str]) -> str:
    """Point a `sha256sum -c` file at the names this mirror actually carries.

    Digests are never touched. The bytes of a shard are identical whatever it
    is called, so a rewritten line is the same claim about the same file, said
    in the name a reader will type.

    Only names in `renamed` move. A line naming something this staging does
    not carry is left exactly as it was rather than guessed at: the arms
    checksum file, for instance, names shards that a covers-only publish never
    sends, and inventing a `.tar.bin` for one of those would assert something
    about a file nobody can download here.
    """
    out = []
    for line in body.splitlines():
        # `sha256sum` writes two spaces between the digest and the name, and
        # a name may itself contain spaces, so the split is bounded.
        digest, sep, name = line.partition("  ")
        if sep and name in renamed:
            out.append(f"{digest}{sep}{renamed[name]}")
        else:
            out.append(line)
    return "\n".join(out) + "\n" if out else ""


def stage(packed: pathlib.Path, staging: pathlib.Path,
          prepared: Plan | None = None) -> dict[str, str]:
    """Put exactly the planned set into `staging`, linked where possible.

    Returns each staged name mapped to "link" or "copy". Re-running over a
    directory that was staged before is safe: every planned name is replaced
    and anything else left there is removed, so an interrupted run cannot
    leave a stale file to be published by the next one.
    """
    prepared = prepared or plan(packed)
    staging = pathlib.Path(staging)
    staging.mkdir(parents=True, exist_ok=True)

    wanted = set(prepared.staged)
    for stale in staging.iterdir():
        if stale.name not in wanted:
            if stale.is_dir() and not stale.is_symlink():
                shutil.rmtree(stale)
            else:
                stale.unlink()

    renamed = {n: kaggle_name(n) for n in prepared.shards}
    how: dict[str, str] = {}
    for name in prepared.sources:
        source = prepared.packed / name
        landed = kaggle_name(name)
        destination = staging / landed
        if destination.exists() or destination.is_symlink():
            destination.unlink()
        if name in CHECKSUM_EXTRAS:
            # Rewritten rather than linked, because its CONTENTS name the
            # files a reader checks and those names have changed. Linking it
            # would publish a checksum file listing containers that are not on
            # this mirror, and a checksum file that cries wolf is worse than
            # none: the reader who meets one either stops trusting the corpus
            # or stops running the check.
            destination.write_text(
                retarget_checksums(source.read_text(encoding="utf-8"), renamed),
                encoding="utf-8")
            how[landed] = "rewritten"
            continue
        try:
            os.link(source, destination)
            how[landed] = "link"
        except OSError:
            # Different filesystem, or a filesystem with no hard links. The
            # bytes are copied instead and the caller says so, because an
            # unexplained several-minute pause on a 3.3 GB tier reads as a
            # hang.
            shutil.copy2(source, destination)
            how[landed] = "copy"
    return how


def describe(prepared: Plan, how: dict[str, str] | None = None) -> str:
    """The report an operator reads before a publish, dry run or live."""
    lines = [f"staged from {prepared.packed}:"]
    sample = ", ".join(kaggle_name(n) for n in prepared.shards[:2])
    if prepared.shards:
        lines.append(f"  {len(prepared.shards):>4} tar shard(s)  e.g. {sample}")
        # Said in the report, not only in a comment. The rename is the whole
        # reason this mirror stopped shattering into 20,000 files, and an
        # operator who does not see it happen cannot notice it stopping.
        lines.append(f"       renamed to *.tar{KAGGLE_SHARD_SUFFIX}, because "
                     f"Kaggle unpacks .tar and nothing else")
    for name in prepared.indexes:
        lines.append(f"     1 pack index    {name}")
    for name in prepared.extras:
        note = ("  (rewritten to name the shards as they arrive)"
                if name in CHECKSUM_EXTRAS else "")
        lines.append(f"       document      {name}{note}")
    for name in prepared.control:
        lines.append(f"       control file  {name}  "
                     f"(Kaggle reads it, and does not upload it)")
    if prepared.absent_optional:
        lines.append("  not written by this release, which is allowed: "
                     + ", ".join(prepared.absent_optional))

    lines.append("")
    # A SILENT EXCLUSION IS HOW THE OPPOSITE BUG STARTS. The published set is
    # named, so a newly added published file would be dropped without a word
    # unless every leftover is printed for somebody to look at.
    if prepared.excluded:
        lines.append(f"found in the release directory and NOT sent "
                     f"({len(prepared.excluded)}):")
        for name in prepared.excluded:
            note = ""
            if name == "ia-metadata.json":
                note = "  (instructions for the Internet Archive, not corpus)"
            lines.append(f"    {name}{note}")
        lines.append("  If any of those should publish, it belongs in "
                     "release_metadata.PUBLISHED_EXTRAS, not in this script.")
    else:
        lines.append("nothing in the release directory was left behind.")

    if how:
        linked = sum(1 for v in how.values() if v == "link")
        copied = len(how) - linked
        lines.append("")
        lines.append(f"{linked} file(s) hard linked, {copied} copied.")
        if copied:
            lines.append("  Copied files could not be linked, usually because "
                         "the staging directory is on another filesystem.")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--packed", required=True, type=pathlib.Path,
                        help="the packed release directory to publish from")
    parser.add_argument("--stage", type=pathlib.Path,
                        help="the directory to stage into; required unless "
                             "--dry-run")
    parser.add_argument("--dry-run", action="store_true",
                        help="report the set and stage nothing")
    args = parser.parse_args(argv)

    if not args.dry_run and args.stage is None:
        parser.error("--stage is required unless --dry-run is given")

    try:
        prepared = plan(args.packed)
        how = None if args.dry_run else stage(args.packed, args.stage, prepared)
    except StagingRefused as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    except OSError as exc:
        print(f"error: staging failed ({exc}). Nothing was sent.", file=sys.stderr)
        return 1

    print(describe(prepared, how))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
