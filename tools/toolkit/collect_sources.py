#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Collect the corresponding source for every copyleft component of an image.

    python3 tools/toolkit/collect_sources.py stegobench/toolkit:seven --out dist/

Publishing a Debian-based image that carries GPL and AGPL programs obliges us
to offer the source those binaries were built from. An offer we cannot fulfil
on demand is not an offer, so this builds the artefact up front rather than
promising one: a directory of source tarballs, plus a manifest naming every
binary package in the image, the source package it came from, the exact
version, and the sha256 of what was fetched.

It over-collects on purpose. Deciding per package whether a licence is
copyleft means parsing a `debian/copyright` file whose format is a convention
rather than a guarantee, and being wrong in the permissive direction is a
licence breach while being wrong the other way costs disk. So every Debian
package in the image gets its source fetched, and the non-Debian components
are listed explicitly below because nothing in the image records where they
came from.

Re-runnable but not resumable: an interrupted run restarts the fetch from the
beginning, because `apt-get source` has no "skip what is already here" mode
and re-deriving that from the manifest would mean trusting a manifest the
interrupted run never finished writing. Re-running is safe and costs about
fifteen minutes.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
from typing import Iterable

# Components that are not Debian packages, so `apt-get source` knows nothing
# about them. Each is recorded with the route to its source, because the image
# itself carries no trace of where these came from.
#
# `obligation` is whether redistributing the binary compels us to supply the
# source. Permissive entries are listed anyway: the manifest is also the
# attribution record, and a reader asking "what is in this image" should get
# one answer rather than two.
NON_DEBIAN = [
    {
        "name": "openstego",
        "version": "0.8.6",
        "licence": "GPL-2.0-only",
        "obligation": "copyleft",
        "source": "https://github.com/syvaidya/openstego",
        "note": "installed from the project's own .deb release asset, which is "
                "built from this repository at tag openstego-0.8.6",
    },
    {
        "name": "stegcore",
        "version": "4.1.0",
        "licence": "AGPL-3.0-or-later",
        "obligation": "copyleft",
        "source": "https://github.com/The-Malware-Files/Stegcore",
        "note": "our own binary, copied from its published image. AGPL section "
                "13 also reaches users who interact with it over a network, "
                "which this image does not arrange but a downstream deployment "
                "might",
    },
    {
        "name": "hstego",
        "version": "0.6.1",
        "licence": "MIT",
        "obligation": "permissive",
        "source": "https://github.com/daniellerch/hstego",
        "note": "installed by pip from git at commit "
                "bf71f6e0d7faaa632ad8a988c0393e94bfd13b2a, which the Dockerfile "
                "pins and verifies against what pip resolved. Two native C "
                "extensions are compiled from that commit during the build",
    },
    {
        "name": "zsteg",
        "version": "0.2.13",
        "licence": "MIT",
        "obligation": "permissive",
        "source": "https://rubygems.org/gems/zsteg/versions/0.2.13",
        "note": "installed as a gem",
    },
    {
        "name": "libjpeg",
        "version": "9e",
        "licence": "IJG",
        "obligation": "permissive",
        "source": "https://www.ijg.org/files/jpegsrc.v9e.tar.gz",
        "note": "built from source in the hstego builder stage, because "
                "hstego reads libjpeg internals that libjpeg-turbo does not "
                "expose",
    },
]

# apt-get source on a full base plus a JRE pulls a lot; openjdk's own source is
# most of it. Measured at 2.9 GB for this image on 2026-10-01, and the margin
# is there because a partial fetch is the failure this check exists to prevent.
REQUIRED_FREE_BYTES = 8 * 1024**3

# Debian packages installed from somewhere other than Debian. dpkg lists them
# like any other package, so apt is asked for a source it has never heard of
# and the run reports a gap that is not one. Their source is in NON_DEBIAN
# above, which is where a reader should be sent.
NOT_FROM_DEBIAN = {"openstego"}

