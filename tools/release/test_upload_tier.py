#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
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
import shutil
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
        self.path.write_text(f"{time.time() + 86_400:.6f}\n", encoding="utf-8")
        budget = SharedBudget(self.path, 1_000_000)
        started = time.monotonic()
        budget.reserve(1000)
        self.assertLess(time.monotonic() - started, 5.0)

    def test_a_corrupt_budget_file_is_treated_as_empty(self):
        self.path.write_text("not a number at all\n", encoding="utf-8")
        budget = SharedBudget(self.path, 1_000_000)
        budget.reserve(1000)  # must not raise
        self.assertTrue(float(self.path.read_text(encoding="utf-8").strip()) > 0)

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
        }), encoding="utf-8")
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
        }), encoding="utf-8")
        (self.packed / "pentimento-core-00000.tar").write_bytes(b"tar!")

    def write(self, *names: str) -> None:
        for name in names:
            (self.packed / name).write_text(f"contents of {name}\n", encoding="utf-8")

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


class IaMetadataReconcileTests(unittest.TestCase):
    """Item metadata does NOT ride along with a PUT to an existing item.

    `x-archive-meta-*` headers are applied by the S3 endpoint only when
    `x-amz-auto-make-bucket` creates the item. On an item that already exists
    they are ignored and the PUT still succeeds, so re-uploading a corrected
    README leaves the item's description exactly as it was.

    That is not hypothetical: `pentimento-core-v1` was created by hand on
    2026-09-19 with a description claiming 5,429 covers require attribution,
    against a corpus that holds 5,453, and it stood for three days.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.packed = pathlib.Path(self.tmp.name)
        (self.packed / "ia-metadata.json").write_text(json.dumps({
            "identifier": "pentimento-core-v1",
            "title": "Pentimento Core",
            "description": "5,453 covers (54.5%) require attribution.",
            "collection": "datasets",
            "subject": ["steganalysis", "dataset"],
        }), encoding="utf-8")
        self.creds = {"IA_ACCESS_KEY": "k", "IA_SECRET_KEY": "s"}
        self.lines = []

    def log(self, message):
        self.lines.append(message)

    def fake_remote(self, metadata, posted):
        """Stand in for the metadata API: a GET, then maybe a POST."""
        import io

        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *a): return False

        def urlopen(request, timeout=None):
            if isinstance(request, str):
                return Response(json.dumps({"metadata": metadata}).encode())
            posted.append(request.data)
            return Response(json.dumps({"success": True}).encode())
        return urlopen

    def test_a_stale_description_is_detected_and_patched(self):
        posted = []
        remote = self.fake_remote({
            "identifier": "pentimento-core-v1",
            "title": "Pentimento Core",
            "description": "5,429 covers (54.3%) require attribution.",
            "collection": "opensource_media",
            "subject": ["steganalysis", "dataset"],
        }, posted)
        original = upload_tier.urllib.request.urlopen
        upload_tier.urllib.request.urlopen = remote
        try:
            upload_tier.reconcile_ia_metadata(
                self.packed, "pentimento-core-v1", self.creds, self.log)
        finally:
            upload_tier.urllib.request.urlopen = original

        self.assertEqual(len(posted), 1, "the patch was never sent")
        body = posted[0].decode()
        self.assertIn("5%2C453", body.replace("%2C", "%2C"))
        joined = "\n".join(self.lines)
        self.assertIn("description", joined)
        # The fields that already agree must not be rewritten: a patch that
        # touches everything clobbers fields the Archive maintains itself.
        self.assertNotIn('"path": "/title"', body)
        # COLLECTION IS REPORTED BUT NOT PATCHED. Including it returns
        # HTTP 400 "Not authorized to add collection(s)" and takes the whole
        # patch down with it, so a correct description fails to land because
        # of a field nobody could have set anyway. Measured against the live
        # API, then again when a live publish crashed on it at byte zero.
        self.assertNotIn("%2Fcollection", body)
        self.assertNotIn('"path": "/collection"', body)
        self.assertIn("only lets its own staff set it", joined)

    def test_an_item_that_already_agrees_is_not_written_to(self):
        posted = []
        remote = self.fake_remote({
            "identifier": "pentimento-core-v1",
            "title": "Pentimento Core",
            "description": "5,453 covers (54.5%) require attribution.",
            "collection": "datasets",
            "subject": ["steganalysis", "dataset"],
        }, posted)
        original = upload_tier.urllib.request.urlopen
        upload_tier.urllib.request.urlopen = remote
        try:
            upload_tier.reconcile_ia_metadata(
                self.packed, "pentimento-core-v1", self.creds, self.log)
        finally:
            upload_tier.urllib.request.urlopen = original
        self.assertEqual(posted, [], "a no-op patch is a write nobody can audit")
        self.assertIn("already matches", "\n".join(self.lines))

    def test_a_dry_run_reports_the_drift_and_sends_nothing(self):
        posted = []
        remote = self.fake_remote({
            "identifier": "pentimento-core-v1",
            "description": "5,429 covers (54.3%) require attribution.",
        }, posted)
        original = upload_tier.urllib.request.urlopen
        upload_tier.urllib.request.urlopen = remote
        try:
            upload_tier.reconcile_ia_metadata(
                self.packed, "pentimento-core-v1", self.creds, self.log,
                apply=False)
        finally:
            upload_tier.urllib.request.urlopen = original
        self.assertEqual(posted, [], "a dry run must not write")
        self.assertIn("WOULD be updated", "\n".join(self.lines))

    def test_an_item_that_does_not_exist_yet_is_left_to_the_upload(self):
        posted = []
        remote = self.fake_remote({}, posted)
        original = upload_tier.urllib.request.urlopen
        upload_tier.urllib.request.urlopen = remote
        try:
            upload_tier.reconcile_ia_metadata(
                self.packed, "pentimento-core-v1", self.creds, self.log)
        finally:
            upload_tier.urllib.request.urlopen = original
        self.assertEqual(posted, [])
        self.assertIn("does not exist yet", "\n".join(self.lines))


class HuggingFaceRepository(unittest.TestCase):
    """Nothing created the repository, and one day it was not there.

    The 2026-09-23 release deleted a stale private repository and the live run
    would then have failed on its first file, after the whole verify pass, with
    a bare 404 from `preupload`.
    """

    def setUp(self):
        self.creds = {"HF_TOKEN": "t"}
        self.lines = []

    def log(self, message):
        self.lines.append(message)

    def fake_remote(self, exists, posted):
        import io

        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *a): return False

        def urlopen(request, timeout=None):
            url = request if isinstance(request, str) else request.full_url
            if url.endswith("/repos/create"):
                posted.append(json.loads(request.data))
                return Response(b'{"url": "x"}')
            if exists:
                return Response(b'{"id": "the-malware-files/pentimento-core"}')
            raise upload_tier.urllib.error.HTTPError(
                url, 404, "Not Found", {}, io.BytesIO(b'{"error":"Repo not found"}'))
        return urlopen

    def run_with(self, remote, **kwargs):
        original = upload_tier.urllib.request.urlopen
        upload_tier.urllib.request.urlopen = remote
        try:
            upload_tier.ensure_huggingface_repo(
                "the-malware-files/pentimento-core", self.creds, self.log,
                **kwargs)
        finally:
            upload_tier.urllib.request.urlopen = original

    def test_a_missing_repository_is_created_public(self):
        posted = []
        self.run_with(self.fake_remote(False, posted))
        self.assertEqual(len(posted), 1)
        self.assertEqual(posted[0]["type"], "dataset")
        self.assertEqual(posted[0]["organization"], "the-malware-files")
        self.assertEqual(posted[0]["name"], "pentimento-core")
        self.assertIs(posted[0]["private"], False,
                      "a private repository is 48 GB nobody can reach")

    def test_an_existing_repository_is_left_alone(self):
        posted = []
        self.run_with(self.fake_remote(True, posted))
        self.assertEqual(posted, [], "creating over an existing repository")
        self.assertIn("exists", "\n".join(self.lines))

    def test_a_dry_run_reports_the_gap_without_creating_anything(self):
        posted = []
        self.run_with(self.fake_remote(False, posted), apply=False)
        self.assertEqual(posted, [])
        self.assertIn("DOES NOT EXIST", "\n".join(self.lines))

    def test_an_error_that_is_not_a_missing_repository_is_raised(self):
        """A 401 must not be read as "absent" and answered by a create."""
        import io

        def remote(request, timeout=None):
            url = request if isinstance(request, str) else request.full_url
            raise upload_tier.urllib.error.HTTPError(
                url, 401, "Unauthorized", {}, io.BytesIO(b"bad token"))

        with self.assertRaises(upload_tier.UploadError):
            self.run_with(remote)

    def test_a_401_whose_body_mentions_404_is_still_not_a_missing_repo(self):
        """This was matched on the message text, which reads the server's body.

        A refusal that happens to quote "404" would have been taken for an
        absent repository and answered by CREATING one under that name.
        """
        import io

        def remote(request, timeout=None):
            url = request if isinstance(request, str) else request.full_url
            raise upload_tier.urllib.error.HTTPError(
                url, 401, "Unauthorized", {},
                io.BytesIO(b'{"error":"token lacks scope; see error 404 docs"}'))

        with self.assertRaises(upload_tier.UploadError) as caught:
            self.run_with(remote)
        self.assertEqual(caught.exception.status, 401)

    def test_a_repository_created_by_the_other_part_is_not_a_failure(self):
        """Both parts go to one repository, so this runs twice per release."""
        import io

        def remote(request, timeout=None):
            url = request if isinstance(request, str) else request.full_url
            if url.endswith("/repos/create"):
                raise upload_tier.urllib.error.HTTPError(
                    url, 409, "Conflict", {}, io.BytesIO(b"already created"))
            raise upload_tier.urllib.error.HTTPError(
                url, 404, "Not Found", {}, io.BytesIO(b'{"error":"Repo not found"}'))

        self.run_with(remote)
        self.assertIn("which is fine", "\n".join(self.lines))


if __name__ == "__main__":
    unittest.main()


class PartsThatShareAnItem(unittest.TestCase):
    """`core-arms` goes into the item `core` created, and ships no metadata.

    That was fatal until 2026-09-23: the live release finished its 3.3 GB
    covers step and died on the first line of the 45 GB arms step, because
    `reconcile_ia_metadata` raised on a file only the covers part carries.
    """

    def setUp(self):
        self.packed = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.packed)
        self.creds = {"IA_ACCESS_KEY": "k", "IA_SECRET_KEY": "s"}
        self.lines = []

    def log(self, message):
        self.lines.append(message)

    def fake_remote(self, exists, posted):
        import io

        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *a): return False

        def urlopen(request, timeout=None):
            if isinstance(request, str):
                body = {"metadata": {"identifier": "i"}} if exists else {}
                return Response(json.dumps(body).encode())
            posted.append(request.data)
            return Response(b'{"success": true}')
        return urlopen

    def run_with(self, remote):
        original = upload_tier.urllib.request.urlopen
        upload_tier.urllib.request.urlopen = remote
        try:
            upload_tier.reconcile_ia_metadata(
                self.packed, "pentimento-core-v1", self.creds, self.log)
        finally:
            upload_tier.urllib.request.urlopen = original

    def test_a_part_with_no_metadata_joins_an_item_that_exists(self):
        posted = []
        self.run_with(self.fake_remote(True, posted))
        self.assertEqual(posted, [])
        self.assertIn("already exists", "\n".join(self.lines))

    def test_a_part_with_no_metadata_refuses_to_create_a_bare_item(self):
        """Otherwise the item goes public with no title and no licence."""
        posted = []
        with self.assertRaises(upload_tier.UploadError) as caught:
            self.run_with(self.fake_remote(False, posted))
        self.assertIn("does not exist yet", str(caught.exception))
        self.assertEqual(posted, [])

    def test_the_per_file_headers_still_ask_for_the_bucket(self):
        """Without `x-amz-auto-make-bucket` the PUT 404s, metadata or not."""
        headers = upload_tier._ia_headers(self.packed, required=False)
        self.assertEqual(headers, {"x-amz-auto-make-bucket": "1"})

    def test_a_part_that_should_carry_metadata_still_fails_loud(self):
        with self.assertRaises(upload_tier.UploadError):
            upload_tier._ia_headers(self.packed)


class HuggingFaceRepoGuards(unittest.TestCase):
    """The panel found three holes in one function, from three lenses.

    A private repository that already exists takes 48 GB to an address no
    reader can reach; a part with no card creates one with no licence and no
    credit list; and `--item` was a free string with nothing tying it to what
    the shipped card tells readers to load.
    """

    def setUp(self):
        self.packed = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.packed)
        self.creds = {"HF_TOKEN": "t"}
        self.lines = []

    def log(self, message):
        self.lines.append(message)

    def card(self, repo="the-malware-files/pentimento-core"):
        (self.packed / "README.md").write_text(
            f'covers = load_dataset("{repo}", "covers", split="full")\n',
            encoding="utf-8")

    def remote(self, *, exists=True, private=False, posted=None):
        import io

        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *a): return False

        def urlopen(request, timeout=None):
            url = request if isinstance(request, str) else request.full_url
            if url.endswith("/repos/create"):
                (posted if posted is not None else []).append(request.data)
                return Response(b'{"url":"x"}')
            if exists:
                return Response(json.dumps({"id": "x", "private": private}).encode())
            raise upload_tier.urllib.error.HTTPError(
                url, 404, "Not Found", {}, io.BytesIO(b'{"error":"not found"}'))
        return urlopen

    def run_with(self, remote, **kwargs):
        original = upload_tier.urllib.request.urlopen
        upload_tier.urllib.request.urlopen = remote
        try:
            upload_tier.ensure_huggingface_repo(
                "the-malware-files/pentimento-core", self.creds, self.log,
                **kwargs)
        finally:
            upload_tier.urllib.request.urlopen = original

    def test_an_existing_private_repository_is_refused(self):
        """The 2026-09-19 failure took the EXISTS branch, not the create one."""
        with self.assertRaises(upload_tier.UploadError) as caught:
            self.run_with(self.remote(exists=True, private=True))
        self.assertIn("PRIVATE", str(caught.exception))

    def test_an_existing_public_repository_is_accepted(self):
        self.run_with(self.remote(exists=True, private=False))
        self.assertIn("exists and is public", "\n".join(self.lines))

    def test_a_part_with_no_card_may_not_create_the_repository(self):
        """Otherwise 45 GB lands with no licence and no credit list."""
        posted = []
        with self.assertRaises(upload_tier.UploadError) as caught:
            self.run_with(self.remote(exists=False, posted=posted),
                          may_create=False)
        self.assertIn("no README.md", str(caught.exception))
        self.assertEqual(posted, [], "a bare public repository was created")

    def test_the_part_that_carries_the_card_may_create_it(self):
        posted = []
        self.run_with(self.remote(exists=False, posted=posted), may_create=True)
        self.assertEqual(len(posted), 1)
        self.assertIs(json.loads(posted[0])["private"], False)

    def test_the_declared_repository_comes_from_the_shipped_card(self):
        self.card("the-malware-files/pentimento-lite")
        self.assertEqual(upload_tier.huggingface_repo_declared(self.packed),
                         "the-malware-files/pentimento-lite")

    def test_a_part_with_no_card_declares_nothing(self):
        self.assertIsNone(upload_tier.huggingface_repo_declared(self.packed))


class ApiBodiesThatAreNotJson(unittest.TestCase):
    """A 200 with an unparseable body killed a 45 GB upload 98 files in.

    The request had succeeded. The object was stored. Only the parse of a
    body nothing reads failed, and it took the whole run with it.
    """

    def call(self, payload, **kwargs):
        import io

        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *a): return False

        original = upload_tier.urllib.request.urlopen
        upload_tier.urllib.request.urlopen = lambda r, timeout=None: Response(payload)
        try:
            return upload_tier._hf_api("https://x/y", "t", **kwargs)
        finally:
            upload_tier.urllib.request.urlopen = original

    def test_a_body_that_is_not_json_is_tolerated_where_it_is_ignored(self):
        self.assertEqual(self.call(b"OK\n", expect_json=False), {})

    def test_whitespace_is_not_a_body(self):
        """`if raw` was true for a newline, so it went to the JSON parser."""
        self.assertEqual(self.call(b"\n"), {})
        self.assertEqual(self.call(b"   "), {})

    def test_a_caller_that_needs_json_still_fails_loud(self):
        with self.assertRaises(upload_tier.UploadError) as caught:
            self.call(b"<html>gateway</html>")
        self.assertIn("not JSON", str(caught.exception))

    def test_real_json_still_parses(self):
        self.assertEqual(self.call(b'{"private": true}'), {"private": True})


class Retries(unittest.TestCase):
    """A day-long upload meets a connection reset. That is the weather.

    Two runs died to one each: a non-JSON body on 2026-09-24 at 02:22, and
    `[Errno 104] Connection reset by peer` at 11:21, 127 files into 770.
    Nothing in the uploader retried anything.
    """

    def setUp(self):
        self.lines = []
        self.slept = []
        self.real_sleep = upload_tier.time.sleep
        upload_tier.time.sleep = self.slept.append
        self.addCleanup(setattr, upload_tier.time, "sleep", self.real_sleep)

    def log(self, message):
        self.lines.append(message)

    def test_a_connection_reset_is_retried_and_succeeds(self):
        calls = []

        def attempt():
            calls.append(1)
            if len(calls) < 3:
                raise ConnectionResetError(104, "Connection reset by peer")
            return "landed"

        self.assertEqual(upload_tier.with_retries(attempt, "shard", self.log),
                         "landed")
        self.assertEqual(len(calls), 3)
        self.assertEqual(len(self.slept), 2)

    def test_the_backoff_grows(self):
        def attempt():
            raise ConnectionResetError(104, "reset")

        with self.assertRaises(ConnectionResetError):
            upload_tier.with_retries(attempt, "shard", self.log, retries=3)
        self.assertEqual(self.slept, [4.0, 8.0, 16.0])

    def test_an_http_refusal_is_not_retried(self):
        """The server answered. Repeating a 401 says no more slowly."""
        calls = []

        def attempt():
            calls.append(1)
            raise upload_tier.UploadError("nope", status=401)

        with self.assertRaises(upload_tier.UploadError):
            upload_tier.with_retries(attempt, "shard", self.log)
        self.assertEqual(len(calls), 1, "an HTTP refusal was retried")
        self.assertEqual(self.slept, [])

    def test_a_connection_level_upload_error_is_retried(self):
        """`_hf_api` wraps URLError as UploadError with no status."""
        calls = []

        def attempt():
            calls.append(1)
            if len(calls) < 2:
                raise upload_tier.UploadError("could not reach it, reset")
            return "landed"

        self.assertEqual(upload_tier.with_retries(attempt, "shard", self.log),
                         "landed")
        self.assertEqual(len(calls), 2)

    def test_it_gives_up_rather_than_looping_for_ever(self):
        def attempt():
            raise ConnectionResetError(104, "reset")

        with self.assertRaises(ConnectionResetError):
            upload_tier.with_retries(attempt, "shard", self.log, retries=2)
