#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the Homebrew and Scoop generator.

    python3 -m unittest discover -s tools/release -p 'test_*.py'

The generator's whole job is to not produce a plausible file with a wrong
number in it, so most of what is below is the refusals. A formula carrying an
empty `sha256`, or a URL for an archive the release never built, installs
nothing on a stranger's machine and reads like something somebody meant to
finish. The tests that matter are the ones proving those cases stop the run.

Two of them are drift tests rather than unit tests, and they are the reason
this file reads `release.yml`. The URLs here are only correct because they
match the names that workflow gives its assets, and nothing but a test
connects the two: a rename over there would leave a generator that emits a
formula which downloads a 404, and the first person to find out would be a
user. The same argument covers the `cargo-binstall` metadata in
`crates/stegobench-cli/Cargo.toml`, which encodes the identical names.

Ruby is not assumed. Where `ruby` is on PATH the formula is handed to `ruby -c`
and that is a real syntax check; where it is not, the fallback is a block
balance count, which is honestly not a parser and is documented as such rather
than reported as one.
"""
from __future__ import annotations

import json
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import generate_packaging  # noqa: E402
from generate_packaging import (  # noqa: E402
    PLATFORMS,
    PackagingError,
    UnknownPlatformError,
    archive_name,
    generate,
    main,
)

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
RELEASE_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "release.yml"
CLI_MANIFEST = REPO_ROOT / "crates" / "stegobench-cli" / "Cargo.toml"

VERSION = "1.2.3"

#: Distinct per platform so a test can tell which digest landed where. A
#: formula that put the Linux digest under the macOS URL would otherwise pass
#: every assertion below.
DIGESTS = {
    "x86_64-unknown-linux-musl": "1" * 64,
    "aarch64-unknown-linux-musl": "2" * 64,
    "universal-apple-darwin": "3" * 64,
    "x86_64-pc-windows-msvc": "4" * 64,
}


def sums(version: str = VERSION, omit: str | None = None, extra: str = "") -> str:
    """A SHA256SUMS in the shape the release job writes, minus what is omitted.

    `omit` names a target triple to leave out, which is how the missing-digest
    case is built. The SBOM files and a signature are included because the real
    file carries twenty-seven lines and the generator has to ignore the
    twenty-three that are not archives.
    """
    lines = []
    for target, digest in DIGESTS.items():
        if target == omit:
            continue
        suffix = ".zip" if "windows" in target else ".tar.gz"
        lines.append(f"{digest}  stegobench-v{version}-{target}{suffix}")
    lines.append(f"{'a' * 64}  stegobench-cli.cdx.json")
    lines.append(f"{'b' * 64}  stegobench-core.cdx.json")
    lines.append(f"{'c' * 64}  stegobench-v{version}-x86_64-unknown-linux-musl.tar.gz.sig")
    if extra:
        lines.append(extra)
    return "\n".join(lines) + "\n"


class ArchiveNaming(unittest.TestCase):
    """The names this file builds URLs from are the workflow's names."""

    def test_the_workflow_still_names_archives_the_way_this_assumes(self) -> None:
        text = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        self.assertIn(
            'name="stegobench-${TAG}-${{ matrix.target }}"',
            text,
            "release.yml no longer assembles archive names as "
            "stegobench-<tag>-<target>. Every URL in generate_packaging.py and "
            "every pkg-url in crates/stegobench-cli/Cargo.toml is derived from "
            "that line and is now wrong.",
        )
        self.assertIn(
            "TAG: ${{ github.ref_name }}",
            text,
            "the archive name no longer interpolates the tag, so the leading "
            "v that every generated URL carries may no longer be there.",
        )

    def test_every_platform_here_is_a_target_the_workflow_builds(self) -> None:
        text = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        built = set(re.findall(r"^\s+target:\s*(\S+)\s*$", text, flags=re.MULTILINE))
        self.assertTrue(built, "found no build matrix targets in release.yml")
        for key, spec in PLATFORMS.items():
            self.assertIn(
                spec.target,
                built,
                f"platform {key} packages {spec.target}, which release.yml "
                "does not build. The formula would point at an archive that "
                "is never uploaded.",
            )

    def test_every_target_the_workflow_builds_is_packaged(self) -> None:
        text = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        built = set(re.findall(r"^\s+target:\s*(\S+)\s*$", text, flags=re.MULTILINE))
        packaged = {spec.target for spec in PLATFORMS.values()}
        self.assertEqual(
            built - packaged,
            set(),
            "release.yml builds a target that nothing here packages. Add it to "
            "PLATFORMS, or users on it are silently never packaged for.",
        )

    def test_windows_is_the_only_zip(self) -> None:
        for key, spec in PLATFORMS.items():
            expected = ".zip" if "windows" in spec.target else ".tar.gz"
            self.assertEqual(spec.suffix, expected, f"{key} has the wrong suffix")


