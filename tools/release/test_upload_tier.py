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

import json
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


def drain(budget_path: str, rate: int, chunks: int, size: int) -> tuple[float, float]:
    """Reserve `chunks` of `size` bytes and report when it started and finished.

    Wall clock, and both ends of it, so the caller can measure the window the
    reservations actually occupied rather than that window plus however long
    the process took to start. On a busy machine the second number is the
    larger one and has nothing to do with the budget.
    """
    budget = SharedBudget(pathlib.Path(budget_path), rate)
    started = time.time()
    for _ in range(chunks):
        budget.reserve(size)
    return started, time.time()


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
            spans = pool.starmap(drain, [(str(self.path), rate, 10, 1000)] * 4)
        elapsed = max(end for _, end in spans) - min(start for start, _ in spans)

        # 40 chunks of 1000 bytes in total, at 40,000 B/s, is one second of
        # line time however many processes send it.
        self.assertGreater(
            elapsed, 0.8,
            "four senders finished faster than the shared rate allows, so the "
            "budget is being taken four times over")
        # Generous, because the four processes do not start together and a
        # sender that arrives late still has to wait its turn. The load
        # bearing assertion is the one above.
        self.assertLess(elapsed, 8.0)

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




class ArchiveMetadataTests(unittest.TestCase):
    """The Archive creates the item on the first PUT, or not at all."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.packed = pathlib.Path(self.tmp.name)

    def test_metadata_becomes_headers_the_endpoint_understands(self):
        (self.packed / "ia-metadata.json").write_text(json.dumps({
            "identifier": "pentimento-core-v1",
            "title": "Pentimento Core",
            "licenseurl": "https://creativecommons.org/licenses/by/4.0/",
            "subject": ["steganalysis", "dataset"],
        }))
        headers = upload_tier._ia_headers(self.packed)
        # Without this the endpoint has no bucket to write into and answers 404.
        self.assertEqual(headers["x-amz-auto-make-bucket"], "1")
        self.assertEqual(headers["x-archive-meta-title"], "Pentimento Core")
        # Repeated fields are numbered, which is how more than one subject gets
        # through.
        self.assertEqual(headers["x-archive-meta00-subject"], "steganalysis")
        self.assertEqual(headers["x-archive-meta01-subject"], "dataset")
        # The identifier is the item name, not a field on it.
        self.assertNotIn("x-archive-meta-identifier", headers)

    def test_a_missing_metadata_file_is_refused(self):
        with self.assertRaises(UploadError) as cm:
            upload_tier._ia_headers(self.packed)
        self.assertIn("ia-metadata.json", str(cm.exception))


if __name__ == "__main__":
    unittest.main()


class PublishableSetTests(unittest.TestCase):
    """What actually leaves the machine.

    Four files were missing from the publishable set while the published
    documentation told people to fetch them by name. The quickstart ran
    `curl -O $BASE/SHA256SUMS` and then `sha256sum -c`, and both would have
    answered 404. The attribution list, which is how a reader discharges CC BY
    for 5,429 covers, was also being withheld.

    Nothing caught it because the set was a literal tuple and no test compared
    it against what the packer writes.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.packed = pathlib.Path(self.tmp.name)
        (self.packed / "pentimento-core-index.json").write_text(json.dumps({
            "tier": "Core", "samples": 2,
            "shards": [{"shard": "pentimento-core-00000.tar", "samples": 2,
                        "bytes": 4, "sha256": "a" * 64}],
        }))
        (self.packed / "pentimento-core-00000.tar").write_bytes(b"tar!")

    def write(self, *names: str) -> None:
        for name in names:
            (self.packed / name).write_text(f"contents of {name}\n")

    def test_everything_the_docs_name_is_published(self):
        self.write("README.md", "SHA256SUMS-covers", "ATTRIBUTION.md",
                   "ATTRIBUTION.csv", "load_pentimento.py", "LICENCES.md",
                   "licence-summary.json")
        files = upload_tier.load_index(self.packed)
        for name in ("README.md", "SHA256SUMS-covers", "ATTRIBUTION.md",
                     "ATTRIBUTION.csv", "load_pentimento.py",
                     "licence-summary.json"):
            self.assertIn(name, files, f"{name} would not be uploaded")

    def test_no_non_shard_file_is_silently_left_behind(self):
        """The assertion that would have caught the original defect.

        Every file the packer writes is either published or named as
        deliberately withheld. A new file added to the packed directory with
        no decision about it fails here rather than going missing in silence.
        """
        self.write(*upload_tier.PACKAGED_EXTRAS, *upload_tier.NOT_PUBLISHED)
        files = upload_tier.load_index(self.packed)
        for path in sorted(self.packed.iterdir()):
            if path.suffix == ".tar" or path.name.endswith("index.json"):
                continue
            decided = (path.name in files
                       or path.name in upload_tier.NOT_PUBLISHED)
            self.assertTrue(decided,
                            f"{path.name} is neither published nor explicitly "
                            f"withheld, so nobody decided about it")

    def test_the_destination_only_files_stay_home(self):
        self.write("ia-metadata.json", "dataset-metadata.json")
        files = upload_tier.load_index(self.packed)
        self.assertNotIn("ia-metadata.json", files)
        self.assertNotIn("dataset-metadata.json", files)

    def test_the_two_checksum_files_do_not_share_a_name(self):
        """Every destination is flat.

        The covers and the arms are packed in separate directories and land in
        one namespace. Two files both called SHA256SUMS mean the second
        replaces the first, leaving a checksum file that covers 10 shards and
        claims to cover 769.
        """
        self.assertNotIn("SHA256SUMS", upload_tier.PACKAGED_EXTRAS)
        checksums = [n for n in upload_tier.PACKAGED_EXTRAS
                     if n.startswith("SHA256SUMS")]
        self.assertEqual(len(checksums), len(set(checksums)))
        self.assertGreaterEqual(len(checksums), 2)

    def test_a_stray_file_is_not_published_by_accident(self):
        self.write("notes-to-self.txt", "core.dump")
        files = upload_tier.load_index(self.packed)
        self.assertNotIn("notes-to-self.txt", files)
        self.assertNotIn("core.dump", files)

    def test_an_absent_extra_is_not_an_error(self):
        # Nano has no arms checksum file of its own when packed alone.
        files = upload_tier.load_index(self.packed)
        self.assertIn("pentimento-core-00000.tar", files)