# snapshot.debian.org is addressed by timestamp. This one is a constant rather
# than `now`: a collection run twice must fetch the same bytes, and a clock
# would make the second run's manifest disagree with the first for no reason a
# reader could see.
#
# The cost of a constant is that a package published AFTER it cannot be found
# in it, which is not a rare case: a security update lands, the mirror drops
# the superseded source within days, and an image built in between carries a
# version that is newer than the stamp and older than the mirror. `--snapshot`
# is how that is answered, and the failure message says so rather than leaving
# the reader to work out why an archive of everything did not have it.
SNAPSHOT_STAMP = "20261001T000000Z"

DOCKER_TIMEOUT = 300
FETCH_TIMEOUT = 1800


class CollectError(Exception):
    """Something went wrong that the caller must see rather than work around."""


def _run(argv: list[str], *, timeout: int, cwd: pathlib.Path | None = None
         ) -> subprocess.CompletedProcess:
    try:
        return subprocess.run(argv, capture_output=True, text=True,
                              timeout=timeout, cwd=cwd, check=False)
    except FileNotFoundError as exc:
        raise CollectError(f"{argv[0]} is not installed: {exc}") from exc
    except subprocess.TimeoutExpired as exc:
        raise CollectError(
            f"{argv[0]} did not finish within {timeout}s: "
            f"{' '.join(argv[:4])}") from exc


def preflight(image: str, out: pathlib.Path) -> list[str]:
    """Everything that would stop the run, reported together rather than one
    per rerun. A rerun costs a full re-fetch."""
    problems = []
    if shutil.which("docker") is None:
        problems.append("docker is not on PATH; this reads the image through it")
    else:
        probe = _run(["docker", "image", "inspect", image], timeout=DOCKER_TIMEOUT)
        if probe.returncode != 0:
            last = (probe.stderr.strip().splitlines() or ["no reason given"])[-1]
            problems.append(f"cannot inspect image {image}: {last}")

    parent = out if out.exists() else out.parent
    if not parent.exists():
        problems.append(f"no output directory and no parent to make it in: {out}")
    else:
        free = shutil.disk_usage(parent).free
        if free < REQUIRED_FREE_BYTES:
            problems.append(
                f"{parent} has {free / 1024**3:.1f} GB free and this collection "
                f"needs at least {REQUIRED_FREE_BYTES / 1024**3:.0f} GB")
    return problems


def installed_packages(image: str) -> list[dict]:
    """Every Debian package in the image, with the source package behind it.

    `source:Package` is empty when the source package shares the binary's
    name, which is the common case, so it is filled in here rather than left
    for every consumer to special-case.
    """
    fmt = "${Package}\\t${Version}\\t${source:Package}\\t${source:Version}\\n"
    result = _run(
        ["docker", "run", "--rm", "--network=none", "--entrypoint", "dpkg-query",
         image, "-W", "-f", fmt],
        timeout=DOCKER_TIMEOUT)
    if result.returncode != 0:
        last = (result.stderr.strip().splitlines() or ["no reason given"])[-1]
        raise CollectError(f"could not list packages in {image}: {last}")

    packages = []
    for line in result.stdout.splitlines():
        if not line.strip():
            continue
        parts = line.split("\t")
        if len(parts) != 4:
            raise CollectError(f"dpkg-query returned a row of {len(parts)} "
                               f"fields, expected 4: {line!r}")
        name, version, src_name, src_version = parts
        packages.append({
            "binary": name,
            "binary_version": version,
            "source": src_name or name,
            "source_version": src_version or version,
        })
    if not packages:
        raise CollectError(f"{image} reports no installed packages, which "
                           f"cannot be true of a Debian image")
    return packages


