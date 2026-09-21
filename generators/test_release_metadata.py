#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the derived release paperwork.

    python3 -m unittest discover -s generators -p 'test_*.py'

These files are the ones a downloader actually reads, and every one of them is
derived rather than typed precisely so a human cannot mistype a licence into
it. That only holds if the derivation is right, so the assertions here are
about the derivation: that a credit line reaches the list when the manifest
says it must, that a checksum file is byte-identical between runs, and that the
loader shipped to strangers can read a shard and refuses a truncated one.
"""
from __future__ import annotations

import io
import json
import pathlib
import subprocess
import sys
import tarfile
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import release_metadata  # noqa: E402


def row(file: str, *, required: bool = True, licence: str = "CC BY 4.0", **kw) -> dict:
    base = {
        "file": file,
        "licence": licence,
        "licence_url": "https://creativecommons.org/licenses/by/4.0/",
        "artist": "A Photographer",
        "title": f"File:{file}",
        "descriptionurl": f"https://commons.wikimedia.org/wiki/File:{file}",
        "attribution": f'"{file}", by A Photographer, {licence}, via Wikimedia Commons',
        "attribution_required": required,
    }
    base.update(kw)
    return base


class ChecksumTests(unittest.TestCase):
    COVER_INDEX = {
        "shards": [
            {"shard": "pentimento-core-00001.tar", "sha256": "b" * 64},
            {"shard": "pentimento-core-00000.tar", "sha256": "a" * 64},
        ]
    }
    ARMS_INDEX = {
        "arms": [
            {"arm": "wow-0200", "shards": [
                {"shard": "pentimento-core-wow-0200-00000.tar", "sha256": "c" * 64}]},
            {"arm": "clean-grey", "shards": [
                {"shard": "pentimento-core-clean-grey-00000.tar", "sha256": "d" * 64}]},
        ]
    }

    def test_the_format_is_the_one_sha256sum_c_expects(self):
        body = release_metadata.sha256sums(self.COVER_INDEX, {})
        self.assertIn(f"{'a' * 64}  pentimento-core-00000.tar", body)
        # Two spaces, not one and not a tab. `sha256sum -c` is strict about it.
        self.assertRegex(body.splitlines()[0], r"^[0-9a-f]{64}  \S")

    def test_arms_indexes_are_read_too(self):
        # The arms index nests shards under arms rather than listing them flat,
        # and writing only the cover form would leave 769 files unverifiable.
        body = release_metadata.sha256sums(self.ARMS_INDEX, {})
        self.assertIn("pentimento-core-wow-0200-00000.tar", body)
        self.assertIn("pentimento-core-clean-grey-00000.tar", body)

    def test_output_is_sorted_so_two_runs_agree(self):
        body = release_metadata.sha256sums(self.COVER_INDEX, {})
        names = [line.split("  ", 1)[1] for line in body.strip().splitlines()]
        self.assertEqual(names, sorted(names))

    def test_the_small_files_are_listed_alongside_the_shards(self):
        body = release_metadata.sha256sums(self.COVER_INDEX, {"README.md": "e" * 64})
        self.assertIn(f"{'e' * 64}  README.md", body)

    def test_an_empty_index_is_not_a_crash(self):
        self.assertEqual(release_metadata.sha256sums({}, {}), "\n")


class AttributionTests(unittest.TestCase):
    def test_only_the_rows_that_require_it_are_listed(self):
        rows = [row("00000.png"), row("00001.png", required=False, licence="CC0")]
        body = release_metadata.attribution(rows)
        self.assertIn("00000.png", body)
        self.assertNotIn("00001.png", body)
        self.assertIn("**1 of 2 covers require attribution.**", body)

    def test_the_credit_line_is_reproduced_not_rebuilt(self):
        # Rebuilding it from the parts is how a mirror ends up asserting a
        # licence the source never granted. The manifest's string is the one
        # that ships.
        rows = [row("00000.png", attribution="whatever the manifest said")]
        self.assertIn("whatever the manifest said",
                      release_metadata.attribution(rows))

    def test_licences_are_counted_by_kind(self):
        rows = [row("00000.png"), row("00001.png"),
                row("00002.png", licence="CC BY 2.0")]
        body = release_metadata.attribution(rows)
        self.assertIn("| CC BY 4.0 | 2 |", body)
        self.assertIn("| CC BY 2.0 | 1 |", body)

    def test_no_attribution_required_still_produces_a_valid_page(self):
        body = release_metadata.attribution([row("00000.png", required=False)])
        self.assertIn("**0 of 1 covers require attribution.**", body)

    def test_the_csv_carries_the_join_keys(self):
        body = release_metadata.attribution_csv([row("00000.png")])
        header, first = body.splitlines()[:2]
        self.assertEqual(header.split(",")[0], "file")
        self.assertIn("00000.png", first)
        self.assertIn("https://creativecommons.org/licenses/by/4.0/", first)

    def test_a_comma_in_a_credit_line_does_not_break_the_csv(self):
        import csv
        rows = [row("00000.png", artist="Smith, John", attribution='"x", by Smith, John')]
        parsed = list(csv.reader(io.StringIO(
            release_metadata.attribution_csv(rows))))
        self.assertEqual(len(parsed), 2)
        self.assertEqual(parsed[1][3], "Smith, John")


class LoaderTests(unittest.TestCase):
    """The loader ships to strangers, so it is executed here rather than read."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = pathlib.Path(self.tmp.name)
        self.script = self.dir / "load_pentimento.py"
        self.script.write_text(release_metadata.loader())

    def shard(self, name: str, *, samples: int = 3, truncate: bool = False) -> pathlib.Path:
        path = self.dir / name
        with tarfile.open(path, "w") as tar:
            for i in range(samples):
                key = f"{i:05d}"
                for suffix, payload in (
                    ("png", b"\x89PNG\r\n\x1a\n" + bytes(64)),
                    ("json", json.dumps({"licence": "CC0", "source_png": f"{key}.png"}).encode()),
                ):
                    if truncate and i == samples - 1 and suffix == "json":
                        continue  # a sample missing its record
                    info = tarfile.TarInfo(f"{key}.{suffix}")
                    info.size = len(payload)
                    tar.addfile(info, io.BytesIO(payload))
        return path

    def run_script(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(self.script), *args],
            capture_output=True, text=True, timeout=60)

    def test_the_emitted_file_is_valid_python(self):
        compile(release_metadata.loader(), "load_pentimento.py", "exec")

    def test_it_reads_a_shard_with_nothing_installed(self):
        result = self.run_script(str(self.shard("ok.tar")))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("3 samples", result.stdout)
        self.assertIn("CC0", result.stdout)

    def test_a_truncated_shard_is_refused_rather_than_silently_short(self):
        # The failure this prevents: a download that lost its tail reads as a
        # smaller corpus and every number computed on it is quietly wrong.
        result = self.run_script(str(self.shard("bad.tar", truncate=True)))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("SHA256SUMS", result.stderr)

    def test_it_explains_itself_when_called_wrong(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 2)
        self.assertIn("Pentimento", result.stderr)


