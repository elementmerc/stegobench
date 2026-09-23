#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the single entry point.

    python3 -m unittest discover -s generators -p 'test_*.py'

The dispatcher's job is to make forty-two programs findable, and the way it
fails is by making one of them quietly unfindable. A module that cannot be
imported because an optional dependency is absent must be NAMED as unavailable,
not omitted: an absence leaves somebody comparing the listing against the
README and concluding the tool is out of date, when the truth is that `conseal`
is not installed.
"""
from __future__ import annotations

import contextlib
import io
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import cli  # noqa: E402
from cli import Unavailable, candidates, describe, normalise  # noqa: E402


def run(*argv: str) -> tuple[int, str, str]:
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = cli.main(list(argv))
    return code, out.getvalue(), err.getvalue()


class DiscoveryTests(unittest.TestCase):
    def test_test_modules_are_not_subcommands(self):
        self.assertFalse([n for n in candidates() if n.startswith("test_")])

    def test_the_dispatcher_does_not_list_itself(self):
        self.assertNotIn("cli", candidates())

    def test_the_known_commands_are_discovered(self):
        found = candidates()
        for name in ("verify_release", "pack_tier", "backfill_covers",
                     "validate_croissant", "select_unpublishable"):
            self.assertIn(name, found)

    def test_a_library_is_not_offered_as_a_command(self):
        """`payloads` has no main(), so running it would do nothing."""
        self.assertIsNone(describe("payloads"))

    def test_hyphens_and_underscores_are_the_same_command(self):
        self.assertEqual(normalise("verify-release"), "verify_release")


class ListingTests(unittest.TestCase):
    def test_the_listing_succeeds_and_names_commands(self):
        code, out, _ = run()
        self.assertEqual(code, 0)
        self.assertIn("verify-release", out)
        self.assertIn("pentimento <command>", out)

    def test_help_is_the_same_as_no_arguments(self):
        self.assertEqual(run("--help")[1], run()[1])

    def test_an_unimportable_command_is_NAMED_not_hidden(self):
        """The fault this guards.

        Dropping a module that exits at import turns "your environment is
        missing conseal" into "that command does not exist", and the second is
        both wrong and unactionable.
        """
        code, out, _ = run()
        self.assertEqual(code, 0)
        broken = [n for n in candidates()
                  if isinstance(describe(n), Unavailable)]
        for name in broken:
            self.assertIn(name.replace("_", "-"), out,
                          f"{name} cannot be imported here and was not named "
                          f"in the listing, so it looks like it does not exist")

    def test_the_count_reported_matches_the_commands_listed(self):
        _, out, _ = run()
        runnable = [n for n in candidates() if isinstance(describe(n), tuple)]
        self.assertIn(f"{len(runnable)} commands", out)


class DispatchTests(unittest.TestCase):
    def test_an_unknown_command_is_refused_with_a_usage_code(self):
        code, _, err = run("not-a-real-command")
        self.assertEqual(code, 2)
        self.assertIn("no such command", err)

    def test_a_near_miss_gets_a_suggestion(self):
        code, _, err = run("verify")
        self.assertEqual(code, 2)
        self.assertIn("verify-release", err)

    def test_a_library_named_directly_says_so_rather_than_crashing(self):
        code, _, err = run("payloads")
        self.assertEqual(code, 2)
        self.assertIn("library", err)

    def test_a_command_is_reached_and_its_own_parser_runs(self):
        """`--help` on a subcommand belongs to that subcommand, not to us."""
        with self.assertRaises(SystemExit) as cm:
            run("verify-release", "--help")
        self.assertEqual(cm.exception.code, 0)

    def test_an_unavailable_command_says_what_to_install(self):
        broken = [n for n in candidates()
                  if isinstance(describe(n), Unavailable)]
        if not broken:
            self.skipTest("every optional dependency is installed here")
        code, _, err = run(broken[0].replace("_", "-"))
        self.assertEqual(code, 2)
        self.assertIn("pip install", err)


if __name__ == "__main__":
    unittest.main()