class BinstallMetadata(unittest.TestCase):
    """`cargo binstall stegobench-cli` has to reach the same asset names."""

    def setUp(self) -> None:
        with CLI_MANIFEST.open("rb") as handle:
            manifest = tomllib.load(handle)
        self.binstall = manifest["package"]["metadata"]["binstall"]

    def resolve(self, template: str, target: str) -> str:
        """Expand the binstall template variables this project actually uses."""
        return (
            template.replace("{ repo }", generate_packaging.REPO_URL)
            .replace("{ version }", VERSION)
            .replace("{ target }", target)
            .replace("{ bin }", "stegobench")
            .replace("{ binary-ext }", ".exe" if "windows" in target else "")
        )

    def test_every_override_url_is_an_asset_the_release_uploads(self) -> None:
        uploaded = {
            archive_name(VERSION, key): key for key in PLATFORMS
        }
        overrides = self.binstall["overrides"]
        self.assertTrue(overrides, "binstall has no per-target overrides")
        for target, override in overrides.items():
            url = self.resolve(override["pkg-url"], target)
            asset = url.rsplit("/", 1)[-1]
            self.assertIn(
                asset,
                uploaded,
                f"the binstall override for {target} downloads {asset}, which "
                "is not one of the archives the release uploads.",
            )
            prefix = f"{generate_packaging.REPO_URL}/releases/download/v{VERSION}/"
            self.assertTrue(
                url.startswith(prefix),
                f"the binstall override for {target} does not point at this "
                f"release's download path: {url}",
            )

    def test_every_bin_dir_matches_the_directory_inside_that_archive(self) -> None:
        for target, override in self.binstall["overrides"].items():
            asset = self.resolve(override["pkg-url"], target).rsplit("/", 1)[-1]
            key = {archive_name(VERSION, k): k for k in PLATFORMS}[asset]
            inside = generate_packaging.extract_dir(VERSION, key)
            binary = PLATFORMS[key].binary
            self.assertEqual(
                self.resolve(override["bin-dir"], target),
                f"{inside}/{binary}",
                f"the binstall override for {target} looks for the binary at a "
                "path the archive does not contain.",
            )

    def test_the_windows_override_says_zip(self) -> None:
        windows = self.binstall["overrides"]["x86_64-pc-windows-msvc"]
        self.assertEqual(windows["pkg-fmt"], "zip")
        self.assertTrue(windows["pkg-url"].endswith(".zip"))

    def test_the_default_is_the_tarball_shape(self) -> None:
        self.assertEqual(self.binstall["pkg-fmt"], "tgz")
        self.assertTrue(self.binstall["pkg-url"].endswith(".tar.gz"))


