#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Generate the Homebrew formula and the Scoop manifest from a release.

    python3 tools/release/generate_packaging.py \\
        --version 1.0.0 --sums dist/SHA256SUMS --out packaging

WHY THIS IS GENERATED AND NOT WRITTEN BY HAND
----------------------------------------------
A Homebrew formula and a Scoop manifest are each four facts about a release:
the version, the URL of an archive, the digest of that archive, and the path
of the binary inside it. Every one of those changes at every tag. A file
carrying all four, maintained by hand, is correct until the first release
somebody is in a hurry during, and the failure is quiet: the formula still
installs, it just installs the previous version, or it fails a checksum on a
stranger's machine rather than on ours.

So the release produces `SHA256SUMS` and this reads it. There is one source of
truth for the digests and it is the file the workflow signed.

WHERE THE OUTPUT GOES
---------------------
`packaging/` in this repository holds an emitted example, so a reader can see
the shape without running a release. The real destinations are two separate
repositories that hold nothing else:

    elementmerc/homebrew-tap   Formula/stegobench.rb
    elementmerc/scoop-bucket   bucket/stegobench.json

Both are our own, so neither needs anybody's review. Writing to them is a
`git push` from the release job, not this script's job; this script only ever
writes files into `--out`.

WHAT IT REFUSES TO DO
---------------------
It never emits a placeholder. A formula with an empty `sha256` installs
nothing and reads like an oversight somebody will fix later; a formula that was
never written is a failure at tag time, which is when it is cheap. So a missing
digest, a malformed digest line, an unrecognised target triple among the
release's archives, and a version that does not look like a version are all
hard errors that name what was wrong and what to do about it.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys
from typing import Dict, NamedTuple

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from release_facts import FactsError  # noqa: E402
from release_facts import load as _load_facts  # noqa: E402

#: Read at import, because a formula cannot be emitted correctly without it and
#: failing here names the field rather than publishing a blank. These three
#: names used to be literals in this file; they moved so that a Homebrew
#: formula, a Scoop manifest, a citation and a codemeta document cannot state
#: three different descriptions of one project.
try:
    FACTS = _load_facts()
except FactsError as _exc:  # pragma: no cover - exercised by the drift test
    raise SystemExit(f"cannot generate packaging: {_exc}") from None

#: The repository the release assets come from. Everything the two package
#: managers download is under this.
REPO_URL = FACTS.repository

#: Kept deliberately short. Homebrew shows it in `brew info` and Scoop in
#: `scoop search`, both of which truncate, and Homebrew's own style guide
#: refuses a description that starts with the formula's own name or with an
#: article. `release_facts` enforces both of those rules on the way in.
DESCRIPTION = FACTS.description

#: The SPDX identifier, the same one the crates and the LICENSE carry. Scoop
#: takes the string as it stands; Homebrew parses it as an SPDX expression.
LICENCE = FACTS.licence

#: A version as the tag carries it, minus the `v`. Pre-release and build
#: metadata are allowed because Cargo allows them, and a release named
#: `1.0.0-rc.1` should package rather than be rejected by this file's opinion
#: of what a version looks like.
VERSION_PATTERN = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.\-]+)?$")

#: sha256sum writes a 64 character lower-case hex digest, two spaces, then the
#: name. The second space is a binary-mode marker in some implementations and
#: an asterisk in others, so both are accepted.
SUMS_LINE = re.compile(r"^(?P<digest>[0-9a-f]{64})\s[\s*](?P<name>\S.*)$")

#: An archive name as `release.yml` assembles it:
#:
#:     name="stegobench-${TAG}-${{ matrix.target }}"
#:
#: with `TAG` being `github.ref_name`, so `stegobench-v1.0.0-<triple>.tar.gz`
#: or `.zip`. This pattern recognises a release archive among the SBOM files
#: and the signatures that sit beside it in the same directory. It deliberately
#: does not try to split the version from the target: both halves contain
#: hyphens and any regexp that guesses where the boundary falls gets
#: `x86_64-unknown-linux-musl` wrong. The version being generated for is known,
#: so the split is done against that instead.
ARCHIVE_NAME = re.compile(r"^stegobench-v(?P<rest>.+?)(?:\.tar\.gz|\.zip)$")