def venv_packages(image: str) -> list[dict]:
    """The Python packages in hstego's virtual environment.

    They are 481 MB of the image and dpkg has never heard of any of them, so
    without this the only machine-readable inventory of the image omits its
    largest single layer. `pip list` rather than `pip freeze`, because freeze
    renders a git-installed package as a URL and the version is then absent
    from the field a reader is looking in.

    A failure here is reported rather than swallowed: an SBOM that is quietly
    missing a third of the image is worse than no SBOM, because nothing about
    it says so.
    """
    result = _run(
        ["docker", "run", "--rm", "--network=none", "--entrypoint",
         "/opt/hstego-venv/bin/pip", image, "list", "--format=json"],
        timeout=DOCKER_TIMEOUT)
    if result.returncode != 0:
        last = (result.stderr.strip().splitlines() or ["no reason given"])[-1]
        raise CollectError(
            f"could not list the Python packages in {image}: {last}")
    try:
        listed = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise CollectError(f"pip did not return JSON for {image}: {exc}") from exc
    return [{"name": entry["name"], "version": entry["version"]}
            for entry in listed]


def image_digest(image: str) -> str | None:
    """The image's own content digest, or None when it has none.

    A locally built image that has never been pushed has no RepoDigest, and
    saying so is the honest answer: that absence is exactly what a reviewer
    needs to know, because it means there is nothing to verify the image
    against.
    """
    result = _run(["docker", "image", "inspect", "--format",
                   "{{index .RepoDigests 0}}", image], timeout=DOCKER_TIMEOUT)
    if result.returncode != 0:
        return None
    digest = result.stdout.strip()
    return digest or None


def build_sbom(image: str, packages: list[dict], venv: list[dict],
               digest: str | None) -> dict:
    """A CycloneDX 1.5 bill of materials for the image.

    Written here rather than taken from a scanner because this script already
    enumerates every component for the source offer, and a second tool reading
    the same image would be a second answer to one question. `syft` or
    `docker buildx` would do it too; neither is installed on the machine that
    builds these images, and a bill of materials nobody can produce is the same
    as none.

    Component identity is a purl where a purl exists, because that is what
    downstream vulnerability tooling matches on. Debian binary packages get
    `pkg:deb/debian/...`, Python packages `pkg:pypi/...`, and the handful that
    belong to no ecosystem get a plain name and a documented source URL
    instead of a made-up purl.
    """
    components: list[dict] = []
    for package in packages:
        components.append({
            "type": "library",
            "name": package["binary"],
            "version": package["binary_version"],
            "purl": f"pkg:deb/debian/{package['binary']}"
                    f"@{package['binary_version']}?arch=amd64",
        })
    for package in venv:
        components.append({
            "type": "library",
            "name": package["name"],
            "version": package["version"],
            "purl": f"pkg:pypi/{package['name'].lower()}@{package['version']}",
        })
    # MERGED BY NAME RATHER THAN APPENDED, because three of these are also
    # visible to dpkg or to pip and appending produced a bill of materials that
    # listed hstego and openstego twice. A duplicate in an inventory is not
    # untidiness: anything counting components counts them twice, and anything
    # matching a vulnerability against a purl now has two records to disagree
    # with each other. The ecosystem entry knows the version; only this table
    # knows the licence and the real origin, so the two are one component
    # described from two directions.
    by_name = {component["name"]: component for component in components}
    for component in NON_DEBIAN:
        existing = by_name.get(component["name"])
        if existing is None:
            components.append({
                "type": "application",
                "name": component["name"],
                "version": component["version"],
                "licenses": [{"expression": component["licence"]}],
                "externalReferences": [
                    {"type": "website", "url": component["source"]},
                ],
                "description": component["note"],
            })
            continue
        existing["licenses"] = [{"expression": component["licence"]}]
        existing["externalReferences"] = [
            {"type": "website", "url": component["source"]},
        ]
        existing["description"] = component["note"]
        # THE PURL GOES, and that is the point of this table existing.
        #
        # A purl names an ecosystem, and being in NON_DEBIAN is precisely the
        # statement that this component did not come from the ecosystem it
        # appears to belong to. `pkg:pypi/hstego` asserts a PyPI release of
        # something pip compiled from a git commit. `pkg:deb/debian/openstego`
        # asserts a Debian package of something Debian does not ship at all,
        # which would have a scanner matching it against the wrong advisories
        # or, worse, finding none and reporting it clean.
        #
        # An absent purl sends a reader to the source URL on the line below. A
        # wrong one gets matched by a machine and believed.
        existing.pop("purl", None)

    # Sorted so two runs over one image produce byte-identical output. dpkg and
    # pip both happen to be ordered today; relying on that is how iteration
    # order leaks into an artefact somebody diffs.
    components.sort(key=lambda c: (c["name"], c["version"]))

    metadata_component: dict = {"type": "container", "name": image}
    if digest:
        algorithm, _, value = digest.rpartition(":")
        metadata_component["hashes"] = [
            {"alg": "SHA-256", "content": value},
        ]
    else:
        metadata_component["description"] = (
            "built locally and never pushed, so it carries no registry digest "
            "and there is nothing to verify these bytes against")

    return {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "tools": [{"name": "collect_sources.py",
                       "vendor": "stegobench"}],
            "component": metadata_component,
        },
        "components": components,
    }


