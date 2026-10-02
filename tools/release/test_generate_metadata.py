#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for `release.toml` and the two files generated from it.

    python3 -m unittest discover -s tools/release -p 'test_*.py'

The load-bearing test in this file is the drift test: the `CITATION.cff` and
`codemeta.json` committed to this repository must be exactly what the generator
produces from `release.toml` today. Without it the generator is a suggestion,
somebody edits the citation by hand because it is right there at the root, and
the single source of truth quietly becomes one of three.

Everything else here is the refusals. A citation with a blank author still
renders, a codemeta with a missing licence still parses, and a Homebrew
description that opens with an article is rejected by a reviewer a week later.
None of those fails at the point the mistake is made, so each is checked on the
way in and each check is tested.
"""
from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import generate_metadata  # noqa: E402
import release_facts  # noqa: E402
from generate_metadata import GenerateError, citation, codemeta  # noqa: E402
from release_facts import FactsError, load  # noqa: E402

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
RELEASE_TOML = pathlib.Path(__file__).resolve().parent / "release.toml"

#: A complete, valid file, used as the base every refusal test breaks one field
#: of. Written out rather than copied from the real one so a test's expectations
#: do not move when the project's description is reworded.
GOOD = """
[project]
name = "Examplebench"
slug = "examplebench"
description = "Measures things repeatably"
abstract = \"\"\"
One paragraph of prose that a citation and a codemeta document both \\
render as a single block.\\
\"\"\"
licence = "AGPL-3.0-or-later"
repository = "https://example.invalid/org/repo"
homepage = "https://example.invalid/org/repo"
documentation = "https://example.invalid/docs/"
issues = "https://example.invalid/org/repo/issues"
copyright_year = "2026"
keywords = ["one", "two"]

