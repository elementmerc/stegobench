#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
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

import fnmatch
import io
import json
import pathlib
import re
import subprocess
import sys
import tarfile
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import release_metadata  # noqa: E402
import release_metadata as rm  # noqa: E402


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


class CitationTests(unittest.TestCase):
    """The CFF record, which is prose a stranger reads outside this project.

    Zotero, Mendeley and EndNote drop `abstract` into a bibliography entry
    verbatim. The first version opened with the BOSSbase caution, so a reader
    who had never heard of this corpus met a warning about a different one
    before a word saying what they were holding.
    """

    def abstract(self, tier: str = "Core") -> str:
        import yaml
        return yaml.safe_load(
            release_metadata.citation("1.0.0", "2026-09-21", tier))["abstract"]

    def test_the_abstract_opens_by_describing_this_corpus(self):
        first = self.abstract().split(". ")[0]
        # A bibliography entry shows the head of the abstract and little else,
        # so the opening sentence has to say what the thing is.
        self.assertTrue(first.startswith("A steganalysis corpus of"), first)
        self.assertNotIn("BOSSbase", first)

    def test_the_bossbase_caution_is_still_in_the_abstract(self):
        """Moved, not dropped.

        Numbers measured here and numbers measured on BOSSbase cannot go in
        the same table, and the citation record is the one artefact that
        travels into a reference manager with no README beside it.
        """
        body = self.abstract()
        self.assertIn(release_metadata.NOT_COMPARABLE, body)
        self.assertGreater(body.index(release_metadata.NOT_COMPARABLE), 0)

    def test_the_covers_are_still_declared_third_party(self):
        # `creator: Daniel Iwugo` beside `license: CC-BY-4.0` reads as a claim
        # of authorship over 10,000 other people's photographs without it.
        self.assertIn("third-party works from Wikimedia Commons", self.abstract())


class CroissantTests(unittest.TestCase):
    """The record Kaggle and HuggingFace index.

    The first version of this file shipped an abbreviated `@context`, which
    looks harmless and is not: the reference validator resolves every Croissant
    term through that map, so the record failed to expand and was rejected
    before a single field in it was read. Nothing in our own test suite noticed,
    because nothing here had ever run the validator.
    """

    COVER_INDEX = {
        "tier": "Core", "samples": 10000,
        "shards": [{"shard": "pentimento-core-00000.tar", "samples": 1000,
                    "bytes": 1, "sha256": "a" * 64}],
    }
    LICENCES = {"total": 10000, "licences": {"CC0": 2597},
                "attribution_required": 5429, "attribution_required_pct": 54.3}
    ARMS = {"arms": [
        {"arm": "wow-0200", "samples": 10000,
         "shards": [{"shard": "w", "samples": 500, "bytes": 1, "sha256": "b" * 64}]},
        {"arm": "clean-grey", "samples": 10000,
         "shards": [{"shard": "c", "samples": 500, "bytes": 1, "sha256": "d" * 64}]},
    ]}

    def record(self, tier: str = "Core", samples: int = 10000) -> dict:
        index = dict(self.COVER_INDEX, tier=tier, samples=samples)
        return release_metadata.croissant(
            index, self.LICENCES, self.ARMS, "1.0.0", "2026-09-21")

    def test_the_context_carries_the_whole_croissant_vocabulary(self):
        context = self.record()["@context"]
        # Every term the spec resolves through the context. A record missing
        # any of these silently fails to expand.
        for term in ("recordSet", "field", "source", "extract", "fileSet",
                     "fileObject", "dataType", "references", "transform",
                     "includes", "regex", "fileProperty", "jsonPath",
                     "equivalentProperty", "@language", "dct", "rai"):
            self.assertIn(term, context, f"context is missing {term}")

    def test_the_cover_glob_does_not_also_match_the_arm_shards(self):
        """`pentimento-core-*.tar` matches `pentimento-core-wow-0200-00000.tar`.

        Left that way, the cover file set swallows all 769 arm shards and a
        loader reading "covers" gets stego images labelled as clean.
        """
        sets = {d["@id"]: d for d in self.record()["distribution"]}
        covers = sets["cover-shards"]["includes"]
        self.assertNotIn("*-*", covers)
        import fnmatch
        self.assertTrue(fnmatch.fnmatch("pentimento-core-00000.tar", covers))
        self.assertFalse(
            fnmatch.fnmatch("pentimento-core-wow-0200-00000.tar", covers),
            "the cover glob still matches an arm shard")

    def test_every_file_set_is_contained_in_something(self):
        for d in self.record()["distribution"]:
            if d["@type"] == "cr:FileSet":
                self.assertIn("containedIn", d, f"{d['@id']} floats free")

    def test_the_tier_reaches_the_urls_and_the_name(self):
        record = self.record("Nano", 200)
        self.assertEqual(record["name"], "pentimento-nano")
        self.assertIn("pentimento-nano-v1", record["url"])
        sets = {d["@id"]: d for d in record["distribution"]}
        self.assertIn("pentimento-nano-", sets["cover-shards"]["includes"])

    def test_it_validates_against_the_reference_implementation(self):
        """The assertion that would have caught the original defect.

        Skipped rather than vendored when mlcroissant is absent, because the
        validator pulls a large dependency tree and this suite runs on three
        operating systems in CI.
        """
        try:
            import mlcroissant
        except ImportError:
            self.skipTest("mlcroissant is not installed")
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "croissant.json"
            path.write_text(json.dumps(self.record(), indent=2), encoding="utf-8")
            mlcroissant.Dataset(jsonld=str(path))


