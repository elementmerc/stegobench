#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Generate `codemeta.json` and `CITATION.cff` from `release.toml`.

    python3 tools/release/generate_metadata.py --out .
    python3 tools/release/generate_metadata.py --out . --version 1.0.0 \\
        --date-released 2026-10-09 --doi 10.5281/zenodo.123456

WHY THESE TWO ARE GENERATED
---------------------------
They state the same facts as every other published manifest, in two more
shapes. `CITATION.cff` is what GitHub renders in the "Cite this repository"
panel and what a reference manager imports; `codemeta.json` is what Software
Heritage, Zenodo and the research-software indexes read. Both are the kind of
file that is written once, is never looked at again, and is wrong from the
first time anything else changes.

WHAT THE VERSION FIELDS DO, AND WHY THEY ARE OPTIONAL
----------------------------------------------------
A citation that names a version, a release date or a DOI is making a claim that
an archive somewhere will resolve. Until something is tagged and something has
minted an identifier, every one of those fields would be a guess, and a guessed
identifier in a citation is exactly the failure this project argues against.

So they are omitted unless supplied, and the generated `CITATION.cff` says in a
comment that they are absent and why. At release time the pipeline passes them
in and the comment goes away on its own.

DETERMINISM
-----------
Two runs on the same inputs produce byte-identical files, because the release
workflow regenerates them and a diff that is only key ordering would make every
release look like a metadata change.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys
from typing import Sequence

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from release_facts import Facts, FactsError, load  # noqa: E402

#: A version as the tag carries it, without the `v`. Pre-release and build
#: metadata are allowed because Cargo allows them, so `1.0.0-rc.1` packages
#: rather than being rejected by this file's opinion of what a version is.
VERSION = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.\-]+)?$")