[[author]]
family_names = "Surname"
given_names = "Forename"
"""


def write(text: str) -> pathlib.Path:
    """Put a release.toml in a fresh temporary directory and return its path."""
    directory = pathlib.Path(tempfile.mkdtemp())
    path = directory / "release.toml"
    path.write_text(text, encoding="utf-8")
    return path


class TheCommittedFilesMatchTheSource(unittest.TestCase):
    """The drift test. If this fails, somebody edited a generated file."""

    def test_citation_and_codemeta_on_disk_are_what_release_toml_generates(self) -> None:
        exit_code = generate_metadata.main(["--check", "--out", str(REPO_ROOT)])
        self.assertEqual(
            exit_code,
            0,
            "CITATION.cff or codemeta.json at the repository root differs from "
            "what tools/release/release.toml says. Edit release.toml and run "
            "tools/release/generate_metadata.py; do not edit the generated "
            "files, because the next release regenerates them and the edit is "
            "silently lost.",
        )

    def test_the_real_release_toml_loads(self) -> None:
        facts = load(RELEASE_TOML)
        self.assertEqual(facts.slug, "stegobench")
        self.assertTrue(facts.repository.startswith("https://"))

    def test_release_toml_carries_no_version(self) -> None:
        # The tag is the only place a version is decided. A copy here would be
        # a second thing to bump and the failure is silent: a formula pinned to
        # the previous release still installs, it just installs the wrong thing.
        text = RELEASE_TOML.read_text(encoding="utf-8")
        for line in text.splitlines():
            stripped = line.strip()
            if stripped.startswith("#"):
                continue
            self.assertFalse(
                stripped.startswith("version"),
                f"release.toml declares a version in {line!r}. The tag is the "
                f"only source of a version; see the comment at the top of that "
                f"file.",
            )


class Determinism(unittest.TestCase):
    def test_two_runs_produce_identical_bytes(self) -> None:
        facts = load(write(GOOD))
        self.assertEqual(codemeta(facts, {}), codemeta(facts, {}))
        self.assertEqual(citation(facts, {}), citation(facts, {}))

    def test_both_files_end_with_exactly_one_newline(self) -> None:
        facts = load(write(GOOD))
        for name, content in (("codemeta", codemeta(facts, {})), ("cff", citation(facts, {}))):
            with self.subTest(name):
                self.assertTrue(content.endswith("\n"))
                self.assertFalse(content.endswith("\n\n"))


class WhatTheGeneratedFilesContain(unittest.TestCase):
    def setUp(self) -> None:
        self.facts = load(write(GOOD))

    def test_codemeta_is_json_and_carries_what_an_index_reads(self) -> None:
        document = json.loads(codemeta(self.facts, {}))
        self.assertEqual(document["name"], "Examplebench")
        self.assertEqual(document["@type"], "SoftwareSourceCode")
        self.assertEqual(
            document["license"], "https://spdx.org/licenses/AGPL-3.0-or-later.html"
        )
        self.assertEqual(document["keywords"], ["one", "two"])
        # An integer, not a string: codemeta's own schema says gYear.
        self.assertEqual(document["copyrightYear"], 2026)
        self.assertEqual(document["author"][0]["familyName"], "Surname")

    def test_codemeta_omits_version_fields_when_nothing_is_released(self) -> None:
        document = json.loads(codemeta(self.facts, {}))
        for absent in ("version", "datePublished", "identifier"):
            self.assertNotIn(absent, document)

    def test_codemeta_carries_them_when_a_release_is_passed(self) -> None:
        release = {"version": "1.0.0", "date_released": "2026-10-09", "doi": "10.5281/zenodo.1"}
        document = json.loads(codemeta(self.facts, release))
        self.assertEqual(document["version"], "1.0.0")
        self.assertEqual(document["datePublished"], "2026-10-09")
        self.assertEqual(document["identifier"], "https://doi.org/10.5281/zenodo.1")

    def test_the_abstract_arrives_as_one_paragraph(self) -> None:
        # The TOML is a multi-line string with line continuations. If the join
        # were wrong the citation would carry an embedded newline inside a YAML
        # folded scalar, which parses as something else entirely.
        self.assertNotIn("\n", self.facts.abstract)
        self.assertIn("single block.", self.facts.abstract)

    def test_the_citation_says_why_it_has_no_version(self) -> None:
        text = citation(self.facts, {})
        self.assertIn("deliberately no `version`", text)
        self.assertNotIn("\nversion:", text)

    def test_that_explanation_goes_away_once_there_is_a_version(self) -> None:
        text = citation(self.facts, {"version": "1.0.0"})
        self.assertNotIn("deliberately no `version`", text)
        self.assertIn('version: "1.0.0"', text)

    def test_the_citation_says_it_is_generated(self) -> None:
        # The one line that stops somebody editing the root file by hand.
        self.assertIn("GENERATED by tools/release/generate_metadata.py", citation(self.facts, {}))

    def test_the_citation_wraps_the_abstract_rather_than_emitting_one_long_line(self) -> None:
        for line in citation(self.facts, {}).splitlines():
            with self.subTest(line=line):
                self.assertLessEqual(len(line), 80)


class RefusalsAboutTheRelease(unittest.TestCase):
    def test_a_doi_without_a_version_is_refused(self) -> None:
        with self.assertRaises(GenerateError) as caught:
            generate_metadata._release_fields(None, None, "10.5281/zenodo.1")
        self.assertIn("Pass --version", str(caught.exception))

    def test_a_release_date_without_a_version_is_refused(self) -> None:
        with self.assertRaises(GenerateError):
            generate_metadata._release_fields(None, "2026-10-09", None)

    def test_a_doi_that_is_not_a_doi_is_refused(self) -> None:
        with self.assertRaises(GenerateError) as caught:
            generate_metadata._release_fields("1.0.0", None, "zenodo.1")
        self.assertIn("dead link", str(caught.exception))

    def test_a_version_with_a_v_prefix_is_refused(self) -> None:
        with self.assertRaises(GenerateError):
            generate_metadata._release_fields("v1.0.0", None, None)

    def test_a_prerelease_version_is_allowed(self) -> None:
        fields = generate_metadata._release_fields("1.0.0-rc.1", None, None)
        self.assertEqual(fields["version"], "1.0.0-rc.1")

    def test_a_date_that_is_not_iso_is_refused(self) -> None:
        with self.assertRaises(GenerateError) as caught:
            generate_metadata._release_fields("1.0.0", "9 October 2026", None)
        self.assertIn("ISO 8601", str(caught.exception))


class RefusalsAboutTheFacts(unittest.TestCase):
    def broken(self, find: str, replace: str) -> FactsError:
        """Break one field of the good file and return the error it raises."""
        text = GOOD.replace(find, replace)
        self.assertNotEqual(text, GOOD, f"the test's own edit {find!r} matched nothing")
        with self.assertRaises(FactsError) as caught:
            load(write(text))
        return caught.exception

    def test_a_missing_file_is_refused_by_name(self) -> None:
        with self.assertRaises(FactsError) as caught:
            load(pathlib.Path(tempfile.mkdtemp()) / "absent.toml")
        self.assertIn("does not exist", str(caught.exception))

    def test_invalid_toml_is_refused(self) -> None:
        with self.assertRaises(FactsError) as caught:
            load(write("[project\nname = "))
        self.assertIn("not valid TOML", str(caught.exception))

    def test_a_file_with_no_project_table_is_refused(self) -> None:
        with self.assertRaises(FactsError) as caught:
            load(write("[other]\nname = 'x'\n"))
        self.assertIn("[project]", str(caught.exception))

    def test_an_empty_field_is_refused_rather_than_published_blank(self) -> None:
        self.assertIn("is empty", str(self.broken('name = "Examplebench"', 'name = "  "')))

    def test_a_missing_field_is_named(self) -> None:
        self.assertIn(
            "description", str(self.broken('description = "Measures things repeatably"', ""))
        )

    def test_a_description_opening_with_the_project_name_is_refused(self) -> None:
        error = self.broken(
            'description = "Measures things repeatably"',
            'description = "Examplebench measures things"',
        )
        self.assertIn("opens with the project's own name", str(error))

    def test_a_description_opening_with_an_article_is_refused(self) -> None:
        error = self.broken(
            'description = "Measures things repeatably"',
            'description = "A tool that measures things"',
        )
        self.assertIn("article", str(error))

    def test_a_description_ending_in_a_full_stop_is_refused(self) -> None:
        error = self.broken(
            'description = "Measures things repeatably"',
            'description = "Measures things repeatably."',
        )
        self.assertIn("full stop", str(error))

    def test_an_upper_case_slug_is_refused(self) -> None:
        error = self.broken('slug = "examplebench"', 'slug = "Examplebench"')
        self.assertIn("case sensitive", str(error))

    def test_a_url_that_is_not_https_is_refused(self) -> None:
        error = self.broken(
            'repository = "https://example.invalid/org/repo"',
            'repository = "http://example.invalid/org/repo"',
        )
        self.assertIn("not an https URL", str(error))

    def test_a_licence_written_out_in_prose_is_refused(self) -> None:
        error = self.broken(
            'licence = "AGPL-3.0-or-later"',
            'licence = "GNU Affero General Public License v3 or later"',
        )
        self.assertIn("SPDX", str(error))

    def test_no_authors_is_refused(self) -> None:
        error = self.broken('[[author]]\nfamily_names = "Surname"\ngiven_names = "Forename"', "")
        self.assertIn("no author", str(error).lower())

    def test_no_keywords_is_refused(self) -> None:
        error = self.broken('keywords = ["one", "two"]', "")
        self.assertIn("keywords", str(error))

    def test_a_duplicated_keyword_is_refused(self) -> None:
        error = self.broken('keywords = ["one", "two"]', 'keywords = ["one", "one"]')
        self.assertIn("twice", str(error))

    def test_a_year_that_is_not_a_year_is_refused(self) -> None:
        error = self.broken('copyright_year = "2026"', 'copyright_year = "26"')
        self.assertIn("four digit year", str(error))

    def test_a_non_string_field_is_refused_rather_than_coerced(self) -> None:
        error = self.broken('copyright_year = "2026"', "copyright_year = 2026")
        self.assertIn("not a string", str(error))


class DerivedValues(unittest.TestCase):
    def setUp(self) -> None:
        self.facts = load(write(GOOD))

    def test_the_copyright_line_is_assembled_once(self) -> None:
        self.assertEqual(self.facts.copyright, "Copyright (C) 2026 Forename Surname")

    def test_the_licence_url_points_at_spdx(self) -> None:
        self.assertEqual(
            self.facts.licence_url, "https://spdx.org/licenses/AGPL-3.0-or-later.html"
        )

    def test_an_author_renders_given_name_first(self) -> None:
        self.assertEqual(self.facts.authors[0].full_name, "Forename Surname")


class TheCommandLine(unittest.TestCase):
    def test_writing_then_checking_passes(self) -> None:
        out = pathlib.Path(tempfile.mkdtemp())
        source = write(GOOD)
        self.assertEqual(
            generate_metadata.main(["--out", str(out), "--release-toml", str(source)]), 0
        )
        self.assertEqual(
            generate_metadata.main(
                ["--check", "--out", str(out), "--release-toml", str(source)]
            ),
            0,
        )

    def test_check_fails_when_a_generated_file_is_absent(self) -> None:
        out = pathlib.Path(tempfile.mkdtemp())
        self.assertEqual(
            generate_metadata.main(
                ["--check", "--out", str(out), "--release-toml", str(write(GOOD))]
            ),
            1,
        )

    def test_check_fails_when_a_generated_file_was_edited_by_hand(self) -> None:
        out = pathlib.Path(tempfile.mkdtemp())
        source = write(GOOD)
        generate_metadata.main(["--out", str(out), "--release-toml", str(source)])
        (out / "CITATION.cff").write_text("title: something else\n", encoding="utf-8")
        self.assertEqual(
            generate_metadata.main(
                ["--check", "--out", str(out), "--release-toml", str(source)]
            ),
            1,
        )

    def test_a_broken_release_toml_exits_one_rather_than_raising(self) -> None:
        source = write("[project]\nname = 'x'\n")
        self.assertEqual(
            generate_metadata.main(["--out", str(pathlib.Path(tempfile.mkdtemp())), "--release-toml", str(source)]),
            1,
        )

    def test_no_partial_file_is_left_behind(self) -> None:
        out = pathlib.Path(tempfile.mkdtemp())
        generate_metadata.main(["--out", str(out), "--release-toml", str(write(GOOD))])
        self.assertEqual([p.name for p in out.glob("*.part")], [])

    def test_the_validator_prints_the_facts_and_exits_zero(self) -> None:
        self.assertEqual(release_facts.main([]), 0)


if __name__ == "__main__":
    unittest.main()
