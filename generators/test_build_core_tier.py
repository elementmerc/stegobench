#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the tier orchestrator's preflight.

    python3 -m unittest discover -s generators -p 'test_*.py'

A tier build is thirty five jobs and several days. The plan that promises it
used to look at nothing: `--dry-run` printed a schedule for a run that could
not start, and the real command then discovered that thirty five times over,
once per job, an hour in. These tests hold the plan to checking what it
promises.
"""
from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from build_core_tier import BYTES_PER_COVER, preflight  # noqa: E402


class TestPreflight(unittest.TestCase):
    def setUp(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.addCleanup(self._dir.cleanup)
        self.tmp = pathlib.Path(self._dir.name)
        self.covers = self.tmp / "covers"
        self.covers.mkdir()
        self.out = self.tmp / "out"
        self.out.mkdir()
        self.manifest = self.covers / "manifest.jsonl"

    def _manifest(self, rows: list[dict]) -> None:
        self.manifest.write_text(
            "".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")

    def _good(self, count: int) -> None:
        self._manifest([{"file": f"{i:05d}.png", "tier_order": i}
                        for i in range(count)])

    def test_a_plan_that_can_run_reports_nothing(self) -> None:
        self._good(10)
        self.assertEqual(preflight(10, self.covers, self.manifest, self.out), [])

    def test_a_missing_manifest_is_caught_before_any_job_starts(self) -> None:
        """Every job selects through it, so all thirty five fail the same way.

        Each one reported that separately, in its own log file, an hour after
        the plan said the build would take six days.
        """
        problems = preflight(10, self.covers, self.manifest, self.out)
        self.assertEqual(len(problems), 1)
        self.assertIn("no manifest at", problems[0])

    def test_a_missing_cover_directory_is_caught(self) -> None:
        self._good(10)
        gone = self.tmp / "not-here"
        problems = preflight(10, gone, self.manifest, self.out)
        self.assertTrue(any("no cover directory" in p for p in problems), problems)

    def test_a_manifest_with_no_tier_order_names_the_repair(self) -> None:
        """The tier selector owns this rule, so the plan asks it rather than
        restating it and drifting from it."""
        self._manifest([{"file": "00000.png"}])
        problems = preflight(1, self.covers, self.manifest, self.out)
        self.assertEqual(len(problems), 1)
        self.assertIn("manifest-repair", problems[0])

    def test_a_tier_larger_than_the_corpus_is_caught(self) -> None:
        self._good(3)
        problems = preflight(10, self.covers, self.manifest, self.out)
        self.assertEqual(len(problems), 1)
        self.assertIn("asked for 10 covers", problems[0])

    def test_a_disk_that_cannot_hold_the_build_is_caught(self) -> None:
        """A build that fills the disk partway leaves arms that look finished.

        Nothing downstream can tell a truncated arm from a complete one
        without counting, and the packer counts what is there rather than what
        should be.
        """
        # A count no disk on this fleet can hold, so the check fires whatever
        # the machine running the tests happens to have free. The manifest
        # stays small deliberately: writing one row per cover here would mean
        # writing two hundred million lines to prove a division.
        self._good(10)
        huge = int(1e15 / BYTES_PER_COVER)
        problems = preflight(huge, self.covers, self.manifest, self.out)
        self.assertTrue(any("free and this build writes at least" in p
                            for p in problems), problems)

    def test_every_problem_is_reported_rather_than_only_the_first(self) -> None:
        """Fixing one and rerunning to find the next is the loop this avoids.

        Each rerun of the real command costs an hour.
        """
        gone = self.tmp / "not-here"
        problems = preflight(10, gone, self.manifest, self.out)
        self.assertEqual(len(problems), 2, problems)


if __name__ == "__main__":
    unittest.main()
