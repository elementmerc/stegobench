#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the release uploader, and mostly for the shared bandwidth budget.

    python3 -m unittest discover -s tools/release -p 'test_*.py'

The budget is the piece worth testing. Everything else here either talks to a
remote service or is a dry run that prints. The budget is local arithmetic
under a lock, it is shared between processes that cannot see each other, and if
it is wrong the failure is silent: three uploads each politely limiting
themselves to 2 MB/s while the line carries six.
"""
from __future__ import annotations

import multiprocessing
import pathlib
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import upload_tier  # noqa: E402
from upload_tier import SharedBudget, UploadError  # noqa: E402

#: The shared budget needs POSIX file locking. On a platform without it the
#: uploader refuses rather than limiting each process separately, so that is
#: what gets tested there.
POSIX = upload_tier.fcntl is not None


def drain(budget_path: str, rate: int, chunks: int, size: int) -> float:
    """Reserve `chunks` of `size` bytes and report how long it took."""
    budget = SharedBudget(pathlib.Path(budget_path), rate)
    started = time.monotonic()
    for _ in range(chunks):
        budget.reserve(size)
    return time.monotonic() - started


@unittest.skipUnless(POSIX, "the shared budget needs POSIX file locking")
class SharedBudgetTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = pathlib.Path(self.tmp.name) / "budget"

    def test_one_sender_is_held_to_the_rate(self):
        # 40 chunks of 1000 bytes at 40,000 B/s is one second of line time.
        budget = SharedBudget(self.path, 40_000)
        started = time.monotonic()
        for _ in range(40):
            budget.reserve(1000)
        elapsed = time.monotonic() - started
        self.assertGreater(elapsed, 0.8)
        self.assertLess(elapsed, 2.0)

    def test_four_senders_share_one_rate_rather_than_taking_four(self):
        """The whole point. Four processes, one line, one rate.

        Each sends a quarter of a second's worth on its own. Sharing correctly,
        the four together take about a second. Sharing incorrectly, they each
        take a quarter of a second and finish in a quarter of the time, which is
        the bug this exists to prevent.
        """
        rate = 40_000
        with multiprocessing.Pool(4) as pool:
            started = time.monotonic()
            pool.starmap(drain, [(str(self.path), rate, 10, 1000)] * 4)
            elapsed = time.monotonic() - started

        # 40 chunks of 1000 bytes in total, at 40,000 B/s, is one second.
        self.assertGreater(
            elapsed, 0.8,
            "four senders finished faster than the shared rate allows, so the "
            "budget is being taken four times over")
        self.assertLess(elapsed, 3.0)

    def test_a_stale_reservation_does_not_stall_the_next_run(self):
        # A killed run can leave a reservation far in the future. Obeying it
        # would park the next upload for as long as it says.
        self.path.write_text(f"{time.time() + 86_400:.6f}\n")
        budget = SharedBudget(self.path, 1_000_000)
        started = time.monotonic()
        budget.reserve(1000)
        self.assertLess(time.monotonic() - started, 5.0)

    def test_a_corrupt_budget_file_is_treated_as_empty(self):
        self.path.write_text("not a number at all\n")
        budget = SharedBudget(self.path, 1_000_000)
        budget.reserve(1000)  # must not raise
        self.assertTrue(float(self.path.read_text().strip()) > 0)

    def test_no_budget_file_still_limits_this_process(self):
        budget = SharedBudget(None, 40_000)
        started = time.monotonic()
        budget.reserve(20_000)
        self.assertGreater(time.monotonic() - started, 0.3)

    def test_a_zero_rate_means_no_limit(self):
        budget = SharedBudget(self.path, 0)
        started = time.monotonic()
        for _ in range(100):
            budget.reserve(1 << 20)
        self.assertLess(time.monotonic() - started, 0.5)




@unittest.skipIf(POSIX, "this is the behaviour on a platform without flock")
class NoFileLockingTests(unittest.TestCase):
    def test_a_shared_budget_is_refused_rather_than_taken_per_process(self):
        with self.assertRaises(UploadError) as cm:
            SharedBudget(pathlib.Path("budget"), 1000)
        self.assertIn("file locking", str(cm.exception))

    def test_a_single_process_budget_still_works(self):
        budget = SharedBudget(None, 40_000)
        started = time.monotonic()
        budget.reserve(20_000)
        self.assertGreater(time.monotonic() - started, 0.3)


if __name__ == "__main__":
    unittest.main()