def _sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def fetch_sources(image: str, sources: Iterable[tuple[str, str]],
                  out: pathlib.Path, snapshot: str = SNAPSHOT_STAMP) -> dict:
    """Fetch each source package at its exact version, inside the image.

    Inside, because the image's own apt knows which suite each version came
    from. Running this on the host would resolve against the host's sources
    list and silently fetch a different version, which is the failure that
    makes a source offer worthless: bytes that do not correspond to the binary
    they are offered for.
    """
    wanted = sorted({f"{name}={version}" for name, version in sources
                     if name not in NOT_FROM_DEBIAN})
    out.mkdir(parents=True, exist_ok=True)

    script = (
        "set -e\n"
        # Debian images ship no deb-src lines, so there is nothing to fetch
        # from until they are added. trixie moved to the deb822 format.
        "printf 'Types: deb-src\\nURIs: http://deb.debian.org/debian\\n"
        "Suites: trixie trixie-updates\\nComponents: main\\n"
        "Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg\\n' "
        "> /etc/apt/sources.list.d/sources.sources\n"
        # Security updates are a SEPARATE archive on a separate host, not a
        # suite of the main one. Without this line every package carrying a
        # security fix has no source route at all, which is both the most
        # likely package to be asked about and the one it is least acceptable
        # to be unable to supply. Found because openssl was the only package
        # the first complete run could not fetch.
        "printf 'Types: deb-src\\nURIs: http://security.debian.org/debian-security\\n"
        "Suites: trixie-security\\nComponents: main\\n"
        "Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg\\n' "
        "> /etc/apt/sources.list.d/security.sources\n"
        # snapshot.debian.org as a separate parts directory, so it is consulted
        # only by the fallback below and never races the mirror for a package
        # the mirror still has.
        "mkdir -p /etc/apt/snapshot.d\n"
        "printf 'Types: deb-src\\nURIs: https://snapshot.debian.org/archive/debian/"
        f"{snapshot}/\\n"
        "Suites: trixie trixie-updates\\nComponents: main\\nSigned-By: "
        "/usr/share/keyrings/debian-archive-keyring.gpg\\nCheck-Valid-Until: no\\n' "
        "> /etc/apt/snapshot.d/snapshot.sources\n"
        "printf 'Types: deb-src\\nURIs: https://snapshot.debian.org/archive/debian-security/"
        f"{snapshot}/\\n"
        "Suites: trixie-security\\nComponents: main\\nSigned-By: "
        "/usr/share/keyrings/debian-archive-keyring.gpg\\nCheck-Valid-Until: no\\n' "
        "> /etc/apt/snapshot.d/security.sources\n"
        "apt-get update -qq\n"
        "cd /out\n"
        "for spec in \"$@\"; do\n"
        "  if apt-get source --download-only -qq \"$spec\" 2>>/out/.fetch-errors; then\n"
        "    continue\n"
        "  fi\n"
        # A security update's source leaves the mirror as soon as the next one
        # supersedes it, which is exactly the version an image built last week
        # is running. snapshot.debian.org keeps every version ever published,
        # so it is the second attempt rather than the first: it is slower and
        # rate limited, and the mirror answers for almost everything.
        "  name=\"${spec%%=*}\"; version=\"${spec#*=}\"\n"
        "  SNAP='-o Dir::Etc::SourceList=/dev/null "
        "-o Dir::Etc::SourceParts=/etc/apt/snapshot.d'\n"
        # The snapshot lists are fetched on first need rather than up front:
        # they are 10 MB, and almost every run never reaches this branch.
        #
        # The marker lives inside the container and NOT in /out, which is a
        # bind mount. A marker in /out outlives the apt lists it stands for,
        # because those are in /var/lib/apt and go when the container does, so
        # the second run skipped the update and then failed every lookup with
        # "you must put some deb-src URIs in your sources.list".
        "  if [ ! -f /tmp/.snapshot-updated ]; then\n"
        "    apt-get $SNAP update -qq 2>>/out/.fetch-errors "
        "&& touch /tmp/.snapshot-updated\n"
        "  fi\n"
        "  if apt-get $SNAP source --download-only -qq "
        "\"$name=$version\" 2>>/out/.fetch-errors; then\n"
        "    echo \"$spec\" >> /out/.fetch-snapshot\n"
        "  else\n"
        # Still reported and skipped rather than failing the whole collection,
        # so one unobtainable package does not cost the other 279. The
        # manifest records it so the gap is visible instead of silent.
        "    echo \"$spec\" >> /out/.fetch-missing\n"
        "  fi\n"
        "done\n"
    )

    result = _run(
        ["docker", "run", "--rm", "-v", f"{out.resolve()}:/out",
         "--entrypoint", "bash", image, "-c", script, "--", *wanted],
        timeout=FETCH_TIMEOUT)

    missing_file = out / ".fetch-missing"
    missing = sorted(set(missing_file.read_text(encoding="utf-8").split())) \
        if missing_file.exists() else []
    snapshot_file = out / ".fetch-snapshot"
    from_snapshot = sorted(set(snapshot_file.read_text(encoding="utf-8").split())) \
        if snapshot_file.exists() else []
    errors_file = out / ".fetch-errors"
    errors = errors_file.read_text(encoding="utf-8").strip() \
        if errors_file.exists() else ""

    if result.returncode != 0 and not missing:
        last = (result.stderr.strip().splitlines()
                or errors.splitlines() or ["no reason given"])[-1]
        raise CollectError(f"fetching source failed before any package was "
                           f"attempted: {last}")

    fetched = {}
    for path in sorted(out.iterdir()):
        if path.is_file() and not path.name.startswith("."):
            fetched[path.name] = {"sha256": _sha256(path), "bytes": path.stat().st_size}
    return {"files": fetched, "missing": missing, "errors": errors,
            "from_snapshot": from_snapshot}