class Refusals(unittest.TestCase):
    """Every way a wrong file could be produced has to stop the run instead."""

    def test_a_missing_digest_refuses_and_names_the_archive(self) -> None:
        for target in DIGESTS:
            with self.subTest(target=target):
                with self.assertRaises(PackagingError) as raised:
                    generate(VERSION, sums(omit=target))
                message = str(raised.exception)
                self.assertIn(target, message)
                self.assertIn("no entry for", message)

    def test_an_unknown_platform_refuses(self) -> None:
        with self.assertRaises(UnknownPlatformError) as raised:
            generate_packaging.platform("linux-x86-64")
        self.assertIn("linux-x86_64", str(raised.exception))
        with self.assertRaises(UnknownPlatformError):
            archive_name(VERSION, "solaris-sparc")

    def test_an_archive_for_an_unpackaged_target_refuses(self) -> None:
        extra = f"{'e' * 64}  stegobench-v{VERSION}-riscv64gc-unknown-linux-musl.tar.gz"
        with self.assertRaises(PackagingError) as raised:
            generate(VERSION, sums(extra=extra))
        self.assertIn("riscv64gc-unknown-linux-musl", str(raised.exception))

    def test_an_archive_from_another_release_refuses(self) -> None:
        extra = f"{'e' * 64}  stegobench-v9.9.9-x86_64-unknown-linux-musl.tar.gz"
        with self.assertRaises(PackagingError) as raised:
            generate(VERSION, sums(extra=extra))
        self.assertIn("not version", str(raised.exception))

    def test_a_malformed_sums_line_refuses(self) -> None:
        for bad in ("not a checksum at all", "abc  short-digest.tar.gz", "  "):
            with self.subTest(line=bad):
                if bad.strip() == "":
                    continue
                with self.assertRaises(PackagingError):
                    generate(VERSION, sums() + bad + "\n")

    def test_an_empty_sums_file_refuses(self) -> None:
        with self.assertRaises(PackagingError):
            generate(VERSION, "\n\n   \n")

    def test_a_contradictory_duplicate_refuses(self) -> None:
        name = archive_name(VERSION, "linux-x86_64")
        with self.assertRaises(PackagingError) as raised:
            generate(VERSION, sums() + f"{'f' * 64}  {name}\n")
        self.assertIn("twice", str(raised.exception))

    def test_an_identical_duplicate_is_allowed(self) -> None:
        name = archive_name(VERSION, "linux-x86_64")
        digest = DIGESTS["x86_64-unknown-linux-musl"]
        generate(VERSION, sums() + f"{digest}  {name}\n")

    def test_a_version_that_is_not_a_version_refuses(self) -> None:
        for bad in ("v1.2.3", "1.2", "", "../../etc", "latest"):
            with self.subTest(version=bad):
                with self.assertRaises(PackagingError):
                    generate(bad, sums())

    def test_a_prerelease_version_is_accepted(self) -> None:
        files = generate("1.0.0-rc.1", sums(version="1.0.0-rc.1"))
        self.assertIn("v1.0.0-rc.1/", files["homebrew/stegobench.rb"])


class HomebrewFormula(unittest.TestCase):
    def setUp(self) -> None:
        self.formula = generate(VERSION, sums())["homebrew/stegobench.rb"]

    def test_every_platform_url_and_digest_is_present_and_paired(self) -> None:
        pairs = re.findall(
            r'url "([^"]+)"\s*\n\s*sha256 "([0-9a-f]{64})"', self.formula
        )
        self.assertEqual(len(pairs), 3, "expected macOS, Linux x86_64 and Linux arm")
        found = {url.rsplit("/", 1)[-1]: digest for url, digest in pairs}
        for key in ("macos-universal", "linux-x86_64", "linux-aarch64"):
            asset = archive_name(VERSION, key)
            self.assertIn(asset, found)
            self.assertEqual(found[asset], DIGESTS[PLATFORMS[key].target])

    def test_it_carries_no_blank_or_placeholder_field(self) -> None:
        self.assertNotIn('sha256 ""', self.formula)
        self.assertNotIn('url ""', self.formula)
        for marker in ("TODO", "FIXME", "PIN-ME", "PLACEHOLDER", "XXXX"):
            self.assertNotIn(marker, self.formula)

    def test_the_licence_and_version_are_the_project_ones(self) -> None:
        self.assertIn('license "AGPL-3.0-or-later"', self.formula)
        self.assertIn(f'version "{VERSION}"', self.formula)
        self.assertIn("class Stegobench < Formula", self.formula)

    def test_the_windows_archive_is_not_in_the_formula(self) -> None:
        self.assertNotIn("windows", self.formula)

    def test_it_parses_as_ruby_where_ruby_exists(self) -> None:
        ruby = shutil.which("ruby")
        if ruby is None:
            self.skipTest("ruby is not on PATH; the balance check below covers it")
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "stegobench.rb"
            path.write_text(self.formula, encoding="utf-8")
            result = subprocess.run(
                [ruby, "-c", str(path)],
                capture_output=True,
                text=True,
                timeout=60,
            )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_its_blocks_balance(self) -> None:
        """Not a Ruby parser, and does not claim to be one.

        It counts block openers against `end` and checks the file closes on
        one. That catches the failure this generator could plausibly have, a
        conditional emitted without its `end`, and it runs on a machine with
        no Ruby, which is every machine in this project's CI.
        """
        openers = 0
        closers = 0
        for line in self.formula.splitlines():
            stripped = line.strip()
            if stripped.startswith("#"):
                continue
            if re.match(r"^(class|def)\s", stripped) or stripped.endswith(" do"):
                openers += 1
            elif stripped == "end":
                closers += 1
        self.assertEqual(openers, closers, "block openers and `end` do not balance")
        self.assertGreater(openers, 0)
        self.assertEqual(self.formula.strip().splitlines()[-1], "end")