class Platform(NamedTuple):
    """One archive the release workflow builds, and what it is called."""

    #: The `matrix.target` value in `release.yml`. Not always a rustup triple:
    #: `universal-apple-darwin` is a name this project invented for the archive
    #: holding the lipo-joined Intel and Apple silicon binary.
    target: str
    #: The archive extension the workflow gives this one.
    suffix: str
    #: The binary's name inside the archive.
    binary: str


#: Every platform the release workflow produces an archive for. The keys are
#: this script's own vocabulary; the targets are `release.yml`'s.
PLATFORMS: Dict[str, Platform] = {
    "linux-x86_64": Platform("x86_64-unknown-linux-musl", ".tar.gz", "stegobench"),
    "linux-aarch64": Platform("aarch64-unknown-linux-musl", ".tar.gz", "stegobench"),
    "macos-universal": Platform("universal-apple-darwin", ".tar.gz", "stegobench"),
    "windows-x86_64": Platform("x86_64-pc-windows-msvc", ".zip", "stegobench.exe"),
}

#: The reverse map, used to notice an archive in SHA256SUMS that this script
#: does not know how to package.
TARGETS_TO_PLATFORM: Dict[str, str] = {p.target: key for key, p in PLATFORMS.items()}


class PackagingError(Exception):
    """Anything that makes a correct formula or manifest impossible to emit."""


class UnknownPlatformError(PackagingError):
    """A platform key, or a target triple, that this script has no entry for."""


def platform(key: str) -> Platform:
    """Look up a platform, refusing one this script does not know.

    The lookup is a function rather than a bare subscript so the failure names
    what it did not recognise and what the known set is. A `KeyError` reading
    `'linux-x86-64'` in a release job at tag time is a worse thirty seconds
    than it needs to be.
    """
    try:
        return PLATFORMS[key]
    except KeyError:
        known = ", ".join(sorted(PLATFORMS))
        raise UnknownPlatformError(
            f"no such platform {key!r}. Known platforms: {known}."
        ) from None


def archive_name(version: str, key: str) -> str:
    """The asset name `release.yml` gives this platform's archive."""
    spec = platform(key)
    return f"stegobench-v{version}-{spec.target}{spec.suffix}"


def archive_url(version: str, key: str) -> str:
    """The GitHub release download URL for this platform's archive."""
    return f"{REPO_URL}/releases/download/v{version}/{archive_name(version, key)}"


def extract_dir(version: str, key: str) -> str:
    """The single directory the workflow puts inside each archive.

    `release.yml` builds `dist/${name}/` and then archives `${name}`, so every
    archive has exactly one top-level directory named after itself without the
    extension. Homebrew strips that automatically; Scoop has to be told.
    """
    spec = platform(key)
    return f"stegobench-v{version}-{spec.target}"


def parse_sums(text: str) -> Dict[str, str]:
    """Read a `SHA256SUMS` file into a name to digest map.

    Blank lines are skipped. Anything else that is not a digest and a name is
    an error rather than something to step over: a line this cannot read is a
    line whose digest is not in the map, and a digest that is silently absent
    is exactly the failure this whole script exists to prevent.
    """
    digests: Dict[str, str] = {}
    for number, line in enumerate(text.splitlines(), start=1):
        if not line.strip():
            continue
        match = SUMS_LINE.match(line)
        if match is None:
            raise PackagingError(
                f"SHA256SUMS line {number} is not a sha256sum line: {line!r}. "
                "Expected 64 hex characters, two spaces, then a file name."
            )
        name = match.group("name").strip()
        digest = match.group("digest")
        if name in digests and digests[name] != digest:
            raise PackagingError(
                f"SHA256SUMS lists {name} twice with different digests "
                f"({digests[name]} and {digest}). One of them is wrong and "
                "this cannot tell which."
            )
        digests[name] = digest
    if not digests:
        raise PackagingError(
            "SHA256SUMS is empty. Nothing can be packaged from it, and a "
            "formula generated from nothing would install nothing."
        )
    return digests