#: ISO 8601 calendar date, which is the only form CITATION.cff accepts.
DATE = re.compile(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$")

#: The languages the repository is actually written in, most of it first. Not
#: derived from a file count: a line-count winner is not the same as what the
#: thing is written in, and a reader of codemeta wants the latter.
LANGUAGES = ("Rust", "Python")


class GenerateError(Exception):
    """Anything that makes a correct metadata file impossible to emit."""


def _release_fields(
    version: str | None, date_released: str | None, doi: str | None
) -> dict[str, str]:
    """Validate the three optional claims, together, because they travel together."""
    fields: dict[str, str] = {}
    if version is not None:
        if not VERSION.match(version):
            raise GenerateError(
                f"{version!r} is not a version. Pass it as the tag carries it, "
                f"without the leading v"
            )
        fields["version"] = version
    if date_released is not None:
        if not DATE.match(date_released):
            raise GenerateError(
                f"{date_released!r} is not an ISO 8601 date. CITATION.cff "
                f"accepts no other form"
            )
        fields["date_released"] = date_released
    if doi is not None:
        # A DOI that is not a DOI renders as a dead link in the citation panel
        # of every repository page, which is worse than no DOI at all.
        if not doi.startswith("10."):
            raise GenerateError(
                f"{doi!r} does not look like a DOI. A DOI begins with `10.` "
                f"and a wrong one publishes as a dead link"
            )
        fields["doi"] = doi
    # A DOI without the version it identifies is a citation pointing at
    # something the reader cannot pin down, and a release date without a version
    # says a release happened without saying which.
    if ("doi" in fields or "date_released" in fields) and "version" not in fields:
        raise GenerateError(
            "a DOI or a release date was given with no version, so the "
            "citation would identify a release it cannot name. Pass --version"
        )
    return fields


def codemeta(facts: Facts, release: dict[str, str]) -> str:
    """Build `codemeta.json`, with a trailing newline, ready to write."""
    document: dict[str, object] = {
        "@context": "https://w3id.org/codemeta/3.0",
        "@type": "SoftwareSourceCode",
        "name": facts.name,
        "description": facts.abstract,
        "license": facts.licence_url,
        "codeRepository": facts.repository,
        "url": facts.homepage,
        "issueTracker": facts.issues,
        "programmingLanguage": list(LANGUAGES),
        "keywords": list(facts.keywords),
        "copyrightYear": int(facts.copyright_year),
        "author": [
            {
                "@type": "Person",
                "familyName": author.family_names,
                "givenName": author.given_names,
            }
            for author in facts.authors
        ],
    }
    if "version" in release:
        document["version"] = release["version"]
    if "date_released" in release:
        document["datePublished"] = release["date_released"]
    if "doi" in release:
        document["identifier"] = f"https://doi.org/{release['doi']}"
    # `indent=2` and `ensure_ascii=False` so a reader can diff it and an author
    # name outside ASCII survives. `sort_keys` is deliberately off: the order
    # above is the order a human reads codemeta in.
    return json.dumps(document, indent=2, ensure_ascii=False) + "\n"


def citation(facts: Facts, release: dict[str, str]) -> str:
    """Build `CITATION.cff`, with a trailing newline, ready to write."""
    lines = [
        f"# SPDX-License-Identifier: {facts.licence}",
        f"# {facts.copyright}",
        "#",
        "# GENERATED by tools/release/generate_metadata.py from",
        "# tools/release/release.toml. Edit that file rather than this one: a test fails",
        "# when the two disagree.",
        "#",
        "# This file cites the HARNESS. The corpus it was built to score, Pentimento,",
        "# lives in its own repository and carries its own citation, because a citation",
        "# should point at the data rather than at the tool that made it.",
    ]
    if "version" not in release:
        lines += [
            "#",
            "# There is deliberately no `version`, no `date-released` and no `doi` here.",
            "# Nothing has been tagged and no archive has minted an identifier, so every one",
            "# of those fields would be a guess, and a guessed identifier in a citation is",
            "# exactly the failure this project argues against elsewhere. They go in when",
            "# they are true.",
        ]
    lines += [
        "cff-version: 1.2.0",
        'message: "If you use this software, please cite it as below."',
        f'title: "{facts.name}"',
        "abstract: >-",
    ]
    # Wrapped at the width the hand-written CFF this replaced already used, so
    # the commit that first generates it does not reflow every line and read as
    # a rewrite of a file whose content did not change.
    lines += _wrap(facts.abstract, width=78, indent="  ")
    lines += ["type: software", "authors:"]
    for author in facts.authors:
        lines += [
            f'  - family-names: "{author.family_names}"',
            f'    given-names: "{author.given_names}"',
        ]
    lines += [
        f"license: {facts.licence}",
        f'repository-code: "{facts.repository}"',
    ]
    if "version" in release:
        lines.append(f'version: "{release["version"]}"')
    if "date_released" in release:
        lines.append(f"date-released: {release['date_released']}")
    if "doi" in release:
        lines.append(f'doi: "{release["doi"]}"')
    lines.append("keywords:")
    lines += [f"  - {keyword}" for keyword in facts.keywords]
    return "\n".join(lines) + "\n"


def _wrap(text: str, width: int, indent: str) -> list[str]:
    """Greedy wrap, so the output does not depend on textwrap's version."""
    out: list[str] = []
    line = ""
    for word in text.split():
        trial = word if not line else f"{line} {word}"
        if line and len(indent) + len(trial) > width:
            out.append(indent + line)
            line = word
        else:
            line = trial
    if line:
        out.append(indent + line)
    return out


def main(argv: Sequence[str] | None = None) -> int:
    here = pathlib.Path(__file__).resolve().parent
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument(
        "--out",
        type=pathlib.Path,
        default=here.parent.parent,
        help="directory to write codemeta.json and CITATION.cff into "
        "(default: the repository root)",
    )
    ap.add_argument("--release-toml", type=pathlib.Path, default=None)
    ap.add_argument("--version", default=None, help="the release version, without the v")
    ap.add_argument("--date-released", default=None, help="ISO 8601, e.g. 2026-10-09")
    ap.add_argument("--doi", default=None, help="the minted DOI, e.g. 10.5281/zenodo.1")
    ap.add_argument(
        "--check",
        action="store_true",
        help="write nothing; exit 1 if what is on disk differs from what this "
        "would generate, naming the files",
    )
    args = ap.parse_args(argv)

    try:
        facts = load(args.release_toml)
        release = _release_fields(args.version, args.date_released, args.doi)
    except (FactsError, GenerateError) as exc:
        print(f"cannot generate metadata: {exc}", file=sys.stderr)
        return 1

    wanted = {
        "codemeta.json": codemeta(facts, release),
        "CITATION.cff": citation(facts, release),
    }

    if args.check:
        stale = []
        for name, content in wanted.items():
            path = args.out / name
            if not path.exists():
                stale.append(f"{name} does not exist")
            elif path.read_text(encoding="utf-8") != content:
                stale.append(f"{name} differs from what release.toml says")
        if stale:
            print(
                "generated metadata is out of date: "
                + "; ".join(stale)
                + ". Run tools/release/generate_metadata.py to refresh it.",
                file=sys.stderr,
            )
            return 1
        print(f"{len(wanted)} generated metadata file(s) match release.toml")
        return 0

    args.out.mkdir(parents=True, exist_ok=True)
    for name, content in wanted.items():
        path = args.out / name
        # Written through a temporary file in the same directory and renamed, so
        # an interrupted run never leaves a half-written citation behind.
        tmp = path.with_suffix(path.suffix + ".part")
        tmp.write_text(content, encoding="utf-8")
        tmp.replace(path)
        print(f"wrote {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