class ScoopManifest(unittest.TestCase):
    def setUp(self) -> None:
        self.text = generate(VERSION, sums())["scoop/stegobench.json"]
        self.manifest = json.loads(self.text)

    def test_it_is_json(self) -> None:
        self.assertIsInstance(self.manifest, dict)
        self.assertTrue(self.text.endswith("\n"))

    def test_the_url_hash_and_extract_dir_agree_with_the_archive(self) -> None:
        arch = self.manifest["architecture"]["64bit"]
        asset = archive_name(VERSION, "windows-x86_64")
        self.assertTrue(arch["url"].endswith(f"/{asset}"))
        self.assertEqual(arch["hash"], DIGESTS["x86_64-pc-windows-msvc"])
        self.assertEqual(arch["extract_dir"], asset[: -len(".zip")])
        self.assertEqual(self.manifest["bin"], "stegobench.exe")

    def test_the_required_scoop_fields_are_all_filled(self) -> None:
        for field in ("version", "description", "homepage", "license", "bin"):
            self.assertTrue(self.manifest[field], f"{field} is empty")
        self.assertEqual(self.manifest["version"], VERSION)
        self.assertEqual(self.manifest["license"], "AGPL-3.0-or-later")

    def test_autoupdate_uses_scoops_own_variable_not_this_release(self) -> None:
        arch = self.manifest["autoupdate"]["architecture"]["64bit"]
        self.assertIn("$version", arch["url"])
        self.assertNotIn(VERSION, arch["url"])
        self.assertIn("SHA256SUMS", self.manifest["autoupdate"]["hash"]["url"])


class CommandLine(unittest.TestCase):
    def test_a_good_run_writes_both_files_and_no_leftovers(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            sums_path = root / "SHA256SUMS"
            sums_path.write_text(sums(), encoding="utf-8")
            out = root / "packaging"
            code = main(
                ["--version", VERSION, "--sums", str(sums_path), "--out", str(out)]
            )
            self.assertEqual(code, 0)
            self.assertTrue((out / "homebrew" / "stegobench.rb").is_file())
            self.assertTrue((out / "scoop" / "stegobench.json").is_file())
            self.assertEqual(list(out.rglob("*.part")), [])

    def test_a_missing_digest_writes_nothing_at_all(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            sums_path = root / "SHA256SUMS"
            sums_path.write_text(
                sums(omit="x86_64-pc-windows-msvc"), encoding="utf-8"
            )
            out = root / "packaging"
            code = main(
                ["--version", VERSION, "--sums", str(sums_path), "--out", str(out)]
            )
            self.assertEqual(code, 1)
            self.assertFalse(out.exists(), "a refused run left files behind")

    def test_an_unreadable_sums_file_refuses(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            code = main(
                [
                    "--version",
                    VERSION,
                    "--sums",
                    str(root / "absent"),
                    "--out",
                    str(root / "packaging"),
                ]
            )
            self.assertEqual(code, 2)


class ShippedExample(unittest.TestCase):
    """`packaging/` holds an emitted example, and it has to still be the shape."""

    def setUp(self) -> None:
        self.example = REPO_ROOT / "packaging" / "example"
        if not self.example.is_dir():
            self.skipTest("packaging/example is not present")

    def test_the_example_regenerates_byte_for_byte(self) -> None:
        version = (self.example / "VERSION").read_text(encoding="utf-8").strip()
        files = generate(
            version, (self.example / "SHA256SUMS").read_text(encoding="utf-8")
        )
        for relative, content in files.items():
            with self.subTest(file=relative):
                on_disk = (self.example / relative).read_text(encoding="utf-8")
                self.assertEqual(
                    on_disk,
                    content,
                    f"packaging/example/{relative} is stale. Regenerate it with "
                    "the command in packaging/README.md.",
                )

    def test_the_example_cannot_be_mistaken_for_a_real_release(self) -> None:
        version = (self.example / "VERSION").read_text(encoding="utf-8").strip()
        self.assertIn(
            "example",
            version,
            "the shipped example carries a version that reads like a real "
            "release, so a reader could copy its digests into a live tap.",
        )


if __name__ == "__main__":
    unittest.main()
