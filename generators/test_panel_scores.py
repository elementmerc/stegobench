#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""What the panel does with a scores file an interrupted run left behind.

    python3 -m unittest discover -s generators -p 'test_*.py'

WHY THIS EXISTS
---------------
`panel.jsonl` is an append log written while containers are running, so the one
thing certain to happen to it eventually is a kill in the middle of a line. The
resume reader parsed every line with no guard, which turned that half line into
a traceback on every future run of a job that costs hours: the corpus was fine,
the detectors were fine, and nothing could be resumed.

The other half is the opposite mistake. A torn line anywhere but at the end was
not written by an interruption, and reading past it would score around whatever
else has been writing to the file.
"""
from __future__ import annotations

import io
import json
import pathlib
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import panel_scores  # noqa: E402


def corpus_with(panel_lines: list[str]) -> tempfile.TemporaryDirectory:
    """A corpus of one pair, plus whatever `panel.jsonl` is said to hold."""
    tmp = tempfile.TemporaryDirectory()
    root = pathlib.Path(tmp.name)
    (root / "manifest.jsonl").write_text(
        json.dumps({"arm": "wow-0200", "clean": "clean/0.png",
                    "stego": "wow-0200/0.png"}) + "\n",
        encoding="utf-8",
    )
    (root / "panel.jsonl").write_text("".join(panel_lines), encoding="utf-8")
    return tmp


def run_report_only(root: pathlib.Path) -> tuple[int, str]:
    buffer = io.StringIO()
    with redirect_stdout(buffer), redirect_stderr(buffer):
        code = panel_scores.main(["--corpus", str(root), "--report-only"])
    return code, buffer.getvalue()


GOOD = json.dumps({"file": "clean/0.png", "aletheia_spa": 0.01}) + "\n"
ALSO_GOOD = json.dumps({"file": "wow-0200/0.png", "aletheia_spa": 0.42}) + "\n"
#: What a kill mid-write leaves: a line that stops in the middle, no newline.
TORN = '{"file": "wow-0200/0.png", "aletheia'


class ResumeTests(unittest.TestCase):
    def test_a_torn_last_line_is_discarded_and_the_rest_resumes(self):
        with corpus_with([GOOD, TORN]) as name:
            code, output = run_report_only(pathlib.Path(name))
        self.assertEqual(code, 0, output)
        self.assertIn("incomplete", output)
        self.assertIn("resuming: 1 files already scored", output)

    def test_a_torn_line_anywhere_else_stops_the_run(self):
        with corpus_with([GOOD, TORN + "\n", ALSO_GOOD]) as name:
            code, output = run_report_only(pathlib.Path(name))
        self.assertEqual(code, 2, output)
        self.assertIn("append log", output)

    def test_a_record_naming_no_file_stops_the_run(self):
        # It scored something, and there is no telling what. Keying it by
        # anything else would attach a detector's answer to the wrong image.
        nameless = json.dumps({"aletheia_spa": 0.5}) + "\n"
        with corpus_with([GOOD, nameless]) as name:
            code, output = run_report_only(pathlib.Path(name))
        self.assertEqual(code, 2, output)
        self.assertIn("names no file", output)

    def test_an_intact_file_resumes_unremarkably(self):
        with corpus_with([GOOD, ALSO_GOOD]) as name:
            code, output = run_report_only(pathlib.Path(name))
        self.assertEqual(code, 0, output)
        self.assertIn("resuming: 2 files already scored", output)
        self.assertNotIn("incomplete", output)


if __name__ == "__main__":
    unittest.main()
