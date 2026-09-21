#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""The README describes every module here, and this is what makes that true.

    python3 -m unittest discover -s generators -p 'test_*.py'

WHY THIS EXISTS
---------------
`README.md` opened by saying the directory had grown to forty-two programs
while the file described three, and then claimed the table "goes stale loudly
rather than quietly". Nothing checked, so it went stale quietly: two modules
were missing from it, and one of them, `stamp_source_digests.py`, is a REQUIRED
step between building the arms and packing them. A reader following the
documented order would have packed a corpus whose arms cannot prove where they
came from, and `verify_release.py` refuses exactly that.

A claim of completeness that nothing verifies is the same fault the corpus
tooling keeps finding elsewhere: "nobody checked" and "checked and found
nothing wrong" render as the same clean page.
"""
from __future__ import annotations

import pathlib
import re
import unittest

HERE = pathlib.Path(__file__).resolve().parent
README = HERE / "README.md"

#: Not programs: test modules, and package plumbing.
def modules() -> set[str]:
    return {
        p.name
        for p in HERE.glob("*.py")
        if not p.name.startswith(("test_", "__"))
    }


def documented() -> set[str]:
    """Every module named in a table row, as `` `name.py` ``."""
    return set(re.findall(r"^\|\s*`([A-Za-z0-9_]+\.py)`", README.read_text(),
                          re.MULTILINE))


WORDS = {
    40: "Forty", 41: "Forty-one", 42: "Forty-two", 43: "Forty-three",
    44: "Forty-four", 45: "Forty-five", 46: "Forty-six", 47: "Forty-seven",
    48: "Forty-eight", 49: "Forty-nine", 50: "Fifty",
}


class ReadmeTests(unittest.TestCase):
    def test_every_module_has_a_row(self):
        missing = sorted(modules() - documented())
        self.assertEqual(
            missing, [],
            f"{len(missing)} module(s) exist here and are not in README.md: "
            f"{', '.join(missing)}. A reader comparing the two concludes the "
            f"tool does not exist, and if it is a step in the release chain "
            f"they skip it")

    def test_no_row_names_a_module_that_is_gone(self):
        stale = sorted(documented() - modules())
        self.assertEqual(
            stale, [],
            f"README.md documents {len(stale)} module(s) that are not here: "
            f"{', '.join(stale)}. A row pointing at nothing is worse than no "
            f"row, because it sends somebody looking")

    def test_the_count_in_the_opening_line_is_the_real_count(self):
        n = len(modules())
        self.assertIn(n, WORDS, f"{n} modules; add the word to WORDS")
        first = README.read_text().splitlines()[2]
        self.assertTrue(
            first.startswith(f"{WORDS[n]} programs"),
            f"README.md opens with {first!r} but there are {n} modules")

    def test_this_test_is_not_vacuous(self):
        """A pass over an empty set is not a pass.

        If the glob ever stops matching, both checks above go green while
        verifying nothing at all.
        """
        self.assertGreater(len(modules()), 20)
        self.assertGreater(len(documented()), 20)


if __name__ == "__main__":
    unittest.main()