def build_manifest(image: str, packages: list[dict], fetched: dict,
                   snapshot: str) -> dict:
    return {
        "schema": "toolkit-sources-v1",
        "image": image,
        "debian_packages": packages,
        "non_debian_components": NON_DEBIAN,
        "fetched": fetched["files"],
        # Named rather than counted: a reader checking whether the source for
        # the package they care about is here needs the name, and a count tells
        # them only that something is absent.
        "missing": fetched["missing"],
        # Fetched from snapshot.debian.org rather than the live mirror, which
        # means the version in the image is no longer the current one. Not a
        # fault; recorded because it is the difference between an offer that
        # can be refreshed from the mirror and one that depends on an archive.
        "from_snapshot": fetched["from_snapshot"],
        "snapshot_stamp": snapshot,
        # Installed through dpkg but not from Debian, so apt has no source for
        # them by design. Their route is in non_debian_components.
        "not_from_debian": sorted(NOT_FROM_DEBIAN),
        "totals": {
            "debian_packages": len(packages),
            "source_tarballs": len(fetched["files"]),
            "from_snapshot": len(fetched["from_snapshot"]),
            "missing": len(fetched["missing"]),
        },
    }


def _write_json(target: pathlib.Path, payload: dict,
                scratch: pathlib.Path) -> pathlib.Path:
    """Write JSON atomically, so an interrupted run never leaves a file that
    parses and undercounts. `sort_keys` because two runs over one image must
    produce byte-identical output."""
    with tempfile.NamedTemporaryFile(
            "w", encoding="utf-8", dir=scratch, delete=False) as handle:
        json.dump(payload, handle, indent=2, sort_keys=True)
        handle.write("\n")
        temporary = pathlib.Path(handle.name)
    temporary.replace(target)
    return target


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Collect corresponding source for an image's copyleft parts")
    parser.add_argument("image", help="image reference, e.g. stegobench/toolkit:seven")
    parser.add_argument("--out", type=pathlib.Path, required=True,
                        help="directory to write source tarballs and the manifest into")
    parser.add_argument("--snapshot", default=SNAPSHOT_STAMP,
                        metavar="YYYYMMDDTHHMMSSZ",
                        help="snapshot.debian.org timestamp the fallback reads. "
                             "Move it later than the newest package in the image; "
                             f"default {SNAPSHOT_STAMP}")
    parser.add_argument("--dry-run", action="store_true",
                        help="check the run could work, list what would be fetched, fetch nothing")
    parser.add_argument("--sbom-only", action="store_true",
                        help="write SBOM.cdx.json and nothing else, fetching no "
                             "source. Needs no disk space worth checking, so the "
                             "free-space pre-flight is skipped")
    args = parser.parse_args(argv)

    # The space check exists for a 2.9 GB source fetch. Applying it to a run
    # that writes one JSON file would refuse the cheap half of this tool on a
    # machine where it would have worked fine, and a reviewer asking for an
    # inventory should not be told to free 8 GB first.
    if args.sbom_only:
        args.out.mkdir(parents=True, exist_ok=True)
        try:
            sbom = build_sbom(args.image, installed_packages(args.image),
                              venv_packages(args.image),
                              image_digest(args.image))
            target = _write_json(args.out / "SBOM.cdx.json", sbom, args.out)
        except CollectError as exc:
            print(f"cannot collect: {exc}", file=sys.stderr)
            return 1
        print(f"{len(sbom['components'])} components -> {target}")
        return 0

    problems = preflight(args.image, args.out)
    if problems:
        for problem in problems:
            print(f"cannot collect: {problem}", file=sys.stderr)
        return 3

    try:
        packages = installed_packages(args.image)
        sources = {(p["source"], p["source_version"]) for p in packages}

        if args.dry_run:
            print(f"{len(packages)} binary packages from {len(sources)} source "
                  f"packages, plus {len(NON_DEBIAN)} components apt does not know about")
            for name, version in sorted(sources):
                print(f"  {name} {version}")
            return 0

        args.out.mkdir(parents=True, exist_ok=True)
        fetched = fetch_sources(args.image, sources, args.out, args.snapshot)
        manifest = build_manifest(args.image, packages, fetched, args.snapshot)
        target = _write_json(args.out / "SOURCES.json", manifest, args.out)

        # The bill of materials rides along with the source offer rather than
        # being a separate errand, because the two answer one question between
        # them: what is in this image, and where did it come from. Published
        # together or a reader has half an answer.
        sbom = build_sbom(args.image, packages, venv_packages(args.image),
                          image_digest(args.image))
        _write_json(args.out / "SBOM.cdx.json", sbom, args.out)
    except CollectError as exc:
        print(f"cannot collect: {exc}", file=sys.stderr)
        return 1

    print(f"{manifest['totals']['source_tarballs']} files for "
          f"{manifest['totals']['debian_packages']} packages -> {target}")
    if manifest["missing"]:
        print(f"{len(manifest['missing'])} source package(s) were on neither "
              f"the mirror nor the {args.snapshot} snapshot, and are named in "
              f"SOURCES.json under \"missing\". A package published after that "
              f"timestamp cannot be in it: rerun with a later --snapshot. The "
              f"offer is incomplete until they are retrieved.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
