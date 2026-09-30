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

import argparse
import ast
import contextlib
import inspect
import io
import pathlib
import re
import sys
import unittest
import unittest.mock

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

    def test_nothing_is_printed_above_the_listing_s_own_header(self):
        """A module that complains at import complained over the header.

        Two lines of "conseal is not installed" arriving before `pentimento`
        has said what it is read as a crash. The reason belongs beside the
        command it is about, and that is where it now goes.
        """
        code, out, err = run()
        self.assertEqual(code, 0)
        self.assertTrue(out.startswith("pentimento "), out[:120])
        self.assertEqual(err, "")

    def test_the_count_reported_matches_the_commands_listed(self):
        _, out, _ = run()
        runnable = [n for n in candidates() if isinstance(describe(n), tuple)]
        self.assertIn(f"{len(runnable)} commands", out)


class VersionTests(unittest.TestCase):
    """A corpus is quoted for years. Which toolkit built it has to be askable."""

    def test_every_spelling_of_the_question_is_answered(self):
        for spelling in ("--version", "-V", "version"):
            code, out, _ = run(spelling)
            self.assertEqual(code, 0, spelling)
            self.assertTrue(out.startswith("pentimento "), out)

    def test_the_listing_carries_the_version_too(self):
        _, out, _ = run()
        self.assertIn(f"pentimento {cli.version()}", out)


class SummaryTests(unittest.TestCase):
    """A hard cut at a fixed column lands mid-word and reads as corruption."""

    ROW = re.compile(r"^  ([a-z][a-z0-9-]+) {2,}(\S.*)$")

    def summaries(self) -> list[str]:
        _, out, _ = run()
        listed = {n.replace("_", "-") for n in candidates()}
        found = []
        for line in out.splitlines():
            m = self.ROW.match(line)
            if m and m.group(1) in listed:
                found.append(m.group(2))
        return found

    def test_the_listing_was_actually_parsed(self):
        self.assertGreater(len(self.summaries()), 20)

    def test_no_summary_runs_past_the_column_it_is_given(self):
        for summary in self.summaries():
            self.assertLessEqual(len(summary), cli.SUMMARY_WIDTH, summary)

    def test_a_summary_that_had_to_be_shortened_says_so(self):
        shortened = [s for s in self.summaries() if s.endswith("...")]
        self.assertTrue(shortened, "nothing here was long enough to shorten")
        for summary in shortened:
            self.assertTrue(summary.endswith(" ..."), summary)
            self.assertNotIn("  ", summary)


class SuggestionTests(unittest.TestCase):
    """`did you mean` earns its place only by refusing to guess wildly."""

    def test_the_distance_is_the_ordinary_one(self):
        self.assertEqual(cli.edit_distance("", ""), 0)
        self.assertEqual(cli.edit_distance("", "list"), 4)
        self.assertEqual(cli.edit_distance("list", ""), 4)
        self.assertEqual(cli.edit_distance("lst", "list"), 1)
        self.assertEqual(cli.edit_distance("kitten", "sitting"), 3)

    def test_a_transposition_is_caught(self):
        code, _, err = run("buidl-core-tier")
        self.assertEqual(code, 2)
        self.assertIn("did you mean: build-core-tier", err)

    def test_an_unrelated_word_gets_silence_rather_than_a_wrong_answer(self):
        for typed in ("wibble", "zzzz", "steganography"):
            code, _, err = run(typed)
            self.assertEqual(code, 2, typed)
            self.assertNotIn("did you mean", err, typed)

    def test_an_abbreviation_is_completed(self):
        self.assertEqual(cli.nearest("pack", candidates()),
                         ["pack_arms", "pack_tier"])

    def test_at_most_three_are_offered(self):
        self.assertLessEqual(len(cli.nearest("build", candidates())), 3)


class HelpConventionTests(unittest.TestCase):
    """What this dispatcher adds to a subcommand's parser without editing it."""

    def setUp(self):
        cli.with_defaults_in_help()
        original = sys.argv[0]
        self.addCleanup(lambda: sys.argv.__setitem__(0, original))

    def parser(self):
        ap = argparse.ArgumentParser()
        ap.add_argument("--count", type=int, default=10000, help="covers")
        ap.add_argument("--bare", type=int, default=7)
        ap.add_argument("--required", required=True)
        ap.add_argument("--flag", action="store_true")
        return ap

    def test_an_option_with_prose_gains_its_default(self):
        self.assertIn("covers (default: 10000)", self.parser().format_help())

    def test_an_option_with_no_prose_still_states_its_default(self):
        self.assertIn("(default: 7)", self.parser().format_help())

    def test_a_flag_is_left_alone(self):
        """`(default: False)` on a switch is noise, not information."""
        self.assertNotIn("default: False", self.parser().format_help())

    def test_an_option_with_no_default_says_nothing(self):
        self.assertNotIn("default: None", self.parser().format_help())

    def test_the_usage_line_names_the_subcommand(self):
        with self.assertRaises(SystemExit) as cm:
            run("verify-release", "--help")
        self.assertEqual(cm.exception.code, 0)
        self.assertEqual(sys.argv[0], "pentimento verify-release")