class LoaderTests(unittest.TestCase):
    """The loader ships to strangers, so it is executed here rather than read."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = pathlib.Path(self.tmp.name)
        self.script = self.dir / "load_pentimento.py"
        self.script.write_text(release_metadata.loader(), encoding="utf-8")

    def shard(self, name: str, *, samples: int = 3, truncate: bool = False) -> pathlib.Path:
        import hashlib

        path = self.dir / name
        with tarfile.open(path, "w") as tar:
            for i in range(samples):
                key = f"{i:05d}"
                image = b"\x89PNG\r\n\x1a\n" + bytes(64)
                # Every real record carries the sha256 of the image beside it,
                # and a fixture without one let `--verify` report a pass over
                # nothing.
                record = {"licence": "CC0", "source_png": f"{key}.png",
                          "sha256": hashlib.sha256(image).hexdigest()}
                for suffix, payload in (
                    ("png", image),
                    ("json", json.dumps(record).encode()),
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

    def unpacked(self, name: str, *, samples: int = 3,
                 corrupt: bool = False) -> pathlib.Path:
        """A shard as Kaggle serves it: a folder of the same members.

        Kaggle extracts archives on upload and offers no way to refuse, so a
        mirror exists where no `.tar` file is present and `SHA256SUMS-covers`
        names ten containers that are not there.
        """
        import hashlib

        path = self.dir / name
        path.mkdir()
        for i in range(samples):
            key = f"{i:05d}"
            image = b"\x89PNG\r\n\x1a\n" + bytes(64) + key.encode()
            (path / f"{key}.png").write_bytes(image)
            digest = hashlib.sha256(image).hexdigest()
            if corrupt and i == 0:
                digest = "0" * 64
            (path / f"{key}.json").write_text(json.dumps(
                {"licence": "CC0", "source_png": f"{key}.png",
                 "sha256": digest}), encoding="utf-8")
        return path

    def test_the_emitted_file_is_valid_python(self):
        compile(release_metadata.loader(), "load_pentimento.py", "exec")

    def test_it_reads_an_unpacked_shard_exactly_like_a_tar(self):
        result = self.run_script(str(self.unpacked("unpacked")))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("3 samples", result.stdout)

    def test_the_two_readers_return_the_same_sequence(self):
        """"Exactly like a tar" is the load-bearing claim, so compare them.

        Asserting only that both exit zero let a real divergence through: the
        directory branch skipped dotfiles and the tar branch did not, so the
        same content read one way and failed the other.
        """
        import hashlib

        folder = self.unpacked("pair")
        tar_path = self.dir / "pair.tar"
        with tarfile.open(tar_path, "w") as tar:
            for member in sorted(folder.iterdir()):
                info = tarfile.TarInfo(member.name)
                payload = member.read_bytes()
                info.size = len(payload)
                tar.addfile(info, io.BytesIO(payload))

        harness = self.dir / "dump.py"
        harness.write_text(
            "import json, sys\n"
            "sys.path.insert(0, %r)\n" % str(self.dir) +
            "from load_pentimento import samples\n"
            "import hashlib\n"
            "print(json.dumps([(k, hashlib.sha256(i).hexdigest(), r)\n"
            "                  for k, i, r in samples(sys.argv[1])]))\n",
            encoding="utf-8")

        def read(path):
            out = subprocess.run([sys.executable, str(harness), str(path)],
                                 capture_output=True, text=True, timeout=60)
            self.assertEqual(out.returncode, 0, out.stderr)
            return json.loads(out.stdout)

        self.assertEqual(read(folder), read(tar_path))

    def test_a_dotfile_is_skipped_in_a_tar_as_well_as_a_folder(self):
        """The card tells a reader to re-pack a folder with `tar cf ... .`

        A `.DS_Store` in that folder goes straight into the tar, so treating
        the two differently breaks the recovery the card recommends.
        """
        folder = self.unpacked("repack")
        (folder / ".DS_Store").write_bytes(b"junk")
        tar_path = self.dir / "repack.tar"
        with tarfile.open(tar_path, "w") as tar:
            for member in sorted(folder.iterdir()):
                info = tarfile.TarInfo(member.name)
                payload = member.read_bytes()
                info.size = len(payload)
                tar.addfile(info, io.BytesIO(payload))
        result = self.run_script(str(tar_path))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("3 samples", result.stdout)

    def test_records_without_checksums_say_so_rather_than_no_samples(self):
        """The two ways of verifying nothing send a reader to different places."""
        path = self.dir / "nosums"
        path.mkdir()
        (path / "00000.png").write_bytes(b"\x89PNG\r\n\x1a\n")
        (path / "00000.json").write_text(json.dumps({"licence": "CC0"}),
                                         encoding="utf-8")
        result = self.run_script("--verify", str(path))
        self.assertEqual(result.returncode, 1)
        self.assertIn("none carried a sha256", result.stderr)
        self.assertNotIn("no samples were found", result.stderr)

    def test_verify_checks_images_against_their_own_records(self):
        """SHA256SUMS names containers, so it cannot check this mirror.

        Each record carries the sha256 of the image beside it, which is a
        finer check than the container's: it names the file that is wrong.
        """
        result = self.run_script("--verify", str(self.unpacked("good")))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("3 image(s) checked, 0 mismatch", result.stdout)

    def test_verify_fails_loud_on_a_corrupted_image(self):
        result = self.run_script("--verify", str(self.unpacked("bad", corrupt=True)))
        self.assertEqual(result.returncode, 1)
        self.assertIn("MISMATCH", result.stderr)
        self.assertIn("1 mismatch", result.stdout)

    def test_verify_works_on_a_tar_too(self):
        """A reader should not have to know which mirror they downloaded."""
        result = self.run_script("--verify", str(self.shard("ok.tar")))
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_several_unpacked_shards_under_one_folder_do_not_collide(self):
        """Every arm restarts its numbering at 00000.

        Keyed on the basename, `wow-0050/00000.png` and `hugo-0400/00000.png`
        are handed out under the same key, so a caller building a dict of
        samples silently keeps half the data.
        """
        parent = self.dir / "both"
        parent.mkdir()
        for arm in ("wow-0050", "hugo-0400"):
            self.unpacked(f"both/{arm}", samples=2)
        result = self.run_script(str(parent))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("4 samples", result.stdout)
        self.assertIn("first sample: hugo-0400/00000", result.stdout,
                      "the key must carry the shard, or two arms share it")

    def test_a_stray_dotfile_does_not_break_the_read(self):
        """A mirror or an operating system can leave one beside the data."""
        path = self.unpacked("withjunk")
        (path / ".DS_Store").write_bytes(b"junk")
        result = self.run_script(str(path))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("3 samples", result.stdout)

    def test_verifying_an_empty_folder_is_not_a_pass(self):
        """"0 checked, 0 mismatches" with a zero exit reads as success."""
        empty = self.dir / "empty"
        empty.mkdir()
        result = self.run_script("--verify", str(empty))
        self.assertEqual(result.returncode, 1)
        self.assertIn("no samples were found", result.stderr)

    def test_an_incomplete_unpacked_sample_is_still_refused(self):
        path = self.unpacked("short")
        next(path.glob("00000.json")).unlink()
        result = self.run_script(str(path))
        self.assertEqual(result.returncode, 1)
        self.assertIn("incomplete sample", result.stderr)

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
        self.assertIn("usage:", result.stderr)
        self.assertIn("shard", result.stderr)

    def test_help_prints_help_rather_than_a_traceback(self):
        # `--help` is the first thing a stranger types at an unfamiliar
        # script. Before there was an argument parser it fell through to
        # `tarfile.open("--help")` and answered with twenty lines ending
        # inside the standard library, which reads as a broken corpus.
        result = self.run_script("--help")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        self.assertNotIn("Traceback", result.stdout)
        self.assertIn("usage:", result.stdout)

    def test_help_makes_verify_discoverable(self):
        # It was documented in one section of the guide and found by luck
        # everywhere else, which is not a discovery route.
        result = self.run_script("--help")
        self.assertIn("--verify", result.stdout)

    def test_help_names_the_split_rule(self):
        # The one thing a reader has to know before training, and the help
        # text is read by people who never open SPLITS.md.
        result = self.run_script("--help")
        self.assertIn("source_png", result.stdout)
        self.assertIn("SPLITS.md", result.stdout)

    def test_the_documented_verify_invocation_still_parses(self):
        # The published guide says `--verify <shard>`, in that order, and an
        # argument parser that took the shard first would break every page
        # already on three mirrors.
        shard = self.shard("ok.tar")
        result = self.run_script("--verify", str(shard))
        self.assertEqual(result.returncode, 0, result.stderr)


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
        }), encoding="utf-8")
        (self.rel / "licence-summary.json").write_text(json.dumps({
            "total": 2, "licences": {"CC BY 4.0": 1, "CC0": 1},
            "licence_urls": {"CC BY 4.0": "https://creativecommons.org/licenses/by/4.0/"},
            "attribution_required": 1, "attribution_required_pct": 50.0,
            "capture_class": {"camera": 2},
        }), encoding="utf-8")
        self.arms_index = self.arms / "pentimento-core-arms-index.json"
        self.arms_index.write_text(json.dumps({
            "tier": "Core", "part": "arms", "total_samples": 2, "total_bytes": 2048,
            "arms": [{"arm": "wow-0200", "samples": 2, "rows_in_manifest": 2,
                      "missing": [], "digest_mismatches": [], "mispaired": [],
                      "shards": [{"shard": "pentimento-core-wow-0200-00000.tar",
                                  "samples": 2, "bytes": 2048, "sha256": "c" * 64}]}],
        }), encoding="utf-8")
        self.manifest = pathlib.Path(self.tmp.name) / "manifest.jsonl"
        self.manifest.write_text(
            json.dumps(row("00000.png")) + "\n"
            + json.dumps(row("00001.png", required=False, licence="CC0")) + "\n", encoding="utf-8")
        self.stamp_summary()

    def stamp_summary(self, digest: str | None = None) -> None:
        """Record which manifest the summary describes, as publish_tier does."""
        p = self.rel / "licence-summary.json"
        d = json.loads(p.read_text(encoding="utf-8"))
        d["source_manifest_sha256"] = digest or rm.digest_of(self.manifest)
        p.write_text(json.dumps(d), encoding="utf-8")

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
                     "ATTRIBUTION.csv", "SHA256SUMS-covers"):
            self.assertTrue((self.rel / name).exists(), f"{name} was not written")
        self.assertTrue((self.arms / "SHA256SUMS-arms").exists())

    def test_the_checksum_file_covers_the_paperwork_written_beside_it(self):
        self.run_main("--covers-manifest", str(self.manifest))
        body = (self.rel / "SHA256SUMS-covers").read_text(encoding="utf-8")
        self.assertIn("README.md", body)
        self.assertIn("pentimento-core-00000.tar", body)
        # It cannot contain its own digest, and claiming to would be worse
        # than the gap.
        self.assertNotIn("  SHA256SUMS-covers", body)

    def test_running_twice_produces_identical_checksums(self):
        self.run_main("--covers-manifest", str(self.manifest))
        first = (self.rel / "SHA256SUMS-covers").read_text(encoding="utf-8")
        self.run_main("--covers-manifest", str(self.manifest))
        self.assertEqual(first, (self.rel / "SHA256SUMS-covers").read_text(encoding="utf-8"))

    def test_without_a_manifest_it_says_so_rather_than_writing_an_empty_list(self):
        self.assertEqual(self.run_main(), 0)
        self.assertFalse((self.rel / "ATTRIBUTION.md").exists())

    def test_a_missing_manifest_is_refused(self):
        self.assertEqual(self.run_main("--covers-manifest", "/nonexistent.jsonl"), 1)

    def test_the_tier_name_reaches_the_reader_facing_files(self):
        """A Nano download must not introduce itself as Core.

        The index is globbed rather than named, so the same run packages every
        tier. If the name did not flow through, Nano would ship a README and a
        citation claiming 10,000 covers while holding 200.
        """
        for path in self.rel.glob("pentimento-*-index.json"):
            path.unlink()
        (self.rel / "pentimento-nano-index.json").write_text(json.dumps({
            "tier": "Nano", "samples": 2,
            "shards": [{"shard": "pentimento-nano-00000.tar", "samples": 2,
                        "bytes": 1024, "sha256": "a" * 64}],
        }), encoding="utf-8")
        self.assertEqual(self.run_main(), 0)
        self.assertIn("# Pentimento Nano", (self.rel / "README.md").read_text(encoding="utf-8"))
        self.assertIn("Pentimento Nano", (self.rel / "CITATION.cff").read_text(encoding="utf-8"))

    def test_a_summary_describing_another_manifest_is_refused(self):
        """The fault this closes, 2026-09-22.

        `publish_tier prepare` writes the summary and this reads it. The chain
        ran the second without the first, so prose generated after a cover
        backfill carried figures computed before it, and the shipped README
        said 5,429 covers require attribution when 5,453 do. Nothing
        downstream could tell: `verify_release` reads the manifest, never the
        sentences derived from it.
        """
        self.stamp_summary(digest="b" * 64)
        self.assertEqual(self.run_main("--covers-manifest", str(self.manifest)), 1)

    def test_an_unstamped_summary_is_refused_rather_than_trusted(self):
        """A summary written before the stamp existed cannot be shown to be
        current, and "cannot be shown" must not read as "is"."""
        p = self.rel / "licence-summary.json"
        d = json.loads(p.read_text(encoding="utf-8"))
        d.pop("source_manifest_sha256", None)
        p.write_text(json.dumps(d), encoding="utf-8")
        self.assertEqual(self.run_main("--covers-manifest", str(self.manifest)), 1)

    def test_a_matching_summary_is_accepted(self):
        """The guard must not refuse the correct case, or it gets removed."""
        self.assertEqual(self.run_main("--covers-manifest", str(self.manifest)), 0)

    def test_a_small_tier_credits_only_its_own_photographers(self):
        """Nano must not ship Core's credit list.

        Naming a photographer whose work is not in the download is a false
        statement about what was used, and it is the kind that only a
        photographer notices.
        """
        rows = [dict(row(f"{i:05d}.png"), tier_order=i) for i in range(4)]
        self.manifest.write_text("".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")
        self.stamp_summary()          # the manifest changed; the summary describes the new one
        for path in self.rel.glob("pentimento-*-index.json"):
            path.unlink()
        (self.rel / "pentimento-nano-index.json").write_text(json.dumps({
            "tier": "Nano", "samples": 2,
            "shards": [{"shard": "pentimento-nano-00000.tar", "samples": 2,
                        "bytes": 1024, "sha256": "a" * 64}],
        }), encoding="utf-8")
        self.assertEqual(self.run_main("--covers-manifest", str(self.manifest)), 0)
        body = (self.rel / "ATTRIBUTION.md").read_text(encoding="utf-8")
        self.assertIn("00000.png", body)
        self.assertIn("00001.png", body)
        self.assertNotIn("00002.png", body)
        self.assertIn("**2 of 2 covers require attribution.**", body)

    def test_an_arms_index_is_not_mistaken_for_the_cover_index(self):
        # Both match pentimento-*-index.json, and the arms one has no "samples"
        # key, so picking it would produce a README claiming zero covers.
        (self.rel / "pentimento-core-arms-index.json").write_text(
            json.dumps({"tier": "Core", "part": "arms", "arms": []}), encoding="utf-8")
        self.assertEqual(self.run_main(), 0)
        self.assertIn("2 permissively licensed", (self.rel / "README.md").read_text(encoding="utf-8"))

    def test_the_split_guide_names_both_fields_that_hold_the_cover(self):
        # The cover shards call it `file` and the arm shards call it
        # `source_png`. A fold function written from this page against one of
        # them raises KeyError on the other, so the page has to say so.
        body = release_metadata.splits()
        self.assertIn("source_png", body)
        self.assertIn('record["file"]', body)

    def test_the_split_guide_warns_that_the_split_field_is_a_second_partition(self):
        # Two split definitions ship in one release and they disagree. A reader
        # who mixes them leaks covers across the boundary, which is the exact
        # failure this file exists to prevent.
        body = release_metadata.splits()
        self.assertIn("split_salt", body)

    def test_the_split_counts_are_derived_from_the_tier_rather_than_restated(self):
        """This test used to assert the literal "8,032" and "1,968".

        Those were the right numbers when they were written and the corpus
        moved underneath them: the manifest holds 8,029 against 1,971. The
        same absolute pair also shipped inside Nano, telling a reader with 200
        covers that their tier held 8,032 training ones. So the assertion was
        pinning the defect in place, which is the most expensive kind of test
        to have.
        """
        rows = ([{"split": "train"}] * 8029) + ([{"split": "test"}] * 1971)
        body = release_metadata.splits(rows)
        self.assertIn("8,029 covers against 1,971", body)
        self.assertNotIn("8,032", body)

        nano = ([{"split": "train"}] * 167) + ([{"split": "test"}] * 33)
        self.assertIn("167 covers against 33", release_metadata.splits(nano))

    def test_the_split_guide_states_no_count_rather_than_a_wrong_one(self):
        """With no rows there is nothing to derive, and the sentence drops the
        figure rather than carrying a stale one. A document missing a number is
        recoverable; one asserting a confident wrong number is not."""
        body = release_metadata.splits(None)
        self.assertIn("`split_salt`", body)
        self.assertNotIn("covers against", body)

    def test_the_two_parts_do_not_write_the_same_filename(self):
        """Every destination is flat.

        The covers and the arms are packed in separate directories and land in
        one namespace at the Archive and on HuggingFace. Two files both called
        SHA256SUMS meant the second replacing the first, leaving a checksum
        file that covers ten shards and claims to cover 769.
        """
        self.run_main("--covers-manifest", str(self.manifest))
        cover_sums = {p.name for p in self.rel.iterdir()
                      if p.name.startswith("SHA256SUMS")}
        arm_sums = {p.name for p in self.arms.iterdir()
                    if p.name.startswith("SHA256SUMS")}
        self.assertTrue(cover_sums)
        self.assertTrue(arm_sums)
        self.assertFalse(cover_sums & arm_sums,
                         f"{cover_sums & arm_sums} would collide on upload")

    def test_a_missing_release_directory_is_refused(self):
        self.assertEqual(release_metadata.main(
            ["--release", "/nonexistent/core"]), 1)


class HuggingFaceConfigs(unittest.TestCase):
    """The `configs:` block is what makes the repository loadable at all."""

    COVERS = {"tier": "Core", "samples": 20, "shards": [
        {"shard": "pentimento-core-00000.tar", "samples": 10, "bytes": 5},
        {"shard": "pentimento-core-00001.tar", "samples": 10, "bytes": 5}]}
    ARMS = {"total_bytes": 9, "arms": [
        {"arm": "wow-0400", "samples": 10, "shards": [
            {"shard": "pentimento-core-wow-0400-00000.tar"}]},
        {"arm": "clean-grey", "samples": 10, "shards": [
            {"shard": "pentimento-core-clean-grey-00000.tar"}]}]}

    def configs(self, covers=None, arms=None):
        import yaml
        block = release_metadata.hf_configs(covers or self.COVERS,
                                            self.ARMS if arms is None else arms)
        return {c["config_name"]: c for c in yaml.safe_load(block)["configs"]}

    def test_every_arm_is_its_own_config_and_covers_is_the_default(self):
        configs = self.configs()
        self.assertEqual(set(configs), {"covers", "wow-0400", "clean-grey"})
        self.assertTrue(configs["covers"]["default"])
        self.assertNotIn("default", configs["wow-0400"])

    def test_the_cover_pattern_does_not_swallow_the_arms(self):
        """`pentimento-core-*.tar` matches every arm shard too.

        The covers and all 39 arms land flat in ONE repository, so a pattern
        that over-matches does not fail: it quietly loads the whole corpus
        under the name of one part.
        """
        pattern = self.configs()["covers"]["data_files"][0]["path"]
        for arm in ("pentimento-core-wow-0400-00000.tar",
                    "pentimento-core-clean-grey-00000.tar"):
            self.assertFalse(fnmatch.fnmatchcase(arm, pattern),
                             f"{pattern} would also load {arm}")
        self.assertTrue(fnmatch.fnmatchcase("pentimento-core-00000.tar", pattern))

    def test_an_arm_pattern_does_not_reach_a_similarly_named_arm(self):
        arms = {"arms": [
            {"arm": "wow-0400", "samples": 1, "shards": [
                {"shard": "pentimento-core-wow-0400-00000.tar"}]},
            {"arm": "wow-04000", "samples": 1, "shards": [
                {"shard": "pentimento-core-wow-04000-00000.tar"}]}]}
        pattern = self.configs(arms=arms)["wow-0400"]["data_files"][0]["path"]
        self.assertFalse(fnmatch.fnmatchcase(
            "pentimento-core-wow-04000-00000.tar", pattern))

    def test_every_shard_lands_in_exactly_one_config(self):
        configs = self.configs()
        shards = ([s["shard"] for s in self.COVERS["shards"]]
                  + [s["shard"] for a in self.ARMS["arms"] for s in a["shards"]])
        for shard in shards:
            owners = [name for name, c in configs.items()
                      if fnmatch.fnmatchcase(shard, c["data_files"][0]["path"])]
            self.assertEqual(len(owners), 1, f"{shard} is claimed by {owners}")

    def test_a_shard_set_that_no_pattern_can_name_is_refused(self):
        """Fail loud rather than emit a config that loads the wrong files."""
        arms = {"arms": [{"arm": "odd", "samples": 1, "shards": [
            {"shard": "pentimento-core-odd-00000.tar"},
            {"shard": "pentimento-core-odd-000001.tar"}]}]}
        with self.assertRaises(release_metadata.ConfigError):
            self.configs(arms=arms)

    def test_a_shard_with_no_numbered_suffix_is_refused_cleanly(self):
        """It used to raise IndexError, which names nothing useful."""
        arms = {"arms": [{"arm": "odd", "samples": 1, "shards": [
            {"shard": "loose.tar"}]}]}
        with self.assertRaises(release_metadata.ConfigError):
            self.configs(arms=arms)

    def test_the_split_is_not_called_train(self):
        """The shards are not laid out along the train and test boundary.

        Calling the only split `train` would hand a reader the cover-leaking
        split that SPLITS.md exists to warn them off.
        """
        for config in self.configs().values():
            self.assertEqual(config["data_files"][0]["split"], "full")

    def test_the_card_frontmatter_stays_valid_yaml_with_the_block(self):
        import yaml
        card = release_metadata.readme(self.COVERS, {"total": 20}, self.ARMS,
                                       "1.0.0")
        front = yaml.safe_load(card.split("---")[1])
        self.assertEqual(front["license"], "cc-by-4.0")
        self.assertEqual(len(front["configs"]), 3)

    def test_the_quick_start_names_an_arm_that_exists(self):
        card = release_metadata.readme(self.COVERS, {"total": 20}, self.ARMS,
                                       "1.0.0")
        quick = card[card.index("## Quick start"):card.index("## What makes")]
        self.assertIn('"wow-0400"', quick)
        self.assertIn(release_metadata.HF_REPO_FORMAT.format(tier="core"), quick)

    def test_a_tier_with_no_arms_still_produces_a_loadable_card(self):
        configs = self.configs(arms={})
        self.assertEqual(set(configs), {"covers"})

    def test_a_card_with_no_arms_does_not_call_the_covers_a_stego_arm(self):
        """The variable name IS the claim a reader reads.

        Falling back to "covers" published `stego = load_dataset(..., "covers")`,
        so somebody believes they are streaming stego, streams covers, and
        nothing errors because every record honestly says cover.
        """
        card = release_metadata.readme(self.COVERS, {"total": 20}, None, "1.0.0")
        quick = card[card.index("## Quick start"):card.index("## What makes")]
        self.assertNotIn("stego = ", quick)
        self.assertIn("covers = ", quick)

    def test_a_card_never_says_the_download_it_saves_you_is_zero(self):
        covers = dict(self.COVERS,
                      shards=[{"shard": "pentimento-core-00000.tar",
                               "samples": 10}])
        with self.assertRaises(release_metadata.ConfigError):
            release_metadata.readme(covers, {"total": 20}, None, "1.0.0")

    def test_the_quick_start_states_the_real_size(self):
        card = release_metadata.readme(self.COVERS, {"total": 20}, self.ARMS,
                                       "1.0.0")
        quick = card[card.index("## Quick start"):card.index("## What makes")]
        self.assertNotIn("rather than 0 GB", quick)


class ChecksumFileScope(unittest.TestCase):
    """`sha256sum -c` is the FIRST thing the README asks a reader to run.

    It listed 23 files while 22 published, so every reader of the Archive copy
    was told two files were missing from a download that was intact.
    """

    def test_the_checksum_file_lists_only_what_publishes(self):
        with tempfile.TemporaryDirectory() as tmp:
            d = pathlib.Path(tmp)
            (d / "pentimento-core-00000.tar").write_bytes(b"")
            (d / "pentimento-core-index.json").write_text(json.dumps({
                "tier": "Core", "samples": 1,
                "shards": [{"shard": "pentimento-core-00000.tar", "samples": 1,
                            "bytes": 5_000_000, "sha256": "a" * 64}],
            }), encoding="utf-8")
            (d / "licence-summary.json").write_text(json.dumps({
                "total": 1, "licences": {"CC0": 1},
                "attribution_required": 0, "attribution_required_pct": 0.0,
            }), encoding="utf-8")
            for name in ("README.md", "LICENCES.md", "load_pentimento.py",
                         "dataset-metadata.json", "ia-metadata.json"):
                (d / name).write_text("x", encoding="utf-8")
            release_metadata.main(["--release", str(d), "--version", "1.0.0"])
            listed = {line.split("  ", 1)[1]
                      for line in (d / "SHA256SUMS-covers").read_text().splitlines()
                      if line.strip()}
        self.assertIn("README.md", listed)
        self.assertIn("pentimento-core-index.json", listed,
                      "the pack index ships, so it needs a checksum too")
        self.assertNotIn("dataset-metadata.json", listed,
                         "Kaggle's control file is not part of the corpus")
        self.assertNotIn("ia-metadata.json", listed,
                         "the Archive's control file is not part of the corpus")

    def test_the_two_copies_of_the_published_set_agree(self):
        """The uploader keeps its own copy because it may not import this one.

        It is bind-mounted alone into a container holding write tokens for
        public archives. The duplication is deliberate; the drift is not.
        """
        uploader = (pathlib.Path(__file__).resolve().parent.parent
                    / "tools" / "release" / "upload_tier.py")
        text = uploader.read_text(encoding="utf-8")
        body = text.split("PACKAGED_EXTRAS = (", 1)[1].split(")", 1)[0]
        theirs = tuple(re.findall(r'"([^"]+)"', body))
        self.assertEqual(theirs, release_metadata.PUBLISHED_EXTRAS)


if __name__ == "__main__":
    unittest.main()
