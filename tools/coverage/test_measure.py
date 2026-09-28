#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the coverage measuring tool.

A tool whose whole job is holding this repository to a coverage figure arrived
with no tests of its own, and the one bug found in it was found by hand. That
is the joke writing itself, and these are the tests.

Nothing here runs a real suite. Every test drives the pure parts: reading and
rewriting the floors, turning per-file figures into a component figure, and
deciding a verdict. Running the real suites is what the tool does; whether it
does it correctly is what CI answers, and a unit test that shelled out to
`cargo` would take minutes and prove nothing extra.
"""

import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import measure  # noqa: E402


def component(name, covered, total, unit="statements and branches"):
    return measure.Component(
        name=name, unit=unit,
        files=[measure.FileCoverage(path="a.py", covered=covered, total=total)],
    )


class FloorsFile:
    """A floors.toml in a temporary directory, swapped in for the real one."""

    def __init__(self, text):
        self.dir = tempfile.TemporaryDirectory()
        self.path = pathlib.Path(self.dir.name) / "floors.toml"
        self.path.write_text(text, encoding="utf-8")
        self.real = measure.FLOORS_PATH

    def __enter__(self):
        measure.FLOORS_PATH = self.path
        return self.path

    def __exit__(self, *exc):
        measure.FLOORS_PATH = self.real
        self.dir.cleanup()
        return False


class PercentTests(unittest.TestCase):
    def test_a_component_sums_its_files_rather_than_averaging_them(self):
        # Averaging per-file percentages would let a one line file with full
        # coverage cancel out a thousand line file with none.
        c = measure.Component(name="x", unit="u", files=[
            measure.FileCoverage("small.py", covered=1, total=1),
            measure.FileCoverage("big.py", covered=0, total=999),
        ])
        self.assertAlmostEqual(c.percent, 0.1, places=2)

    def test_nothing_to_measure_is_full_coverage_rather_than_a_crash(self):
        self.assertEqual(measure.FileCoverage("a.py", 0, 0).percent, 100.0)
        self.assertEqual(measure.Component("x", "u", []).percent, 100.0)


class LoadFloorsTests(unittest.TestCase):
    def test_the_shipped_floors_file_is_valid(self):
        floors = measure.load_floors()
        self.assertIn("generators", floors)
        self.assertIn("rust", floors)
        for name, floor in floors.items():
            self.assertTrue(0 <= floor <= 100, f"{name} is {floor}")

    def test_a_missing_file_is_refused_rather_than_treated_as_no_floor(self):
        # No file would otherwise mean no floor, so a ratchet could be removed
        # by deleting it, which is the one thing a ratchet must not allow.
        with FloorsFile("") as path:
            path.unlink()
            with self.assertRaises(measure.MeasurementError) as caught:
                measure.load_floors()
        self.assertIn("missing", str(caught.exception))

    def test_a_file_recording_no_components_is_refused(self):
        with FloorsFile("# nothing here\n"):
            with self.assertRaises(measure.MeasurementError) as caught:
                measure.load_floors()
        self.assertIn("no components", str(caught.exception))

    def test_a_component_with_no_floor_is_refused(self):
        with FloorsFile('[component.x]\nnote = "soon"\n'):
            with self.assertRaises(measure.MeasurementError) as caught:
                measure.load_floors()
        self.assertIn("no `floor`", str(caught.exception))

    def test_a_floor_that_is_not_a_percentage_is_refused(self):
        for bad in ("floor = 101", "floor = -1", 'floor = "ninety"'):
            with FloorsFile(f"[component.x]\n{bad}\n"):
                with self.assertRaises(measure.MeasurementError):
                    measure.load_floors()

    def test_broken_toml_says_so_rather_than_reporting_no_floors(self):
        with FloorsFile("[component.x\nfloor = 1\n"):
            with self.assertRaises(measure.MeasurementError) as caught:
                measure.load_floors()
        self.assertIn("not valid TOML", str(caught.exception))


class BumpTests(unittest.TestCase):
    def test_a_bare_header_is_rewritten(self):
        with FloorsFile("[component.generators]\nfloor = 43\n") as path:
            measure.bump_floors([component("generators", 50, 100)])
            self.assertIn("floor = 50", path.read_text())

    def test_a_quoted_header_is_rewritten(self):
        # The bug the author found by hand: only the quoted form was tried, so
        # a bare header raised rather than bumping. Both forms are legal TOML
        # and this file uses both, because `tools/release` has to be quoted.
        with FloorsFile('[component."tools/release"]\nfloor = 67\n') as path:
            measure.bump_floors([component("tools/release", 70, 100)])
            self.assertIn("floor = 70", path.read_text())

    def test_a_floor_is_never_lowered(self):
        # A ratchet that can go down is not a ratchet. A bad run, a skipped
        # suite or a machine without a tool would otherwise quietly lower the
        # bar and nobody would see it in the diff.
        with FloorsFile("[component.generators]\nfloor = 80\n") as path:
            measure.bump_floors([component("generators", 50, 100)])
            self.assertIn("floor = 80", path.read_text())

    def test_the_figure_is_rounded_down_not_to_nearest(self):
        # 89.9 becoming a floor of 90 would fail the very next run on the same
        # code, which is a gate that fails for no reason anybody can act on.
        with FloorsFile("[component.generators]\nfloor = 10\n") as path:
            measure.bump_floors([component("generators", 899, 1000)])
            self.assertIn("floor = 89", path.read_text())

    def test_only_the_named_component_moves(self):
        text = ('[component.generators]\nfloor = 43\n\n'
                '[component."tools/release"]\nfloor = 67\n')
        with FloorsFile(text) as path:
            measure.bump_floors([component("generators", 50, 100)])
            written = path.read_text()
        self.assertIn("floor = 50", written)
        self.assertIn("floor = 67", written)

    def test_a_component_the_file_has_never_heard_of_gets_a_first_floor(self):
        # It used to be skipped in silence, so a newly added crate or suite
        # would be measured and never gated, which is the "ratchet with no
        # record" this file's own error message warns about.
        with FloorsFile("[component.generators]\nfloor = 43\n") as path:
            measure.bump_floors([component("newcrate", 77, 100)])
            written = path.read_text()
        self.assertIn("[component.newcrate]", written)
        self.assertIn("floor = 77", written)
        self.assertIn("floor = 43", written, "the existing floor was disturbed")
        # And it parses, which a hand-built TOML section is not guaranteed to.
        with FloorsFile(written):
            self.assertEqual(measure.load_floors()["newcrate"], 77.0)

    def test_a_first_floor_for_a_name_needing_quotes_is_written_quoted(self):
        with FloorsFile("[component.generators]\nfloor = 43\n") as path:
            measure.bump_floors([component("tools/release", 68, 100)])
            written = path.read_text()
        self.assertIn('[component."tools/release"]', written)
        with FloorsFile(written):
            self.assertEqual(measure.load_floors()["tools/release"], 68.0)

    def test_a_first_floor_is_appended_even_when_the_file_lacks_a_final_newline(self):
        with FloorsFile("[component.generators]\nfloor = 43") as path:
            measure.bump_floors([component("newcrate", 50, 100)])
            with FloorsFile(path.read_text()):
                floors = measure.load_floors()
        self.assertEqual(floors["newcrate"], 50.0)
        self.assertEqual(floors["generators"], 43.0)

    def test_a_floor_written_unexpectedly_is_refused_rather_than_guessed_at(self):
        with FloorsFile("[component.generators]\nfloor=43\n"):
            with self.assertRaises(measure.MeasurementError) as caught:
                measure.bump_floors([component("generators", 50, 100)])
        self.assertIn("by hand", str(caught.exception))

    def test_an_unchanged_bump_leaves_the_file_byte_identical(self):
        text = "[component.generators]\nfloor = 43\n"
        with FloorsFile(text) as path:
            before = path.read_bytes()
            measure.bump_floors([component("generators", 43, 100)])
            self.assertEqual(path.read_bytes(), before)

    def test_no_part_file_survives_a_bump(self):
        with FloorsFile("[component.generators]\nfloor = 43\n") as path:
            measure.bump_floors([component("generators", 50, 100)])
            leftovers = list(path.parent.glob("*.part"))
        self.assertEqual(leftovers, [], "a half written floors file was left behind")

    def test_comments_and_surrounding_prose_survive(self):
        # The file carries the reasoning for each floor, and a rewriter that
        # ate it would make the next reader wonder why the number is what it is.
        text = ("# why this floor is what it is\n"
                "[component.generators]\nfloor = 43\n# trailing note\n")
        with FloorsFile(text) as path:
            measure.bump_floors([component("generators", 50, 100)])
            written = path.read_text()
        self.assertIn("# why this floor is what it is", written)
        self.assertIn("# trailing note", written)


class RenderTests(unittest.TestCase):
    def test_meeting_the_floor_passes(self):
        _, ok = measure.render([component("x", 50, 100)], {"x": 43.0})
        self.assertTrue(ok)

    def test_falling_below_the_floor_fails(self):
        report, ok = measure.render([component("x", 40, 100)], {"x": 43.0})
        self.assertFalse(ok)
        self.assertIn("x", report)

    def test_exactly_the_floor_passes(self):
        # A ratchet set to the measured figure must not fail on the run that
        # set it.
        _, ok = measure.render([component("x", 43, 100)], {"x": 43.0})
        self.assertTrue(ok)

    def test_a_component_with_no_recorded_floor_is_visible_in_the_report(self):
        report, _ = measure.render([component("newthing", 50, 100)], {})
        self.assertIn("newthing", report)

    def test_the_unit_is_carried_into_the_report(self):
        # Rust is region coverage and Python is statements and branches. A
        # table that printed bare percentages would invite comparing them.
        report, _ = measure.render(
            [component("rust", 90, 100, unit="regions")], {"rust": 89.0})
        self.assertIn("regions", report)

    def test_files_below_the_changed_file_bar_are_named_not_counted(self):
        c = measure.Component(name="x", unit="u", files=[
            measure.FileCoverage("well_covered.py", covered=100, total=100),
            measure.FileCoverage("barely_covered.py", covered=5, total=100),
        ])
        report, _ = measure.render([c], {"x": 10.0})
        self.assertIn("barely_covered.py", report)


class ConfigurationTests(unittest.TestCase):
    def test_the_changed_file_bar_matches_the_written_rule(self):
        # CLAUDE.md section 7 says 90% on changed files. If somebody lowers
        # this constant, the tool stops reporting against the rule it claims
        # to report against.
        self.assertEqual(measure.CHANGED_FILE_BAR, 90.0)

    def test_every_subprocess_has_a_deadline(self):
        self.assertGreater(measure.PYTHON_TIMEOUT_S, 0)
        self.assertGreater(measure.RUST_TIMEOUT_S, 0)

    def test_the_coverage_configuration_ships_beside_the_tool(self):
        self.assertTrue(measure.COVERAGERC.exists(), measure.COVERAGERC)

    def test_branch_coverage_is_on_for_the_python_half(self):
        # Line coverage would overstate it, and the rule is about branches.
        self.assertIn("branch = True", measure.COVERAGERC.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