def digest_for(digests: Dict[str, str], version: str, key: str) -> str:
    """The digest of one platform's archive, or a refusal naming the gap."""
    name = archive_name(version, key)
    digest = digests.get(name)
    if digest is None:
        listed = ", ".join(sorted(digests)) or "nothing"
        raise PackagingError(
            f"SHA256SUMS has no entry for {name}, so its digest cannot be "
            "filled in and no formula will be written. Check that the release "
            f"built that platform. SHA256SUMS lists: {listed}."
        )
    return digest


def check_version(version: str) -> str:
    """Refuse a version that would put something odd into a URL."""
    if not VERSION_PATTERN.match(version):
        raise PackagingError(
            f"{version!r} does not look like a release version. Pass it as the "
            "tag carries it without the leading v, for example 1.0.0."
        )
    return version


def check_no_unknown_archives(digests: Dict[str, str], version: str) -> None:
    """Refuse a release carrying an archive this script cannot package.

    The point is the release that adds a fifth build target. Without this, the
    formula and the manifest would simply carry on describing four platforms
    and nobody would notice the fifth was never packaged until a user on it
    asked why. This turns that into a failure at tag time with the triple
    named.
    """
    prefix = f"{version}-"
    unknown = []
    for name in sorted(digests):
        match = ARCHIVE_NAME.match(name)
        if match is None:
            continue
        rest = match.group("rest")
        if not rest.startswith(prefix):
            raise PackagingError(
                f"SHA256SUMS carries {name}, which is not version {version}. "
                "Generating packaging from a mixed directory would publish a "
                "digest for one release under another release's URL."
            )
        target = rest[len(prefix):]
        if target not in TARGETS_TO_PLATFORM:
            unknown.append(target)
    if unknown:
        raise PackagingError(
            "the release carries archives for target(s) this script has no "
            f"entry for: {', '.join(unknown)}. Add them to PLATFORMS in "
            "tools/release/generate_packaging.py, or the users on those "
            "platforms will silently never be packaged for."
        )


def homebrew_formula(version: str, digests: Dict[str, str]) -> str:
    """Emit `Formula/stegobench.rb` for the tap.

    One formula covering three downloads. macOS gets the universal archive
    whichever Mac it is, which is why there is no `on_intel` inside
    `on_macos`: there is one file and it holds both architectures.
    """
    macos_url = archive_url(version, "macos-universal")
    macos_sha = digest_for(digests, version, "macos-universal")
    linux_x86_url = archive_url(version, "linux-x86_64")
    linux_x86_sha = digest_for(digests, version, "linux-x86_64")
    linux_arm_url = archive_url(version, "linux-aarch64")
    linux_arm_sha = digest_for(digests, version, "linux-aarch64")

    return f"""# Author:  Daniel Iwugo
# Comment: Christ is King
# typed: false
# frozen_string_literal: true

# Generated by tools/release/generate_packaging.py in elementmerc/stegobench.
# Do not edit by hand: the next release overwrites this file.
class Stegobench < Formula
  desc "{DESCRIPTION}"
  homepage "{REPO_URL}"
  license "{LICENCE}"
  version "{version}"

  on_macos do
    # One archive for both architectures, joined with lipo by the release
    # workflow, so an Intel Mac and an Apple silicon Mac download the same file.
    url "{macos_url}"
    sha256 "{macos_sha}"
  end

  on_linux do
    on_intel do
      url "{linux_x86_url}"
      sha256 "{linux_x86_sha}"
    end
    on_arm do
      url "{linux_arm_url}"
      sha256 "{linux_arm_sha}"
    end
  end

  def install
    bin.install "stegobench"
    # The Linux and macOS archives carry generated man pages; the Windows one
    # does not, and neither does an archive built before build.rs managed to
    # write them, so this is conditional rather than assumed.
    man1.install Dir["man/*.1"] if Dir.exist?("man")
  end

  test do
    assert_match version.to_s, shell_output("#{{bin}}/stegobench --version")
    # Asserted on the exit status rather than on text inside the schema, so
    # this stays true when the schema's contents change and false when the
    # binary cannot run.
    system bin/"stegobench", "schema", "result-v1"
  end
end
"""