class LockWarningTests(unittest.TestCase):
    """A build outside the locked compiler set says so before it starts."""

    def test_the_lock_names_the_compilers_the_warning_is_about(self):
        pinned = cli.locked_versions()
        self.assertEqual(sorted(pinned), ["llvmlite", "numba"])

    def test_a_command_that_reads_rather_than_embeds_is_left_quiet(self):
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            cli.warn_if_unlocked("verify_release")
        self.assertEqual(err.getvalue(), "")

    def test_a_drifted_environment_is_named_and_the_fix_given(self):
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            with unittest.mock.patch.object(
                    cli, "locked_versions",
                    return_value={"numba": "0.0.0-not-a-real-version"}):
                cli.warn_if_unlocked("build_adaptive_arms")
        text = err.getvalue()
        self.assertIn("numba", text)
        self.assertIn("requirements.lock", text)

    def test_asking_what_a_command_does_is_not_warned_about(self):
        """Nothing is about to be built, so the environment does not matter."""
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            with unittest.mock.patch.object(
                    cli, "locked_versions", return_value={"numba": "0.0.0"}):
                cli.warn_if_unlocked("build_adaptive_arms", ["--help"])
                cli.warn_if_unlocked("build_adaptive_arms", ["-h"])
        self.assertEqual(err.getvalue(), "")

    def test_an_unreadable_lock_is_silence_rather_than_a_false_alarm(self):
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            with unittest.mock.patch.object(cli, "locked_versions",
                                            return_value={}):
                cli.warn_if_unlocked("build_adaptive_arms")
        self.assertEqual(err.getvalue(), "")


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


HERE = pathlib.Path(__file__).resolve().parent


def accepts(signature: inspect.Signature, *args: object) -> bool:
    try:
        signature.bind(*args)
    except TypeError:
        return False
    return True


def declared_signature(name: str) -> inspect.Signature | None:
    """`main`'s signature read from the file, for a module that will not import.

    Half the subcommands that carried this fault need an optional dependency,
    so a check that can only inspect imported modules is a check that would
    have caught half of them. Reading the source catches the rest on a machine
    where `conseal` or `jpeglib` is absent. Only a module-level `def main`
    counts: one generator carries a whole script in a string literal, and the
    `main` inside that string is not this module's entry point.
    """
    source = (HERE / f"{name}.py").read_text(encoding="utf-8")
    for node in ast.parse(source).body:
        if not isinstance(node, ast.FunctionDef) or node.name != "main":
            continue
        args = node.args
        positional = args.posonlyargs + args.args
        first_default = len(positional) - len(args.defaults)
        params = []
        for index, arg in enumerate(positional):
            kind = (inspect.Parameter.POSITIONAL_ONLY
                    if index < len(args.posonlyargs)
                    else inspect.Parameter.POSITIONAL_OR_KEYWORD)
            default = (inspect.Parameter.empty if index < first_default
                       else ast.unparse(args.defaults[index - first_default]))
            params.append(inspect.Parameter(arg.arg, kind, default=default))
        if args.vararg:
            params.append(inspect.Parameter(
                args.vararg.arg, inspect.Parameter.VAR_POSITIONAL))
        for arg, default in zip(args.kwonlyargs, args.kw_defaults):
            params.append(inspect.Parameter(
                arg.arg, inspect.Parameter.KEYWORD_ONLY,
                default=(inspect.Parameter.empty if default is None
                         else ast.unparse(default))))
        if args.kwarg:
            params.append(inspect.Parameter(
                args.kwarg.arg, inspect.Parameter.VAR_KEYWORD))
        return inspect.Signature(params)
    return None


class EntryPointShapeTests(unittest.TestCase):
    """Every subcommand has to answer to both documented ways of running it.

    The dispatcher calls `module.main(argv[1:])`; `python3 generators/x.py`
    reaches the same function through `main()`. The root README and
    docs/guide/quickstart.md both spell out the direct route, and the
    dispatcher's own docstring spells out the other. Six subcommands shipped
    as `def main() -> int:`, which
    meant every invocation through `pentimento` died on a TypeError inside the
    dispatcher before the subcommand's own parser ever ran.

    This walks the dispatch table rather than naming today's modules, so the
    next module added with the wrong shape fails here instead of at somebody's
    first invocation.
    """

    def signatures(self) -> list[tuple[str, inspect.Signature]]:
        found = []
        for name in candidates():
            described = describe(name)
            if described is None:
                continue
            signature = (declared_signature(name)
                         if isinstance(described, Unavailable)
                         else inspect.signature(described[0].main))
            if signature is not None:
                found.append((name, signature))
        return found

    def test_the_check_actually_reaches_the_subcommands(self):
        """A walk that finds nothing would pass every assertion below."""
        self.assertGreater(len(self.signatures()), 30)

    def test_every_main_can_be_called_the_way_the_dispatcher_calls_it(self):
        for name, signature in self.signatures():
            with self.subTest(module=name):
                self.assertTrue(
                    accepts(signature, []),
                    f"cli.py calls {name}.main(argv[1:]), and "
                    f"main{signature} cannot be called that way. Give it "
                    f"`argv: list[str] | None = None` and pass argv through "
                    f"to parse_args, as the other subcommands do")

    def test_every_main_can_still_be_called_with_no_arguments(self):
        for name, signature in self.signatures():
            with self.subTest(module=name):
                self.assertTrue(
                    accepts(signature),
                    f"the README documents running a generator directly, and "
                    f"{name}.main{signature} cannot be called with no "
                    f"arguments, so `python3 generators/{name}.py` is broken")


if __name__ == "__main__":
    unittest.main()
