#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Read `release.toml`, the one place this project states its own facts.

Every generated manifest, formula, citation and container label takes its
description, licence, URLs and keywords from here, so that two published files
cannot disagree about what this project is.

    from release_facts import load
    facts = load()
    facts.description     # "Reproducible benchmark for steganalysis"

WHY THIS VALIDATES RATHER THAN TRUSTING THE FILE
------------------------------------------------
The consumers of these facts are files a stranger installs from. A Homebrew
formula with an empty description still installs; a codemeta.json with a
missing licence still parses; a citation with a blank author still renders. Not
one of those fails at the point the mistake is made, so every one of them is
checked here instead, where the failure is a message naming the field rather
than a wrong page somebody reads for a year.

The checks are deliberately about meaning and not only about presence. A
description that opens with the project's own name is refused because
Homebrew's style guide refuses it, and finding that out from a rejected pull
request is a week later than finding it out here.
"""
from __future__ import annotations

import pathlib
from typing import NamedTuple, Sequence

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python < 3.11
    import tomli as tomllib  # type: ignore[no-redef]

#: `release.toml` sits beside this file. Resolved from `__file__` rather than
#: from the working directory, because a release job runs these scripts from
#: wherever it happens to be and a relative default would work in testing and
#: fail in CI.
DEFAULT_PATH = pathlib.Path(__file__).resolve().parent / "release.toml"

#: An SPDX identifier as this project uses them. Not a full SPDX expression
#: grammar: the point is to catch an empty string, a licence name written out
#: in prose, and a stray `v` prefix, all of which parse as TOML and then fail
#: somewhere a user can see.
_SPDX = ("AGPL-3.0-or-later", "AGPL-3.0-only", "MIT", "Apache-2.0", "CC0-1.0")


class FactsError(Exception):
    """A `release.toml` that cannot produce correct published metadata."""


class Author(NamedTuple):
    """One author, in the shape both CITATION.cff and codemeta want."""

    family_names: str
    given_names: str

    @property
    def full_name(self) -> str:
        """`Given Family`, which is what a container label and a formula want."""
        return f"{self.given_names} {self.family_names}"


class Facts(NamedTuple):
    """The facts, validated. Every field is non-empty by construction."""

    name: str
    slug: str
    description: str
    abstract: str
    licence: str
    repository: str
    homepage: str
    documentation: str
    issues: str
    copyright_year: str
    authors: tuple[Author, ...]
    keywords: tuple[str, ...]

    @property
    def licence_url(self) -> str:
        """The SPDX page for the licence, which codemeta wants as a URL."""
        return f"https://spdx.org/licenses/{self.licence}.html"

    @property
    def copyright(self) -> str:
        """The notice line, assembled once so three consumers cannot differ."""
        holders = ", ".join(a.full_name for a in self.authors)
        return f"Copyright (C) {self.copyright_year} {holders}"


def _require(table: dict, field: str, where: str, path: pathlib.Path) -> str:
    value = table.get(field)
    if value is None:
        raise FactsError(f"{path}: [{where}] states no {field}")
    if not isinstance(value, str):
        raise FactsError(f"{path}: [{where}] {field} is {type(value).__name__}, not a string")
    stripped = value.strip()
    if not stripped:
        raise FactsError(
            f"{path}: [{where}] {field} is empty. An empty value is published "
            f"as an empty value rather than refused by the thing that reads it"
        )
    return stripped


def _check_url(value: str, field: str, path: pathlib.Path) -> str:
    if not value.startswith("https://"):
        raise FactsError(
            f"{path}: {field} is {value!r}, which is not an https URL. Every "
            f"consumer of this publishes it as a link somebody clicks"
        )
    return value


def _check_description(value: str, name: str, path: pathlib.Path) -> str:
    # Homebrew's own style guide refuses both of these, and a rejected pull
    # request is a far slower way to learn it than a failure here.
    lowered = value.lower()
    if lowered.startswith(name.lower()):
        raise FactsError(
            f"{path}: description opens with the project's own name, which "
            f"Homebrew refuses. Say what it does, not what it is called"
        )
    first = lowered.split(" ", 1)[0]
    if first in ("a", "an", "the"):
        raise FactsError(
            f"{path}: description opens with the article {first!r}, which "
            f"Homebrew refuses"
        )
    if value.endswith("."):
        raise FactsError(
            f"{path}: description ends with a full stop. Both package managers "
            f"render it as a label rather than a sentence"
        )
    return value


def load(path: pathlib.Path | None = None) -> Facts:
    """Load and validate the facts, or raise [`FactsError`] saying what is wrong."""
    path = DEFAULT_PATH if path is None else path
    try:
        with path.open("rb") as handle:
            parsed = tomllib.load(handle)
    except FileNotFoundError:
        raise FactsError(
            f"{path} does not exist. Every generated manifest reads it, so "
            f"there is nothing to generate from"
        ) from None
    except tomllib.TOMLDecodeError as exc:
        raise FactsError(f"{path} is not valid TOML: {exc}") from None

    project = parsed.get("project")
    if not isinstance(project, dict):
        raise FactsError(f"{path} has no [project] table")

    name = _require(project, "name", "project", path)
    slug = _require(project, "slug", "project", path)
    if slug != slug.lower():
        raise FactsError(
            f"{path}: slug is {slug!r}. It is a command name, a crate prefix "
            f"and an archive prefix, all of which are case sensitive and all "
            f"of which are lower case"
        )

    description = _check_description(
        _require(project, "description", "project", path), name, path
    )

    authors_raw = parsed.get("author")
    if not isinstance(authors_raw, list) or not authors_raw:
        raise FactsError(
            f"{path} declares no [[author]]. A citation with no author is a "
            f"citation nobody can credit"
        )
    authors = []
    for entry in authors_raw:
        if not isinstance(entry, dict):
            raise FactsError(f"{path}: an [[author]] entry is not a table")
        authors.append(
            Author(
                family_names=_require(entry, "family_names", "author", path),
                given_names=_require(entry, "given_names", "author", path),
            )
        )

    keywords_raw = project.get("keywords")
    if not isinstance(keywords_raw, list) or not keywords_raw:
        raise FactsError(
            f"{path} declares no keywords. crates.io, PyPI and the citation "
            f"all list them, and an empty list is how a project becomes "
            f"unfindable"
        )
    keywords: list[str] = []
    for keyword in keywords_raw:
        if not isinstance(keyword, str) or not keyword.strip():
            raise FactsError(f"{path}: {keyword!r} is not a usable keyword")
        if keyword in keywords:
            raise FactsError(f"{path}: the keyword {keyword!r} appears twice")
        keywords.append(keyword.strip())

    licence = _require(project, "licence", "project", path)
    if licence not in _SPDX:
        raise FactsError(
            f"{path}: licence is {licence!r}, which is not an SPDX identifier "
            f"this project recognises. Homebrew parses it as an SPDX "
            f"expression and codemeta publishes it as a URL, so a prose "
            f"licence name fails in both"
        )

    year = _require(project, "copyright_year", "project", path)
    if not (len(year) == 4 and year.isdigit()):
        raise FactsError(f"{path}: copyright_year is {year!r}, not a four digit year")

    return Facts(
        name=name,
        slug=slug,
        description=description,
        # Written as a TOML multi-line string with line continuations, so it
        # arrives as one paragraph with single spaces and no trailing newline.
        abstract=" ".join(_require(project, "abstract", "project", path).split()),
        licence=licence,
        repository=_check_url(
            _require(project, "repository", "project", path), "repository", path
        ),
        homepage=_check_url(_require(project, "homepage", "project", path), "homepage", path),
        documentation=_check_url(
            _require(project, "documentation", "project", path), "documentation", path
        ),
        issues=_check_url(_require(project, "issues", "project", path), "issues", path),
        copyright_year=year,
        authors=tuple(authors),
        keywords=tuple(keywords),
    )


def main(argv: Sequence[str] | None = None) -> int:
    """Validate the file and print what it says, so a human can check it."""
    try:
        facts = load()
    except FactsError as exc:
        print(f"release.toml is not usable: {exc}")
        return 1
    print(f"{facts.name} ({facts.slug})")
    print(f"  {facts.description}")
    print(f"  {facts.licence}  {facts.copyright}")
    print(f"  {facts.repository}")
    print(f"  keywords: {', '.join(facts.keywords)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