def scoop_manifest(version: str, digests: Dict[str, str]) -> str:
    """Emit `bucket/stegobench.json` for the bucket.

    Scoop reads JSON and nothing else, so this is built as a dictionary and
    serialised rather than formatted as text: a manifest that does not parse
    is a manifest Scoop reports as a corrupt bucket rather than as a bad
    release.
    """
    manifest = {
        "version": version,
        "description": DESCRIPTION,
        "homepage": REPO_URL,
        "license": LICENCE,
        "architecture": {
            "64bit": {
                "url": archive_url(version, "windows-x86_64"),
                "hash": digest_for(digests, version, "windows-x86_64"),
                "extract_dir": extract_dir(version, "windows-x86_64"),
            }
        },
        "bin": "stegobench.exe",
        "checkver": {"github": REPO_URL},
        "autoupdate": {
            "architecture": {
                "64bit": {
                    "url": (
                        f"{REPO_URL}/releases/download/v$version/"
                        "stegobench-v$version-x86_64-pc-windows-msvc.zip"
                    ),
                    "extract_dir": "stegobench-v$version-x86_64-pc-windows-msvc",
                }
            },
            # Scoop reads the digest out of the release's own SHA256SUMS rather
            # than recomputing it, which keeps the signed file the source of
            # truth here too.
            "hash": {
                "url": f"{REPO_URL}/releases/download/v$version/SHA256SUMS",
            },
        },
    }
    return json.dumps(manifest, indent=4) + "\n"


def generate(version: str, sums_text: str) -> Dict[str, str]:
    """Everything the two package managers need, as a path to content map.

    Both files are built before either is written, so a release missing the
    Windows archive fails without having left a half-updated tap behind.
    """
    check_version(version)
    digests = parse_sums(sums_text)
    check_no_unknown_archives(digests, version)
    return {
        "homebrew/stegobench.rb": homebrew_formula(version, digests),
        "scoop/stegobench.json": scoop_manifest(version, digests),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Generate the Homebrew formula and Scoop manifest for a release, "
            "with the digests read from the release's own SHA256SUMS."
        )
    )
    parser.add_argument(
        "--version",
        required=True,
        help="the release version as the tag carries it, without the v",
    )
    parser.add_argument(
        "--sums",
        required=True,
        type=pathlib.Path,
        help="path to the release's SHA256SUMS",
    )
    parser.add_argument(
        "--out",
        required=True,
        type=pathlib.Path,
        help="directory to write homebrew/ and scoop/ into",
    )
    args = parser.parse_args(argv)

    try:
        sums_text = args.sums.read_text(encoding="utf-8")
    except OSError as error:
        print(f"stegobench: cannot read {args.sums}: {error}", file=sys.stderr)
        return 2

    try:
        files = generate(args.version, sums_text)
    except PackagingError as error:
        print(f"stegobench: {error}", file=sys.stderr)
        return 1

    for relative, content in sorted(files.items()):
        destination = args.out / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        # Written through a neighbouring temporary file and renamed, so an
        # interrupted run leaves either the previous file or the new one and
        # never half of either.
        staging = destination.with_name(destination.name + ".part")
        staging.write_text(content, encoding="utf-8")
        staging.replace(destination)
        print(f"wrote {destination}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