class EndToEndTests(unittest.TestCase):
    """main() over a directory shaped like the real release."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.rel = pathlib.Path(self.tmp.name) / "core"
        self.arms = pathlib.Path(self.tmp.name) / "core-arms"
        self.rel.mkdir()
        self.arms.mkdir()
        (self.rel / "pentimento-core-index.json").write_text(json.dumps({
            "tier": "Core", "samples": 2,
            "shards": [{"shard": "pentimento-core-00000.tar", "samples": 2,
                        "bytes": 1024, "sha256": "a" * 64}],
        }))
        (self.rel / "licence-summary.json").write_text(json.dumps({
            "total": 2, "licences": {"CC BY 4.0": 1, "CC0": 1},
            "licence_urls": {"CC BY 4.0": "https://creativecommons.org/licenses/by/4.0/"},
            "attribution_required": 1, "attribution_required_pct": 50.0,
            "capture_class": {"camera": 2},
        }))
        self.arms_index = self.arms / "pentimento-core-arms-index.json"
        self.arms_index.write_text(json.dumps({
            "tier": "Core", "part": "arms", "total_samples": 2, "total_bytes": 2048,
            "arms": [{"arm": "wow-0200", "samples": 2, "rows_in_manifest": 2,
                      "missing": [], "digest_mismatches": [], "mispaired": [],
                      "shards": [{"shard": "pentimento-core-wow-0200-00000.tar",
                                  "samples": 2, "bytes": 2048, "sha256": "c" * 64}]}],
        }))
        self.manifest = pathlib.Path(self.tmp.name) / "manifest.jsonl"
        self.manifest.write_text(
            json.dumps(row("00000.png")) + "\n"
            + json.dumps(row("00001.png", required=False, licence="CC0")) + "\n")

    def run_main(self, *extra: str) -> int:
        return release_metadata.main([
            "--release", str(self.rel),
            "--arms-index", str(self.arms_index),
            *extra,
        ])

    def test_every_promised_file_appears(self):
        self.assertEqual(self.run_main("--covers-manifest", str(self.manifest)), 0)
        for name in ("README.md", "CITATION.cff", "croissant.json", "DATASHEET.md",
                     "SPLITS.md", "load_pentimento.py", "ATTRIBUTION.md",
                     "ATTRIBUTION.csv", "SHA256SUMS"):
            self.assertTrue((self.rel / name).exists(), f"{name} was not written")
        self.assertTrue((self.arms / "SHA256SUMS").exists())

    def test_the_checksum_file_covers_the_paperwork_written_beside_it(self):
        self.run_main("--covers-manifest", str(self.manifest))
        body = (self.rel / "SHA256SUMS").read_text()
        self.assertIn("README.md", body)
        self.assertIn("pentimento-core-00000.tar", body)
        # It cannot contain its own digest, and claiming to would be worse
        # than the gap.
        self.assertNotIn("  SHA256SUMS", body)

    def test_running_twice_produces_identical_checksums(self):
        self.run_main("--covers-manifest", str(self.manifest))
        first = (self.rel / "SHA256SUMS").read_text()
        self.run_main("--covers-manifest", str(self.manifest))
        self.assertEqual(first, (self.rel / "SHA256SUMS").read_text())

    def test_without_a_manifest_it_says_so_rather_than_writing_an_empty_list(self):
        self.assertEqual(self.run_main(), 0)
        self.assertFalse((self.rel / "ATTRIBUTION.md").exists())

    def test_a_missing_manifest_is_refused(self):
        self.assertEqual(self.run_main("--covers-manifest", "/nonexistent.jsonl"), 1)

    def test_a_missing_release_directory_is_refused(self):
        self.assertEqual(release_metadata.main(
            ["--release", "/nonexistent/core"]), 1)


if __name__ == "__main__":
    unittest.main()
